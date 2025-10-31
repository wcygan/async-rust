use std::{
    collections::HashMap,
    future::Future,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::Result;
use tokio::{
    io::BufReader,
    net::{TcpListener, TcpStream},
    select,
    sync::{Mutex, broadcast},
};
use tracing::{debug, info, warn};

use crate::message::{ClientToServer, ServerToClient, read_message, write_message};

type ClientId = u64;

pub struct Broker {
    listener: TcpListener,
    state: Arc<BrokerState>,
}

impl Broker {
    pub fn new(listener: TcpListener) -> Self {
        Self {
            listener,
            state: Arc::new(BrokerState::new()),
        }
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub async fn run_until<F>(self, shutdown: F) -> Result<()>
    where
        F: Future<Output = ()> + Send,
    {
        let Broker { listener, state } = self;
        tokio::pin!(shutdown);

        loop {
            select! {
                _ = &mut shutdown => {
                    // Tell connected clients why the broker is disappearing so they can exit cleanly.
                    info!("broker shutting down");
                    state.broadcast(ServerToClient::Error {
                        message: "broker shutting down".to_string(),
                    });
                    break;
                }
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, peer)) => {
                            let state = Arc::clone(&state);
                            // Spin each connection on its own task so slow clients do not block new accepts.
                            tokio::spawn(async move {
                                if let Err(err) = handle_connection(stream, state).await {
                                    warn!(peer = %peer, error = ?err, "client connection closed with error");
                                }
                            });
                        }
                        Err(err) => {
                            warn!(error = ?err, "failed to accept connection");
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn run_until_ctrl_c(self) -> Result<()> {
        self.run_until(async {
            if let Err(err) = tokio::signal::ctrl_c().await {
                warn!(error = ?err, "failed to install ctrl-c handler");
            }
        })
        .await
    }
}

struct BrokerState {
    clients: Mutex<HashMap<ClientId, ClientRecord>>,
    broadcaster: broadcast::Sender<ServerToClient>,
    next_id: AtomicU64,
}

#[derive(Clone)]
struct ClientRecord {
    nickname: String,
}

impl BrokerState {
    fn new() -> Self {
        // Broadcast channel buffers a modest number of messages before lagging clients get warned.
        let (broadcaster, _) = broadcast::channel(128);
        Self {
            clients: Mutex::new(HashMap::new()),
            broadcaster,
            next_id: AtomicU64::new(1),
        }
    }

    fn next_id(&self) -> ClientId {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn register_client(
        &self,
        id: ClientId,
        nickname: String,
    ) -> Result<Vec<String>, RegisterClientError> {
        let mut clients = self.clients.lock().await;

        if clients.values().any(|client| client.nickname == nickname) {
            return Err(RegisterClientError::NicknameTaken);
        }

        let roster = clients
            .values()
            .map(|client| client.nickname.clone())
            .collect();

        clients.insert(id, ClientRecord { nickname });
        Ok(roster)
    }

    async fn remove_client(&self, id: ClientId) -> Option<ClientRecord> {
        let mut clients = self.clients.lock().await;
        clients.remove(&id)
    }

    fn broadcast(&self, message: ServerToClient) {
        if let Err(error) = self.broadcaster.send(message) {
            warn!(?error, "failed to broadcast message");
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<ServerToClient> {
        self.broadcaster.subscribe()
    }
}

#[derive(Debug)]
enum RegisterClientError {
    NicknameTaken,
}

async fn handle_connection(stream: TcpStream, state: Arc<BrokerState>) -> Result<()> {
    let peer = stream.peer_addr().ok();
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    let hello = match read_message::<_, ClientToServer>(&mut reader).await? {
        Some(message) => message,
        None => return Ok(()),
    };

    let nickname = match hello {
        ClientToServer::Hello { nickname } => nickname.trim().to_string(),
        _ => {
            write_message(
                &mut writer,
                &ServerToClient::Error {
                    message: "expected hello message first".to_string(),
                },
            )
            .await?;
            return Ok(());
        }
    };

    if nickname.is_empty() {
        write_message(
            &mut writer,
            &ServerToClient::Error {
                message: "nickname cannot be empty".to_string(),
            },
        )
        .await?;
        return Ok(());
    }

    let client_id = state.next_id();
    let roster = match state.register_client(client_id, nickname.clone()).await {
        Ok(roster) => roster,
        Err(RegisterClientError::NicknameTaken) => {
            write_message(
                &mut writer,
                &ServerToClient::Error {
                    message: format!("nickname '{nickname}' is already in use"),
                },
            )
            .await?;
            return Ok(());
        }
    };

    write_message(
        &mut writer,
        &ServerToClient::Welcome {
            nickname: nickname.clone(),
        },
    )
    .await?;

    if !roster.is_empty() {
        write_message(
            &mut writer,
            &ServerToClient::Roster {
                participants: roster,
            },
        )
        .await?;
    }

    info!(?peer, nickname, "client joined");

    state.broadcast(ServerToClient::UserJoined {
        nickname: nickname.clone(),
    });

    let mut inbox = state.subscribe();

    loop {
        select! {
            client_message = read_message::<_, ClientToServer>(&mut reader) => {
                match client_message? {
                    Some(ClientToServer::Chat { text }) => {
                        // Skip empty lines to avoid spamming blank chat messages.
                        if text.trim().is_empty() {
                            continue;
                        }
                        state.broadcast(ServerToClient::Chat {
                            nickname: nickname.clone(),
                            text,
                        });
                    }
                    Some(ClientToServer::Hello { .. }) => {
                        write_message(
                            &mut writer,
                            &ServerToClient::Error {
                                message: "already connected".to_string(),
                            },
                        )
                        .await?;
                    }
                    None => break,
                }
            }
            broadcast_message = inbox.recv() => {
                match broadcast_message {
                    Ok(message) => {
                        // Forward the broadcast; failure means the socket is unhealthy so we drop it.
                        if let Err(err) = write_message(&mut writer, &message).await {
                            debug!(?err, "failed to deliver message to client");
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        // Let the client know they fell behind so they can reconnect if desired.
                        let warning = ServerToClient::Error {
                            message: format!(
                                "you are behind by {skipped} messages; consider reconnecting"
                            ),
                        };
                        if let Err(err) = write_message(&mut writer, &warning).await {
                            debug!(?err, "failed to notify client about lag");
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }

    if let Some(ClientRecord { nickname }) = state.remove_client(client_id).await {
        info!(?peer, %nickname, "client disconnected");
        state.broadcast(ServerToClient::UserLeft { nickname });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::ServerToClient;

    #[tokio::test]
    async fn state_rejects_duplicate_nicknames() {
        let state = BrokerState::new();
        let id_a = state.next_id();
        state
            .register_client(id_a, "alice".into())
            .await
            .expect("first registration should pass");
        let id_b = state.next_id();
        let result = state.register_client(id_b, "alice".into()).await;
        assert!(matches!(result, Err(RegisterClientError::NicknameTaken)));
    }

    #[tokio::test]
    async fn broadcast_delivers_to_multiple_clients() {
        let state = BrokerState::new();
        let mut rx_one = state.subscribe();
        let mut rx_two = state.subscribe();

        state.broadcast(ServerToClient::UserJoined {
            nickname: "alice".into(),
        });

        let first = rx_one.recv().await.expect("first receiver");
        let second = rx_two.recv().await.expect("second receiver");

        assert_eq!(
            first,
            ServerToClient::UserJoined {
                nickname: "alice".into()
            }
        );
        assert_eq!(first, second);
    }
}
