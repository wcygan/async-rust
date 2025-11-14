//! REPL command parsing for the interactive shell.
//!
//! This module handles user input from the command-line interface. It parses
//! text commands (like "PUT foo bar") into structured enum variants.
//!
//! This is separate from [`crate::command::CommandPayload`] because:
//! - `ConsoleCommand` represents the REPL interface (including meta-commands like STATUS, EXIT)
//! - `CommandPayload` represents only the commands that go through Raft replication
//!
//! Not all console commands trigger Raft operations (e.g., STATUS is a local read).

use anyhow::{Result, anyhow};

/// Commands that can be entered at the REPL prompt.
///
/// Includes both cluster operations (PUT, GET) and meta-commands (STATUS, HELP, EXIT).
#[derive(Debug, PartialEq)]
pub enum ConsoleCommand {
    Put { key: String, value: String },
    Get { key: String },
    Status,
    Exit,
    Help,
}

impl ConsoleCommand {
    /// Parses a line of user input into a command.
    ///
    /// # Parameters
    /// - `line`: Raw input from the REPL
    /// - `allow_put`: If false, PUT commands will be rejected with an error
    ///
    /// # Why `allow_put`?
    ///
    /// This parameter exists for future extensions where we might want read-only
    /// shells (e.g., observer nodes, debugging interfaces). Currently all shells
    /// allow PUT, but the infrastructure is here if needed.
    ///
    /// # Syntax
    /// - `PUT <key> <value>` - Store a key-value pair (requires `allow_put = true`)
    /// - `GET <key>` - Retrieve a value
    /// - `STATUS` - Show node role, leader, and store contents
    /// - `HELP` - Print command reference
    /// - `EXIT` - Shut down this node
    pub fn parse(line: &str, allow_put: bool) -> Result<Self> {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("exit") {
            return Ok(ConsoleCommand::Exit);
        }
        if trimmed.eq_ignore_ascii_case("help") {
            return Ok(ConsoleCommand::Help);
        }
        if trimmed.eq_ignore_ascii_case("status") {
            return Ok(ConsoleCommand::Status);
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        match parts.as_slice() {
            ["GET", key] => Ok(ConsoleCommand::Get {
                key: key.to_string(),
            }),
            ["PUT", key, value] if allow_put => Ok(ConsoleCommand::Put {
                key: key.to_string(),
                value: value.to_string(),
            }),
            _ => Err(anyhow!(
                "invalid command. Try PUT <key> <value>, GET <key>, STATUS, HELP, or EXIT"
            )),
        }
    }
}
