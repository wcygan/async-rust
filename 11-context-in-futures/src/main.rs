use std::time::Duration;
use tokio::time::timeout;

/// A context only serves to provide access to a waker
/// to wake a task. A waker is a handle that notifies
/// the executor when the task is ready to be run.
///
/// While this is the primary role of `Context` today,
/// it's important to note that this functionality
/// might evolve in the future. The design of context
/// has allowed space for expansion, such as the introduction
/// of additional responsibilities or capabilities
/// as Rust's async ecosystem grows.
///
/// We can see that futures are used with an `async/await` function,
/// but let's think about how else they can be used. We can also use
/// a timeout on a thread of execution: the thread finishes when a
/// certain amount of time has elapsed, so we do not end up with
/// a program that hangs indefinitely. This is useful when we
/// have a function that can be slow to complete and we want
/// to move on or error early. Remember that threads provide
/// the underlying functionality for executing tasks.

#[tokio::main]
async fn main() {
    let duration = Duration::from_secs(3);
    let result = timeout(duration, slow_task()).await;

    match result {
        Ok(value) => println!("Task completed successfully: {}", value),
        Err(_) => println!("Task timed out"),
    }
}

async fn slow_task() -> &'static str {
    tokio::time::sleep(Duration::from_secs(10)).await;
    "slow task completed"
}
