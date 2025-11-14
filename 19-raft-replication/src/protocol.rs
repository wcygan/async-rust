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
    Keys,
    Scan,
    Clear,
    Status,
    Campaign,
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
    /// Commands are case-insensitive and support aliases:
    /// - `PUT <key> <value>` (alias: `p`) - Store a key-value pair (requires `allow_put = true`)
    /// - `GET <key>` (alias: `g`) - Retrieve a value
    /// - `KEYS` (alias: `k`) - List all keys in the store
    /// - `SCAN` (alias: `sc`) - Display all key-value pairs
    /// - `CLEAR` (alias: `cl`) - Clear the output window
    /// - `STATUS` (alias: `s`) - Show node role, leader, and store contents
    /// - `CAMPAIGN` (alias: `c`) - Force this node to start an election
    /// - `HELP` (alias: `h`) - Print command reference
    /// - `EXIT` (alias: `e`) - Shut down this node
    pub fn parse(line: &str, allow_put: bool) -> Result<Self> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("empty command"));
        }

        // Split into parts and normalize the first word
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.is_empty() {
            return Err(anyhow!("empty command"));
        }

        // Normalize first word: uppercase and expand aliases
        let cmd = parts[0].to_uppercase();
        let normalized_cmd = match cmd.as_str() {
            "P" => "PUT",
            "G" => "GET",
            "K" => "KEYS",
            "SC" => "SCAN",
            "CL" => "CLEAR",
            "S" => "STATUS",
            "C" => "CAMPAIGN",
            "H" => "HELP",
            "E" => "EXIT",
            other => other,
        };

        // Handle single-word commands
        match normalized_cmd {
            "EXIT" => return Ok(ConsoleCommand::Exit),
            "HELP" => return Ok(ConsoleCommand::Help),
            "KEYS" => return Ok(ConsoleCommand::Keys),
            "SCAN" => return Ok(ConsoleCommand::Scan),
            "CLEAR" => return Ok(ConsoleCommand::Clear),
            "STATUS" => return Ok(ConsoleCommand::Status),
            "CAMPAIGN" => return Ok(ConsoleCommand::Campaign),
            _ => {}
        }

        // Handle multi-word commands
        match (normalized_cmd, parts.len()) {
            ("GET", 2) => Ok(ConsoleCommand::Get {
                key: parts[1].to_string(),
            }),
            ("PUT", 3) if allow_put => Ok(ConsoleCommand::Put {
                key: parts[1].to_string(),
                value: parts[2].to_string(),
            }),
            ("PUT", 3) => Err(anyhow!("PUT not allowed in this context")),
            ("GET", _) => Err(anyhow!("GET requires exactly one argument: GET <key>")),
            ("PUT", _) => Err(anyhow!("PUT requires exactly two arguments: PUT <key> <value>")),
            _ => Err(anyhow!(
                "invalid command. Try: PUT/p, GET/g, KEYS/k, SCAN/sc, CLEAR/cl, STATUS/s, CAMPAIGN/c, HELP/h, EXIT/e"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_case_insensitive_commands() {
        assert!(matches!(ConsoleCommand::parse("exit", true), Ok(ConsoleCommand::Exit)));
        assert!(matches!(ConsoleCommand::parse("EXIT", true), Ok(ConsoleCommand::Exit)));
        assert!(matches!(ConsoleCommand::parse("ExIt", true), Ok(ConsoleCommand::Exit)));

        assert!(matches!(ConsoleCommand::parse("help", true), Ok(ConsoleCommand::Help)));
        assert!(matches!(ConsoleCommand::parse("HELP", true), Ok(ConsoleCommand::Help)));

        assert!(matches!(ConsoleCommand::parse("status", true), Ok(ConsoleCommand::Status)));
        assert!(matches!(ConsoleCommand::parse("STATUS", true), Ok(ConsoleCommand::Status)));
    }

    #[test]
    fn test_aliases() {
        // Exit
        assert!(matches!(ConsoleCommand::parse("e", true), Ok(ConsoleCommand::Exit)));
        assert!(matches!(ConsoleCommand::parse("E", true), Ok(ConsoleCommand::Exit)));

        // Help
        assert!(matches!(ConsoleCommand::parse("h", true), Ok(ConsoleCommand::Help)));
        assert!(matches!(ConsoleCommand::parse("H", true), Ok(ConsoleCommand::Help)));

        // Status
        assert!(matches!(ConsoleCommand::parse("s", true), Ok(ConsoleCommand::Status)));
        assert!(matches!(ConsoleCommand::parse("S", true), Ok(ConsoleCommand::Status)));

        // Get
        assert!(matches!(
            ConsoleCommand::parse("g foo", true),
            Ok(ConsoleCommand::Get { key }) if key == "foo"
        ));
        assert!(matches!(
            ConsoleCommand::parse("G bar", true),
            Ok(ConsoleCommand::Get { key }) if key == "bar"
        ));

        // Put
        assert!(matches!(
            ConsoleCommand::parse("p key val", true),
            Ok(ConsoleCommand::Put { key, value }) if key == "key" && value == "val"
        ));
        assert!(matches!(
            ConsoleCommand::parse("P KEY VAL", true),
            Ok(ConsoleCommand::Put { key, value }) if key == "KEY" && value == "VAL"
        ));
    }

    #[test]
    fn test_full_commands_case_insensitive() {
        // GET
        assert!(matches!(
            ConsoleCommand::parse("get mykey", true),
            Ok(ConsoleCommand::Get { key }) if key == "mykey"
        ));
        assert!(matches!(
            ConsoleCommand::parse("GET mykey", true),
            Ok(ConsoleCommand::Get { key }) if key == "mykey"
        ));
        assert!(matches!(
            ConsoleCommand::parse("GeT mykey", true),
            Ok(ConsoleCommand::Get { key }) if key == "mykey"
        ));

        // PUT
        assert!(matches!(
            ConsoleCommand::parse("put k v", true),
            Ok(ConsoleCommand::Put { key, value }) if key == "k" && value == "v"
        ));
        assert!(matches!(
            ConsoleCommand::parse("PUT k v", true),
            Ok(ConsoleCommand::Put { key, value }) if key == "k" && value == "v"
        ));
        assert!(matches!(
            ConsoleCommand::parse("PuT k v", true),
            Ok(ConsoleCommand::Put { key, value }) if key == "k" && value == "v"
        ));
    }

    #[test]
    fn test_invalid_commands() {
        assert!(ConsoleCommand::parse("", true).is_err());
        assert!(ConsoleCommand::parse("   ", true).is_err());
        assert!(ConsoleCommand::parse("INVALID", true).is_err());
        assert!(ConsoleCommand::parse("GET", true).is_err()); // Missing key
        assert!(ConsoleCommand::parse("PUT key", true).is_err()); // Missing value
        assert!(ConsoleCommand::parse("PUT", true).is_err()); // Missing both
    }

    #[test]
    fn test_allow_put_flag() {
        assert!(ConsoleCommand::parse("PUT k v", true).is_ok());
        assert!(ConsoleCommand::parse("PUT k v", false).is_err());
        assert!(ConsoleCommand::parse("p k v", true).is_ok());
        assert!(ConsoleCommand::parse("p k v", false).is_err());
    }

    #[test]
    fn test_campaign_command() {
        assert!(matches!(ConsoleCommand::parse("campaign", true), Ok(ConsoleCommand::Campaign)));
        assert!(matches!(ConsoleCommand::parse("CAMPAIGN", true), Ok(ConsoleCommand::Campaign)));
        assert!(matches!(ConsoleCommand::parse("Campaign", true), Ok(ConsoleCommand::Campaign)));
        assert!(matches!(ConsoleCommand::parse("c", true), Ok(ConsoleCommand::Campaign)));
        assert!(matches!(ConsoleCommand::parse("C", true), Ok(ConsoleCommand::Campaign)));
    }

    #[test]
    fn test_keys_command() {
        assert!(matches!(ConsoleCommand::parse("keys", true), Ok(ConsoleCommand::Keys)));
        assert!(matches!(ConsoleCommand::parse("KEYS", true), Ok(ConsoleCommand::Keys)));
        assert!(matches!(ConsoleCommand::parse("Keys", true), Ok(ConsoleCommand::Keys)));
        assert!(matches!(ConsoleCommand::parse("k", true), Ok(ConsoleCommand::Keys)));
        assert!(matches!(ConsoleCommand::parse("K", true), Ok(ConsoleCommand::Keys)));
    }

    #[test]
    fn test_scan_command() {
        assert!(matches!(ConsoleCommand::parse("scan", true), Ok(ConsoleCommand::Scan)));
        assert!(matches!(ConsoleCommand::parse("SCAN", true), Ok(ConsoleCommand::Scan)));
        assert!(matches!(ConsoleCommand::parse("Scan", true), Ok(ConsoleCommand::Scan)));
        assert!(matches!(ConsoleCommand::parse("sc", true), Ok(ConsoleCommand::Scan)));
        assert!(matches!(ConsoleCommand::parse("SC", true), Ok(ConsoleCommand::Scan)));
    }

    #[test]
    fn test_clear_command() {
        assert!(matches!(ConsoleCommand::parse("clear", true), Ok(ConsoleCommand::Clear)));
        assert!(matches!(ConsoleCommand::parse("CLEAR", true), Ok(ConsoleCommand::Clear)));
        assert!(matches!(ConsoleCommand::parse("Clear", true), Ok(ConsoleCommand::Clear)));
        assert!(matches!(ConsoleCommand::parse("cl", true), Ok(ConsoleCommand::Clear)));
        assert!(matches!(ConsoleCommand::parse("CL", true), Ok(ConsoleCommand::Clear)));
    }
}
