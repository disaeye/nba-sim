#!/usr/bin/env bash
set -e

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"

BIN="$DIR/target/release/nba-sim"

if [ ! -f "$BIN" ]; then
    echo "📦 Compiling NBA-Sim workspace (Release Mode)..."
    (cd "$DIR" && cargo build --release --bin nba-sim)
fi

SEED="${1:-42}"
OUT_FILE="${2:-$DIR/output/game.ticks.ndjson}"
SCOPE="${3:-1q}"

mkdir -p "$(dirname "$OUT_FILE")"
"$BIN" "$SEED" "$OUT_FILE" "$SCOPE"

# Optional gzip-compressed stream for downstream consumers
if command -v gzip >/dev/null 2>&1; then
    gzip -f -k -9 "$OUT_FILE" 2>/dev/null || true
fi
