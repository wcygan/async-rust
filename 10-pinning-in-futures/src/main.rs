use std::pin::Pin;
use std::ptr;

/// In Rust, the compiler often moves values around in memory.
/// For instance, if we move a variable into a function,
/// the memory may be moved.
///
/// It's not just moving values that may result in moving
/// memory addresses. Collection can also change memory addresses.
/// For instance, if a vector gets to capacity, the vector
/// will be reallocated in memory, changing the memory address.
///
/// Most normal primitives such as number, string, bool, structs,
/// and enum implement the `Unpin` trait, enabling them to be
/// moved around.
///
/// We know that Futures can get moved, as we use `async move` in
/// our code when spawning a task. However, moving can be
/// dangerous. To demonstrate the data, we can build
/// a basic struct that references itself:

struct SelfReferential {
    data: String,
    self_pointer: *const String,
}

impl SelfReferential {
    fn new(data: String) -> SelfReferential {
        let mut sr = SelfReferential {
            data,
            self_pointer: ptr::null(),
        };
        sr.self_pointer = &sr.data as *const String;
        sr
    }

    fn print(&self) {
        unsafe {
            println!(
                "Data address: {:p}, Pointer points to: {:p}",
                &self.data as *const String, self.self_pointer
            );
            println!("Data value: {}", *self.self_pointer);
        }
    }
}

struct PinnedSelfReferential {
    data: String,
    self_pointer: *const String,
}

impl PinnedSelfReferential {
    fn new(data: String) -> Pin<Box<Self>> {
        let mut boxed = Box::new(PinnedSelfReferential {
            data,
            self_pointer: ptr::null(),
        });
        boxed.self_pointer = &boxed.data as *const String;
        Box::pin(*boxed)
    }

    fn print(self: Pin<&Self>) {
        unsafe {
            println!(
                "Data address: {:p}, Pointer points to: {:p}",
                &self.data as *const String, self.self_pointer
            );
            println!("Data value: {}", *self.self_pointer);
        }
    }
}

#[tokio::main]
async fn main() {
    println!("=== Testing PinnedSelfReferential (Safe) ===");
    let pinned = PinnedSelfReferential::new("pinned data".to_string());
    println!("Before move:");
    pinned.as_ref().print();

    println!("\nAfter move (Pin prevents memory address change):");
    let moved_pinned = pinned;
    moved_pinned.as_ref().print();

    println!("\n=== Testing SelfReferential (Unsafe - demonstrates the problem) ===");
    let first = SelfReferential::new("unpinned data".to_string());
    println!("Before move:");
    first.print();

    println!("\nAfter move (addresses may differ, causing UB):");
    let moved_first = first;
    moved_first.print();
}
