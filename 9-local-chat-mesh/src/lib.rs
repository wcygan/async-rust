//! Async chat example that runs entirely on one machine.
//!
//! See `README.md` for an overview, usage instructions, and the JSON
//! message protocol. Each module focuses on a concrete responsibility:
//!
//! - [`cli`] parses the command-line interface for broker and client modes.
//! - [`broker`] accepts TCP connections, keeps track of participants, and
//!   broadcasts messages over a Tokio `broadcast` channel.
//! - [`client`] connects to the broker, multiplexing stdin and server
//!   messages for a terminal user.
//! - [`message`] provides the JSON line protocol plus helpers for async
//!   reads and writes.
//!
//! Integration and unit tests use this crate directly to exercise the
//! broker state machine and wire protocol.

pub mod broker;
pub mod cli;
pub mod client;
pub mod message;
