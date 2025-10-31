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
    let (mut reader, mut writer) = establish_connection(&args).await?;
    send_handshake(&mut writer, &args.nickname).await?;

    let mut stdin = BufReader::new(tokio::io::stdin());
    let mut input = String::new();

    run_client_loop(&mut reader, &mut writer, &mut stdin, &mut input).await?;
    shutdown_connection(&mut writer).await;

    Ok(())
}

async fn establish_connection(
    args: &ClientArgs,
) -> Result<(
    BufReader<tokio::net::tcp::OwnedReadHalf>,
    tokio::net::tcp::OwnedWriteHalf,
)> {
    let stream = TcpStream::connect(args.server)
        .await
        .with_context(|| format!("failed to connect to {}", args.server))?;

    info!("connected to {}", args.server);

    let (reader, writer) = stream.into_split();
    Ok((BufReader::new(reader), writer))
}

async fn send_handshake(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    nickname: &str,
) -> Result<()> {
    write_message(
        writer,
        &ClientToServer::Hello {
            nickname: nickname.to_string(),
        },
    )
    .await?;
    Ok(())
}

async fn run_client_loop(
    reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    stdin: &mut BufReader<tokio::io::Stdin>,
    input: &mut String,
) -> Result<()> {
    loop {
        input.clear();
        select! {
            server_message = read_message::<_, ServerToClient>(reader) => {
                if !handle_server_message(server_message).await? {
                    break;
                }
            }
            bytes_read = stdin.read_line(input) => {
                if !handle_stdin_input(bytes_read, input, writer).await? {
                    break;
                }
            }
            ctrl_c = tokio::signal::ctrl_c() => {
                handle_ctrl_c(ctrl_c);
                break;
            }
        }
    }
    Ok(())
}

async fn handle_server_message(message: io::Result<Option<ServerToClient>>) -> Result<bool> {
    match message? {
        Some(message) => {
            render_server_message(message).await?;
            Ok(true)
        }
        None => {
            write_stdout("*** server closed the connection").await?;
            Ok(false)
        }
    }
}

async fn handle_stdin_input(
    bytes_read: io::Result<usize>,
    input: &str,
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
) -> Result<bool> {
    let bytes_read = bytes_read?;
    if bytes_read == 0 {
        return Ok(false);
    }

    let text = input.trim_end();
    if text.is_empty() {
        return Ok(true);
    }

    if text.eq_ignore_ascii_case("/quit") {
        write_stdout("*** leaving chat").await?;
        return Ok(false);
    }

    write_message(
        writer,
        &ClientToServer::Chat {
            text: text.to_string(),
        },
    )
    .await?;
    Ok(true)
}

fn handle_ctrl_c(result: io::Result<()>) {
    if let Err(error) = result {
        warn!(?error, "ctrl-c handler failed");
    }
}

async fn shutdown_connection(writer: &mut tokio::net::tcp::OwnedWriteHalf) {
    if let Err(error) = writer.shutdown().await {
        warn!(?error, "failed to shutdown client writer cleanly");
    }
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
