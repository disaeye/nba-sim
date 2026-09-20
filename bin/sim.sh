#!/usr/bin/env bash
# 单场模拟入口。默认输出到临时目录，避免在工作区累积大文件
# （gap.md §16.4 / problem.md §14.4）。
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"

BIN="$DIR/target/release/nba-sim"
SCOPE="${3:-1q}"
STREAM_MODE="${4:-facts}"

if [ ! -x "$BIN" ]; then
    echo "📦 Compiling NBA-Sim CLI (release)..."
    (cd "$DIR" && cargo build --release -p nba-sim-cli)
fi

SEED="${1:-42}"
if [ -n "${2:-}" ]; then
    OUT_FILE="$2"
else
    # 默认写入临时目录并带时间戳，避免覆盖与累积。
    OUT_FILE="${NBA_TEMP_ROOT:-/home/ubuntu/basketball}/nba_sim_seed${SEED}_$(date +%s).ndjson"
fi

mkdir -p "$(dirname "$OUT_FILE")"

echo "▶️  seed=$SEED scope=$SCOPE mode=$STREAM_MODE out=$OUT_FILE"
"$BIN" "$SEED" "$OUT_FILE" "$SCOPE" --stream-mode "$STREAM_MODE"

SIZE="$(du -h "$OUT_FILE" 2>/dev/null | awk '{print $1}')"
echo "📦 stream size: ${SIZE:-unknown}"
echo "   (delete with: rm -f '$OUT_FILE'*)"
