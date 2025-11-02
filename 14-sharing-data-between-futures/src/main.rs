use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::sync::Mutex;

/// Although it can complicate things, we can share data between
/// futures. We may want to share data between futures for
/// the following reasons:
///
///   1. Aggregating results
///   2. Dependent computations
///   3. Caching results
///   4. Synchronization
///   5. Shared state
///   6. Task coordination and supervision
///   7. Resource management
///   8. Error propagation
///
/// While sharing data between futures is useful,
/// there are some things that we need to be mindful
/// of when doing so.

#[derive(Debug)]
enum CounterType {
    Increment,
    Decrement,
}

struct SharedData {
    counter: i32,
}

impl SharedData {
    fn increment(&mut self) {
        self.counter += 1;
    }

    fn decrement(&mut self) {
        self.counter -= 1;
    }
}

/// We use a std::sync::Mutex instead of a tokio::sync::Mutex
/// here because the poll function cannot be async. Here lies
/// a problem; if we acquire the Mutex using the standard lock
/// function, we can block the thread until the lock is acquired.
/// This would defeat the purpose of the async runtime if we locked
/// the entire thread until the Mutex is acquired. Instead, we use
/// the try_lock function, which attempts to acquire the lock without
/// blocking. If the lock is not available, we return Poll::Pending
/// and wake the task to try again later.
///
/// If we do get the lock, then we move forward in the poll function
/// to act on the shared data
struct CounterFuture {
    counter_type: CounterType,
    data_reference: Arc<Mutex<SharedData>>,
    count: u32,
}

impl Future for CounterFuture {
    type Output = u32;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        std::thread::sleep(Duration::from_secs(1));
        let mut guard = match self.data_reference.try_lock() {
            Ok(mut guard) => guard,
            Err(error) => {
                println!("Error for {:?}: {}", self.counter_type, error);
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
        };

        let value = &mut *guard;

        match self.counter_type {
            CounterType::Increment => {
                value.increment();
                println!("after increment: {}", value.counter);
            }
            CounterType::Decrement => {
                value.decrement();
                println!("after decrement: {}", value.counter);
            }
        }

        drop(guard);
        self.count += 1;

        if self.count < 3 {
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            Poll::Ready(self.count)
        }
    }
}

#[tokio::main]
async fn main() {
    let shared_data = Arc::new(Mutex::new(SharedData { counter: 0 }));

    let increment_future = CounterFuture {
        counter_type: CounterType::Increment,
        data_reference: Arc::clone(&shared_data),
        count: 0,
    };

    let decrement_future = CounterFuture {
        counter_type: CounterType::Decrement,
        data_reference: Arc::clone(&shared_data),
        count: 0,
    };

    let increment_handle = tokio::spawn(async move {
        let result = increment_future.await;
        println!("Increment future completed with count: {}", result);
    });

    let decrement_handle = tokio::spawn(async move {
        let result = decrement_future.await;
        println!("Decrement future completed with count: {}", result);
    });

    let (r1, r2) = tokio::join!(increment_handle, decrement_handle);

    match r1 {
        Ok(_) => println!("Increment task finished successfully."),
        Err(e) => println!("Increment task failed: {}", e),
    }

    match r2 {
        Ok(_) => println!("Decrement task finished successfully."),
        Err(e) => println!("Decrement task failed: {}", e),
    }

    println!("Finished!");

    println!("Trying now the async function version:");
    let shared_data_async = Arc::new(tokio::sync::Mutex::new(SharedData { counter: 0 }));
    let shared_data_async2 = shared_data_async.clone();

    let increment_handle_async = tokio::spawn(async move {
        let result = count(3, shared_data_async, CounterType::Increment).await;
        println!("Async Increment future completed with count: {}", result);
    });

    let decrement_handle_async = tokio::spawn(async move {
        let result = count(3, shared_data_async2, CounterType::Decrement).await;
        println!("Async Decrement future completed with count: {}", result);
    });

    let (r1_async, r2_async) = tokio::join!(increment_handle_async, decrement_handle_async);
    match r1_async {
        Ok(_) => println!("Async Increment task finished successfully."),
        Err(e) => println!("Async Increment task failed: {}", e),
    }
    match r2_async {
        Ok(_) => println!("Async Decrement task finished successfully."),
        Err(e) => println!("Async Decrement task failed: {}", e),
    }
    println!("Finished async function version!");
}

/// This method is functionally equivalent to the CounterFuture
/// struct and its Future implementation above. However, it uses
/// an async function instead of implementing the Future trait manually.
#[allow(dead_code)]
async fn count(
    count: u32,
    data: Arc<tokio::sync::Mutex<SharedData>>,
    counter_type: CounterType,
) -> u32 {
    for _ in 0..count {
        let mut guard = data.lock().await;
        match counter_type {
            CounterType::Increment => {
                println!("Incrementing counter");
                guard.increment();
                println!("Current counter value: {}", guard.counter);
            }
            CounterType::Decrement => {
                println!("Decrementing counter");
                guard.decrement();
                println!("Current counter value: {}", guard.counter);
            }
        }
        drop(guard);
        std::thread::sleep(Duration::from_secs(1));
    }

    count
}
