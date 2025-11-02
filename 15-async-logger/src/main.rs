use std::fs::{File, OpenOptions};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

/// This program will use ideas from the past few programs
/// {10, 11, 12, 13, 14} to create an asynchronous logger
/// that writes log messages to a file without blocking the main thread.
///
/// We can conceive that we have a server or daemon that receives
/// requests or messages. The data received needs to be logged to a file
/// in case we need to inspect what happened. This problem means that
/// we cannot predict when a log will happen. For instance, if we are just
/// writing to a file in a single program, our write operations can be
/// blocking. However, receiving multiple requests from different programs
/// can result in considerable overhead. It makes sense to send a write task
/// to the async runtime and have the log written to the file when it is
/// possible.
///
/// In the following example we are going to create an audit trail for an
/// application logs interactions. This is an important part of many products
/// that use sensitive data, like in the medical field. We want to log the
/// user's actions, but we do not want that logging action to hold up the
/// program because we still want to facilitate a quick user experience.

/// Seeing as we do not want two tasks writing to the file at the same time,
/// it makes sense to use a Mutex to ensure that only one task is writing to the file
/// at any given time.
///
/// We also might want to write to multiple files. For instance, we might want to
/// write all logins to one file, and error messages to another file. If you have
/// medical patients in your system, you want to have a log file per patient (as
/// you would probably inspect log files on a per-patient basis), and you'd want to
/// prevent unauthorized people looking at actions on a patient that they are
/// not allowed to view.
type AsyncFileHandle = Arc<Mutex<File>>;
type FileJoinHandle = JoinHandle<Result<bool, String>>;

/// Considering there are needs for multiple files when logging, we can create a
/// function that creates a file or obtains the handle of an existing file:
fn get_file_handle(file_path: &dyn ToString) -> AsyncFileHandle {
    match OpenOptions::new().append(true).open(file_path.to_string()) {
        Ok(opened_file) => {
            Arc::new(Mutex::new(opened_file))
        }
        Err(_) => {
            let created_file = File::create(file_path.to_string())
                .expect("Failed to create log file");
            Arc::new(Mutex::new(created_file))
        }
    }
}

#[tokio::main]
async fn main() {
    println!("Hello, world!");
}
