# NBA-Sim 验收记录

> 本文件记录最终验收状态。判定数字以 `docs/dev/evidence/` 下的工件与 `.work/evidence/final-validation.summary` 为准。

## 1. 验收入口

- 权威方案：`docs/dev/current/full_simulation_plan.md`
- 当前状态：`docs/dev/status.md`
- 实施进度：`docs/dev/current/implementation.md`
- 阶段与证据对应：`docs/dev/evidence/stage_gate_map.md`
- 设计说明：`docs/dev/action_chain_engine.md`
- 裁决设计：`docs/dev/current/adjudication_design.md`
- 公开协议：`docs/protocol.md`
- 验收链：`scripts/final_validation.sh`
- CI 守卫：`.github/workflows/ci.yml`、`.github/workflows/docs.yml`

## 2. 内核与规则状态

`crates/sim` 是唯一比赛模拟内核。NBA 与 FIBA 档案从 CLI、WASM、debug-server 与会话检查点进入同一内核，`MatchSetup`、磁盘检查点与回放清单可完整恢复。`LeagueProfile` 按 11 个策略族保存，缺字段直接失败。

`World::step` 收集 `RawIncident`，`AdjudicationBatch` 生成完整计划，`World::adjudicate` 原子提交。特殊犯规、伤病、教练挑战、复核程序、球队技术犯规与转换阻断由完整比赛自发产生；抽样式接触事实只在活球阶段成立。22 项规则程序都有版本、条款、场景矩阵与生产消费者，`data/rules/rule_catalog.json` 的 `known_current_gaps` 为空。

16 个动作族与 111 个动作条目各有生产候选、结算器、场景测试与渲染配方。`StreamTick` 带比赛阶段、程序、球态、计时器运行标志、规则版本、校准版本、引擎模型版本、活动动作与程序资格。`FrameEvent` 带事件身份、控制序列身份与来源事件；事件流可独立重建比分、控制、时间、犯规、资格、暂停、罚则、动作生命周期与节末结算。

## 3. 门禁与认证

`nba-sim sim-core --gate` 的固定 16 场判定覆盖 39 条分布与过程判据，逐条观测值写入 `docs/dev/evidence/gap_closure_cycle/gate.txt`，汇总写入同目录的 `verification.json`。几何违规数与语境违规数均为零。

NBA 认证与机制实验工件位于 `docs/dev/evidence/action_chain_validation/`，FIBA 认证与回放清单位于 `docs/dev/evidence/fiba_validation/`。两个联赛的 `run.json` 记录生成工件的代码提交、规则版本、校准版本与引擎模型版本。13 项机制实验的逐项观测、配对区间与裁决写入 `mechanism_core.json`，认证状态写入 `certification.json`。`data/calibration/known_insufficient.json` 的登记表为空。

`scripts/calibrate/freeze.py --check` 核对三张冻结表。基线指纹按最终行为与参数向量保存在 `crates/sim/tests/baseline/fingerprint.json`。`scripts/check_final_artifacts.py` 核对门禁、认证、机制实验、资源工件与规则目录的一致性。

## 4. 测试与资源

全量工作区测试日志位于 `.pi/tasks/final-workspace-tests.log`，退出码为零，全部测试二进制零失败。验收链逐项退出码位于 `.work/evidence/final-validation.summary`。`git diff --check` 干净。

`docs/dev/evidence/performance.json` 记录常规比赛、连续加时、频繁犯规、密集传球、多次暂停、录像复核与 FIBA 档案七类场景的峰值常驻内存、运行时间、事件数、动作数、加时数与每 tick 分配。测试与测量进程由 `.cargo/config.toml`、`scripts/mem-capped-runner.sh` 与 `scripts/run-tests.sh` 约束在 2 GiB 地址空间内。

## 5. 静态守卫与渲染验证

`scripts/check_docs.py` 核对引用与文档纪律。`scripts/check_animation_catalog.py`、`scripts/check_action_runtime_catalog.py`、`scripts/check_production_incidents.py`、`scripts/audit_rule_programs.py` 与 `scripts/check_r6_tendency_wiring.py` 核对动画目录、动作生产链、原始事实调用图、规则程序与倾向字段。`scripts/check_max_source_lines.py` 核对逐文件行数预算，`scripts/check_threshold_integrity.py` 核对判定基准与被测源码的提交边界。

`node scripts/test_court3d.mjs` 与 `scripts/test_court_visuals.py` 覆盖两类球场渲染器；`scripts/run_browser_checks.sh` 在验收链中统一执行。`cargo clippy --workspace --all-targets -- -D warnings` 零告警。

## 6. 覆盖范围

覆盖全场 5v5 比赛的传球、突破、急停中投、后撤三分、反击转换快攻、空切配合、低位背打、前场篮板补篮、掩护读防决策、犯规加罚、末节关键球时钟管理与垃圾时间换人。NBA 与 FIBA 使用同一内核与各自的规则档案。
