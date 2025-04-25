use std::process::Command;
use std::thread;
use std::time::Duration;
use sysinfo::{Pid, System};

fn get_memory_info() -> (u64, u64) {
    let sys = System::new_all();
    let total_memory = sys.total_memory(); // KB
    let used_memory = total_memory - sys.available_memory();
    (used_memory, total_memory)
}

fn demonstrate_threads() {
    println!("\n=== Thread Demonstration ===");
    let (initial_used, total) = get_memory_info();
    println!(
        "Initial memory usage: {:.1} MB / {:.1} MB",
        initial_used as f64 / 1024.0,
        total as f64 / 1024.0
    );

    let mut handles = vec![];
    let shared_data = vec![1, 2, 3, 4, 5];

    // Spawn 3 threads that can all access the same data
    for id in 0..3 {
        let data = shared_data.clone();
        let handle = thread::spawn(move || {
            println!("Thread {} is running", id);
            println!("Thread {} sees shared data: {:?}", id, data);

            // Get memory info from within thread
            let (used, total) = get_memory_info();
            println!(
                "Thread {} memory usage: {:.1} MB / {:.1} MB",
                id,
                used as f64 / 1024.0,
                total as f64 / 1024.0
            );

            thread::sleep(Duration::from_millis(500));
            println!("Thread {} finished", id);
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    let (final_used, _) = get_memory_info();
    println!(
        "Final memory usage: {:.1} MB / {:.1} MB",
        final_used as f64 / 1024.0,
        total as f64 / 1024.0
    );
}

fn demonstrate_processes() {
    println!("\n=== Process Demonstration ===");
    let (initial_used, total) = get_memory_info();
    println!(
        "Initial memory usage: {:.1} MB / {:.1} MB",
        initial_used as f64 / 1024.0,
        total as f64 / 1024.0
    );

    let mut sys = System::new_all();

    // Spawn 3 separate processes
    for id in 0..3 {
        let child = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "echo Process {} is running with PID $$; sleep 0.5",
                id
            ))
            .spawn()
            .expect("Failed to spawn process");

        // Get the child process info
        thread::sleep(Duration::from_millis(100)); // Give process time to start
        sys.refresh_all();

        if let Some(process) = sys.process(Pid::from_u32(child.id())) {
            println!(
                "Process {} (PID: {}) memory usage: {:.1} MB",
                id,
                child.id(),
                process.memory() as f64 / 1024.0 / 1024.0
            );
        }

        let output = child
            .wait_with_output()
            .expect("Failed to wait for process");
        print!("{}", String::from_utf8_lossy(&output.stdout));
    }

    let (final_used, _) = get_memory_info();
    println!(
        "Final memory usage: {:.1} MB / {:.1} MB",
        final_used as f64 / 1024.0,
        total as f64 / 1024.0
    );
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
