# Local Chat Mesh

Local Chat Mesh is a Tokio-based example that demonstrates how to build a small chat system entirely on one machine. A single broker process accepts TCP connections on the loopback interface, while terminal clients connect, exchange JSON line messages, and receive updates about roster changes plus chat messages.

## Running the Example

```bash
# terminal 1 – start the broker
cargo run -p local_chat_mesh -- broker --listen 127.0.0.1:5000

# terminal 2 – connect as alice
cargo run -p local_chat_mesh -- client --nickname alice --server 127.0.0.1:5000

# terminal 3 – connect as bob
cargo run -p local_chat_mesh -- client --nickname bob --server 127.0.0.1:5000
```

Each client sends a JSON `hello` handshake on connect, displays roster/user join/leave notifications, and can send chat lines by typing into stdin. Enter `/quit` or hit `Ctrl+D` to leave.

## Project Layout

| Module | Purpose |
| ------ | ------- |
| `cli` | Defines the broker/client subcommands and shared options. |
| `broker` | Runs the listener, tracks connection state, and fans out messages to subscribed clients. |
| `client` | Handles stdin/stdout interaction for a single terminal user and forwards messages to the broker. |
| `message` | Declares the JSON message types and provides async helpers for reading/writing framed lines. |

`src/main.rs` wires everything together: it installs tracing, parses CLI arguments, and dispatches to the chosen mode. `src/lib.rs` re-exports modules so tests can import them easily.

## Message Protocol

All traffic is newline-delimited JSON objects.

### Client → Server

- `{"type":"hello","nickname":"alice"}` — handshake (must be first message).
- `{"type":"chat","text":"hello world"}` — send a chat line.

### Server → Client

- `welcome` — acknowledges the nickname reserved by the broker.
- `roster` — current participants when you join.
- `user_joined` / `user_left` — notify all clients about roster changes.
- `chat` — broadcasted chat lines.
- `error` — human-readable error messages (e.g., nickname in use, lag warnings, shutdown notice).

## Testing

The crate ships with unit, integration, and CLI end-to-end tests:

```bash
cargo test -p local_chat_mesh
```

Coverage includes JSON codec round-tripping, broker state validation, an integration harness that exercises the broker module directly, and a full binary-level e2e test that spawns the broker and two CLI clients to verify join/chat/shutdown semantics.
