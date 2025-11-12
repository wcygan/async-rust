use std::pin::Pin;
use std::task::{Context, Poll};

#[tokio::main]
async fn main() {
    tokio::spawn(BackgroundProcess {});
    std::thread::sleep(std::time::Duration::from_secs(5));
}

#[derive(Debug, Clone, Copy)]
struct BackgroundProcess {}

impl Future for BackgroundProcess {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        println!("Poll called on background process");
        std::thread::sleep(std::time::Duration::from_secs(1));
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}
