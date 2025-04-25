use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

struct CountingFuture {
    target: u32,
    current: u32,
    last_wake: Instant,
}

impl CountingFuture {
    fn new(target: u32) -> Self {
        CountingFuture {
            target,
            current: 0,
            // Initialize to allow the first poll to proceed
            last_wake: Instant::now() - Duration::from_secs(2),
        }
    }
}

impl Future for CountingFuture {
    type Output = u32;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Simulate work by only proceeding if some time has passed since the last wake.
        // This prevents a tight loop that would consume 100% CPU.
        // A real future would typically rely on an external event (like I/O readiness)
        // and register the waker with that event source.
        if self.last_wake.elapsed() < Duration::from_millis(50) {
            // Not enough time passed, ask to be woken up later.
            // We clone the waker because wake() consumes it.
            let waker = cx.waker().clone();
            // Request a wake-up in the future (e.g., using tokio::time::sleep)
            // For simplicity here, we just request a wake immediately, relying on the
            // elapsed time check to provide the delay.
            // In a more complex scenario, you might schedule a timer.
            waker.wake();
            return Poll::Pending;
        }

        if self.current < self.target {
            self.current += 1;
            println!("CountingFuture polled, current count: {}", self.current);
            self.last_wake = Instant::now(); // Record the time of this poll

            // We made progress, but haven't reached the target yet.
            // We need to ensure the executor polls us again.
            // Calling wake_by_ref schedules the task to be polled again.
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            println!("CountingFuture reached target: {}", self.target);
            Poll::Ready(self.current)
        }
    }
}

#[tokio::main]
async fn main() {
    println!("Starting CountingFuture...");
    let counter = CountingFuture::new(3);
    let result = counter.await;
    println!("CountingFuture finished with result: {}", result);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn test_counting_future_reaches_target() {
        let target = 5;
        let counter_future = CountingFuture::new(target);

        // Add a timeout to prevent the test from hanging indefinitely
        // if the future fails to complete.
        match timeout(Duration::from_secs(1), counter_future).await {
            Ok(result) => assert_eq!(
                result, target,
                "Future should complete with the target value"
            ),
            Err(_) => panic!("Future timed out before completing"),
        }
    }

    #[tokio::test]
    async fn test_counting_future_zero_target() {
        let target = 0;
        let counter_future = CountingFuture::new(target);

        match timeout(Duration::from_secs(1), counter_future).await {
            Ok(result) => assert_eq!(
                result, target,
                "Future should complete immediately with 0 for target 0"
            ),
            Err(_) => panic!("Future timed out even with target 0"),
        }
    }
}
