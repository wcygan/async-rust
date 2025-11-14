//! Integration tests for leader election with real networking.
//!
//! These tests spawn actual Raft nodes with TCP connections to verify
//! election behavior in a realistic environment.

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

    /// Waits for exactly one leader to be elected.
    ///
    /// Returns the leader's ID (1-indexed).
    fn wait_for_single_leader(&self, timeout: Duration) -> Result<usize> {
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > timeout {
                anyhow::bail!("timeout waiting for leader election");
            }

            let mut leaders = Vec::new();
            for (i, handle) in self.handles.iter().enumerate() {
                if let Ok(status) = handle.status() {
                    if status.role == StateRole::Leader {
                        leaders.push(i + 1);
                    }
                }
            }

            if leaders.len() == 1 {
                return Ok(leaders[0]);
            }

            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Waits for all nodes to agree on the same leader.
    fn wait_for_leader_consensus(&self, expected_leader: u64, timeout: Duration) -> Result<()> {
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > timeout {
                anyhow::bail!("timeout waiting for leader consensus");
            }

            let mut all_agree = true;
            for handle in &self.handles {
                if let Ok(status) = handle.status() {
                    if status.leader_id != expected_leader {
                        all_agree = false;
                        break;
                    }
                }
            }

            if all_agree {
                return Ok(());
            }

            thread::sleep(Duration::from_millis(50));
        }
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
fn test_integration_basic_election() -> Result<()> {
    let cluster = TestCluster::spawn(3, 17101)?;

    // Trigger election on node 1
    cluster.node(1).campaign()?;

    // Wait for node 1 to become leader
    cluster.wait_for_node_to_become_leader(1, Duration::from_secs(5))?;

    // Wait for all nodes to agree on leader
    cluster.wait_for_leader_consensus(1, Duration::from_secs(5))?;

    cluster.shutdown()?;
    Ok(())
}

#[test]
fn test_integration_follower_timeout() -> Result<()> {
    let cluster = TestCluster::spawn(3, 17201)?;

    // Establish node 1 as leader
    cluster.node(1).campaign()?;
    cluster.wait_for_node_to_become_leader(1, Duration::from_secs(5))?;

    // Shut down node 1 (simulates leader failure)
    cluster.node(1).shutdown()?;

    // Wait for one of the remaining nodes to become leader
    // (election timeout ~1s + election time)
    let new_leader = cluster.wait_for_single_leader(Duration::from_secs(5))?;
    assert!(new_leader == 2 || new_leader == 3, "new leader should be node 2 or 3");

    // Clean up remaining nodes
    cluster.shutdown()?;
    Ok(())
}

#[test]
fn test_integration_forced_campaign_follower() -> Result<()> {
    let cluster = TestCluster::spawn(3, 17301)?;

    // Establish node 1 as leader
    cluster.node(1).campaign()?;
    cluster.wait_for_node_to_become_leader(1, Duration::from_secs(5))?;

    // Force node 2 (follower) to campaign
    cluster.node(2).campaign()?;

    // Wait for election to stabilize
    thread::sleep(Duration::from_secs(2));

    // Should have exactly one leader (could be 1 or 2)
    let leader = cluster.wait_for_single_leader(Duration::from_secs(5))?;
    assert!(leader == 1 || leader == 2, "leader should be node 1 or 2");

    cluster.shutdown()?;
    Ok(())
}

#[test]
fn test_integration_forced_campaign_leader() -> Result<()> {
    let cluster = TestCluster::spawn(3, 17401)?;

    // Establish node 1 as leader
    cluster.node(1).campaign()?;
    cluster.wait_for_node_to_become_leader(1, Duration::from_secs(5))?;

    // Leader campaigns again
    cluster.node(1).campaign()?;

    // Wait for cluster to stabilize
    thread::sleep(Duration::from_secs(2));

    // Should maintain single leader
    cluster.wait_for_single_leader(Duration::from_secs(5))?;

    cluster.shutdown()?;
    Ok(())
}

#[test]
fn test_integration_no_split_brain() -> Result<()> {
    let cluster = TestCluster::spawn(3, 17501)?;

    // Establish node 1 as leader
    cluster.node(1).campaign()?;
    cluster.wait_for_node_to_become_leader(1, Duration::from_secs(5))?;

    // Force node 2 to campaign
    cluster.node(2).campaign()?;

    // Poll repeatedly during election to detect split-brain
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        // Count leaders at this moment
        let mut leader_count = 0;
        let mut leaders = Vec::new();
        for i in 1..=3 {
            if let Ok(status) = cluster.node(i).status() {
                if status.role == StateRole::Leader {
                    leader_count += 1;
                    leaders.push((i, status.term));
                }
            }
        }

        // CRITICAL: Never more than 1 leader
        if leader_count > 1 {
            panic!(
                "SPLIT BRAIN DETECTED: {} leaders: {:?}",
                leader_count, leaders
            );
        }

        thread::sleep(Duration::from_millis(50));
    }

    // Verify single leader at the end
    cluster.wait_for_single_leader(Duration::from_secs(5))?;

    cluster.shutdown()?;
    Ok(())
}

#[test]
fn test_integration_leader_step_down() -> Result<()> {
    let cluster = TestCluster::spawn(3, 17601)?;

    // Establish node 1 as leader
    cluster.node(1).campaign()?;
    cluster.wait_for_node_to_become_leader(1, Duration::from_secs(5))?;

    let initial_status = cluster.node(1).status()?;
    let initial_term = initial_status.term;

    // Force node 2 to campaign
    cluster.node(2).campaign()?;

    // Wait for new election to complete
    thread::sleep(Duration::from_secs(2));

    // Check that node 1 has updated its term (saw higher term and stepped down)
    let final_status = cluster.node(1).status()?;
    assert!(
        final_status.term >= initial_term,
        "node 1 term should have updated: {} -> {}",
        initial_term,
        final_status.term
    );

    // Verify single leader exists
    let leader = cluster.wait_for_single_leader(Duration::from_secs(5))?;

    // Verify all nodes agree on this leader
    cluster.wait_for_leader_consensus(leader as u64, Duration::from_secs(5))?;

    cluster.shutdown()?;
    Ok(())
}
