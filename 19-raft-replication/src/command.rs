use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Commands that flow through the Raft replicated log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommandPayload {
    Put { key: String, value: String },
}

impl CommandPayload {
    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(bincode::serialize(self)?)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Ok(bincode::deserialize(bytes)?)
    }
}
