use std::{net::SocketAddr, time::Duration};

use anyhow::Result;
use local_chat_mesh::{
    broker::Broker,
    message::{ClientToServer, ServerToClient, read_message, write_message},
};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    net::{
        TcpListener, TcpStream,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
    time::timeout,
};

#[tokio::test]
async fn clients_receive_broadcasts() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let broker = Broker::new(listener);

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let shutdown = async move {
            let _ = shutdown_rx.await;
        };
        let _ = broker.run_until(shutdown).await;
    });

    let (mut alice_reader, mut alice_writer) = connect_and_join(addr, "alice").await?;
    let (mut bob_reader, mut bob_writer) = connect_and_join(addr, "bob").await?;

    let roster = timeout(
        Duration::from_secs(1),
        read_message::<_, ServerToClient>(&mut bob_reader),
    )
    .await??
    .expect("bob should receive a roster");
    assert_eq!(
        roster,
        ServerToClient::Roster {
            participants: vec!["alice".into()]
        }
    );

    let joined = timeout(
        Duration::from_secs(1),
        read_message::<_, ServerToClient>(&mut alice_reader),
    )
    .await??
    .expect("alice should see bob join");
    assert_eq!(
        joined,
        ServerToClient::UserJoined {
            nickname: "bob".into()
        }
    );

    write_message(
        &mut alice_writer,
        &ClientToServer::Chat {
            text: "hello bob".into(),
        },
    )
    .await?;

    let chat = timeout(
        Duration::from_secs(1),
        read_message::<_, ServerToClient>(&mut bob_reader),
    )
    .await??
    .expect("bob should read chat from alice");

    assert_eq!(
        chat,
        ServerToClient::Chat {
            nickname: "alice".into(),
            text: "hello bob".into()
        }
    );

    alice_writer.shutdown().await?;
    bob_writer.shutdown().await?;
    drop(alice_reader);
    drop(bob_reader);

    let _ = shutdown_tx.send(());
    let _ = server.await;

    Ok(())
}

async fn connect_and_join(
    addr: SocketAddr,
    nickname: &str,
) -> Result<(BufReader<OwnedReadHalf>, OwnedWriteHalf)> {
    let stream = TcpStream::connect(addr).await?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    write_message(
        &mut writer,
        &ClientToServer::Hello {
            nickname: nickname.to_string(),
        },
    )
    .await?;

    match read_message::<_, ServerToClient>(&mut reader).await? {
        Some(ServerToClient::Welcome { nickname: welcome }) => {
            assert_eq!(welcome, nickname);
        }
        other => panic!("unexpected handshake response: {other:?}"),
    }

    Ok((reader, writer))
}
