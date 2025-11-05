use async_task::{Runnable, Task};
use std::panic::catch_unwind;
use std::pin::Pin;
use std::sync::LazyLock;
use std::task::{Context, Poll};

fn main() {
    let one = CounterFuture { count: 0 };
    let two = CounterFuture { count: 0 };
    let t_one = spawn_task(one);
    let t_two = spawn_task(two);
    let t_three = spawn_task(async {
        async_fn().await;
        async_fn().await;
        async_fn().await;
        async_fn().await;
    });
    std::thread::sleep(std::time::Duration::from_secs(5));
    println!("Before the blocking wait");
    futures_lite::future::block_on(t_one);
    futures_lite::future::block_on(t_two);
    futures_lite::future::block_on(t_three);
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
    let schedule = |runnable: Runnable| {
        QUEUE.send(runnable).expect("Failed to send runnable to the queue");
    };

    let (runnable, task) = async_task::spawn(
        future, schedule,
    );

    runnable.schedule();
    println!(
        "Here is the queue count: {}",
        QUEUE.len()
    );

    task
}

/// `Runnable` is a handle for a runnable task. Every spawned task has a single Runnable
/// handle, which exists only when the task is scheduled to run. The handle has the `run`
/// function that polls that task's future once. Then the runnable is dropped. The
/// runnable appears again only when the waker wakes the task in turn, scheduling the task
/// again.
static QUEUE: LazyLock<flume::Sender<Runnable>> = LazyLock::new(|| {
    let (sender, receiver) = flume::unbounded::<Runnable>();

    let thread_count = 4;
    for _ in 0..thread_count {
        let queue = receiver.clone();
        std::thread::spawn(move || {
            while let Ok(runnable) = queue.recv() {
                let _ = catch_unwind(|| runnable.run());
            }
        });
    }

    sender
});

struct CounterFuture {
    count: u32,
}

/// Example 1
/// A simple future that counts to 3, printing its count each time it is polled.
impl Future for CounterFuture {
    type Output = u32;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.count += 1;
        println!("Future count: {}", self.count);
        std::thread::sleep(std::time::Duration::from_secs(1));

        if self.count >= 3 {
            Poll::Ready(self.count)
        } else {
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

/// Example 2
/// An asynchronous function that simulates a delay using thread sleep.
/// Thread sleep (blocking) is used explicitly here for demonstration purposes.
async fn async_fn() {
    std::thread::sleep(std::time::Duration::from_secs(1));
    println!("Async function executed");
}

struct AsyncSleep {
    start: std::time::Instant,
    duration: std::time::Duration,
}

impl AsyncSleep {
    fn new(duration: std::time::Duration) -> Self {
        Self {
            start: std::time::Instant::now(),
            duration,
        }
    }
}

/// Example 3
/// A future that demonstrates asynchronous sleep by checking elapsed time.
/// It returns Poll::Pending until the specified duration has passed.
impl Future for AsyncSleep {
    type Output = bool;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let elapsed = self.start.elapsed();
        if elapsed >= self.duration {
            Poll::Ready(true)
        } else {
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}