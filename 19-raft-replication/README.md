# 19-raft-replication

Raft-backed reimagining of the primary/replica example. Instead of pushing
updates over bespoke TCP messages, this crate spins up a tiny in-memory Raft
cluster (three `RawNode` instances) and exposes two interactive shells that talk
to each other over a simple TCP protocol.

- `node` — single binary you run once per process. Each instance owns one Raft
  peer, participates in leader election, and exposes a CLI for `PUT`/`GET`.

## Running the demos

Every node needs a unique ID, a listen address, and the full peer list:

```bash
# Terminal 1
cargo run -p raft_replication --bin node -- \
  --id 1 --listen 127.0.0.1:7101 \
  --peer 1=127.0.0.1:7101,2=127.0.0.1:7102,3=127.0.0.1:7103

# Terminal 2
cargo run -p raft_replication --bin node -- \
  --id 2 --listen 127.0.0.1:7102 \
  --peer 1=127.0.0.1:7101,2=127.0.0.1:7102,3=127.0.0.1:7103

# Terminal 3
cargo run -p raft_replication --bin node -- \
  --id 3 --listen 127.0.0.1:7103 \
  --peer 1=127.0.0.1:7101,2=127.0.0.1:7102,3=127.0.0.1:7103
```

After all three boot, they automatically elect a leader. In any terminal you can
type:

```
PUT <key> <value>
GET <key>
STATUS
HELP
EXIT
```

Every `PUT` becomes a `CommandPayload`, is proposed on the current leader, and
only prints “OK” in the CLI once the log entry commits locally. The networking
layer forwards actual Raft messages between the configured peers using a tiny
length-prefixed TCP protocol, so leader election and replication happen across
processes.

## File structure

- `src/command.rs` — `CommandPayload` definitions and serialization helpers.
- `src/store.rs` — simple `KvStore` with snapshots for debugging.
- `src/node.rs` — thin wrapper around `RawNode<MemStorage>` that mirrors the
  Ready loop from the Raft docs.
- `src/cluster.rs` — in-memory network that routes Raft messages between nodes
  and exposes helpers for proposing, waiting, and inspecting snapshots.
- `src/topology.rs` — keeps node listings consistent and ensures odd sizes.
- `src/protocol.rs` — CLI parsing shared by the REPL and (minimal) TCP helpers.
- `src/runtime.rs` — orchestrates the Raft-ready loop, networking, and command
  queue for a single node.
- `src/bin/node.rs` — CLI front-end for each process.

This crate intentionally mirrors the structure of the earlier primary/replica
example while grounding the behaviour in the real Raft state machine from
`raft-rs`.
