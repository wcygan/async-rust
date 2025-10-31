use anyhow::Result;
use clap::Parser;
use tokio::net::TcpListener;
use tracing::{info, warn};

use local_chat_mesh::{
    broker,
    cli::{Cli, Command},
    client,
};

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).with_target(false).try_init();
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cli = Cli::parse();
    match cli.command {
        Command::Broker(args) => {
            let listener = TcpListener::bind(args.listen).await?;
            let broker = broker::Broker::new(listener);
            let addr = broker.local_addr()?;
            info!("broker listening on {}", addr);
            if let Err(err) = broker.run_until_ctrl_c().await {
                warn!("broker exited with error: {err:?}");
                return Err(err);
            }
        }
        Command::Client(args) => client::run(args).await?,
    }

    Ok(())
}
