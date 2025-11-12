#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <node-id>"
  echo "example: $0 2"
  exit 1
fi

NODE_ID="$1"
case "$NODE_ID" in
  1) LISTEN="127.0.0.1:7101" ;;
  2) LISTEN="127.0.0.1:7102" ;;
  3) LISTEN="127.0.0.1:7103" ;;
  *)
    echo "unknown node id: $NODE_ID (expected 1, 2, or 3)"
    exit 1
    ;;
esac

PEERS="1=127.0.0.1:7101,2=127.0.0.1:7102,3=127.0.0.1:7103"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$REPO_ROOT"
cargo run -p raft_replication --bin node -- \
  --id "$NODE_ID" \
  --listen "$LISTEN" \
  --peer "$PEERS"
