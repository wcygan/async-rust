use anyhow::{bail, Context, Error};
use async_native_tls::TlsStream;
use async_task::{Runnable, Task};
use flume::{Receiver, Sender};
use futures_lite::AsyncRead;
use futures_lite::AsyncWrite;
use http::{header, Request, Uri};
use smol::Async;
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::panic::catch_unwind;
use std::pin::Pin;
use std::sync::LazyLock;
use std::task::Poll;
use tokio::io::ReadBuf;

/// A macro to simplify spawning tasks with an optional priority argument.
/// If no priority is provided, it defaults to `FuturePriority::Low`.
macro_rules! spawn_task {
    ($future:expr) => {
        // Spawn with default low priority
        spawn_task!($future, FuturePriority::Low)
    };
    ($future:expr, $order:expr) => {
        // Spawn with specified priority
        spawn_task($future, $order)
    };
}

macro_rules! join {
    ($($future:expr),*) => {
      {
        // Create a vector to hold the results
        let mut results = Vec::new();

        // For each future, block on its completion and collect the results
        $(
            results.push(futures_lite::future::block_on($future));
        )*

        // Return the collected results
        results
      }
    };
}

macro_rules! try_join {
    ($($future:expr),*) => {{
        let mut results = Vec::new();

        $(
            let result = catch_unwind(|| futures_lite::future::block_on($future));
            results.push(result);
        )*

        results
    }};
}

/// Chapter 4:
/// An executor is responsible for running futures to completion.
/// It is the part of the runtime that schedules tasks and makes sure they run (or are executed)
/// when they are ready. We need an executor to introduce networking into our runtime because
/// without it, our futures such as HTTP requests would be created but never actually run.
///
/// A connector in networking is a component that establishes a connection between our application
/// and the server we want to connect to. It handles activities like opening TCP connections
/// and maintaining them through the lifetime of the request.
fn main() {
    Runtime::new().run();

    // demo_one();

    let url = "http://rust-lang.org";
    let uri: Uri = url.parse().unwrap();

    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::USER_AGENT, "hyper.0.14.2")
        .header(header::ACCEPT, "text/html")
        .body(hyper::Body::empty())
        .unwrap();

    let future = async {
        let client = hyper::Client::new();
        client.request(request).await.unwrap()
    };

    let test = spawn_task!(future);
    let resp = futures_lite::future::block_on(test);
    println!("Response: {:?}", resp);
}

struct CustomExectutor {}

/// This code defines our custom executor and the behavior of the execute function.
/// Inside this function, we call `spawn_task!` to spawn the future onto our custom task queues.
/// We call `detach` to avoid closing the channel due to the task being dropped.
///
impl<F: Future + Send + 'static> hyper::rt::Executor<F> for CustomExectutor {
    fn execute(&self, fut: F) {
        spawn_task!(async {
            println!("Executing");
            fut.await;
        })
            .detach();
    }
}

enum CustomStream {
    Plain(Async<TcpStream>),
    Tls(TlsStream<Async<TcpStream>>),
}

/// The AsyncRead trait is similar to the std::io::Read trait but integrates with asynchronous
/// task systems. When implementing AsyncRead, we have to define only the poll_read method,
/// which returns a Poll as a result. If we return Poll::Ready, we are saying that the data was
/// immediately read and placed into the output buffer. If we return Poll:Pending, we are
/// saying that no data was read into the buffer that we provided. We are also saying that
/// the I/O object is not currently readable but may become readable in the future. The return
/// of Pending results in the current future's task being scheduled to be unpark when the
/// object is readable. The final Poll enum variant that we can return is Poll::Ready but with
/// an error that would usually be a standard I/O error.
impl tokio::io::AsyncRead for CustomStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match &mut *self {
            CustomStream::Plain(s) => {
                Pin::new(s)
                    .poll_read(cx, buf.initialize_unfilled())
                    .map_ok(|size| {
                        buf.advance(size);
                    })
            }
            CustomStream::Tls(s) => {
                Pin::new(s)
                    .poll_read(cx, buf.initialize_unfilled())
                    .map_ok(|size| {
                        buf.advance(size);
                    })
            }
        }
    }
}

/// The AsyncWrite trait is similar to the std::io::Write trait but integrates with asynchronous
/// task systems. It writes bytes asynchronously.
impl tokio::io::AsyncWrite for CustomStream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>, buf: &[u8]) -> Poll<Result<usize, std::io::Error>> {
        match &mut *self {
            CustomStream::Plain(s) => {
                Pin::new(s).poll_write(cx, buf)
            }
            CustomStream::Tls(s) => {
                Pin::new(s).poll_write(cx, buf)
            }
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Result<(), std::io::Error>> {
        match &mut *self {
            CustomStream::Plain(s) => {
                Pin::new(s).poll_flush(cx)
            }
            CustomStream::Tls(s) => {
                Pin::new(s).poll_flush(cx)
            }
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Result<(), std::io::Error>> {
        match &mut *self {
            CustomStream::Plain(s) => {
                s.get_mut().shutdown(Shutdown::Write)?;
                Poll::Ready(Ok(()))
            }
            CustomStream::Tls(s) => {
                Pin::new(s).poll_close(cx)
            }
        }
    }
}

#[derive(Clone)]
struct CustomConnector {}

impl hyper::service::Service<Uri> for CustomConnector {
    type Response = CustomStream;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output=Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Uri) -> Self::Future {
        Box::pin(async move {
            let host = req.host().context("cannot get host")?;

            match req.scheme_str() {
                Some("http") => {
                    let port = req.port_u16().unwrap_or(80);
                    let socket_addr = {
                        let host = host.to_string();
                        smol::unblock(move || (host.as_str(), port).to_socket_addrs())
                            .await?
                            .next()
                            .context("cannot resolve host")?
                    };
                    let stream = Async::<TcpStream>::connect(socket_addr).await?;
                    Ok(CustomStream::Plain(stream))
                }
                Some("https") => {
                    let socket_addr = {
                        let host = host.to_string();
                        let port = req.port_u16().unwrap_or(443);
                        smol::unblock(move || (host.as_str(), port).to_socket_addrs())
                            .await?
                            .next()
                            .context("cannot resolve host")?
                    };
                    let stream = Async::<TcpStream>::connect(socket_addr).await?;
                    let stream = async_native_tls::TlsConnector::new()
                        .connect(host, stream)
                        .await?;

                    Ok(CustomStream::Tls(stream))
                }
                _ => bail!("unsupported scheme"),
            }
        })
    }
}

struct Runtime {
    /// Number of threads for the high priority queue
    high_num: usize,
    /// Number of threads for the low priority queue
    low_num: usize,
}

impl Runtime {
    fn new() -> Self {
        let num_cores = std::thread::available_parallelism().unwrap().get();
        Self {
            high_num: num_cores,
            low_num: 1,
        }
    }

    pub fn with_high_priority_threads(mut self, num: usize) -> Self {
        self.high_num = num;
        self
    }

    pub fn with_low_priority_threads(mut self, num: usize) -> Self {
        self.low_num = num;
        self
    }

    pub fn run(&self) {
        unsafe {
            std::env::set_var("HIGH_PRIORITY_THREADS", self.high_num.to_string());
            std::env::set_var("LOW_PRIORITY_THREADS", self.low_num.to_string());
        }

        let high = spawn_task!(async {}, FuturePriority::High);
        let low = spawn_task!(async {}, FuturePriority::Low);
        join!(high, low);
    }
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
fn spawn_task<F, T>(future: F, order: FuturePriority) -> Task<T>
where
    F: Future<Output=T> + Send + 'static,
    T: Send + 'static,
{
    let schedule_high_priority = |runnable: Runnable| {
        HIGH_PRIORITY_QUEUE
            .send(runnable)
            .expect("Failed to send runnable to the high priority queue");
    };

    let schedule_low_priority = |runnable: Runnable| {
        LOW_PRIORITY_QUEUE
            .send(runnable)
            .expect("Failed to send runnable to the low priority queue");
    };

    let schedule = match order {
        FuturePriority::High => schedule_high_priority,
        FuturePriority::Low => schedule_low_priority,
    };

    let (runnable, task) = async_task::spawn(future, schedule);

    runnable.schedule();
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

static HIGH_PRIORITY_QUEUE: LazyLock<flume::Sender<Runnable>> = LazyLock::new(|| {
    let (sender, receiver) = flume::unbounded::<Runnable>();

    let high_thread_count: usize = std::env::var("HIGH_PRIORITY_THREADS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    for _ in 0..high_thread_count {
        let high_receiver = HIGH_CHANNEL.clone();
        let low_receiver = LOW_CHANNEL.clone();
        std::thread::spawn(move || {
            loop {
                match high_receiver.1.try_recv() {
                    Ok(runnable) => {
                        let _ = catch_unwind(|| runnable.run());
                    }
                    Err(_) => {
                        // Task stealing when there is no high priority task
                        match low_receiver.1.try_recv() {
                            Ok(runnable) => {
                                let _ = catch_unwind(|| runnable.run());
                            }
                            Err(_) => {
                                std::thread::sleep(std::time::Duration::from_millis(100));
                            }
                        }
                    }
                }
            }
        });
    }

    HIGH_CHANNEL.0.clone()
});

static LOW_PRIORITY_QUEUE: LazyLock<flume::Sender<Runnable>> = LazyLock::new(|| {
    let (sender, receiver) = flume::unbounded::<Runnable>();

    let low_thread_count: usize = std::env::var("LOW_PRIORITY_THREADS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    for _ in 0..low_thread_count {
        let high_receiver = HIGH_CHANNEL.clone();
        let low_receiver = LOW_CHANNEL.clone();
        std::thread::spawn(move || {
            loop {
                match low_receiver.1.recv() {
                    Ok(runnable) => {
                        let _ = catch_unwind(|| runnable.run());
                    }
                    Err(_) => {
                        // Task stealing when there is no low priority task
                        match high_receiver.1.try_recv() {
                            Ok(runnable) => {
                                let _ = catch_unwind(|| runnable.run());
                            }
                            Err(_) => {
                                std::thread::sleep(std::time::Duration::from_millis(100));
                            }
                        }
                    }
                }
            }
        });
    }

    LOW_CHANNEL.0.clone()
});

static HIGH_CHANNEL: LazyLock<(Sender<Runnable>, Receiver<Runnable>)> =
    LazyLock::new(|| flume::unbounded::<Runnable>());

static LOW_CHANNEL: LazyLock<(Sender<Runnable>, Receiver<Runnable>)> =
    LazyLock::new(|| flume::unbounded::<Runnable>());

struct CounterFuture {
    count: u32,
    order: FuturePriority,
}

impl PrioritizedFuture for CounterFuture {
    fn priority(&self) -> FuturePriority {
        self.order
    }
}

/// Example 1
/// A simple future that counts to 3, printing its count each time it is polled.
impl Future for CounterFuture {
    type Output = u32;

    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
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

/// Example 2 (unused after adding PrioritizedFuture)
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

impl PrioritizedFuture for AsyncSleep {
    fn priority(&self) -> FuturePriority {
        FuturePriority::Low
    }
}

/// Example 3
/// A future that demonstrates asynchronous sleep by checking elapsed time.
/// It returns Poll::Pending until the specified duration has passed.
impl Future for AsyncSleep {
    type Output = bool;

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let elapsed = self.start.elapsed();
        if elapsed >= self.duration {
            Poll::Ready(true)
        } else {
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum FuturePriority {
    High,
    Low,
}

trait PrioritizedFuture: Future {
    fn priority(&self) -> FuturePriority;
}

fn demo_one() {
    let one = CounterFuture {
        count: 0,
        order: FuturePriority::High,
    };
    let two = CounterFuture {
        count: 0,
        order: FuturePriority::Low,
    };
    let t_one = spawn_task!(one);
    let t_two = spawn_task(two, FuturePriority::High);
    let t_tree = spawn_task!(async_fn());
    let outcome: Vec<u32> = join!(t_one, t_two);
    let outcome_two: Vec<()> = join!(t_tree);
    println!("Outcome one: {:?}", outcome);
    println!("Outcome two: {:?}", outcome_two);
}
