#!/usr/bin/env bash
# Text live broadcast of a simulated game.
# Usage: ./scripts/broadcast.sh [options]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

SEED=42
CONFIG=config/demo-game.json
FILTER=mid
LANG=cn
DELAY=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      cat <<'EOF'
broadcast.sh — Chinese/English play-by-play to stdout

Usage:
  ./scripts/broadcast.sh
  ./scripts/broadcast.sh --seed 7 --filter high
  ./scripts/broadcast.sh --lang en --delay 25 --filter mid

Options:
  --seed N          PRNG seed (default 42)
  --config PATH     game config JSON (default config/demo-game.json)
  --filter all|mid|high   intensity filter (default mid)
  --lang cn|en      language (default cn)
  --delay MS        pause between lines (default 0)
EOF
      exit 0
      ;;
    --seed) SEED="${2:?}"; shift 2 ;;
    --config) CONFIG="${2:?}"; shift 2 ;;
    --filter) FILTER="${2:?}"; shift 2 ;;
    --lang) LANG="${2:?}"; shift 2 ;;
    --delay) DELAY="${2:?}"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 1 ;;
  esac
done

exec npm run broadcast -- \
  --seed "$SEED" \
  --config "$CONFIG" \
  --filter "$FILTER" \
  --lang "$LANG" \
  --delay "$DELAY"
