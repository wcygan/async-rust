# File Writer

This program continuously writes to a file.

## Usage

In one process, I've started the [file writer](../5-file-writer/readme.md):

```bash
cargo run -- --filepath log.txt
OSSCybnrXzwd3DNo0sd3y
ST4a9qdpYbfaQVmt103Df
Dz3HdKPbL8r_Om0cV1BJ6
```

In another process, I've started the [file watcher](../4-file-watcher/readme.md):

```bash
cargo run -- --filepath ../5-file-writer/log.txt
OSSCybnrXzwd3DNo0sd3y

OSSCybnrXzwd3DNo0sd3y

OSSCybnrXzwd3DNo0sd3y
ST4a9qdpYbfaQVmt103Df

OSSCybnrXzwd3DNo0sd3y
ST4a9qdpYbfaQVmt103Df
Dz3HdKPbL8r_Om0cV1BJ6
```

Notice how it doesn't behave like `tail -f`, it prints out the entire file each time it changes.

We will improve this in a future example.