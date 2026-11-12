# 当前实施进度

> 唯一权威实施范围、依赖和验收条件见 [`full_simulation_plan.md`](full_simulation_plan.md)。本文件记录当前工作区的实施状态与验证证据，判定数字以 `docs/dev/evidence/` 下的工件为准。阶段与证据的逐项对应见 [`../evidence/stage_gate_map.md`](../evidence/stage_gate_map.md)。

## 当前目标

按 `docs/dev/current/full_simulation_plan.md` 的 A-H 工作包完成 NBA 与 FIBA 完整比赛模拟内核。状态、进度、阶段证据、门禁证据、验证工件与 CI 守卫均描述最终状态。

## 阶段进度

| 阶段 | 状态 | 当前记录 |
| --- | --- | --- |
| A. 规则与覆盖目录 | 已完成 | 22 项规则程序按版本、条款、场景矩阵、测试身份与生产消费者审计；16 个动作族与 111 个动作条目按 `ActionSpecTable` 逐项接入。`data/rules/rule_catalog.json` 的 `known_current_gaps` 为空，动作目录每条证据指向真实测试。 |
| B. 权威状态与联赛入口 | 已完成 | 时钟以整数微秒为唯一单位，`ClockDirective` 有 `Start`、`Stop`、`Set`、`Restore` 四种形态，`ClockController` 是唯一改钟入口。七种球态由迁移表约束，`ControlState` 是控制权唯一权威，事件信封带程序身份、控制序列身份与来源事件。11 个策略族、完整 `MatchSetup`、磁盘检查点与回放清单从 CLI、WASM、debug-server 和会话检查点恢复同一场比赛。 |
| C. 规则程序与特殊事件 | 已完成 | 22 项规则程序都有生产消费路径与场景矩阵。原始事实由 `World::step` 主循环检出，裁决层按联赛档案生成 `AdjudicationBatch` 与 `PenaltyQueue`，由统一提交入口原子写入。特殊犯规、技术犯规、伤病、球队技术犯规与转换阻断由完整比赛自发产生；抽样式接触事实只在活球阶段成立。 |
| D. 复核与节末结算 | 已完成 | 11 个 `ReviewScope` 与联赛档案双向映射，复核按来源事件身份抽样观察误差。教练挑战支持实质改判，节末结算按出手瞬间比赛钟裁定进球；`PeriodSettlementCompleted` 发布后比分、球权、动作与计时器冻结。 |
| E. 并行动作系统 | 已完成 | `World::advance_parallel_actions` 逐人推进每名球员自己的 `ActionRuntime`，多人配合由 `World::interactions` 按交互编号结算；防守动作有 `CloseoutContest`、`StealAttempt`、`BlockJump` 三种运行时。能力、倾向、战术与实时几何共同驱动候选、执行与后果。 |
| F. 事件、回放与协议投影 | 已完成 | 事件派生球员与球队账本，独立验证器只读事件流重建时钟与控制序列。`StreamTick` 带比赛阶段、程序、球态、计时器运行标志、三个版本号、活动动作与程序资格。`FrameRules`、`FrameEvent` 与 `RenderAction` 使用同一动作身份。 |
| G. 校准与验收 | 已完成 | 39 条门禁判据全部位于带宽内，逐条观测值由 `scripts/refresh_gate_evidence.py` 刷新。冻结三表哈希、基线指纹、NBA/FIBA 认证与 13 项机制实验由 `scripts/final_validation.sh` 统一生成并核对。样本量按各属性效应量给出。 |
| H. 动作动画与资源观测 | 已完成 | 两类球场渲染器读取同一 `ActionSpecTable` 与动作配方，渲染映射、未知值负面对照和回放定位都有自动验证。七类资源场景的峰值常驻内存、运行时间、事件数、动作数、加时数与每 tick 分配写入 `docs/dev/evidence/performance.json`。 |

## 验证证据

各阶段对应的测试身份、输入范围与代码路径集中在 `docs/dev/evidence/stage_gate_map.md`；本页不重复维护测试清单。行为与参数冻结后由 `scripts/final_validation.sh` 运行一次完整工作区测试，逐项退出码写入 `.work/evidence/final-validation.summary`，全量测试日志写入 `.pi/tasks/final-workspace-tests.log`。每个测试进程受 `.cargo/config.toml`、`scripts/mem-capped-runner.sh` 与 `scripts/run-tests.sh` 的 2 GiB 地址空间上限约束。

`scripts/final_validation.sh` 统一运行门禁证据刷新、文档与目录守卫、源码守卫、冻结表校验、资源测量、两类渲染器脚本、基线重录、NBA/FIBA 认证、全量工作区测试与最终工件一致性守卫。`python3 scripts/check_final_artifacts.py` 核对门禁、认证、机制实验、资源工件与规则目录登记的一致性。

## 校准记录

`scripts/calibrate/fit_policy.py` 按职责分组调 `policy.json` 的权重，目标是 `shot_profile.json` 的区域与创造方式份额，以及 `possession.json` 的回合数、每回合出手、失误率、罚球与得分。影响行为的系数都从参数表读取；每次拟合后用 `validate-core` 记录残差。当前 39 条判据的观测值与带宽判定以 `docs/dev/evidence/gap_closure_cycle/gate.txt` 为准。

`nba-sim validate-core --mechanism` 逐项把单个属性从 0.10 抬到 0.95，与同一批种子的基线做配对 bootstrap。样本量写在 `crates/validation/src/mechanism_core.rs` 的 `REQUIRED_SEEDS` 里，默认值是 160 个种子。13 项属性的逐项观测、配对区间与裁决写在 `docs/dev/evidence/action_chain_validation/mechanism_core.json`，认证状态写在同目录的 `certification.json`。`known_insufficient.json` 的登记表为空。

## 当前残差

五维诊断里 `events_per_game` 的上界是量程守卫，用来挡住事件日志失控。限制区计时、发球五秒、非法触球、突破变体、背身步法、对位站位姿态、干扰球与 111 个动作条目的生命周期事实都从生产路径发布，当前上界与理由记在 `data/diagnostics/five_dimension_bands.json` 的 `band_rationale` 里。

## 验收证据入口

A-H 工作包按 `docs/dev/current/full_simulation_plan.md` 的最终验收条件完成；当前状态由 `docs/dev/status.md`、阶段证据 `docs/dev/evidence/stage_gate_map.md` 与最终证据工件给出。
