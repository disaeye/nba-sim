#!/usr/bin/env bash
# 受约束的测试运行器（gap.md §16.4 / problem.md §14.4）。
#
# 为什么需要它：历史上默认直接跑 `cargo test`，导致
#   - target/ 累积到 9.1 GiB 并写满根分区；
#   - 测试 panic 后临时 NDJSON 残留，多轮累积到数 GB。
#
# 本脚本把资源约束前置到测试流程里：
#   1. 运行前检查磁盘余量与 target/ 体积，超限直接拒绝启动；
#   2. 测试临时文件统一收敛到本次运行的私有 TMPDIR，退出时无条件清理；
#   3. 结束后再次检查并报告磁盘变化。
#
# 用法：
#   ./scripts/run-tests.sh                     # workspace release 测试
#   ./scripts/run-tests.sh -p nba-engine       # 传给 cargo test 的参数
#   ./scripts/run-tests.sh --release --test constraint_system
set -uo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$DIR"

MIN_FREE_MB="${NBA_MIN_FREE_MB:-3072}"
MAX_TARGET_GB="${NBA_MAX_TARGET_GB:-8}"
# 临时文件专用根目录（不与 /tmp、/dev/shm 混用）。
TEMP_ROOT="${NBA_TEMP_ROOT:-/home/ubuntu/basketball}"
mkdir -p "$TEMP_ROOT"
RUN_TMP="$(mktemp -d "$TEMP_ROOT/nba_testrun_XXXXXX")"
START_FREE="$(df -Pm "$DIR" | awk 'NR==2{print $4}')"

cleanup() {
    rm -rf "$RUN_TMP"
}
trap cleanup EXIT INT TERM

fail() {
    echo "❌ $*" >&2
    exit 1
}

echo "🧪 NBA-Sim constrained test runner"
echo "   temp root  : $TEMP_ROOT"
echo "   run TMPDIR : $RUN_TMP"
echo "   free start : ${START_FREE} MiB"

# 1. 前置资源门
if [ "$START_FREE" -lt "$MIN_FREE_MB" ]; then
    fail "only ${START_FREE} MiB free, below the ${MIN_FREE_MB} MiB safety line. Run: python3 scripts/check_disk_budget.py --clean"
fi

TARGET_GB=$(du -sm target 2>/dev/null | awk '{printf "%d", $1/1024}')
TARGET_GB=${TARGET_GB:-0}
if [ "$TARGET_GB" -gt "$MAX_TARGET_GB" ]; then
    fail "target/ is ${TARGET_GB} GiB, above the ${MAX_TARGET_GB} GiB limit. Run: cargo clean"
fi

# 2. 关闭增量编译（纯属浪费），并把临时目录收敛到本次运行
export CARGO_INCREMENTAL=0
export TMPDIR="$RUN_TMP"
export NBA_TEST_TMP="$RUN_TMP"

echo "   cargo args : $*"
echo

cargo test --release "$@"
STATUS=$?

# 3. 后置检查与清理
echo
# 只统计**有数据**的产物；空目录（进程 workspace 骨架）不占空间。
LEFTOVER_FILES="$(find "$RUN_TMP" -type f 2>/dev/null | head -20 || true)"
LEFTOVER_BYTES="$(du -sb "$RUN_TMP" 2>/dev/null | awk '{print $1}')"
cleanup

END_FREE="$(df -Pm "$DIR" | awk 'NR==2{print $4}')"
echo "📊 Resources: free ${START_FREE} MiB → ${END_FREE} MiB, tmp bytes=${LEFTOVER_BYTES:-0}"

if [ -z "$LEFTOVER_FILES" ]; then
    echo "✅ no temp artifacts left behind (panic-safe cleanup verified)"
else
    echo "⚠️  test left temp artifacts behind:"
    echo "$LEFTOVER_FILES"
fi

python3 scripts/check_disk_budget.py --report || true

exit "$STATUS"
