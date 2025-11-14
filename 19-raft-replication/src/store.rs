//! In-memory key-value storage for the Raft state machine.
//!
//! This module provides the application state machine - the actual data
//! that Raft replicates. Commands from the Raft log are applied here.

use std::collections::BTreeMap;
use std::sync::Mutex;

/// Thread-safe key-value store used by each Raft node.
///
/// This is the state machine that Raft replicates. Each node has its own
/// `KvStore` instance, and Raft ensures they all converge to the same state
/// by replicating the log of PUT commands.
///
/// # Why Mutex instead of RwLock?
///
/// Writes (PUT) happen on every committed command, while reads (GET) are
/// relatively infrequent in this demo. The simpler `Mutex` is sufficient.
///
/// In a production system with read-heavy workloads, `RwLock` would allow
/// concurrent reads while maintaining exclusive writes.
///
/// # Why BTreeMap instead of HashMap?
///
/// BTreeMap provides deterministic iteration order, which is helpful for:
/// - Debugging (consistent STATUS output)
/// - Future snapshot implementation (stable serialization)
/// - Testing (predictable behavior)
///
/// The performance difference is negligible for typical cluster sizes.
#[derive(Default)]
pub struct KvStore {
    data: Mutex<BTreeMap<String, String>>,
}

impl KvStore {
    /// Creates an empty key-value store.
    pub fn new() -> Self {
        Self {
            data: Mutex::new(BTreeMap::new()),
        }
    }

    /// Stores a key-value pair, overwriting any existing value.
    ///
    /// Called when applying committed PUT commands from the Raft log.
    pub fn put(&self, key: String, value: String) {
        self.data.lock().unwrap().insert(key, value);
    }

    /// Retrieves the current value for a key.
    ///
    /// Returns `None` if the key doesn't exist. This is a local read
    /// with no Raft consensus - returns whatever this node has applied.
    pub fn get(&self, key: &str) -> Option<String> {
        self.data.lock().unwrap().get(key).cloned()
    }

    /// Returns a snapshot of all key-value pairs.
    ///
    /// Clones the entire map to avoid holding the lock during iteration.
    /// This is a deliberate tradeoff:
    /// - **Pro**: Simple, no lock held during caller's iteration
    /// - **Con**: Memory allocation proportional to store size
    ///
    /// For a production system with large datasets, consider:
    /// - Returning an iterator that holds the lock (risky - caller could stall)
    /// - Using a copy-on-write data structure (like im::HashMap)
    /// - Implementing proper Raft snapshots with streaming
    pub fn snapshot(&self) -> BTreeMap<String, String> {
        self.data.lock().unwrap().clone()
    }
}
