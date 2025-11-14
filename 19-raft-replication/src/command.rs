//! Commands that are replicated through the Raft log.
//!
//! This module defines the actual state machine operations that get committed
//! via Raft consensus. These are serialized into log entries and replicated
//! to all nodes in the cluster.
//!
//! Unlike [`crate::protocol::ConsoleCommand`] which includes meta-commands
//! (STATUS, HELP, EXIT), this enum contains ONLY operations that modify
//! replicated state.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Operations that modify the replicated key-value store.
///
/// Each variant represents a state machine command that will be:
/// 1. Serialized and appended to the Raft log
/// 2. Replicated to a majority of nodes
/// 3. Committed once replicated
/// 4. Applied to each node's local state machine
///
/// Currently only PUT is supported. Future extensions might add Delete, CAS, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommandPayload {
    Put { key: String, value: String },
}

impl CommandPayload {
    /// Serializes the command into bytes for storage in the Raft log.
    ///
    /// Uses bincode because:
    /// - Compact binary format (smaller logs)
    /// - Fast serialization (low overhead)
    /// - Type-safe deserialization
    ///
    /// JSON would be more debuggable but wastes space and CPU for this use case.
    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(bincode::serialize(self)?)
    }

    /// Deserializes a command from log entry bytes.
    ///
    /// Called when applying committed entries from the Raft log to the state machine.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Ok(bincode::deserialize(bytes)?)
    }
}
