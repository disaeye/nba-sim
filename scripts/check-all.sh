#!/usr/bin/env bash
# Full quality gate: foundation lint + tsc + complete reality scan + tests + pace (optional skip).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

SKIP_PACE=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      cat <<'EOF'
check-all.sh — foundation:lint + tsc + test + reality:check + pace:check

Usage:
  ./scripts/check-all.sh
  ./scripts/check-all.sh --skip-pace   # faster (no 256-game sweep)
EOF
      exit 0
      ;;
    --skip-pace) SKIP_PACE=1; shift ;;
    *) echo "unknown arg: $1" >&2; exit 1 ;;
  esac
done

echo "═══ 1/5 foundation:lint ═══"
npm run foundation:lint

echo "═══ 2/5 tsc --noEmit ═══"
npx tsc --noEmit

echo "═══ 3/5 reality:scan (16 seeds) ═══"
npm run reality:check

echo "═══ 4/5 vitest ═══"
npm test

if [[ "$SKIP_PACE" -eq 0 ]]; then
  echo "═══ 5/5 pace:check (256 seeds) ═══"
  npm run pace:check
else
  echo "═══ 5/5 pace:check SKIPPED ═══"
fi

echo "═══ ALL GREEN ═══"
