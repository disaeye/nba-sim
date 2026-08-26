#!/usr/bin/env bash
set -e

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"

if [ ! -f "$DIR/engine/target/release/nba-sim-engine" ]; then
    echo "📦 Compiling NBA-Sim Rust Engine (Release Mode)..."
    (cd "$DIR/engine" && cargo build --release)
fi

SEED="${1:-42}"
OUT_FILE="${2:-$DIR/web/game.ticks.ndjson}"
TICKS="${3:-7200}"

exec "$DIR/engine/target/release/nba-sim-engine" "$SEED" "$OUT_FILE" "$TICKS"
