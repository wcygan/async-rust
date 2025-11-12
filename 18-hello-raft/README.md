# 18-hello-raft

Hands-on Raft playground built on [`raft-rs`](https://github.com/tikv/raft-rs). The crate walks through the Ready loop, shows how replicated entries feed a toy key-value store, and lets you script small cluster experiments without spinning up a full distributed system.

## Quick Start

```bash
cargo run -p 18-hello-raft -- demo basic
```

That command (once implemented) will:

1. Bring up three `RawNode` instances backed by `MemStorage`.
2. Let them elect a leader.
3. Propose a `Command::Put` entry and apply it across the cluster, printing the resulting key/value state per node.

Additional subcommands (lagging follower, drop leader, async runtime, etc.) are sketched in `PLAN.md`.

## Modules (planned)

- `command`: serializable enum describing log payloads (`Put`, future `Delete`, config changes).
- `state_machine`: minimal `HashMap` wrapper that applies decoded commands.
- `node`: thin façade over `RawNode<MemStorage>` exposing `tick`, `propose`, `drain_ready`.
- `network`: in-memory router with hooks for latency/drops to keep scenarios deterministic.
- `cluster`: ties nodes + network together and implements the CLI demo scenarios.

## Learning Roadmap

See [`PLAN.md`](./PLAN.md) for milestones, module boundaries, and acceptance criteria. Each phase is intentionally small so you can stop after any milestone and still have a working example to reason about.

## References

1. https://github.com/tikv/raft-rs
2. https://tikv.org/blog/implement-raft-in-rust/
3. https://docs.rs/raft/latest/raft/
