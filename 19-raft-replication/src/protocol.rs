use anyhow::{Result, anyhow};

/// Commands supported by the interactive shells.
#[derive(Debug, PartialEq)]
pub enum ConsoleCommand {
    Put { key: String, value: String },
    Get { key: String },
    Status,
    Exit,
    Help,
}

impl ConsoleCommand {
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
