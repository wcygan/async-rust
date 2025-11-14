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

    /// Returns the current Raft term.
    ///
    /// The term monotonically increases with each election. It's used to detect
    /// stale information and ensure safety properties. When a node sees a higher
    /// term, it immediately updates and becomes a follower.
    pub fn term(&self) -> u64 {
        self.raw.raft.term
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Test harness for simulating a Raft cluster without networking.
    ///
    /// Routes messages between nodes in-memory for deterministic testing
    /// of election and replication logic.
    struct TestCluster {
        nodes: HashMap<u64, RaftNode>,
    }

    impl TestCluster {
        /// Creates a cluster of N nodes with sequential IDs starting from 1.
        fn new(n: usize) -> Result<Self> {
            let ids: Vec<u64> = (1..=n as u64).collect();
            let mut nodes = HashMap::new();
            for &id in &ids {
                nodes.insert(id, RaftNode::new(id, &ids)?);
            }
            Ok(Self { nodes })
        }

        /// Gets a mutable reference to a node.
        fn node_mut(&mut self, id: u64) -> &mut RaftNode {
            self.nodes.get_mut(&id).expect("node not found")
        }

        /// Gets an immutable reference to a node.
        fn node(&self, id: u64) -> &RaftNode {
            self.nodes.get(&id).expect("node not found")
        }

        /// Advances all nodes by one tick.
        fn tick_all(&mut self) {
            for node in self.nodes.values_mut() {
                node.tick();
            }
        }

        /// Processes ready states for all nodes and routes messages.
        ///
        /// Returns number of messages delivered (useful for detecting quiescence).
        fn deliver_messages(&mut self) -> Result<usize> {
            let mut total_delivered = 0;
            loop {
                let mut messages = Vec::new();

                // Collect messages from all nodes
                for node in self.nodes.values_mut() {
                    if let Some(bundle) = node.poll_ready()? {
                        messages.extend(bundle.messages);
                    }
                }

                if messages.is_empty() {
                    break;
                }

                total_delivered += messages.len();

                // Deliver messages to destinations
                for msg in messages {
                    if let Some(node) = self.nodes.get_mut(&msg.to) {
                        node.step(msg)?;
                    }
                }
            }
            Ok(total_delivered)
        }

        /// Runs ticks + message delivery until messages stop flowing or max iterations.
        ///
        /// Returns number of iterations performed.
        fn stabilize(&mut self, max_iters: usize) -> Result<usize> {
            for i in 0..max_iters {
                self.tick_all();
                let delivered = self.deliver_messages()?;
                if delivered == 0 {
                    return Ok(i + 1);
                }
            }
            Ok(max_iters)
        }

        /// Verifies exactly one leader exists and returns its ID.
        fn assert_single_leader(&self) -> u64 {
            let leaders: Vec<u64> = self
                .nodes
                .iter()
                .filter(|(_, n)| n.role() == StateRole::Leader)
                .map(|(id, _)| *id)
                .collect();
            assert_eq!(leaders.len(), 1, "expected exactly one leader, found: {:?}", leaders);
            leaders[0]
        }

        /// Verifies all nodes agree on the same leader.
        fn assert_leader_consensus(&self, expected_leader: u64) {
            for (&id, node) in &self.nodes {
                let leader = node.leader_id();
                assert_eq!(
                    leader, expected_leader,
                    "node {} sees leader {} but expected {}",
                    id, leader, expected_leader
                );
            }
        }
    }

    #[test]
    fn test_basic_three_node_election() -> Result<()> {
        let mut cluster = TestCluster::new(3)?;

        // Initially all nodes are followers with no leader
        for id in 1..=3 {
            assert_eq!(cluster.node(id).role(), StateRole::Follower);
            assert_eq!(cluster.node(id).leader_id(), 0);
        }

        // Node 1 campaigns and stabilize to complete election
        cluster.node_mut(1).campaign()?;
        cluster.stabilize(10)?;

        // Node 1 should win election and become leader
        assert_eq!(cluster.node(1).role(), StateRole::Leader);

        // Verify single leader and consensus
        let leader = cluster.assert_single_leader();
        assert_eq!(leader, 1);
        cluster.assert_leader_consensus(1);

        // Other nodes should be followers
        assert_eq!(cluster.node(2).role(), StateRole::Follower);
        assert_eq!(cluster.node(3).role(), StateRole::Follower);

        Ok(())
    }

    #[test]
    fn test_follower_timeout_election() -> Result<()> {
        let mut cluster = TestCluster::new(3)?;

        // Establish node 1 as leader
        cluster.node_mut(1).campaign()?;
        cluster.stabilize(10)?;
        assert_eq!(cluster.assert_single_leader(), 1);

        // Simulate leader failure: stop processing node 1
        // Tick followers past election timeout (10 ticks)
        for _ in 0..15 {
            cluster.node_mut(2).tick();
            cluster.node_mut(3).tick();
        }

        // One of the followers should campaign
        cluster.deliver_messages()?;

        let candidates: Vec<u64> = cluster
            .nodes
            .iter()
            .filter(|(_, n)| n.role() == StateRole::Candidate || n.role() == StateRole::Leader)
            .map(|(id, _)| *id)
            .collect();
        assert!(!candidates.is_empty(), "at least one follower should campaign");

        // Stabilize to complete election (excluding failed node 1)
        for _ in 0..10 {
            cluster.node_mut(2).tick();
            cluster.node_mut(3).tick();
            // Deliver messages only between nodes 2 and 3
            let mut messages = Vec::new();
            if let Some(bundle) = cluster.node_mut(2).poll_ready()? {
                messages.extend(bundle.messages);
            }
            if let Some(bundle) = cluster.node_mut(3).poll_ready()? {
                messages.extend(bundle.messages);
            }
            for msg in messages {
                if msg.to != 1 {
                    cluster.nodes.get_mut(&msg.to).unwrap().step(msg)?;
                }
            }
        }

        // Verify new leader elected (either 2 or 3)
        let leaders: Vec<u64> = vec![2, 3]
            .into_iter()
            .filter(|&id| cluster.node(id).role() == StateRole::Leader)
            .collect();
        assert_eq!(leaders.len(), 1, "exactly one of nodes 2,3 should be leader");

        Ok(())
    }

    #[test]
    fn test_forced_campaign_from_follower() -> Result<()> {
        let mut cluster = TestCluster::new(3)?;

        // Establish node 1 as leader
        cluster.node_mut(1).campaign()?;
        cluster.stabilize(10)?;
        assert_eq!(cluster.assert_single_leader(), 1);

        // Force node 2 (follower) to campaign
        cluster.node_mut(2).campaign()?;
        cluster.stabilize(20)?;

        // Should have exactly one leader (could be 1 or 2 depending on timing)
        let leader = cluster.assert_single_leader();

        // Verify the new leader is either still 1 or newly elected 2
        assert!(leader == 1 || leader == 2, "leader should be node 1 or 2, got {}", leader);

        Ok(())
    }

    #[test]
    fn test_forced_campaign_from_leader() -> Result<()> {
        let mut cluster = TestCluster::new(3)?;

        // Establish node 1 as leader
        cluster.node_mut(1).campaign()?;
        cluster.stabilize(10)?;
        assert_eq!(cluster.assert_single_leader(), 1);

        // Leader campaigns again
        // In tikv/raft, a leader calling campaign() while already leader
        // may be a no-op or may trigger internal state transitions
        cluster.node_mut(1).campaign()?;
        cluster.stabilize(20)?;

        // Should maintain single leader and cluster stability
        cluster.assert_single_leader();

        // Verify cluster remains functional by checking all nodes agree on leader
        let leader = cluster.assert_single_leader();
        cluster.assert_leader_consensus(leader);

        Ok(())
    }

    #[test]
    fn test_no_split_brain_during_forced_campaign() -> Result<()> {
        let mut cluster = TestCluster::new(3)?;

        // Establish node 1 as leader
        cluster.node_mut(1).campaign()?;
        cluster.stabilize(10)?;
        assert_eq!(cluster.assert_single_leader(), 1);

        // Force node 2 to campaign
        cluster.node_mut(2).campaign()?;

        // Poll repeatedly during election process to detect any split-brain
        for _ in 0..20 {
            cluster.tick_all();
            cluster.deliver_messages()?;

            // Count leaders at this point in time
            let leader_count = cluster
                .nodes
                .values()
                .filter(|n| n.role() == StateRole::Leader)
                .count();

            // CRITICAL: Must never have more than 1 leader
            assert!(
                leader_count <= 1,
                "SPLIT BRAIN DETECTED: {} leaders at same time",
                leader_count
            );
        }

        // Final stabilization
        cluster.stabilize(10)?;
        cluster.assert_single_leader();

        Ok(())
    }

    #[test]
    fn test_leader_steps_down_on_higher_term() -> Result<()> {
        let mut cluster = TestCluster::new(3)?;

        // Establish node 1 as leader
        cluster.node_mut(1).campaign()?;
        cluster.stabilize(10)?;
        assert_eq!(cluster.assert_single_leader(), 1);
        let initial_term = cluster.node(1).term();

        // Force node 2 to campaign (triggers new term)
        cluster.node_mut(2).campaign()?;
        cluster.stabilize(20)?;

        // Verify new term is higher
        let node1_term = cluster.node(1).term();
        let node2_term = cluster.node(2).term();
        assert!(
            node1_term > initial_term || node2_term > initial_term,
            "term should have incremented"
        );

        // Verify old leader (node 1) is no longer leader
        // It should have stepped down to follower when it saw the higher term
        let final_leader = cluster.assert_single_leader();

        // Either node 1 stepped down and node 2 won, or node 1 won the re-election
        // But importantly, node 1 MUST have seen the higher term and updated
        assert!(
            cluster.node(1).term() >= node2_term,
            "node 1 should have updated to higher term"
        );

        // Verify all nodes agree on leader
        cluster.assert_leader_consensus(final_leader);

        Ok(())
    }
}
