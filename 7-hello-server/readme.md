# Hello World Server

This is a simple server that responds with "Hello, World!" to every request.

## Quick Start

```bash
cargo run
```

Send some requests:

```bash
curl http://127.0.0.1:3000/               
Hello, World!
```

```bash
curl -X POST http://127.0.0.1:3000/users \
  -H "Content-Type: application/json" \
  -d '{"username": "alice"}' | jq 
{
  "id": 1337,
  "username": "alice"
}
```
