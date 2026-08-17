#!/usr/bin/env bash
# Run one full simulation and print the score line.
# Usage: ./scripts/sim.sh [seed] [config]
#        ./scripts/sim.sh --seed 42 --config config/demo-game.json
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

SEED=42
CONFIG=config/demo-game.json

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      cat <<'EOF'
sim.sh — run one NBA sim game

Usage:
  ./scripts/sim.sh [seed]
  ./scripts/sim.sh --seed 42 --config config/demo-game.json

Defaults: seed=42, config=config/demo-game.json
EOF
      exit 0
      ;;
    --seed) SEED="${2:?}"; shift 2 ;;
    --config) CONFIG="${2:?}"; shift 2 ;;
    *)
      if [[ "$1" =~ ^[0-9]+$ ]]; then SEED="$1"; shift
      else echo "unknown arg: $1" >&2; exit 1
      fi
      ;;
  esac
done

exec npm run sim -- --seed "$SEED" --config "$CONFIG"
