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
#   ./scripts/run-tests.sh tier1      # CI fast-test 层：domain、protocol、wasm（秒级）
#   ./scripts/run-tests.sh tier2      # 判定与调试服务：nba-validation、nba-debug-server
#   ./scripts/run-tests.sh tier3      # 动作链内核的整场测试（十二个种子）
#   ./scripts/run-tests.sh tier2+3    # tier2 与 tier3 连续执行
#   ./scripts/run-tests.sh validation # validation 体系：验证目标 + CLI 32 场批次与机制实验
#   ./scripts/run-tests.sh -- ...     # 全量 workspace 测试（不受 CI 依赖）
#
# 全部模式都经 scripts/par_test.py 并行执行：cargo 自身按目标串行调度，
# 按目标并行后 4 核机器上秒级完成。
# 日常验证用 tier1 / tier2，提交前用 validation。
set -uo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$DIR"

# ── 内存上限：整个测试流程放进独立的 systemd scope ──
# 失控的测试二进制曾经涨到 19 GiB：内核判定整机内存耗尽后按全局规则挑目标终止，
# systemd 又把被终止进程所在的 scope 标成失败并清理其余进程，于是同一个 tmux
# 窗格里的编辑器、web 服务都被一起终止。把测试放进自己的 scope 后，超限只会
# 让内核在这个 scope 内部挑目标终止，scope 之外的进程完全不受影响。
# 上限可用 NBA_TEST_MEMORY_MAX 调整；
# 没有可用用户级 systemd 的环境（例如 CI 容器）会跳过这层并提示。
NBA_TEST_MEMORY_MAX="${NBA_TEST_MEMORY_MAX:-8G}"
if [ -z "${NBA_TEST_SCOPED:-}" ]; then
    if timeout 5 systemd-run --user --scope --quiet --collect true >/dev/null 2>&1; then
        echo "🔒 内存上限：MemoryMax=$NBA_TEST_MEMORY_MAX（scope nba-tests-$$）"
        export NBA_TEST_SCOPED=1
        exec systemd-run --user --scope --quiet --collect \
            --unit "nba-tests-$$" \
            -p MemoryAccounting=yes \
            -p MemoryMax="$NBA_TEST_MEMORY_MAX" \
            -p MemorySwapMax=0 \
            -p OOMPolicy=continue \
            -- "$0" "$@"
    fi
    echo "⚠️  没有可用的用户级 systemd，本次测试不带内存上限" >&2
fi

MIN_FREE_MB="${NBA_MIN_FREE_MB:-3072}"
MAX_TARGET_GB="${NBA_MAX_TARGET_GB:-8}"
# 临时文件专用根目录（不与 /tmp、/dev/shm 混用）。
# 默认为仓库内的 .work/tmp：迁移主机后无需改代码，与
# check_disk_budget.py 的「不内置机器特定路径」口径一致。
TEMP_ROOT="${NBA_TEMP_ROOT:-$DIR/.work/tmp}"
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
# tier1：nba-domain、nba-protocol、nba-wasm。
# tier2：nba-validation、nba-debug-server。
# tier3：nba-sim-core 整场测试。
# validation：nba-validation 与 nba-sim-core 全目标；随后跑
#   CLI validate-core（32 场批次 + 机制实验）并写出工件。
MODE="${1:-}"
shift || true
if [ "$MODE" = "--" ]; then
    MODE_LABEL="full workspace"
elif [[ "$MODE" =~ ^(tier1|tier2|tier3|validation|tier2\+3)$ ]]; then
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

# validation 模式追加动作链内核的批次判定与机制实验。
# 退出码：0 通过；1 缺陷或未登记的不足（阻断）；2 执行错误（阻断）；
# 3 只有已登记的不足（记录，不阻断）。
if [ "$MODE" = "validation" ] && [ "$STATUS" -eq 0 ]; then
    echo
    echo "── CLI validate-core（seed 1, 32 场 + 机制实验）──"
    VALIDATE_OUT="$RUN_TMP/validate-core"
    cargo build --release -p nba-sim-cli
    "$DIR/target/release/nba-sim" validate-core --seed 1 --games 32 --mechanism \
        --out "$VALIDATE_OUT"
    VALIDATE_STATUS=$?
    if [ "$VALIDATE_STATUS" -eq 3 ]; then
        echo "⚠️  validate-core：存在已登记的证据不足（exit 3，不阻断）" >&2
    elif [ "$VALIDATE_STATUS" -ne 0 ]; then
        echo "❌ validate-core 判定未通过（exit $VALIDATE_STATUS）" >&2
        STATUS=$VALIDATE_STATUS
    fi
fi

# 3. 后置检查与清理
echo
# 只统计**有数据**的产物；空目录（进程 workspace 骨架）不占空间。
LEFTOVER_FILES="$(find "$RUN_TMP" -type f -not -path "*/validate-core/*" 2>/dev/null | head -20 || true)"
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
    python3 scripts/check_no_index_identity.py || STATUS=$?
fi
if [ "$STATUS" -eq 0 ]; then
    python3 scripts/check_threshold_integrity.py || STATUS=$?
fi
if [ "$STATUS" -eq 0 ]; then
    python3 scripts/check_r6_tendency_wiring.py || STATUS=$?
fi
if [ "$STATUS" -eq 0 ]; then
    python3 scripts/calibrate/freeze.py --check || STATUS=$?
fi

exit "$STATUS"
