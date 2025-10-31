use anyhow::{Context, Result};
use tokio::{
    io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    select,
};
use tracing::{info, warn};

use crate::{
    cli::ClientArgs,
    message::{ClientToServer, ServerToClient, read_message, write_message},
};

pub async fn run(args: ClientArgs) -> Result<()> {
    let stream = TcpStream::connect(args.server)
        .await
        .with_context(|| format!("failed to connect to {}", args.server))?;

    info!("connected to {}", args.server);

    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut stdin = BufReader::new(tokio::io::stdin());
    let mut input = String::new();

    // Reserve our nickname before we start reading stdin to avoid echo races.
    write_message(
        &mut writer,
        &ClientToServer::Hello {
            nickname: args.nickname.clone(),
        },
    )
    .await?;

    loop {
        input.clear();
        select! {
            server_message = read_message::<_, ServerToClient>(&mut reader) => {
                match server_message? {
                    // Server drives most updates; render them immediately for a responsive TUI.
                    Some(message) => render_server_message(message).await?,
                    None => {
                        write_stdout("*** server closed the connection").await?;
                        break;
                    }
                }
            }
            bytes_read = stdin.read_line(&mut input) => {
                let bytes_read = bytes_read?;
                if bytes_read == 0 {
                    break;
                }

                let text = input.trim_end().to_string();
                if text.is_empty() {
                    continue;
                }

                if text.eq_ignore_ascii_case("/quit") {
                    write_stdout("*** leaving chat").await?;
                    break;
                }

                // Fan our input upstream; broker handles validation and rebroadcasting.
                write_message(
                    &mut writer,
                    &ClientToServer::Chat {
                        text,
                    },
                ).await?;
            }
            ctrl_c = tokio::signal::ctrl_c() => {
                if let Err(error) = ctrl_c {
                    warn!(?error, "ctrl-c handler failed");
                }
                break;
            }
        }
    }

    if let Err(error) = writer.shutdown().await {
        // Socket is already half-closed, so a failure here is purely informational.
        warn!(?error, "failed to shutdown client writer cleanly");
    }

    Ok(())
}

async fn render_server_message(message: ServerToClient) -> io::Result<()> {
    match message {
        ServerToClient::Welcome { nickname } => {
            write_stdout(&format!("*** connected as {nickname}")).await
        }
        ServerToClient::Roster { participants } => {
            if participants.is_empty() {
                return Ok(());
            }
            write_stdout(&format!(
                "*** currently online: {}",
                participants.join(", ")
            ))
            .await
        }
        ServerToClient::UserJoined { nickname } => {
            write_stdout(&format!("*** {nickname} joined the chat")).await
        }
        ServerToClient::UserLeft { nickname } => {
            write_stdout(&format!("*** {nickname} left the chat")).await
        }
        ServerToClient::Chat { nickname, text } => {
            write_stdout(&format!("<{nickname}> {text}")).await
        }
        ServerToClient::Error { message } => write_stderr(&format!("!!! {message}")).await,
    }
}

async fn write_stdout(line: &str) -> io::Result<()> {
    let mut stdout = tokio::io::stdout();
    stdout.write_all(line.as_bytes()).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await
}

async fn write_stderr(line: &str) -> io::Result<()> {
    let mut stderr = tokio::io::stderr();
    stderr.write_all(line.as_bytes()).await?;
    stderr.write_all(b"\n").await?;
    stderr.flush().await
}
