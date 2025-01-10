use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() {
    let task1 = async {
        sleep(Duration::from_secs(1)).await;
        println!("Task 1: Completed after 1 second");
        "Task 1 result"
    };

    let task2 = async {
        sleep(Duration::from_millis(1500)).await;
        println!("Task 2: Completed after 1.5 seconds");
        42
    };

    let task3 = async {
        sleep(Duration::from_millis(800)).await;
        println!("Task 3: Completed after 0.8 seconds");
        vec![1, 2, 3]
    };

    let task4 = async {
        sleep(Duration::from_secs(2)).await;
        println!("Task 4: Completed after 2 seconds");
        true
    };

    let task5 = async {
        sleep(Duration::from_millis(500)).await;
        println!("Task 5: Completed after 0.5 seconds");
        ("hello", 123)
    };

    // Wait for all tasks to complete and get their results
    let (result1, result2, result3, result4, result5) =
        tokio::join!(task1, task2, task3, task4, task5);

    // Print the final results
    println!("\nFinal Results:");
    println!("Task 1 returned: {:?}", result1);
    println!("Task 2 returned: {:?}", result2);
    println!("Task 3 returned: {:?}", result3);
    println!("Task 4 returned: {:?}", result4);
    println!("Task 5 returned: {:?}", result5);
}
