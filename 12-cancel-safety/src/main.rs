use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

/// https://rfd.shared.oxide.computer/rfd/400
/// 
/// Since a Future could be cancelled if it doesn't complete within
/// a certain amount of time, we must be careful about the notion
/// of "Cancel Safety".
/// 
/// Cancel Safety ensures that when a future is cancled 
#[tokio::main]
async fn main() {
    let shared = Arc::new(Mutex::new(0u32));

    // Spawn a background task that simulates normal work.
    {
        let shared = shared.clone();
        tokio::spawn(async move {
            let mut guard = shared.lock().await;
            println!("Task A: acquired lock, setting value = 42");
            *guard = 42;
            sleep(Duration::from_secs(5)).await;
            println!("Task A: releasing lock");
        });
    }

    // Construct a future that will hold the lock for a long time.
    let fut = {
        let shared = shared.clone();
        async move {
            let mut guard = shared.lock().await;
            println!("Task B: acquired lock, starting long work");
            *guard += 1;
            sleep(Duration::from_secs(10)).await;
            println!("Task B: finished work, releasing lock");
        }
    };

    // Pin it so we can poll it partially via &mut
    let mut fut = Box::pin(fut);

    // We only wait a short time before timing out.
    tokio::select! {
        _ = &mut fut => {
            println!("Task B: finished early");
        }
        _ = sleep(Duration::from_secs(1)) => {
            println!("Main: timeout — abandoning Task B mid-lock");
        }
    }

    // Now we try to acquire the lock again.
    println!("Main: trying to acquire the lock after B...");
    let mut guard = shared.lock().await;
    println!("Main: got the lock! value = {}", *guard);
}
