use std::fs::File;
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

type AsyncFileHandle = Arc<Mutex<File>>;
type FileJoinHandle = JoinHandle<Result<bool, String>>;

#[tokio::main]
async fn main() {
    println!("Hello, world!");
}
