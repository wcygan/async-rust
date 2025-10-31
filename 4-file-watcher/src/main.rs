use clap::Parser;
use std::path::PathBuf;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::sync::watch;
use tokio::time::sleep;

/// This is a simple file watcher that watches a file for changes and prints the changes to the console.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the file to watch
    #[arg(short, long)]
    filepath: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let (tx, mut rx) = watch::channel(false);

    // Spawn a task to watch for file changes
    tokio::spawn(watch_file_changes(tx, args.filepath.clone()));

    // Read the file and print the contents when the file changes
    loop {
        let _ = rx.changed().await;
        let contents = read_file(&args.filepath).await.unwrap();
        println!("{}", contents);
    }
}

async fn read_file(file_path: &str) -> Result<String, std::io::Error> {
    let mut file = File::open(file_path).await?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).await?;
    Ok(contents)
}

async fn watch_file_changes(tx: watch::Sender<bool>, file_path: String) {
    let path = PathBuf::from(file_path);
    let mut last_modified = None;
    loop {
        if let Ok(metadata) = path.metadata() {
            let modified = metadata.modified().unwrap();

            // If the file has been modified, send a message to the receiver
            if last_modified != Some(modified) {
                last_modified = Some(modified);
                tx.send(true).unwrap();
            }

            // Check every 500ms
            sleep(Duration::from_millis(500)).await;
        }
    }
}
