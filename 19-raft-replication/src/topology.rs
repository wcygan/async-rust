use anyhow::{Result, ensure};

/// Simple topology helper that guarantees an odd number of nodes.
pub struct ReplicaTopology {
    nodes: Vec<u64>,
}

impl ReplicaTopology {
    pub fn new(nodes: Vec<u64>) -> Result<Self> {
        ensure!(!nodes.is_empty(), "topology requires at least one node");
        ensure!(
            nodes.len() % 2 == 1,
            "topology must have an odd number of nodes"
        );
        Ok(Self { nodes })
    }

    pub fn demo() -> Self {
        Self::new(vec![1, 2, 3]).expect("valid demo topology")
    }

    pub fn ids(&self) -> &[u64] {
        &self.nodes
    }

    pub fn primary_id(&self) -> u64 {
        self.nodes[0]
    }
}
