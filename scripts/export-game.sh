#!/usr/bin/env bash
# Export SpectatorPackage JSON for the 2D web player.
# Usage: ./scripts/export-game.sh [seed] [out-path]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

SEED=42
CONFIG=config/demo-game.json
OUT=spectator/game
GZIP=1
while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      cat <<'EOF'
export-game.sh — write spectator/game.json + game.ticks.ndjson(.gz) for the web replay shell

Usage:
  ./scripts/export-game.sh
  ./scripts/export-game.sh 7
  ./scripts/export-game.sh --seed 7 --out spectator/game-seed7

Defaults: seed=42, out=spectator/game
EOF
      exit 0
      ;;
    --seed) SEED="${2:?}"; shift 2 ;;
    --config) CONFIG="${2:?}"; shift 2 ;;
    --out) OUT="${2:?}"; shift 2 ;;
    --no-gzip) GZIP=0; shift ;;
    *)
      if [[ "$1" =~ ^[0-9]+$ ]]; then SEED="$1"; shift
      else echo "unknown arg: $1" >&2; exit 1
      fi
      ;;
  esac
done

if [[ "$GZIP" -eq 1 ]]; then
  npm run export-ndjson -- --seed "$SEED" --config "$CONFIG" --out "$OUT"
else
  npm run export-ndjson -- --seed "$SEED" --config "$CONFIG" --out "$OUT" --no-gzip
fi
echo "Tip: open with  ./scripts/watch.sh --no-export   (listens on 0.0.0.0:4173)"
