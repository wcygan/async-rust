//! Core Raft node implementation.
//!
//! This module wraps the tikv/raft library to provide a simplified interface for
//! running a Raft consensus node. The main abstraction is [`RaftNode`], which combines
//! the raw Raft state machine with application-level key-value storage.

use anyhow::{Context, Result};
use raft::StateRole;
use raft::prelude::{ConfState, Config, Entry, EntryType, Message, RawNode};
use raft::storage::MemStorage;
use slog::{Logger, o};

use crate::command::CommandPayload;
use crate::store::KvStore;

/// Creates a logger that discards all output.
///
/// The tikv/raft library requires a logger, but we handle logging at the application
/// level instead. This avoids duplicate/verbose Raft internals in our output.
fn silent_logger() -> Logger {
    Logger::root(slog::Discard, o!())
}

/// Records details when a command is applied to the state machine.
///
/// After Raft commits an entry, we apply it to the local key-value store and
/// generate this report. The runtime uses these reports to notify clients
/// waiting for their PUT operations to complete.
///
/// Includes Raft metadata (index, term) for debugging and for matching pending
/// requests to their completion.
pub struct ApplyReport {
    pub node_id: u64,
    pub key: String,
    pub value: String,
    pub index: u64,
    pub term: u64,
}

/// Output from processing a Raft ready state.
///
/// When Raft has work to do (via `poll_ready`), we consolidate two things:
/// - **messages**: Raft messages to send to other nodes
/// - **applied**: Commands that were committed and applied locally
///
/// This bundles both phases of Raft processing (Ready + LightReady) into a single
/// return value to simplify the caller's event loop.
pub struct ReadyBundle {
    pub messages: Vec<Message>,
    pub applied: Vec<ApplyReport>,
}

/// A Raft consensus node with integrated key-value storage.
///
/// This wraps tikv/raft's `RawNode` and combines it with:
/// - **storage**: Raft's replicated log (entries, hard state, snapshots)
/// - **store**: The application state machine (key-value pairs)
///
/// We use `MemStorage` because this is a learning/demo project. A production
/// system would use persistent storage to survive restarts.
///
/// The node exposes a simplified interface: `propose` to submit commands,
/// `step` to process incoming Raft messages, `poll_ready` to advance state.
pub struct RaftNode {
    id: u64,
    raw: RawNode<MemStorage>,
    storage: MemStorage,
    store: KvStore,
}

impl RaftNode {
    /// Creates a new Raft node with the given ID and cluster configuration.
    ///
    /// # Parameters
    /// - `id`: This node's unique identifier
    /// - `voters`: IDs of all voting members (must include this node's ID)
    ///
    /// # Raft timing configuration
    ///
    /// - `election_tick: 10`: Elections triggered after ~1 second of no leader heartbeats
    /// - `heartbeat_tick: 3`: Leader sends heartbeats every ~300ms
    /// - Assumes each `tick()` call happens every 100ms (set by runtime)
    ///
    /// These values balance responsiveness (detect failures quickly) against
    /// stability (avoid spurious elections on temporary network delays).
    pub fn new(id: u64, voters: &[u64]) -> Result<Self> {
        let cfg = Config {
            id,
            election_tick: 10,
            heartbeat_tick: 3,
            max_inflight_msgs: 256,
            ..Default::default()
        };
        let storage = MemStorage::new_with_conf_state(ConfState::from((voters.to_vec(), vec![])));
        let logger = silent_logger();
        let raw = RawNode::new(&cfg, storage.clone(), &logger)
            .with_context(|| format!("failed to construct RawNode {id}"))?;
        Ok(Self {
            id,
            raw,
            storage,
            store: KvStore::new(),
        })
    }

    /// Advances Raft's logical clock by one tick.
    ///
    /// Must be called periodically (typically every 100ms) to drive timeouts.
    /// Election and heartbeat timers are measured in ticks.
    pub fn tick(&mut self) {
        self.raw.tick();
    }

    /// Starts an election to become leader.
    ///
    /// Typically called at startup for one or more nodes to bootstrap the cluster.
    /// Once a leader is elected, subsequent leadership changes happen automatically
    /// via election timeouts.
    pub fn campaign(&mut self) -> Result<()> {
        self.raw.campaign().context("campaign failed")
    }

    /// Returns this node's ID.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Returns this node's current role (Follower, Candidate, or Leader).
    pub fn role(&self) -> StateRole {
        self.raw.raft.state
    }

    /// Returns the current leader's ID, or 0 if no leader is known.
    pub fn leader_id(&self) -> u64 {
        self.raw.raft.leader_id
    }

    /// Proposes a command to be replicated via Raft.
    ///
    /// The command is serialized and appended to the local log. If this node
    /// is the leader, it will replicate to followers. If not the leader, this
    /// will fail with an error (clients should retry on the actual leader).
    ///
    /// Success here only means "added to log", not "committed". Use `poll_ready()`
    /// to discover when entries are committed and applied.
    pub fn propose(&mut self, payload: &CommandPayload) -> Result<()> {
        let data = payload.encode().context("encode command failed")?;
        self.raw.propose(vec![], data).context("propose failed")
    }

    /// Processes a Raft message from another node.
    ///
    /// Messages include RequestVote, AppendEntries, heartbeats, etc.
    /// This drives the Raft state machine forward based on peer communication.
    pub fn step(&mut self, msg: Message) -> Result<()> {
        self.raw.step(msg).context("step failed")
    }

    /// Checks if Raft has work to do, processes it, and returns results.
    ///
    /// This is the core event loop integration point. Returns `None` if nothing
    /// to do, or `Some(ReadyBundle)` with messages to send and commands applied.
    ///
    /// # Processing flow
    ///
    /// Raft uses a two-phase advancement protocol (Ready → LightReady):
    ///
    /// **Phase 1 (Ready)**: Updates requiring durability
    /// 1. Persist hard state (term, vote, commit index) if changed
    /// 2. Append new log entries to storage
    /// 3. Apply any snapshot received
    /// 4. Apply committed entries to the state machine
    /// 5. Collect outbound messages
    ///
    /// **Phase 2 (LightReady)**: Updates after Phase 1 acknowledgment
    /// 1. Update commit index if it advanced
    /// 2. Apply any additional committed entries
    /// 3. Collect additional outbound messages
    ///
    /// # Why two phases?
    ///
    /// This allows overlapping I/O with Raft progress. In a real system with
    /// disk persistence, Phase 1 would write to disk, then Phase 2 would continue
    /// processing while that I/O completes in the background.
    ///
    /// Since we use `MemStorage`, both phases are immediate, but we follow the
    /// protocol for correctness.
    pub fn poll_ready(&mut self) -> Result<Option<ReadyBundle>> {
        if !self.raw.has_ready() {
            return Ok(None);
        }

        let mut ready = self.raw.ready();
        let mut applied = Vec::new();
        let mut outbound = Vec::new();

        // Phase 1: Persist durable state

        if let Some(hard_state) = ready.hs() {
            self.storage.wl().set_hardstate(hard_state.clone());
        }

        if !ready.entries().is_empty() {
            self.storage
                .wl()
                .append(ready.entries())
                .context("append entries failed")?;
        }

        if !ready.snapshot().is_empty() {
            self.storage
                .wl()
                .apply_snapshot(ready.snapshot().clone())
                .context("apply snapshot failed")?;
        }

        applied.extend(self.apply_entries(ready.take_committed_entries())?);
        outbound.extend(ready.take_messages());
        outbound.extend(ready.take_persisted_messages());

        // Phase 2: Continue processing after persistence acknowledgment

        let mut light_ready = self.raw.advance(ready);

        if let Some(commit) = light_ready.commit_index() {
            self.storage.wl().mut_hard_state().set_commit(commit);
        }

        applied.extend(self.apply_entries(light_ready.take_committed_entries())?);
        outbound.extend(light_ready.take_messages());

        self.raw.advance_apply();

        Ok(Some(ReadyBundle {
            messages: outbound,
            applied,
        }))
    }

    /// Applies committed entries to the state machine.
    ///
    /// Entries can be:
    /// - Empty (configuration changes, no-ops) → skipped
    /// - Normal commands (PUT operations) → applied to key-value store
    ///
    /// We filter out empty entries to avoid unnecessary processing. Only normal
    /// entries carry application commands that modify state.
    fn apply_entries(&mut self, entries: Vec<Entry>) -> Result<Vec<ApplyReport>> {
        let mut applied = Vec::new();
        for entry in entries {
            if entry.data.is_empty() {
                continue;
            }
            if entry.entry_type() == EntryType::EntryNormal {
                let command =
                    CommandPayload::decode(&entry.data).context("decode command failed")?;
                applied.push(self.apply_command(entry.index, entry.term, command));
            }
        }
        Ok(applied)
    }

    /// Executes a single command on the state machine and generates a report.
    ///
    /// Currently only handles `Put` commands. Future extensions (Get, Delete, etc.)
    /// would add more match arms here.
    fn apply_command(&mut self, index: u64, term: u64, cmd: CommandPayload) -> ApplyReport {
        match cmd {
            CommandPayload::Put { key, value } => {
                self.store.put(key.clone(), value.clone());
                ApplyReport {
                    node_id: self.id,
                    key,
                    value,
                    index,
                    term,
                }
            }
        }
    }

    /// Reads the current value for a key from local storage.
    ///
    /// Returns the latest applied value, or `None` if the key doesn't exist.
    /// This is a local read (no Raft consensus required), so it reflects
    /// whatever has been committed and applied on this specific node.
    pub fn value_for(&self, key: &str) -> Option<String> {
        self.store.get(key)
    }

    /// Returns a snapshot of all key-value pairs currently in the store.
    ///
    /// Used for displaying node state in the `STATUS` command.
    pub fn snapshot(&self) -> std::collections::BTreeMap<String, String> {
        self.store.snapshot()
    }
}
