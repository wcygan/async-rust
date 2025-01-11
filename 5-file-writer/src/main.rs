use clap::Parser;
use nanoid::nanoid;
use std::time::Duration;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::time::sleep;

/// This is a simple file writer that continuously appends nanoids to a file.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the file to write
    #[arg(short, long)]
    filepath: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Spawn a task to continuously append nanoids to the file
    tokio::spawn(append_data(args.filepath.clone()));

    // Keep the main function alive
    loop {
        sleep(Duration::from_secs(60)).await;
    }
}

async fn append_data(file_path: String) {
    loop {
        let nanoid = nanoid!();
        println!("{}", nanoid);
        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&file_path)
            .await
            .unwrap();
        file.write_all(format!("{}\n", nanoid).as_bytes())
            .await
            .unwrap();

        // Append a new nanoid every second
        sleep(Duration::from_secs(1)).await;
    }
}