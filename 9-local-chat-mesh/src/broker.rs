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
                    handle_shutdown(&state);
                    break;
                }
                accept_result = listener.accept() => {
                    handle_accept_result(accept_result, &state);
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

fn handle_shutdown(state: &Arc<BrokerState>) {
    info!("broker shutting down");
    state.broadcast(ServerToClient::Error {
        message: "broker shutting down".to_string(),
    });
}

fn handle_accept_result(
    result: std::io::Result<(TcpStream, SocketAddr)>,
    state: &Arc<BrokerState>,
) {
    match result {
        Ok((stream, peer)) => spawn_client_handler(stream, peer, state),
        Err(err) => warn!(error = ?err, "failed to accept connection"),
    }
}

fn spawn_client_handler(stream: TcpStream, peer: SocketAddr, state: &Arc<BrokerState>) {
    let state = Arc::clone(state);
    tokio::spawn(async move {
        if let Err(err) = handle_connection(stream, state).await {
            warn!(peer = %peer, error = ?err, "client connection closed with error");
        }
    });
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
    let (reader, writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut writer = writer;

    let nickname = perform_handshake(&mut reader, &mut writer).await?;
    let client_id = register_and_welcome(&state, &mut writer, &nickname).await?;

    info!(?peer, nickname, "client joined");
    state.broadcast(ServerToClient::UserJoined {
        nickname: nickname.clone(),
    });

    run_client_session(&state, &mut reader, &mut writer, &nickname).await?;
    cleanup_client_disconnect(&state, client_id, peer).await;

    Ok(())
}

async fn perform_handshake<R, W>(reader: &mut R, writer: &mut W) -> Result<String>
where
    R: tokio::io::AsyncBufRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let hello = match read_message::<_, ClientToServer>(reader).await? {
        Some(message) => message,
        None => anyhow::bail!("connection closed before handshake"),
    };

    let nickname = extract_nickname(hello)?;
    validate_nickname(&nickname, writer).await?;

    Ok(nickname)
}

fn extract_nickname(message: ClientToServer) -> Result<String> {
    match message {
        ClientToServer::Hello { nickname } => Ok(nickname.trim().to_string()),
        _ => anyhow::bail!("expected hello message first"),
    }
}

async fn validate_nickname<W>(nickname: &str, writer: &mut W) -> Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    if nickname.is_empty() {
        write_message(
            writer,
            &ServerToClient::Error {
                message: "nickname cannot be empty".to_string(),
            },
        )
        .await?;
        anyhow::bail!("nickname cannot be empty");
    }
    Ok(())
}

async fn register_and_welcome<W>(
    state: &BrokerState,
    writer: &mut W,
    nickname: &str,
) -> Result<ClientId>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let client_id = state.next_id();
    let roster = match state.register_client(client_id, nickname.to_string()).await {
        Ok(roster) => roster,
        Err(RegisterClientError::NicknameTaken) => {
            write_message(
                writer,
                &ServerToClient::Error {
                    message: format!("nickname '{nickname}' is already in use"),
                },
            )
            .await?;
            anyhow::bail!("nickname already taken");
        }
    };

    send_welcome_messages(writer, nickname, roster).await?;
    Ok(client_id)
}

async fn send_welcome_messages<W>(writer: &mut W, nickname: &str, roster: Vec<String>) -> Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    write_message(
        writer,
        &ServerToClient::Welcome {
            nickname: nickname.to_string(),
        },
    )
    .await?;

    if !roster.is_empty() {
        write_message(
            writer,
            &ServerToClient::Roster {
                participants: roster,
            },
        )
        .await?;
    }

    Ok(())
}

async fn run_client_session<R, W>(
    state: &BrokerState,
    reader: &mut R,
    writer: &mut W,
    nickname: &str,
) -> Result<()>
where
    R: tokio::io::AsyncBufRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut inbox = state.subscribe();

    loop {
        select! {
            client_message = read_message::<_, ClientToServer>(reader) => {
                if !handle_client_message(client_message, writer, state, nickname).await? {
                    break;
                }
            }
            broadcast_message = inbox.recv() => {
                if !handle_broadcast_message(broadcast_message, writer).await? {
                    break;
                }
            }
        }
    }

    Ok(())
}

async fn handle_client_message<W>(
    message: Result<Option<ClientToServer>, std::io::Error>,
    writer: &mut W,
    state: &BrokerState,
    nickname: &str,
) -> Result<bool>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    match message? {
        Some(ClientToServer::Chat { text }) => {
            if !text.trim().is_empty() {
                state.broadcast(ServerToClient::Chat {
                    nickname: nickname.to_string(),
                    text,
                });
            }
            Ok(true)
        }
        Some(ClientToServer::Hello { .. }) => {
            write_message(
                writer,
                &ServerToClient::Error {
                    message: "already connected".to_string(),
                },
            )
            .await?;
            Ok(true)
        }
        None => Ok(false),
    }
}

async fn handle_broadcast_message<W>(
    message: Result<ServerToClient, broadcast::error::RecvError>,
    writer: &mut W,
) -> Result<bool>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    match message {
        Ok(message) => {
            if let Err(err) = write_message(writer, &message).await {
                debug!(?err, "failed to deliver message to client");
                return Ok(false);
            }
            Ok(true)
        }
        Err(broadcast::error::RecvError::Lagged(skipped)) => {
            let warning = ServerToClient::Error {
                message: format!("you are behind by {skipped} messages; consider reconnecting"),
            };
            if let Err(err) = write_message(writer, &warning).await {
                debug!(?err, "failed to notify client about lag");
                return Ok(false);
            }
            Ok(true)
        }
        Err(broadcast::error::RecvError::Closed) => Ok(false),
    }
}

async fn cleanup_client_disconnect(
    state: &BrokerState,
    client_id: ClientId,
    peer: Option<SocketAddr>,
) {
    if let Some(ClientRecord { nickname }) = state.remove_client(client_id).await {
        info!(?peer, %nickname, "client disconnected");
        state.broadcast(ServerToClient::UserLeft { nickname });
    }
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
