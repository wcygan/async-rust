use std::collections::HashMap;

use anyhow::{Context, Result};
use clap::{Parser, ValueHint};
use raft_replication::protocol::ConsoleCommand;
use raft_replication::runtime::{NodeConfig, spawn_node};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

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

fn main() -> Result<()> {
    let args = Args::parse();
    let peers = parse_peers(&args.peer)?;

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

fn print_help() {
    println!("Commands:");
    println!("  PUT <key> <value>  -- replicate a value via Raft");
    println!("  GET <key>          -- read most recent committed value");
    println!("  STATUS             -- show role/leader and local store");
    println!("  HELP               -- print this message");
    println!("  EXIT               -- shut down this node");
}
