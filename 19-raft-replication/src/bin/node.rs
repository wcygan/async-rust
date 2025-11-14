//! CLI entry point for running a Raft node with an interactive REPL.
//!
//! This binary starts a Raft node and provides a command-line interface for
//! interacting with the distributed key-value store.
//!
//! # Example usage
//!
//! Start a 3-node cluster:
//! ```bash
//! # Terminal 1 (node 1)
//! cargo run --bin node -- \
//!   --id 1 --listen 127.0.0.1:7101 \
//!   --peer 1=127.0.0.1:7101 --peer 2=127.0.0.1:7102 --peer 3=127.0.0.1:7103
//!
//! # Terminal 2 (node 2)
//! cargo run --bin node -- \
//!   --id 2 --listen 127.0.0.1:7102 \
//!   --peer 1=127.0.0.1:7101 --peer 2=127.0.0.1:7102 --peer 3=127.0.0.1:7103
//!
//! # Terminal 3 (node 3)
//! cargo run --bin node -- \
//!   --id 3 --listen 127.0.0.1:7103 \
//!   --peer 1=127.0.0.1:7101 --peer 2=127.0.0.1:7102 --peer 3=127.0.0.1:7103
//! ```

use std::collections::HashMap;

use anyhow::{Context, Result};
use clap::{Parser, ValueHint};
use raft_replication::protocol::ConsoleCommand;
use raft_replication::runtime::{NodeConfig, spawn_node};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

/// Command-line arguments for the Raft node.
#[derive(Parser, Debug)]
#[command(author, version, about = "Run a Raft node with interactive REPL")]
struct Args {
    /// Numeric node ID (must match one entry in --peer)
    #[arg(long)]
    id: u64,

    /// Address this node should listen on for Raft messages, e.g. 127.0.0.1:7101
    #[arg(long, value_hint = ValueHint::Hostname)]
    listen: String,

    /// Comma-separated peer map: id=addr,id=addr,... (must include self)
    #[arg(long, value_delimiter = ',', value_hint = ValueHint::Other)]
    peer: Vec<String>,
}

/// Main entry point: spawns a Raft node and runs the REPL.
///
/// # Flow
///
/// 1. Parse command-line arguments
/// 2. Validate that `--id` matches one of the `--peer` entries
/// 3. Spawn worker and network listener threads
/// 4. Run REPL loop until user types EXIT or Ctrl-C
/// 5. Shut down gracefully
///
/// # Why validate id/listen consistency?
///
/// Each node needs to know its own address in the peer map (to build
/// the voter set correctly). Requiring `--peer` to include self ensures
/// all nodes have a consistent view of cluster membership.
fn main() -> Result<()> {
    let args = Args::parse();
    let peers = parse_peers(&args.peer)?;

    // Validate that this node's ID maps to its listen address
    if peers.get(&args.id).map(String::as_str) != Some(args.listen.as_str()) {
        return Err(anyhow::anyhow!(
            "self id {} must map to listen addr {} via --peer entries",
            args.id,
            args.listen
        ));
    }

    let handle = spawn_node(NodeConfig {
        id: args.id,
        listen_addr: args.listen.clone(),
        peers,
    })?;

    println!(
        "Node {} ready. Peers configured: {}",
        args.id,
        args.peer.join(",")
    );
    print_help();

    // REPL loop
    let mut rl = DefaultEditor::new()?;
    loop {
        match rl.readline(&format!("node-{}> ", args.id)) {
            Ok(line) => match ConsoleCommand::parse(&line, true) {
                Ok(ConsoleCommand::Put { key, value }) => match handle.put(key.clone(), value) {
                    Ok(new_value) => println!("OK {key} = {new_value}"),
                    Err(err) => println!("Error proposing value: {err}"),
                },
                Ok(ConsoleCommand::Get { key }) => match handle.get(key.clone())? {
                    Some(value) => println!("{key} -> {value}"),
                    None => println!("{key} -> not found"),
                },
                Ok(ConsoleCommand::Status) => {
                    let status = handle.status()?;
                    println!(
                        "node {} role: {:?}, leader: {}",
                        status.node_id, status.role, status.leader_id
                    );
                    if status.store.is_empty() {
                        println!("store empty");
                    } else {
                        for (key, value) in status.store {
                            println!("  {key} = {value}");
                        }
                    }
                }
                Ok(ConsoleCommand::Help) => print_help(),
                Ok(ConsoleCommand::Exit) => {
                    handle.shutdown()?;
                    println!("Goodbye!");
                    break;
                }
                Err(err) => println!("{err}"),
            },
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => {
                handle.shutdown()?;
                println!("Goodbye!");
                break;
            }
            Err(err) => {
                handle.shutdown()?;
                println!("Readline error: {err}");
                break;
            }
        }
    }

    Ok(())
}

/// Parses peer entries from the command line into a map.
///
/// Each entry must be in the format `id=address`, for example:
/// - `1=127.0.0.1:7101`
/// - `2=192.168.1.10:7102`
///
/// # Why require all peers at startup?
///
/// This demo uses static membership - all nodes must know all peers from
/// the start. Raft supports dynamic membership changes, but that adds
/// complexity (joint consensus, configuration log entries).
///
/// For a learning/demo project, static membership is simpler and covers
/// the core Raft algorithm without the membership change complications.
fn parse_peers(entries: &[String]) -> Result<HashMap<u64, String>> {
    let mut peers = HashMap::new();
    for entry in entries {
        let Some((id_str, addr)) = entry.split_once('=') else {
            return Err(anyhow::anyhow!(
                "invalid peer entry '{entry}', expected id=addr"
            ));
        };
        let id: u64 = id_str
            .parse()
            .with_context(|| format!("invalid peer id in '{entry}'"))?;
        peers.insert(id, addr.to_string());
    }
    if peers.is_empty() {
        return Err(anyhow::anyhow!(
            "at least one --peer entry is required (include self)"
        ));
    }
    Ok(peers)
}

/// Prints the help message showing available REPL commands.
fn print_help() {
    println!("Commands:");
    println!("  PUT <key> <value>  -- replicate a value via Raft");
    println!("  GET <key>          -- read most recent committed value");
    println!("  STATUS             -- show role/leader and local store");
    println!("  HELP               -- print this message");
    println!("  EXIT               -- shut down this node");
}
