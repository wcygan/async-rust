use std::io;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

const LINE_ENDINGS: &[char] = &['\n', '\r'];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientToServer {
    Hello { nickname: String },
    Chat { text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerToClient {
    Welcome { nickname: String },
    Roster { participants: Vec<String> },
    UserJoined { nickname: String },
    UserLeft { nickname: String },
    Chat { nickname: String, text: String },
    Error { message: String },
}

pub async fn read_message<R, T>(reader: &mut R) -> io::Result<Option<T>>
where
    R: AsyncBufRead + Unpin,
    T: DeserializeOwned,
{
    // Simple line-oriented framing keeps interoperability with netcat-style tools.
    let mut line = String::new();
    loop {
        line.clear();
        let bytes = reader.read_line(&mut line).await?;
        if bytes == 0 {
            return Ok(None);
        }

        let trimmed = line.trim_end_matches(LINE_ENDINGS);
        if trimmed.is_empty() {
            continue;
        }

        let parsed = serde_json::from_str(trimmed).map_err(to_io_error)?;
        return Ok(Some(parsed));
    }
}

pub async fn write_message<W, T>(writer: &mut W, message: &T) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    // Encode to JSON once, append a newline delimiter, and flush so peers get timely updates.
    let mut encoded = serde_json::to_vec(message).map_err(to_io_error)?;
    encoded.push(b'\n');
    writer.write_all(&encoded).await?;
    writer.flush().await?;
    Ok(())
}

fn to_io_error(err: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn roundtrip_server_message() {
        let (mut writer, reader) = tokio::io::duplex(1024);
        let mut reader = tokio::io::BufReader::new(reader);
        let message = ServerToClient::Chat {
            nickname: "alice".into(),
            text: "hello".into(),
        };

        write_message(&mut writer, &message)
            .await
            .expect("write message");
        let parsed = read_message::<_, ServerToClient>(&mut reader)
            .await
            .expect("read message")
            .expect("expected message");

        assert_eq!(message, parsed);
    }
}
