//! Worker runtime and network handling for Raft nodes.
//!
//! This module orchestrates the threading model and network communication:
//!
//! - **Worker thread**: Runs the Raft event loop, processes client requests
//! - **Network listener thread**: Accepts TCP connections from peers
//! - **Connection handler threads**: Short-lived threads that read messages and forward to worker
//!
//! Communication uses crossbeam channels to keep the worker single-threaded
//! (simplifies Raft state management) while network I/O runs concurrently.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use crossbeam_channel::{Receiver, Sender, unbounded};
use prost::Message as ProstMessage;
use raft::StateRole;
use raft::prelude::Message;

use crate::command::CommandPayload;
use crate::node::{ApplyReport, RaftNode};

/// Raft logical clock interval.
///
/// The worker calls `node.tick()` every 100ms, which drives Raft's timeout logic:
/// - Heartbeat timeout: 3 ticks = 300ms
/// - Election timeout: 10 ticks = 1000ms
///
/// This value balances responsiveness (detect failures quickly) against stability
/// (avoid spurious elections on temporary network delays).
const TICK_INTERVAL: Duration = Duration::from_millis(100);

/// Log messages emitted by the worker for display in the TUI.
///
/// The worker sends these through a channel instead of printing directly,
/// allowing the TUI to display them in a scrollable output window alongside
/// command results.
#[derive(Debug, Clone)]
pub enum LogMessage {
    /// Informational message (role changes, applied entries, etc.)
    Info(String),
    /// Error message (network failures, Raft errors, etc.)
    Error(String),
}

/// Configuration for spawning a Raft node.
///
/// Specifies the node's identity and how to reach all cluster members.
pub struct NodeConfig {
    /// This node's unique ID (must appear in `peers`)
    pub id: u64,
    /// Address to bind for incoming Raft messages (e.g., "127.0.0.1:7101")
    pub listen_addr: String,
    /// Map of node ID → network address for all cluster members (including self)
    pub peers: HashMap<u64, String>,
}

/// Handle for sending requests to a running Raft node.
///
/// The worker thread owns the actual `RaftNode` and processes requests via
/// a channel. This handle provides a type-safe API for the main thread to
/// interact with the worker.
///
/// # Why channels instead of shared state?
///
/// - **Simpler reasoning**: Worker owns all mutable state, no locks needed
/// - **No data races**: Rust's type system enforces single ownership
/// - **Clean separation**: Main thread does I/O, worker does Raft logic
pub struct NodeHandle {
    request_tx: Sender<ClientRequest>,
}

impl NodeHandle {
    /// Proposes a PUT operation via Raft consensus.
    ///
    /// This sends the request to the worker, which proposes it to Raft.
    /// Blocks until the value is committed and applied, then returns the
    /// committed value.
    ///
    /// # Errors
    /// - If this node is not the leader (propose will fail)
    /// - If the worker has shut down
    /// - If Raft rejects the proposal
    pub fn put(&self, key: String, value: String) -> Result<String> {
        let (resp_tx, resp_rx) = unbounded();
        self.request_tx
            .send(ClientRequest::Put {
                key,
                value,
                respond_to: resp_tx,
            })
            .context("failed to send put request")?;
        resp_rx.recv().context("put response channel closed")?
    }

    /// Reads a value from the local state machine (no Raft consensus).
    ///
    /// Returns whatever this node has currently applied. This is not a
    /// linearizable read - if this node is partitioned, it may return
    /// stale data.
    ///
    /// For linearizable reads, a production system would need read quorum
    /// or leader leases. This demo prioritizes simplicity.
    pub fn get(&self, key: String) -> Result<Option<String>> {
        let (resp_tx, resp_rx) = unbounded();
        self.request_tx
            .send(ClientRequest::Get {
                key,
                respond_to: resp_tx,
            })
            .context("failed to send get request")?;
        resp_rx
            .recv()
            .context("get response channel closed")
            .map_err(Into::into)
    }

    /// Retrieves the node's current status (role, leader, store snapshot).
    pub fn status(&self) -> Result<NodeStatus> {
        let (resp_tx, resp_rx) = unbounded();
        self.request_tx
            .send(ClientRequest::Status {
                respond_to: resp_tx,
            })
            .context("failed to send status request")?;
        resp_rx
            .recv()
            .context("status response channel closed")
            .map_err(Into::into)
    }

    /// Forces this node to start an election campaign.
    ///
    /// This can be used to:
    /// - Trigger an election manually for testing
    /// - Simulate a leader crash (call from current leader)
    /// - Simulate timeout detection (call from follower)
    /// - Force a specific node to try becoming leader
    ///
    /// # Behavior by role
    /// - **Follower**: Transitions to Candidate and starts election
    /// - **Candidate**: Restarts the election with a new term
    /// - **Leader**: Steps down and starts a new election (simulates crash)
    ///
    /// The election may succeed or fail depending on votes from peers.
    pub fn campaign(&self) -> Result<String> {
        let (resp_tx, resp_rx) = unbounded();
        self.request_tx
            .send(ClientRequest::Campaign {
                respond_to: resp_tx,
            })
            .context("failed to send campaign request")?;
        resp_rx.recv().context("campaign response channel closed")?
    }

    /// Signals the worker to shut down gracefully.
    pub fn shutdown(&self) -> Result<()> {
        self.request_tx
            .send(ClientRequest::Shutdown)
            .context("failed to send shutdown")?;
        Ok(())
    }
}

/// Snapshot of a node's current state.
///
/// Returned by the `STATUS` command to show Raft state and store contents.
pub struct NodeStatus {
    pub node_id: u64,
    pub role: StateRole,
    pub leader_id: u64,
    pub term: u64,
    pub store: BTreeMap<String, String>,
}

/// Requests sent from the main thread to the worker thread.
///
/// Uses the request-response pattern: most variants include a one-shot
/// channel for the worker to send back the result.
enum ClientRequest {
    Put {
        key: String,
        value: String,
        respond_to: Sender<Result<String>>,
    },
    Get {
        key: String,
        respond_to: Sender<Option<String>>,
    },
    Status {
        respond_to: Sender<NodeStatus>,
    },
    Campaign {
        respond_to: Sender<Result<String>>,
    },
    Shutdown,
}

/// Tracks a PUT request that's been proposed but not yet committed.
///
/// When the worker proposes a PUT to Raft, it stores this struct to
/// remember which client is waiting. When the entry is committed and
/// applied, we find the matching PendingPut and respond via its channel.
///
/// # Why this approach?
///
/// Raft doesn't give us request IDs - it only tells us "entry X was applied".
/// We match on (key, value) to find the right pending request. This works
/// for this demo but has limitations:
/// - Duplicate PUTs of the same (key, value) will confuse matching
/// - No way to correlate partial failures
///
/// A production system would embed request IDs in the command payload.
struct PendingPut {
    key: String,
    value: String,
    respond_to: Sender<Result<String>>,
}

/// Spawns a Raft node and returns a handle to interact with it.
///
/// This creates three components:
/// 1. Worker thread - runs the Raft event loop
/// 2. Network listener - accepts incoming connections from peers
/// 3. NodeHandle - allows the caller to send requests to the worker
/// 4. Log receiver - allows the caller to receive log messages for display
///
/// # Threading architecture
///
/// - Worker owns the `RaftNode` (single-threaded, no locks)
/// - Network listener runs in background, forwards messages via channel
/// - Caller uses handle to send requests via channel
/// - Caller receives log messages via returned channel
///
/// # Errors
///
/// Returns error if:
/// - `config.id` is not present in `config.peers`
/// - Network listener fails to bind to `config.listen_addr`
/// - Failed to create RaftNode
pub fn spawn_node(config: NodeConfig) -> Result<(NodeHandle, Receiver<LogMessage>)> {
    let voters: Vec<u64> = {
        let mut v: Vec<u64> = config.peers.keys().copied().collect();
        v.sort_unstable();
        v
    };
    if !config.peers.contains_key(&config.id) {
        return Err(anyhow!(
            "listen node id {} missing from peers map",
            config.id
        ));
    }

    let node = RaftNode::new(config.id, &voters)?;
    let (client_tx, client_rx) = unbounded();
    let (network_tx, network_rx) = unbounded();
    let (log_tx, log_rx) = unbounded();

    spawn_network_listener(config.listen_addr.clone(), network_tx, log_tx.clone())?;

    let log_tx_for_worker = log_tx.clone();
    thread::Builder::new()
        .name(format!("raft-worker-{}", config.id))
        .spawn(move || {
            if let Err(err) = Worker::new(node, config.peers, client_rx, network_rx, log_tx_for_worker).run() {
                // Send critical error to log channel before thread exits
                let _ = log_tx.send(LogMessage::Error(
                    format!("CRITICAL: raft worker crashed: {err:?}")
                ));
            }
        })
        .expect("failed to spawn raft worker");

    Ok((NodeHandle {
        request_tx: client_tx,
    }, log_rx))
}

/// The worker that runs the Raft event loop.
///
/// Owns the RaftNode and processes three types of events:
/// 1. **Client requests** (PUT, GET, STATUS) from the main thread
/// 2. **Network messages** (Raft protocol) from peers
/// 3. **Tick events** (every 100ms) to drive Raft timeouts
///
/// The event loop uses `crossbeam_channel::select!` to wait on multiple
/// channels simultaneously, processing whichever event arrives first.
struct Worker {
    node: RaftNode,
    peers: HashMap<u64, String>,
    client_rx: Receiver<ClientRequest>,
    network_rx: Receiver<Message>,
    log_tx: Sender<LogMessage>,
    pending_puts: Vec<PendingPut>,
    last_role: StateRole,
}

impl Worker {
    fn new(
        node: RaftNode,
        peers: HashMap<u64, String>,
        client_rx: Receiver<ClientRequest>,
        network_rx: Receiver<Message>,
        log_tx: Sender<LogMessage>,
    ) -> Self {
        let last_role = node.role();
        Self {
            node,
            peers,
            client_rx,
            network_rx,
            log_tx,
            pending_puts: Vec::new(),
            last_role,
        }
    }

    /// Runs the main event loop until shutdown.
    ///
    /// # Event processing order
    ///
    /// Each iteration:
    /// 1. Wait (with timeout) for client request or network message
    /// 2. Check if it's time to tick (every 100ms)
    /// 3. Process any Ready state from Raft
    /// 4. Log role changes (for debugging)
    ///
    /// # Why this order?
    ///
    /// - Process events immediately (responsive to clients and peers)
    /// - Tick only when needed (don't spin-loop wasting CPU)
    /// - Always check Ready after any Raft interaction (messages, ticks)
    /// - Role logging at end (doesn't block event processing)
    fn run(&mut self) -> Result<()> {
        let mut last_tick = Instant::now();
        loop {
            // Calculate time until next tick
            let timeout = TICK_INTERVAL
                .checked_sub(last_tick.elapsed())
                .unwrap_or(Duration::from_secs(0));

            // Wait for events with timeout
            crossbeam_channel::select! {
                recv(self.client_rx) -> req => {
                    match req {
                        Ok(req) => {
                            if !self.handle_client_request(req)? {
                                break; // Shutdown requested
                            }
                        }
                        Err(_) => break, // Main thread dropped the sender
                    }
                }
                recv(self.network_rx) -> msg => {
                    if let Ok(msg) = msg {
                        self.node.step(msg)?;
                    } else {
                        break; // Network listener died
                    }
                }
                default(timeout) => {}
            }

            // Drive Raft's logical clock
            if last_tick.elapsed() >= TICK_INTERVAL {
                self.node.tick();
                last_tick = Instant::now();
            }

            self.process_ready()?;
            self.log_role_change();
        }

        Ok(())
    }

    /// Handles a request from the main thread.
    ///
    /// Returns `false` if shutdown was requested, `true` otherwise.
    ///
    /// # Request handling
    ///
    /// - **PUT**: Propose to Raft, track as pending
    /// - **GET**: Read local state, respond immediately
    /// - **STATUS**: Snapshot state, respond immediately
    /// - **CAMPAIGN**: Force election, respond with result
    /// - **Shutdown**: Signal loop exit
    fn handle_client_request(&mut self, req: ClientRequest) -> Result<bool> {
        match req {
            ClientRequest::Put {
                key,
                value,
                respond_to,
            } => {
                let payload = CommandPayload::Put {
                    key: key.clone(),
                    value: value.clone(),
                };
                if let Err(err) = self.node.propose(&payload) {
                    let _ = respond_to.send(Err(err));
                } else {
                    self.pending_puts.push(PendingPut {
                        key,
                        value,
                        respond_to,
                    });
                }
            }
            ClientRequest::Get { key, respond_to } => {
                let _ = respond_to.send(self.node.value_for(&key));
            }
            ClientRequest::Status { respond_to } => {
                let status = NodeStatus {
                    node_id: self.node.id(),
                    role: self.node.role(),
                    leader_id: self.node.leader_id(),
                    term: self.node.term(),
                    store: self.node.snapshot(),
                };
                let _ = respond_to.send(status);
            }
            ClientRequest::Campaign { respond_to } => {
                let old_role = self.node.role();
                match self.node.campaign() {
                    Ok(()) => {
                        let msg = format!(
                            "Campaign initiated! Previous role: {:?}, starting election...",
                            old_role
                        );
                        let _ = self.log_tx.send(LogMessage::Info(
                            format!("[node {}] {}", self.node.id(), msg)
                        ));
                        let _ = respond_to.send(Ok(msg));
                    }
                    Err(err) => {
                        let _ = respond_to.send(Err(err));
                    }
                }
            }
            ClientRequest::Shutdown => return Ok(false),
        }
        Ok(true)
    }

    /// Drains all ready state from Raft and processes it.
    ///
    /// Raft may produce multiple Ready batches in quick succession
    /// (e.g., processing a burst of incoming messages). We loop until
    /// `poll_ready()` returns None to ensure we're fully caught up.
    ///
    /// # Processing order
    ///
    /// 1. **Dispatch messages** to peers (or back to self)
    /// 2. **Notify waiting clients** about applied PUTs
    ///
    /// We send messages before notifying clients to maximize replication
    /// throughput. The client doesn't care about microseconds of latency,
    /// but Raft benefits from batching network sends.
    fn process_ready(&mut self) -> Result<()> {
        while let Some(bundle) = self.node.poll_ready()? {
            for msg in bundle.messages {
                self.dispatch_message(msg)?;
            }
            for report in bundle.applied {
                self.notify_put(report);
            }
        }
        Ok(())
    }

    /// Sends a Raft message to its destination.
    ///
    /// Messages addressed to this node are fed back into `node.step()`.
    /// Messages for other nodes are sent over the network.
    ///
    /// If the destination address is unknown (shouldn't happen in normal
    /// operation), we log and drop the message rather than crashing.
    fn dispatch_message(&mut self, msg: Message) -> Result<()> {
        if msg.to == self.node.id() {
            self.node.step(msg)?;
            return Ok(());
        }

        let to = msg.to;
        let Some(addr) = self.peers.get(&to) else {
            let _ = self.log_tx.send(LogMessage::Error(
                format!("no address for peer {to}, dropping message")
            ));
            return Ok(());
        };
        send_message(addr, &msg, &self.log_tx);
        Ok(())
    }

    /// Logs applied entries and notifies any waiting clients.
    ///
    /// This function handles two responsibilities:
    /// 1. **Log all applied entries** (for followers and leaders alike)
    /// 2. **Notify waiting clients** (only on leader that proposed the entry)
    ///
    /// Followers will log applied entries but won't have matching pending PUTs,
    /// so the client notification step is skipped.
    ///
    /// # Why linear search for pending PUTs?
    ///
    /// We expect few pending PUTs at a time (typical workload: 1-10).
    /// Linear search is simpler than maintaining a HashMap and faster
    /// for small N.
    ///
    /// For high-throughput systems with hundreds of pending requests,
    /// consider a HashMap keyed by request ID embedded in the command.
    fn notify_put(&mut self, report: ApplyReport) {
        // Always log applied entries (for both followers and leaders)
        let msg = format!(
            "[node {}] applied key={} value={} (index {}, term {})",
            self.node.id(),
            report.key,
            report.value,
            report.index,
            report.term
        );
        let _ = self.log_tx.send(LogMessage::Info(msg));

        // Additionally notify waiting client if this node proposed the entry
        if let Some(index) = self
            .pending_puts
            .iter()
            .position(|pending| pending.key == report.key && pending.value == report.value)
        {
            let pending = self.pending_puts.remove(index);
            let _ = pending.respond_to.send(Ok(report.value));
        }
    }

    /// Logs a message when the node's role changes.
    ///
    /// Helps with debugging and understanding cluster dynamics.
    /// Sends the message through the log channel for display in the TUI.
    fn log_role_change(&mut self) {
        let current = self.node.role();
        if current != self.last_role {
            let msg = format!(
                "[node {}] role changed {:?} -> {:?} (leader: {})",
                self.node.id(),
                self.last_role,
                current,
                self.node.leader_id()
            );
            let _ = self.log_tx.send(LogMessage::Info(msg));
            self.last_role = current;
        }
    }
}

/// Spawns a background thread that listens for incoming Raft messages.
///
/// For each connection:
/// 1. Accept the connection
/// 2. Spawn a short-lived handler thread
/// 3. Handler reads one message and forwards to worker via `tx`
/// 4. Handler exits
///
/// # Why one thread per connection?
///
/// Simplicity. Each handler does a single blocking read + channel send,
/// then exits. No need for async I/O or connection pooling.
///
/// A production system would use async I/O (tokio, async-std) to handle
/// thousands of concurrent connections efficiently.
fn spawn_network_listener(addr: String, tx: Sender<Message>, log_tx: Sender<LogMessage>) -> Result<()> {
    thread::Builder::new()
        .name(format!("raft-net-listener-{addr}"))
        .spawn(move || {
            let listener = TcpListener::bind(&addr)
                .unwrap_or_else(|err| panic!("failed to bind {addr}: {err}"));
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let tx = tx.clone();
                        let log_tx = log_tx.clone();
                        thread::spawn(move || {
                            if let Err(err) = handle_connection(stream, tx) {
                                let _ = log_tx.send(LogMessage::Error(
                                    format!("connection error: {err}")
                                ));
                            }
                        });
                    }
                    Err(err) => {
                        let _ = log_tx.send(LogMessage::Error(
                            format!("accept error: {err}")
                        ));
                    }
                }
            }
        })
        .map(|_| ())
        .context("failed to spawn network listener")
}

/// Reads a single Raft message from a TCP connection and forwards to the worker.
///
/// # Protocol
///
/// Messages are length-prefixed:
/// - 4 bytes: message length (big-endian u32)
/// - N bytes: protobuf-encoded Message
///
/// This simple framing protocol allows reading the exact number of bytes
/// needed without scanning for delimiters or buffering partial messages.
fn handle_connection(mut stream: TcpStream, tx: Sender<Message>) -> Result<()> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    let msg =
        Message::decode(&buf[..]).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    tx.send(msg)
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "worker gone"))?;
    Ok(())
}

/// Sends a Raft message to a peer.
///
/// Creates a new TCP connection for each message (fire-and-forget).
/// Errors are logged to the TUI via the log channel.
///
/// # Why not connection pooling?
///
/// Raft messages are relatively infrequent (heartbeats every 300ms,
/// AppendEntries on demand). The overhead of TCP handshake is acceptable
/// for this demo.
///
/// Production systems would maintain persistent connections to peers
/// to reduce latency and connection overhead.
fn send_message(addr: &str, msg: &Message, log_tx: &Sender<LogMessage>) {
    let bytes = msg.encode_to_vec();
    if let Err(err) = try_send(addr, &bytes) {
        let _ = log_tx.send(LogMessage::Error(
            format!("failed to send message to {addr}: {err}")
        ));
    }
}

/// Attempts to send bytes to an address using the length-prefixed protocol.
///
/// Opens a connection, writes length prefix + message, closes connection.
fn try_send(addr: &str, bytes: &[u8]) -> io::Result<()> {
    use std::net::TcpStream;
    let mut stream = TcpStream::connect(addr)?;
    let len = bytes.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(bytes)?;
    Ok(())
}
