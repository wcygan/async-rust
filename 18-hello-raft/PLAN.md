# 18-hello-raft Plan

This plan keeps the demo focused on Raft fundamentals (elections, log replication, applying committed entries) while preventing the codebase from turning into a monolith. Each milestone is scoped to an isolated module so we can pause after any step and still have a runnable, teachable artifact.

## Learning Goals

1. **Connect docs to code** — mirror the [`raft::RawNode` ready loop](https://docs.rs/raft/latest/raft/struct.RawNode.html) so readers can trace each bullet point (persist, send, apply).
2. **Touch a state machine** — show how replicated log entries become key/value mutations with minimal serialization overhead.
3. **Understand quorum behavior** — surface what happens when a node lags or stops ticking by letting readers toggle scenarios.

## Milestones

| Phase | Focus | Deliverable | Notes |
| --- | --- | --- | --- |
| 0 | Baseline | `cargo run -p 18-hello-raft -- demo basic` prints a 3-node election and a single `Put` command. | Single-threaded loop, `MemStorage`, no networking. |
| 1 | Modularity | Modules for `command`, `state_machine`, `node`, `network`, `cluster`. | Each <150 LOC with doc comments. |
| 2 | Scenarios | CLI subcommands: `basic`, `lagging-follower`, `drop-leader`. | Implement by muting ticks or message delivery. |
| 3 | Async bridge (optional) | Feature flag `runtime-tokio` spawns one task per node and communicates via channels. | Off by default to avoid cognitive overload. |
| 4 | Persistence (stretch) | Swap `MemStorage` for custom `FileStorage` implementing `RaftStorage`. | Only after learners grasp in-memory flow. |

## Module Boundaries

```
app (main.rs)
└── cluster::{Cluster, Scenario}
    ├── network::{Network, Envelope}
    ├── node::{DemoNode, ReadyOutcome}
    │   ├── command::{Command, Codec}
    │   └── state_machine::{KvStore}
    └── scenario::* helpers (e.g., drop leader)
```

- `command`: defines the log payload enum plus `encode/decode`.
- `state_machine`: wraps `HashMap<String, String>` with `apply(Command) -> EntryOutcome`.
- `node`: owns `RawNode<MemStorage>` and exposes `tick`, `campaign`, `propose`, `drain_ready`.
- `network`: in-memory router with hooks to simulate latency or drops.
- `cluster`: drives ticks, detects leaders, orchestrates scenarios.

## Implementation Notes

- Stick to `MemStorage::new_with_conf_state(ConfState::from((peers, vec![])))` until Phase 4.
- Use `bincode` or `serde_json` for command bytes; keep payloads tiny to avoid overwhelming the Ready loop.
- Annotate each `node` method with the equivalent bullet from the Raft docs (e.g., `// 1. Persist HardState + Entries`).
- Instrument with `tracing` or `println!` behind a `--verbose` flag so default output stays approachable.

## Acceptance Criteria

1. `cargo run -p 18-hello-raft -- demo basic` shows:
   - leader election among three nodes,
   - a proposed `Put` command,
   - the final replicated key/value pair on all nodes.
2. README documents modules + demos, and links to PLAN.md.
3. PLAN.md stays updated as we move between milestones.
