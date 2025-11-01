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
    self_pointer: *const String
}

impl SelfReferential {
    fn new(data: String) -> SelfReferential {
        let mut sr = SelfReferential {
            data,
            self_pointer: ptr::null()
        };
        sr.self_pointer = &sr.data as *const String;
        sr
    }
    
    fn print(&self) {
        unsafe {
            println!("{}", *self.self_pointer);
        }
    }
}

#[tokio::main]
async fn main() {
    println!("Hello, world!");
    let first = SelfReferential::new("first".to_string());
    let moved_first = first;
    moved_first.print();
}
