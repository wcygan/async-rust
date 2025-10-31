use std::net::SocketAddr;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the chat broker, accepting local TCP connections.
    Broker(BrokerArgs),
    /// Connect to a broker and participate in the chat.
    Client(ClientArgs),
}

#[derive(Args, Debug, Clone)]
pub struct BrokerArgs {
    /// Socket address the broker should bind to. Use 0 for an ephemeral port.
    #[arg(long, default_value = "127.0.0.1:5000")]
    pub listen: SocketAddr,
}

#[derive(Args, Debug, Clone)]
pub struct ClientArgs {
    /// Nickname used when joining the chat mesh.
    #[arg(long)]
    pub nickname: String,

    /// Address of the broker to connect to.
    #[arg(long, default_value = "127.0.0.1:5000")]
    pub server: SocketAddr,
}
