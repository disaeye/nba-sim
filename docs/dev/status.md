# NBA-Sim · 当前实现状态

> 状态类型：当前快照。
> 证据原则：本页只写当前工作区可以由代码、测试、守卫或最终验收工件复核的结论。周期记录保存在 `docs/dev/cycles/` 与 `docs/dev/evidence/`。

## 1. 当前工作区状态

实施范围与验收条件以 `docs/dev/current/full_simulation_plan.md` 为准；阶段与证据的逐项对应见 `docs/dev/evidence/stage_gate_map.md`。`crates/sim` 是唯一比赛模拟实现。同一完成比赛的 CLI、WASM、debug-server 与会话检查点进入同一内核；`MatchSetup`、磁盘检查点与回放清单可完整恢复。

动作状态按球员逐人保存：`World::advance_parallel_actions` 推进每名球员自己的 `ActionRuntime`，多人配合由交互编号结算，防守动作另有 `CloseoutContest`、`StealAttempt`、`BlockJump` 三种运行时。并发性由 `crates/sim/tests/parallel_actions.rs` 直接验证。动作目录登记 16 个族与 111 个动作条目；运行时动作类型、协议投影与两类渲染器按 `ActionSpecTable` 逐项接入，`scripts/check_action_runtime_catalog.py` 核对每条生产候选、结算器与场景测试。

规则程序目录的 22 项程序按版本、条款、场景矩阵与生产消费者审计，`scripts/audit_rule_programs.py` 逐条核对，`known_current_gaps` 为空。原始事实的生产调用链由 `scripts/check_production_incidents.py` 核对；特殊犯规、技术犯规、伤病、球队技术犯规与转换阻断由完整比赛自发检出。抽样式接触事实只在活球阶段成立，停表程序里不会按上一回合的速度与球权产生判罚。

裁决按 `World::step` 收集 `RawIncident`、`AdjudicationBatch` 生成完整计划、`World::adjudicate` 原子提交。`ClockState` 使用整数微秒，`ClockController` 是时钟写入口；内核按职责分到 `crates/sim/src/clock/`、`program/`、`officiating/`、`actions/`、`control/`、`ledger/`。`LeagueProfile` 按 11 个策略族保存，加载时缺字段直接失败。`BallState` 与 `BallFlight` 分存，`clock::advance` 按飞行种类决定计时器推进。

`StreamTick` 给出比赛阶段、程序、球态、计时器运行标志、规则版本、校准版本、引擎模型版本、动作状态与程序资格。事件流可独立重建比分、控制、时间、犯规、资格、暂停、罚则、动作生命周期与节末结算；`FrameEvent` 的因果字段由事件身份与来源事件给出。

## 2. 验证与资源

`nba-sim sim-core --gate` 的固定 16 场判定覆盖 39 条分布与过程判据，逐条观测值写入 `docs/dev/evidence/gap_closure_cycle/gate.txt`，汇总写入同目录的 `verification.json`。NBA 与 FIBA 的 `validate-core` 工件分别保存在 `docs/dev/evidence/action_chain_validation/` 与 `docs/dev/evidence/fiba_validation/`；`run.json` 记录提交身份、规则版本、校准版本与引擎模型版本。13 项机制实验的逐项观测、配对区间与裁决写入 `mechanism_core.json`，认证状态写入 `certification.json`，样本量按各属性效应量给出。

基线指纹按最终参数向量保存在 `crates/sim/tests/baseline/fingerprint.json`。冻结三张表由 `scripts/calibrate/freeze.py --check` 核对。全量工作区测试日志写入 `.pi/tasks/final-workspace-tests.log`，验收链逐项退出码写入 `.work/evidence/final-validation.summary`。

`docs/dev/evidence/performance.json` 记录常规比赛、连续加时、频繁犯规、密集传球、多次暂停、录像复核与 FIBA 档案七类场景的峰值常驻内存、运行时间、事件数、动作数、加时数与每 tick 分配。测试与测量进程由 `.cargo/config.toml`、`scripts/mem-capped-runner.sh` 与 `scripts/run-tests.sh` 约束在 2 GiB 地址空间内。

## 3. 证据入口

- 权威方案：`docs/dev/current/full_simulation_plan.md`
- 阶段与证据对应：`docs/dev/evidence/stage_gate_map.md`
- 设计说明：`docs/dev/action_chain_engine.md`
- 门禁逐条结果：`docs/dev/evidence/gap_closure_cycle/gate.txt`
- 门禁汇总与验收现场：`docs/dev/evidence/gap_closure_cycle/verification.json`
- NBA 认证与机制实验：`docs/dev/evidence/action_chain_validation/`
- FIBA 认证与回放清单：`docs/dev/evidence/fiba_validation/`
- 资源与性能工件：`docs/dev/evidence/performance.json`
- 验收链逐项退出码：`.work/evidence/final-validation.summary`
- 全量工作区测试日志：`.pi/tasks/final-workspace-tests.log`
- 基线指纹：`crates/sim/tests/baseline/fingerprint.json`
