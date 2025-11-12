use anyhow::{Context, Result};
use raft::StateRole;
use raft::prelude::{ConfState, Config, Entry, EntryType, Message, RawNode};
use raft::storage::MemStorage;
use slog::{Logger, o};

use crate::command::CommandPayload;
use crate::store::KvStore;

fn silent_logger() -> Logger {
    Logger::root(slog::Discard, o!())
}

pub struct ApplyReport {
    pub node_id: u64,
    pub key: String,
    pub value: String,
    pub index: u64,
    pub term: u64,
}

pub struct ReadyBundle {
    pub messages: Vec<Message>,
    pub applied: Vec<ApplyReport>,
}

pub struct RaftNode {
    id: u64,
    raw: RawNode<MemStorage>,
    storage: MemStorage,
    store: KvStore,
}

impl RaftNode {
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

    pub fn tick(&mut self) {
        self.raw.tick();
    }

    pub fn campaign(&mut self) -> Result<()> {
        self.raw.campaign().context("campaign failed")
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn role(&self) -> StateRole {
        self.raw.raft.state
    }

    pub fn leader_id(&self) -> u64 {
        self.raw.raft.leader_id
    }

    pub fn propose(&mut self, payload: &CommandPayload) -> Result<()> {
        let data = payload.encode().context("encode command failed")?;
        self.raw.propose(vec![], data).context("propose failed")
    }

    pub fn step(&mut self, msg: Message) -> Result<()> {
        self.raw.step(msg).context("step failed")
    }

    pub fn poll_ready(&mut self) -> Result<Option<ReadyBundle>> {
        if !self.raw.has_ready() {
            return Ok(None);
        }

        let mut ready = self.raw.ready();
        let mut applied = Vec::new();
        let mut outbound = Vec::new();

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

    pub fn value_for(&self, key: &str) -> Option<String> {
        self.store.get(key)
    }

    pub fn snapshot(&self) -> std::collections::BTreeMap<String, String> {
        self.store.snapshot()
    }
}
