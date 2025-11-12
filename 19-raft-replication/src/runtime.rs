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

const TICK_INTERVAL: Duration = Duration::from_millis(100);

pub struct NodeConfig {
    pub id: u64,
    pub listen_addr: String,
    pub peers: HashMap<u64, String>,
}

pub struct NodeHandle {
    request_tx: Sender<ClientRequest>,
}

impl NodeHandle {
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

    pub fn shutdown(&self) -> Result<()> {
        self.request_tx
            .send(ClientRequest::Shutdown)
            .context("failed to send shutdown")?;
        Ok(())
    }
}

pub struct NodeStatus {
    pub node_id: u64,
    pub role: StateRole,
    pub leader_id: u64,
    pub store: BTreeMap<String, String>,
}

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
    Shutdown,
}

struct PendingPut {
    key: String,
    value: String,
    respond_to: Sender<Result<String>>,
}

pub fn spawn_node(config: NodeConfig) -> Result<NodeHandle> {
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

    spawn_network_listener(config.listen_addr.clone(), network_tx)?;

    thread::Builder::new()
        .name(format!("raft-worker-{}", config.id))
        .spawn(move || {
            if let Err(err) = Worker::new(node, config.peers, client_rx, network_rx).run() {
                eprintln!("raft worker crashed: {err:?}");
            }
        })
        .expect("failed to spawn raft worker");

    Ok(NodeHandle {
        request_tx: client_tx,
    })
}

struct Worker {
    node: RaftNode,
    peers: HashMap<u64, String>,
    client_rx: Receiver<ClientRequest>,
    network_rx: Receiver<Message>,
    pending_puts: Vec<PendingPut>,
}

impl Worker {
    fn new(
        node: RaftNode,
        peers: HashMap<u64, String>,
        client_rx: Receiver<ClientRequest>,
        network_rx: Receiver<Message>,
    ) -> Self {
        Self {
            node,
            peers,
            client_rx,
            network_rx,
            pending_puts: Vec::new(),
        }
    }

    fn run(&mut self) -> Result<()> {
        let mut last_tick = Instant::now();
        loop {
            let timeout = TICK_INTERVAL
                .checked_sub(last_tick.elapsed())
                .unwrap_or(Duration::from_secs(0));

            crossbeam_channel::select! {
                recv(self.client_rx) -> req => {
                    match req {
                        Ok(req) => {
                            if !self.handle_client_request(req)? {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                recv(self.network_rx) -> msg => {
                    if let Ok(msg) = msg {
                        self.node.step(msg)?;
                    } else {
                        break;
                    }
                }
                default(timeout) => {}
            }

            if last_tick.elapsed() >= TICK_INTERVAL {
                self.node.tick();
                last_tick = Instant::now();
            }

            self.process_ready()?;
        }

        Ok(())
    }

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
                    store: self.node.snapshot(),
                };
                let _ = respond_to.send(status);
            }
            ClientRequest::Shutdown => return Ok(false),
        }
        Ok(true)
    }

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

    fn dispatch_message(&mut self, msg: Message) -> Result<()> {
        if msg.to == self.node.id() {
            self.node.step(msg)?;
            return Ok(());
        }

        let to = msg.to;
        let Some(addr) = self.peers.get(&to) else {
            eprintln!("no address for peer {to}, dropping message");
            return Ok(());
        };
        send_message(addr, &msg);
        Ok(())
    }

    fn notify_put(&mut self, report: ApplyReport) {
        if let Some(index) = self
            .pending_puts
            .iter()
            .position(|pending| pending.key == report.key && pending.value == report.value)
        {
            let pending = self.pending_puts.remove(index);
            let _ = pending.respond_to.send(Ok(report.value));
        }
    }
}

fn spawn_network_listener(addr: String, tx: Sender<Message>) -> Result<()> {
    thread::Builder::new()
        .name(format!("raft-net-listener-{addr}"))
        .spawn(move || {
            let listener = TcpListener::bind(&addr)
                .unwrap_or_else(|err| panic!("failed to bind {addr}: {err}"));
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let tx = tx.clone();
                        thread::spawn(move || {
                            if let Err(err) = handle_connection(stream, tx) {
                                eprintln!("connection error: {err}");
                            }
                        });
                    }
                    Err(err) => eprintln!("accept error: {err}"),
                }
            }
        })
        .map(|_| ())
        .context("failed to spawn network listener")
}

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

fn send_message(addr: &str, msg: &Message) {
    let bytes = msg.encode_to_vec();
    if let Err(err) = try_send(addr, &bytes) {
        eprintln!("failed to send message to {addr}: {err}");
    }
}

fn try_send(addr: &str, bytes: &[u8]) -> io::Result<()> {
    use std::net::TcpStream;
    let mut stream = TcpStream::connect(addr)?;
    let len = bytes.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(bytes)?;
    Ok(())
}
