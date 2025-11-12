use std::collections::BTreeMap;
use std::sync::Mutex;

/// Thread-safe key/value store used by each Raft node.
#[derive(Default)]
pub struct KvStore {
    data: Mutex<BTreeMap<String, String>>,
}

impl KvStore {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn put(&self, key: String, value: String) {
        self.data.lock().unwrap().insert(key, value);
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.data.lock().unwrap().get(key).cloned()
    }

    pub fn snapshot(&self) -> BTreeMap<String, String> {
        self.data.lock().unwrap().clone()
    }
}
