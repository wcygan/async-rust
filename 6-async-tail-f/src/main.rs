use clap::Parser;
use std::path::PathBuf;
use std::time::Duration;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
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

    // Open the file and seek to the end initially
    let mut file = OpenOptions::new()
        .read(true)
        .open(&args.filepath)
        .await
        .expect("Failed to open file");
    let mut offset = file.seek(SeekFrom::End(0))
        .await
        .expect("Failed to seek to end");

    loop {
        // Wait for a change notification
        let _ = rx.changed().await;

        // Check for new content
        let new_bytes = read_new_content(&args.filepath, &mut offset).await;
        if let Ok(content) = new_bytes {
            if !content.is_empty() {
                print!("{}", content);
            }
        }
    }
}

async fn read_new_content(file_path: &str, offset: &mut u64) -> Result<String, std::io::Error> {
    let mut file = OpenOptions::new()
        .read(true)
        .open(file_path)
        .await?;

    // Seek to last read offset
    file.seek(SeekFrom::Start(*offset)).await?;

    let mut buffer = String::new();
    let bytes_read = file.read_to_string(&mut buffer).await?;

    // Update the offset by the number of bytes read
    *offset += bytes_read as u64;
    Ok(buffer)
}

async fn watch_file_changes(tx: watch::Sender<bool>, file_path: String) {
    let path = PathBuf::from(file_path);
    let mut last_modified = None;
    loop {
        if let Ok(metadata) = path.metadata() {
            if let Ok(modified) = metadata.modified() {
                // If the file has been modified, send a message to the receiver
                if last_modified != Some(modified) {
                    last_modified = Some(modified);
                    // It's okay if the receiver has dropped; ignore errors
                    let _ = tx.send(true);
                }
            }
        }
        // Check every 500ms
        sleep(Duration::from_millis(500)).await;
    }
}