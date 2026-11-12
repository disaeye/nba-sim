#!/usr/bin/env zsh
# 最终验收：刷新工件、跑全部静态守卫、验证门禁与认证、记录全量测试。
#
# 只在行为与参数冻结后运行一次。任何一项失败都会让脚本以非零退出码
# 结束，并把每项结果写进 .work/evidence/final-validation.summary。
set -uo pipefail
cd /home/ubuntu/workspace/code/nba-sim

EVIDENCE=.work/evidence
SUMMARY="$EVIDENCE/final-validation.summary"
: > "$SUMMARY"

run() {
    name="$1"
    shift
    log="$EVIDENCE/final-$name.log"
    "$@" > "$log" 2>&1
    command_status=$?
    printf "%-46s %s\n" "$name" "$command_status" | tee -a "$SUMMARY"
    if [ "$command_status" -ne 0 ]; then
        tail -20 "$log" >> "$SUMMARY"
        FAILED=$((FAILED + 1))
    fi
}

FAILED=0

# 构建验收入口。
cargo build --release -p nba-sim-core --bin sim2 --bin measure > "$EVIDENCE/final-build.log" 2>&1
BUILD_STATUS=$?
cargo build --release -p nba-sim-cli >> "$EVIDENCE/final-build.log" 2>&1
BUILD_STATUS=$((BUILD_STATUS + $?))
printf "%-46s %s\n" "build" "$BUILD_STATUS" | tee -a "$SUMMARY"
if [ "$BUILD_STATUS" -ne 0 ]; then
    tail -20 "$EVIDENCE/final-build.log" >> "$SUMMARY"
    FAILED=$((FAILED + 1))
fi

# 门禁证据取自 CLI 的固定 16 场判定入口。
run refresh_gate_evidence python3 scripts/refresh_gate_evidence.py
run gate_cli ./target/release/nba-sim sim-core --seed 1 --gate

# 静态守卫。
run check_docs python3 scripts/check_docs.py
run check_animation_catalog python3 scripts/check_animation_catalog.py
run check_action_runtime_catalog python3 scripts/check_action_runtime_catalog.py
run check_production_incidents python3 scripts/check_production_incidents.py
run audit_rule_programs python3 scripts/audit_rule_programs.py
run check_r6_tendency_wiring python3 scripts/check_r6_tendency_wiring.py
run check_no_index_identity python3 scripts/check_no_index_identity.py
run check_inline_constants python3 scripts/check_inline_constants.py
run check_inline_constants_self python3 scripts/check_inline_constants.py --self-test
run check_max_source_lines python3 scripts/check_max_source_lines.py
run check_max_source_lines_self python3 scripts/check_max_source_lines.py --self-test
run check_threshold_integrity python3 scripts/check_threshold_integrity.py
run check_threshold_integrity_self python3 scripts/check_threshold_integrity.py --self-test
run check_runtime_tables python3 scripts/check_runtime_tables.py
run check_result_fields python3 scripts/check_result_fields.py
run check_tick_source python3 scripts/check_tick_source.py
run check_position_writes python3 scripts/check_position_writes.py
run test_guards python3 scripts/test_guards.py
run check_disk_budget python3 scripts/check_disk_budget.py --report
run freeze_check python3 scripts/calibrate/freeze.py --check
run measure_resources_report python3 scripts/measure_resources.py --report
run measure_resources_check python3 scripts/measure_resources.py --check
run browser_renderers zsh scripts/run_browser_checks.sh
run git_diff_check git diff --check

# 基线指纹按最终参数向量重录，随后跑一次比对测试。
run record_baseline cargo test -p nba-sim-core --test baseline_fingerprint -- --ignored record_baseline_fingerprint
run baseline_compare cargo test -p nba-sim-core --test baseline_fingerprint

# NBA 与 FIBA 认证：两个联赛的工件都写进规范证据目录，供一致性守卫核对。
run validate_nba ./target/release/nba-sim validate-core --seed 1 --games 16 --mechanism --league nba --out docs/dev/evidence/action_chain_validation
run validate_fiba ./target/release/nba-sim validate-core --seed 1 --games 16 --league fiba --out docs/dev/evidence/fiba_validation

# 全量工作区测试：只跑一次，输出同时进入验收合同指定的日志。
run workspace_tests zsh -c 'set -o pipefail; cargo test --workspace > .pi/tasks/final-workspace-tests.log 2>&1; rc=$?; echo "EXIT=$rc" | tee -a .pi/tasks/final-workspace-tests.log; exit "$rc"'

# 认证与门禁工件写完后，核对工件之间的提交身份与判定数字。
run final_artifacts python3 scripts/check_final_artifacts.py

printf "\n==== 最终验收摘要 ====\n" | tee -a "$SUMMARY"
cat "$SUMMARY"
if [ "$FAILED" -ne 0 ]; then
    printf "\n最终验收失败：%s 项\n" "$FAILED" | tee -a "$SUMMARY"
    exit 1
fi
exit 0
