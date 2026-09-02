#!/usr/bin/env bash
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"

BIN="$DIR/target/release/nba-debug-server"
if [ ! -x "$BIN" ]; then
    (cd "$DIR" && cargo build --release -p nba-debug-server)
fi

PORT="${1:-4173}"
exec "$BIN" "$PORT"
