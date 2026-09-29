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
#   ./scripts/run-tests.sh tier1      # CI fast-test 层：不变量与隔离测试（分钟级）
#   ./scripts/run-tests.sh tier2      # CI full-gate 层：机制与因果测试
#   ./scripts/run-tests.sh tier3      # 8 种子全场统计基线（最重，建议后台任务执行）
#   ./scripts/run-tests.sh tier2+3    # CI full-gate 等价：tier2 与 tier3 连续执行
#   ./scripts/run-tests.sh -- ...     # 传统全量 workspace 测试（不受 CI 依赖，最重）
#
# 全部模式都经 scripts/par_test.py 并行执行：cargo 自身按目标串行调度，
# 4 核机器上全量超过 1000s；按目标并行 + 目标内多种子并行后实测约 950s。
# 4 核 / 3.7GB 内存下全量 5 分钟不可达：总核秒约 2400，理论下限 600s，
# 且多路全场模拟同跑会触发 OOM（实测 stats_baseline 与其他重目标同跑
# 被内核杀掉，par_test 已把 stats_baseline 排除出并行池串行独占）。
# 日常验证用 tier1（约 1 分钟）/ tier2（约 4 分钟）/ tier2+3（约 11 分钟），
# 全量留在提交前或 CI。
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

# ── 分层选择由 par_test.py 根据 Cargo metadata 执行并核对 ──
# tier1：nba-domain、nba-invariants、快照、投影与 golden hash。
# tier2：机制与因果目标，加上 Play、D25 action_phase 与 D26 整场测试。
# tier3：stats_baseline 整场测试。D26 与 stats_baseline 均由 --serial 独占运行。
MODE="${1:-}"
shift || true
if [ "$MODE" = "--" ]; then
    MODE_LABEL="full workspace"
elif [[ "$MODE" =~ ^(tier1|tier2|tier3|tier2\+3)$ ]]; then
    MODE_LABEL="$MODE"
else
    fail "unknown mode: $MODE (expected tier1 | tier2 | tier3 | tier2+3 | --)"
fi

echo "🧪 NBA-Sim constrained test runner"
echo "   temp root  : $TEMP_ROOT"
echo "   run TMPDIR : $RUN_TMP"
echo "   free start : ${START_FREE} MiB"
echo "   mode       : $MODE_LABEL"

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

echo

# 先按当前 Cargo metadata 校验 tier 选择器、预期名称与命中集合。
python3 scripts/par_test.py --self-test
SELECTION_STATUS=$?
if [ "$SELECTION_STATUS" -ne 0 ]; then
    exit "$SELECTION_STATUS"
fi

# 先构建全部测试二进制；par_test.py 随后核对 Cargo metadata 与产物目标集合。
cargo test --release --workspace --no-run
BUILD_STATUS=$?
if [ "$BUILD_STATUS" -ne 0 ]; then
    exit "$BUILD_STATUS"
fi

# tier 选择由 par_test.py 使用 package::target 精确筛选；全量模式不筛选。
if [ "$MODE" = "--" ]; then
    python3 scripts/par_test.py --jobs 2 --threads-per-bin 2
else
    python3 scripts/par_test.py --jobs 2 --threads-per-bin 2 --tier "$MODE"
fi
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

# 3.5 静态守卫门（与 CI guards matrix 同源）：文档引用、源文件行数、
# 内联行为常数棘轮。任何一项失败使本次运行变红。
if [ "$STATUS" -eq 0 ]; then
    python3 scripts/check_docs.py || STATUS=$?
fi
if [ "$STATUS" -eq 0 ]; then
    python3 scripts/check_max_source_lines.py || STATUS=$?
fi
if [ "$STATUS" -eq 0 ]; then
    python3 scripts/check_inline_constants.py || STATUS=$?
fi
if [ "$STATUS" -eq 0 ]; then
    python3 scripts/check_ball_state_writes.py || STATUS=$?
fi

# 4. 性能基准带（quality.md §4.1）：fixture 冻结的吞吐带由守卫比对。
# 计入退出码：接在测试后面但 `|| true` 会让它等于没接——性能退化与
# 行为退化同属回归，失败必须使本次运行变红。测量基于本机 release 构建，
# 此处 target/ 刚被全套件热过，CPU 状态与提交前验证一致。
if [ "$STATUS" -eq 0 ]; then
    python3 scripts/check_perf_fixture.py || STATUS=$?
fi

exit "$STATUS"
