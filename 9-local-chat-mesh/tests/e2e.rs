use std::{path::Path, process::Stdio, time::Duration};

use anyhow::{Context, Result, anyhow};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};

const READ_TIMEOUT: Duration = Duration::from_secs(3);

#[tokio::test]
async fn cli_chat_end_to_end() -> Result<()> {
    let binary = assert_cmd::cargo::cargo_bin!("local_chat_mesh");

    let (mut broker_child, mut broker_stdout) = spawn_broker(&binary).await?;
    let addr = read_broker_addr(&mut broker_stdout).await?;

    // Drain additional broker logs in the background so the pipe never fills.
    let broker_log_task = tokio::spawn(async move {
        drain_stdout(broker_stdout).await;
    });

    let mut alice = spawn_client(&binary, "alice", &addr).await?;
    let mut bob = spawn_client(&binary, "bob", &addr).await?;

    // Bob should see Alice in the roster and Alice observes Bob's arrival.
    let bob_roster = read_line_expect(&mut bob.stdout, "waiting for bob roster").await?;
    assert_eq!(bob_roster, "*** currently online: alice");
    let alice_sees_bob =
        read_line_expect(&mut alice.stdout, "waiting for alice join notice").await?;
    assert_eq!(alice_sees_bob, "*** bob joined the chat");

    // Alice greets Bob; broadcast is delivered to both participants.
    alice
        .send_line("Hello from Alice")
        .await
        .context("alice send line")?;
    let bob_hears_alice =
        read_line_expect(&mut bob.stdout, "waiting for bob to hear alice").await?;
    assert_eq!(bob_hears_alice, "<alice> Hello from Alice");
    let alice_echo = read_line_expect(&mut alice.stdout, "waiting for alice echo").await?;
    assert_eq!(alice_echo, "<alice> Hello from Alice");

    // Bob replies and both clients see the message, including Bob's self-echo.
    bob.send_line("Hi Alice!").await.context("bob send line")?;
    let alice_hears_bob =
        read_line_expect(&mut alice.stdout, "waiting for alice to hear bob").await?;
    assert_eq!(alice_hears_bob, "<bob> Hi Alice!");
    let bob_echo = read_line_expect(&mut bob.stdout, "waiting for bob echo").await?;
    assert_eq!(bob_echo, "<bob> Hi Alice!");

    // Alice quits; Bob receives the departure notification.
    alice.send_line("/quit").await.context("alice send quit")?;
    let alice_quit =
        read_line_expect(&mut alice.stdout, "waiting for alice quit confirmation").await?;
    assert_eq!(alice_quit, "*** leaving chat");
    let bob_sees_departure =
        read_line_expect(&mut bob.stdout, "waiting for bob to see alice leave").await?;
    assert_eq!(bob_sees_departure, "*** alice left the chat");

    // Bob quits to wrap up the session.
    bob.send_line("/quit").await.context("bob send quit")?;
    let bob_quit = read_line_expect(&mut bob.stdout, "waiting for bob quit confirmation").await?;
    assert_eq!(bob_quit, "*** leaving chat");

    ensure_success(&mut alice.child, "alice client").await?;
    ensure_success(&mut bob.child, "bob client").await?;

    // Broker stays up after clients disconnect; terminate it manually.
    let _ = broker_child.kill().await;
    let _ = broker_child.wait().await;
    let _ = broker_log_task.await;

    Ok(())
}

struct ClientProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl ClientProcess {
    async fn send_line(&mut self, line: &str) -> Result<()> {
        self.stdin
            .write_all(line.as_bytes())
            .await
            .with_context(|| format!("failed to send line '{line}'"))?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }
}

async fn spawn_broker(binary: &Path) -> Result<(Child, BufReader<ChildStdout>)> {
    let mut cmd = Command::new(binary);
    cmd.arg("broker")
        .arg("--listen")
        .arg("127.0.0.1:0")
        .env("RUST_LOG_STYLE", "never")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = cmd.spawn().context("failed to spawn broker")?;
    let stdout = child
        .stdout
        .take()
        .context("broker stdout missing after spawn")?;

    Ok((child, BufReader::new(stdout)))
}

async fn read_broker_addr(reader: &mut BufReader<ChildStdout>) -> Result<String> {
    let line = read_line(reader)
        .await?
        .context("broker did not emit listening address")?;
    let trimmed = line.trim();
    let addr = trimmed
        .split_whitespace()
        .last()
        .context("unexpected broker banner format")?;
    if !addr.contains(':') {
        return Err(anyhow!("broker banner missing socket: {trimmed}"));
    }
    Ok(addr.to_string())
}

async fn spawn_client(binary: &Path, nickname: &str, addr: &str) -> Result<ClientProcess> {
    let mut cmd = Command::new(binary);
    cmd.arg("client")
        .arg("--nickname")
        .arg(nickname)
        .arg("--server")
        .arg(addr)
        .env("RUST_LOG", "warn")
        .env("RUST_LOG_STYLE", "never")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn client {nickname}"))?;

    let stdin = child
        .stdin
        .take()
        .context("client stdin missing after spawn")?;
    let stdout = child
        .stdout
        .take()
        .context("client stdout missing after spawn")?;

    let mut process = ClientProcess {
        child,
        stdin,
        stdout: BufReader::new(stdout),
    };

    let welcome = read_line_expect(&mut process.stdout, "waiting for welcome banner").await?;
    if welcome != format!("*** connected as {nickname}") {
        return Err(anyhow!(
            "expected welcome banner for {nickname}, got '{welcome}'"
        ));
    }

    Ok(process)
}

async fn read_line_expect(
    reader: &mut BufReader<ChildStdout>,
    description: &str,
) -> Result<String> {
    match read_line(reader).await {
        Ok(Some(line)) => Ok(line),
        Ok(None) => Err(anyhow!("{description}: stream closed")),
        Err(err) => Err(err.context(format!("{description}: failed to read line"))),
    }
}

async fn read_line(reader: &mut BufReader<ChildStdout>) -> Result<Option<String>> {
    let mut line = String::new();
    let read_future = reader.read_line(&mut line);
    let bytes_io = match timeout(READ_TIMEOUT, read_future).await {
        Ok(result) => result,
        Err(_) => return Err(anyhow!("timed out waiting for line")),
    };
    let byte_count = bytes_io?;
    if byte_count == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim_end_matches(['\r', '\n']).to_string()))
}

async fn drain_stdout(mut reader: BufReader<ChildStdout>) {
    let mut buffer = String::new();
    while reader
        .read_line(&mut buffer)
        .await
        .map(|bytes| {
            let has_data = bytes > 0;
            if has_data {
                buffer.clear();
            }
            has_data
        })
        .unwrap_or(false)
    {}
}

async fn ensure_success(child: &mut Child, name: &str) -> Result<()> {
    let status = child
        .wait()
        .await
        .with_context(|| format!("failed to await {name} process"))?;
    if !status.success() {
        return Err(anyhow!("{name} exited with status {status}"));
    }
    Ok(())
}
