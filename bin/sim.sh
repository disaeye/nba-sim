#!/usr/bin/env bash
set -e

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"

if [ ! -f "$DIR/engine/target/release/nba-sim-engine" ]; then
    echo "📦 Compiling NBA-Sim Rust Engine (Release Mode)..."
    (cd "$DIR/engine" && cargo build --release)
fi

SEED="${1:-42}"
OUT_FILE="${2:-$DIR/output/game.ticks.ndjson}"
SCOPE="${3:-1q}"

"$DIR/engine/target/release/nba-sim-engine" "$SEED" "$OUT_FILE" "$SCOPE"

# Generate gzip-compressed stream for high-performance web spectator
if command -v gzip >/dev/null 2>&1; then
    gzip -f -k -9 "$OUT_FILE" 2>/dev/null || true
fi
