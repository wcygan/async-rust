//! Integration tests for write operations in the Raft cluster.
//!
//! These tests verify that write operations (PUT) are correctly restricted
//! to leader nodes only, making the Raft consensus model explicit and educational.

use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use raft::StateRole;
use raft_replication::runtime::{NodeConfig, NodeHandle, spawn_node};

/// Test harness for managing a cluster of real Raft nodes with TCP networking.
struct TestCluster {
    handles: Vec<NodeHandle>,
}

impl TestCluster {
    /// Spawns N nodes on localhost with sequential ports starting from base_port.
    fn spawn(n: usize, base_port: u16) -> Result<Self> {
        let mut handles = Vec::new();
        let mut peers = HashMap::new();

        // Build peer map
        for i in 0..n {
            let id = (i + 1) as u64;
            let port = base_port + i as u16;
            peers.insert(id, format!("127.0.0.1:{}", port));
        }

        // Spawn all nodes
        for i in 0..n {
            let id = (i + 1) as u64;
            let port = base_port + i as u16;
            let listen_addr = format!("127.0.0.1:{}", port);

            let (handle, _log_rx) = spawn_node(NodeConfig {
                id,
                listen_addr,
                peers: peers.clone(),
            })?;

            handles.push(handle);
        }

        // Give nodes time to start listening
        thread::sleep(Duration::from_millis(100));

        Ok(Self { handles })
    }

    /// Gets a reference to a node handle by 1-indexed ID.
    fn node(&self, id: usize) -> &NodeHandle {
        &self.handles[id - 1]
    }

    /// Waits for a specific node to become leader.
    fn wait_for_node_to_become_leader(&self, node_id: usize, timeout: Duration) -> Result<()> {
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > timeout {
                anyhow::bail!("timeout waiting for node {} to become leader", node_id);
            }

            if let Ok(status) = self.node(node_id).status() {
                if status.role == StateRole::Leader {
                    return Ok(());
                }
            }

            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Shuts down all nodes.
    fn shutdown(self) -> Result<()> {
        for handle in self.handles {
            let _ = handle.shutdown();
        }
        // Give nodes time to shut down
        thread::sleep(Duration::from_millis(100));
        Ok(())
    }
}

#[test]
fn test_leader_accepts_writes() -> Result<()> {
    let cluster = TestCluster::spawn(3, 18101)?;

    // Trigger election on node 1
    cluster.node(1).campaign()?;
    cluster.wait_for_node_to_become_leader(1, Duration::from_secs(5))?;

    // Leader should accept writes
    let result = cluster.node(1).put("key1".to_string(), "value1".to_string());
    assert!(result.is_ok(), "Leader should accept write operations");
    assert_eq!(result?, "value1");

    // Verify write was applied
    let get_result = cluster.node(1).get("key1".to_string())?;
    assert_eq!(get_result, Some("value1".to_string()));

    cluster.shutdown()?;
    Ok(())
}

#[test]
fn test_follower_rejects_writes() -> Result<()> {
    let cluster = TestCluster::spawn(3, 18201)?;

    // Establish node 1 as leader
    cluster.node(1).campaign()?;
    cluster.wait_for_node_to_become_leader(1, Duration::from_secs(5))?;

    // Give followers time to recognize the leader
    thread::sleep(Duration::from_secs(1));

    // Follower should reject writes
    let result = cluster.node(2).put("key1".to_string(), "value1".to_string());

    // The write should fail
    assert!(result.is_err(), "Follower should reject write operations");

    // Note: We can't easily check the exact error message from the runtime layer
    // because that's internal to the worker. The TUI layer check (in node.rs)
    // provides the user-friendly error message.

    cluster.shutdown()?;
    Ok(())
}

#[test]
fn test_follower_rejects_before_election() -> Result<()> {
    let cluster = TestCluster::spawn(3, 18301)?;

    // Before any election, all nodes are followers with no leader
    // Try to write to node 1
    let result = cluster.node(1).put("key1".to_string(), "value1".to_string());

    // Should fail because there's no leader
    assert!(result.is_err(), "Node should reject writes before leader is elected");

    cluster.shutdown()?;
    Ok(())
}

#[test]
fn test_write_workflow() -> Result<()> {
    let cluster = TestCluster::spawn(3, 18401)?;

    // Elect node 1 as leader
    cluster.node(1).campaign()?;
    cluster.wait_for_node_to_become_leader(1, Duration::from_secs(5))?;

    // Leader writes
    cluster.node(1).put("key1".to_string(), "value1".to_string())?;
    cluster.node(1).put("key2".to_string(), "value2".to_string())?;

    // Give replication time to propagate
    thread::sleep(Duration::from_secs(1));

    // Followers can read replicated data
    assert_eq!(
        cluster.node(2).get("key1".to_string())?,
        Some("value1".to_string()),
        "Follower should see replicated write"
    );
    assert_eq!(
        cluster.node(3).get("key2".to_string())?,
        Some("value2".to_string()),
        "Follower should see replicated write"
    );

    // But followers still can't write
    assert!(
        cluster.node(2).put("key3".to_string(), "value3".to_string()).is_err(),
        "Follower should reject writes even after seeing replicated data"
    );

    cluster.shutdown()?;
    Ok(())
}
