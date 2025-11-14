//! Raft-based replicated key-value store.
//!
//! This crate implements a distributed key-value store using the Raft consensus algorithm.
//! Multiple nodes form a cluster where writes are replicated through Raft's log, ensuring
//! all nodes eventually converge to the same state.
//!
//! # Architecture
//!
//! The system uses a multi-threaded architecture to separate concerns:
//!
//! - **Main thread**: Runs the REPL, processes user commands
//! - **Worker thread**: Drives the Raft state machine, processes client requests
//! - **Network listener thread**: Accepts incoming Raft messages from peers
//! - **Connection handler threads**: Short-lived threads that forward received messages to the worker
//!
//! Communication between threads uses crossbeam channels. This design keeps the Raft state machine
//! single-threaded (simpler reasoning, no locks) while network I/O runs concurrently.
//!
//! # Why this design?
//!
//! - **Blocking channels**: Simpler than async, sufficient for this workload
//! - **Single-threaded worker**: Raft state machine has complex state; avoiding concurrent
//!   access eliminates entire classes of bugs
//! - **Separate network threads**: I/O can block without stalling Raft processing
//!
//! # Modules
//!
//! - [`node`]: Core Raft node wrapping tikv/raft library
//! - [`runtime`]: Worker loop, network handling, node spawning
//! - [`store`]: Thread-safe in-memory key-value storage
//! - [`command`]: Commands replicated through Raft log
//! - [`protocol`]: REPL command parsing

pub mod command;
pub mod node;
pub mod protocol;
pub mod runtime;
pub mod store;
