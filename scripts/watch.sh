#!/usr/bin/env bash
# One-shot: simulate → export JSON → serve spectator web UI.
# Usage: ./scripts/watch.sh [--seed N] [--port PORT] [--no-export]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

SEED=42
CONFIG=config/demo-game.json
OUT=spectator/game
PORT=4173
DO_EXPORT=1

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      cat <<'EOF'
watch.sh — export a game (optional) and serve the 2D spectator

Usage:
  ./scripts/watch.sh
  ./scripts/watch.sh --seed 7
  ./scripts/watch.sh --seed 7 --port 5000
  ./scripts/watch.sh --no-export          # only serve existing spectator/game.json

Then open: http://0.0.0.0:${PORT:-4173} (or this host's LAN IP) → "Load demo path"
EOF
      exit 0
      ;;
    --seed) SEED="${2:?}"; shift 2 ;;
    --config) CONFIG="${2:?}"; shift 2 ;;
    --out) OUT="${2:?}"; shift 2 ;;
    --port) PORT="${2:?}"; shift 2 ;;
    --no-export) DO_EXPORT=0; shift ;;
    *) echo "unknown arg: $1" >&2; exit 1 ;;
  esac
done

if [[ "$DO_EXPORT" -eq 1 ]]; then
  echo "→ export seed=$SEED → $OUT (.json + .ticks.ndjson)"
  npm run export-ndjson -- --seed "$SEED" --config "$CONFIG" --out "$OUT"
fi

DATA_FILE="$OUT"
if [[ "$DATA_FILE" != *.json ]]; then DATA_FILE="${DATA_FILE}.json"; fi
if [[ ! -f "$DATA_FILE" ]]; then
  echo "missing $DATA_FILE — run without --no-export first" >&2
  exit 1
fi

if command -v ss >/dev/null 2>&1; then
  LISTENER="$(ss -ltnp "( sport = :${PORT} )" 2>/dev/null || true)"
  if [[ "$LISTENER" == *LISTEN* ]]; then
    PID=""
    if [[ "$LISTENER" =~ pid=([0-9]+) ]]; then PID="${BASH_REMATCH[1]}"; fi
    CMD="${PID:+$(ps -p "$PID" -o args= 2>/dev/null || true)}"
    if [[ "$CMD" == *"serve spectator"* || "$CMD" == *"serve-spectator.py"* ]]; then
      echo "spectator already running on ${PORT} (pid ${PID:-unknown}); refreshed files are already visible"
      exit 0
    fi
    echo "port ${PORT} is occupied by: ${CMD:-unknown process}" >&2
    echo "use --port <free-port> or stop that process before starting watch.sh" >&2
    exit 1
  fi
fi

LISTEN="0.0.0.0:${PORT}"
echo "→ serve spectator/ on ${LISTEN}"
echo "   local:  http://127.0.0.1:${PORT}"
echo "   LAN:    http://<this-host-ip>:${PORT}"
echo "   click 「Load demo path」 in the page"
if [[ "$PORT" == "4173" ]]; then
  PORT="$PORT" exec python3 scripts/serve-spectator.py
fi
exec python3 -m http.server "$PORT" --bind 0.0.0.0 --directory spectator
