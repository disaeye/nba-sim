#!/usr/bin/env bash
# Regenerate golden seed fixtures after intentional kernel/config changes.
# Usage: ./scripts/regen-golden.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "→ regenerate fixtures/golden-seed-{42,7}.json"
npx tsx scripts/generate-golden.ts
echo "→ run canonical golden determinism tests"
npx vitest run tests/simulate/canonical-game.test.ts
echo "done"
