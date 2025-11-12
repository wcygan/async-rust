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

    pub fn entries(&self) -> Vec<(String, String)> {
        self.data
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    pub fn replace_all<I>(&self, entries: I)
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let mut guard = self.data.lock().unwrap();
        guard.clear();
        for (k, v) in entries {
            guard.insert(k, v);
        }
    }
}
