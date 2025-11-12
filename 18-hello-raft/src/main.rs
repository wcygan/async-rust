use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use raft::StateRole;
use raft::prelude::{ConfState, Config, Entry, EntryType, Message, RawNode};
use raft::storage::MemStorage;
use serde::{Deserialize, Serialize};
use slog::{Logger, o};

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Demo { scenario } => match scenario {
            DemoScenario::Basic => run_basic_demo(),
        },
    }
}

#[derive(Parser)]
#[command(author, version, about = "Raft-ready loop demo playground")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run canned Raft walkthroughs
    Demo {
        /// Which scenario to execute
        #[arg(value_enum, default_value_t = DemoScenario::Basic)]
        scenario: DemoScenario,
    },
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum DemoScenario {
    Basic,
}

fn silent_logger() -> Logger {
    Logger::root(slog::Discard, o!())
}

fn run_basic_demo() -> Result<()> {
    println!("Bootstrapping 3-node Raft cluster (single-threaded)...");
    let mut cluster = Cluster::new(&[1, 2, 3])?;
    cluster.campaign(1)?;

    let mut proposal_sent = false;
    const MAX_TICKS: usize = 200;

    for tick in 0..MAX_TICKS {
        cluster.tick_all();
        let reports = cluster.process_ready()?;

        for report in &reports {
            println!(
                "node {} applied {} = {} (index {}, term {})",
                report.node_id, report.key, report.value, report.index, report.term
            );
        }

        if !proposal_sent {
            if let Some(leader) = cluster.current_leader() {
                println!("leader {} elected at tick {}", leader, tick);
                cluster.propose_put(leader, "demo-key", format!("value-{tick}"))?;
                proposal_sent = true;
            }
        }

        if proposal_sent {
            if let Some(value) = cluster.all_nodes_agree("demo-key") {
                println!("All nodes replicated demo-key = {}", value);
                cluster.print_snapshots();
                return Ok(());
            }
        }
    }

    bail!("demo did not reach consensus within {MAX_TICKS} ticks");
}

struct Cluster {
    nodes: BTreeMap<u64, DemoNode>,
}

impl Cluster {
    fn new(ids: &[u64]) -> Result<Self> {
        let mut nodes = BTreeMap::new();
        for id in ids {
            let node = DemoNode::new(*id, ids)?;
            nodes.insert(*id, node);
        }
        Ok(Self { nodes })
    }

    fn campaign(&mut self, id: u64) -> Result<()> {
        self.nodes
            .get_mut(&id)
            .with_context(|| format!("node {id} missing"))?
            .campaign()
    }

    fn tick_all(&mut self) {
        for node in self.nodes.values_mut() {
            node.tick();
        }
    }

    fn process_ready(&mut self) -> Result<Vec<ApplyReport>> {
        let mut applied = Vec::new();

        loop {
            let mut made_progress = false;
            let mut outbound = Vec::new();

            for node in self.nodes.values_mut() {
                if let Some(outcome) = node.poll_ready()? {
                    made_progress = true;
                    outbound.extend(outcome.messages);
                    applied.extend(outcome.applied);
                }
            }

            if outbound.is_empty() {
                if made_progress {
                    continue;
                } else {
                    break;
                }
            }

            for msg in outbound {
                if let Some(target) = self.nodes.get_mut(&msg.to) {
                    target.step(msg)?;
                }
            }
        }

        Ok(applied)
    }

    fn current_leader(&self) -> Option<u64> {
        self.nodes
            .values()
            .find(|node| node.role() == StateRole::Leader)
            .map(|node| node.id)
    }

    fn propose_put(&mut self, leader_id: u64, key: &str, value: String) -> Result<()> {
        let cmd = CommandPayload::Put {
            key: key.to_string(),
            value,
        };
        self.nodes
            .get_mut(&leader_id)
            .with_context(|| format!("leader {leader_id} missing"))?
            .propose(&cmd)
    }

    fn all_nodes_agree(&self, key: &str) -> Option<String> {
        let mut nodes = self.nodes.values();
        let first = nodes.next()?.value_for(key)?.clone();
        if nodes.all(|node| node.value_for(key) == Some(&first)) {
            Some(first)
        } else {
            None
        }
    }

    fn print_snapshots(&self) {
        println!("\nFinal key/value snapshots:");
        for (id, node) in &self.nodes {
            println!("node {id}: {:?}", node.snapshot());
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum CommandPayload {
    Put { key: String, value: String },
}

struct DemoNode {
    id: u64,
    raw: RawNode<MemStorage>,
    storage: MemStorage,
    state_machine: HashMap<String, String>,
}

impl DemoNode {
    fn new(id: u64, voters: &[u64]) -> Result<Self> {
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
            .with_context(|| format!("failed to create RawNode {id}"))?;
        Ok(Self {
            id,
            raw,
            storage,
            state_machine: HashMap::new(),
        })
    }

    fn campaign(&mut self) -> Result<()> {
        self.raw.campaign().context("campaign failed")
    }

    fn tick(&mut self) {
        self.raw.tick();
    }

    fn role(&self) -> StateRole {
        self.raw.raft.state
    }

    fn step(&mut self, msg: Message) -> Result<()> {
        self.raw.step(msg).context("step failed")
    }

    fn propose(&mut self, cmd: &CommandPayload) -> Result<()> {
        let data = bincode::serialize(cmd).context("encode command failed")?;
        self.raw.propose(vec![], data).context("propose failed")
    }

    fn value_for(&self, key: &str) -> Option<&String> {
        self.state_machine.get(key)
    }

    fn snapshot(&self) -> HashMap<String, String> {
        self.state_machine.clone()
    }

    fn poll_ready(&mut self) -> Result<Option<ReadyOutcome>> {
        if !self.raw.has_ready() {
            return Ok(None);
        }

        let mut ready = self.raw.ready();
        let mut all_applied = Vec::new();
        let mut all_messages = Vec::new();

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

        all_applied.extend(self.apply_entries(ready.take_committed_entries())?);
        all_messages.extend(ready.take_messages());

        if !ready.persisted_messages().is_empty() {
            all_messages.extend(ready.take_persisted_messages());
        }

        let mut light_ready = self.raw.advance(ready);

        if let Some(commit_index) = light_ready.commit_index() {
            self.storage.wl().mut_hard_state().set_commit(commit_index);
        }

        all_applied.extend(self.apply_entries(light_ready.take_committed_entries())?);
        all_messages.extend(light_ready.take_messages());

        self.raw.advance_apply();

        Ok(Some(ReadyOutcome {
            messages: all_messages,
            applied: all_applied,
        }))
    }

    fn apply_entries(&mut self, entries: Vec<Entry>) -> Result<Vec<ApplyReport>> {
        let mut applied = Vec::new();
        for entry in entries {
            if entry.data.is_empty() {
                continue;
            }

            if entry.entry_type() == EntryType::EntryNormal {
                let command: CommandPayload =
                    bincode::deserialize(&entry.data).context("decode command failed")?;
                applied.push(self.apply_command(entry.index, entry.term, command));
            }
        }
        Ok(applied)
    }

    fn apply_command(&mut self, index: u64, term: u64, cmd: CommandPayload) -> ApplyReport {
        match cmd {
            CommandPayload::Put { key, value } => {
                self.state_machine.insert(key.clone(), value.clone());
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
}

struct ReadyOutcome {
    messages: Vec<Message>,
    applied: Vec<ApplyReport>,
}

struct ApplyReport {
    node_id: u64,
    key: String,
    value: String,
    index: u64,
    term: u64,
}
