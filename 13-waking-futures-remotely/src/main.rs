use std::future::Future;
use std::io::Read;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use tokio::sync::mpsc;
use tokio::task;

/// Imagine that we make a network call to another
/// computer using async Rust. The routing of the network
/// call and receiving of the response happens outside of
/// our Rust program. Considering this, it does not make
/// sense to constantly poll our networking future until
/// we get a signal from the OS that data has been
/// received at the port we are listening to. We can
/// hold on the polling of the future by externally
/// referencing the future's waker and waking the
/// future when we need to.

struct MyFuture {
    state: Arc<Mutex<MyFutureState>>,
}

struct MyFutureState {
    data: Option<Vec<u8>>,
    waker: Option<Waker>,
}

impl MyFuture {
    fn new() -> (Self, Arc<Mutex<MyFutureState>>) {
        let state = Arc::new(Mutex::new(MyFutureState {
            data: None,
            waker: None,
        }));
        (
            MyFuture {
                state: state.clone(),
            },
            state,
        )
    }
}

impl Future for MyFuture {
    type Output = String;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        println!("Polling the future");
        let mut state = self.state.lock().unwrap();

        if state.data.is_some() {
            let data = state.data.take().unwrap();
            Poll::Ready(String::from_utf8(data).unwrap())
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

#[tokio::main]
async fn main() {
    let (fut, state) = MyFuture::new();
    let (tx, mut rx) = mpsc::channel::<()>(1);
    let task_handle = task::spawn(async {
        fut.await;
    });

    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
    println!("spawning trigger task");

    let trigger_task = task::spawn(async move {
        rx.recv().await;
        let mut state = state.lock().unwrap();
        state.data = Some(b"Hello from the outside".to_vec());
        loop {
            if let Some(waker) = state.waker.take() {
                waker.wake();
                break;
            }
        }
    });

    tx.send(()).await.unwrap();

    let outcome = task_handle.await.unwrap();
    println!("Future completed with: {:?}", outcome);
    trigger_task.await.unwrap();
}
