use async_task::{Runnable, Task};
use std::sync::LazyLock;

fn main() {
    println!("Hello, world!");
}

/// `spawn_task` is a generic function that accepts any types
/// that implements both `Future` and `Send` traits.
/// The `Future` trait denotes that our future is going to result
/// in either an error or the value T. Our future needs the `Send`
/// trait because we are going to be sending our future into a
/// different thread where the queue is based. The `Send` trait
/// enforces constraints that ensure that our future can be safely
/// shared among threads.
///
/// The `'static` lifetime specifier indicates that the future
/// does not contain any references that have a shorter lifetime
/// than the static lifetime. Therefore, the future can be used for
/// as long as the program is running. Ensuring this lifetime is
/// essential because we cannot force programmers to wait for a
/// task to finish. If the developer never waits for a task, the
/// task could run for the entire lifetime of the program.
/// Because we cannot guarantee when a task is finished, we must
/// ensure that the lifetime of our task is `'static`.
///
/// Using `async move`, this is where we move the ownership of
/// variables used in the async closure to the task so we can
/// ensure that the lifetime is `'static`.
fn spawn_task<F, T>(future: F) -> Task<T>
where
    F: Future<Output=T> + Send + 'static,
    T: Send + 'static,
{
    todo!()
}

static QUEUE: LazyLock<flume::Sender<Runnable>> = LazyLock::new(|| {
    let (sender, receiver) = flume::unbounded::<Runnable>();

    // Spawn a worker thread to process the task queue
    std::thread::spawn(move || {
        while let Ok(runnable) = receiver.recv() {
            // Execute the task
            runnable.run();
        }
    });

    sender
});