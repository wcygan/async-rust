use std::process::Command;
use std::thread;
use std::time::Duration;

fn demonstrate_threads() {
    println!("\n=== Thread Demonstration ===");
    let mut handles = vec![];
    let shared_data = vec![1, 2, 3, 4, 5];

    // Spawn 3 threads that can all access the same data
    for id in 0..3 {
        let data = shared_data.clone(); // Threads can share data through cloning
        let handle = thread::spawn(move || {
            println!("Thread {} is running", id);
            println!("Thread {} sees shared data: {:?}", id, data);
            thread::sleep(Duration::from_millis(500));
            println!("Thread {} finished", id);
        });
        handles.push(handle);
    }

    // Wait for all threads to complete
    for handle in handles {
        handle.join().unwrap();
    }
}

fn demonstrate_processes() {
    println!("\n=== Process Demonstration ===");

    // Spawn 3 separate processes
    for id in 0..3 {
        let child = Command::new("sh")
            .arg("-c")
            .arg(format!("echo Process {} is running with PID $$", id))
            .spawn()
            .expect("Failed to spawn process");

        // Wait for the process to complete
        let output = child
            .wait_with_output()
            .expect("Failed to wait for process");
        print!("{}", String::from_utf8_lossy(&output.stdout));
    }
}

fn main() {
    println!("Demonstrating the differences between Threads and Processes in Rust");
    println!("Main process PID: {}", std::process::id());

    println!("\nKey Differences:");
    println!("1. Threads share memory space within the same process");
    println!("2. Processes have independent memory spaces");
    println!("3. Threads are lighter weight than processes");
    println!("4. Processes provide better isolation");

    demonstrate_threads();
    demonstrate_processes();
}
