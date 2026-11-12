# 阶段交付与证据对应

> 本文件按 `full_simulation_plan.md` §12 的 A–H 阶段逐项列出交付物、代码路径与通过测试。测试身份写作 `测试文件::测试函数`，与 `data/rules/rule_catalog.json`、`data/rules/action_catalog.json` 中的登记一致。

## A. 规则与覆盖目录

- 交付物：规则条款登记、程序目录（22 项）、动作能力目录（16 个动作族）、NBA 2025-26 与 FIBA 2026 v1.1 档案身份、来源文档。
- 数据：`data/rules/rule_catalog.json`、`data/rules/action_catalog.json`、`data/rules/league_nba.json`、`data/rules/league_fiba.json`。
- 来源记录：`docs/dev/evidence/rule_catalog_sources.md`。
- 证据：`crates/domain/tests/rule_catalog.rs` 4 项（程序与动作目录结构、场景身份唯一性、联赛字段生产消费者、每条目录证据指向真实测试；动作族的 `missing_evidence` 为空）。
- 状态：目录结构完成；程序与违例的实现状态逐条记录在 `rule_catalog.json` 的 `implementation`、`passing_scenarios`、`missing_scenarios`、`scenario_tests`。

## B. 状态与时间基础

- 交付物：`Clock`（整数 tick 与仿真时间）、`Phase` 时钟推进表、`Fact` 事件身份与序号、`World::step` 的固定步长循环、控制与球权状态。
- 代码：`crates/sim/src/state.rs`、`crates/sim/src/clock/mod.rs`、`crates/sim/src/world.rs`。
- 证据：`crates/sim/tests/frame_stream.rs`（帧流因果 ID）、`crates/sim/tests/event_sequence.rs`（事件序号与游标缺口）、`crates/sim/tests/interface_determinism.rs`（重复运行与检查点续跑一致）、`crates/validation/tests/event_ledger.rs`（事件派生账本）、`crates/sim/tests/runtime_tables.rs`（传球飞行消耗两个时钟、罚球飞行停钟）。
- 状态：`World::adjudicate` 是停表后果的提交入口；`StoppageCause`、`PenaltyPlan`、`RestartPlan`、`ResumeContext`、`InboundLocation` 与 `ControlState` 一起描述裁决结果与恢复阶段。

## C. 程序与判罚

- 交付物：开球与跳球、发球、罚球、暂停、换人、违例、犯规罚则、节末与加时。
- 代码：`crates/sim/src/world.rs`（跳球、发球、时段收尾）、`crates/sim/src/resolve/{foul,freethrow,rebound,shot}.rs`、`crates/sim/src/program/timeouts.rs`、`crates/sim/src/endgame.rs`、`crates/sim/src/program/rotation.rs`。
- 证据：
  - 跳球与交替拥有：`crates/sim/tests/jump_ball.rs` 5 项。
  - 规则矩阵：`crates/sim/tests/engine.rs` 10 项、`crates/sim/tests/rules_flow.rs` 11 项、`crates/validation/tests/rule_matrix.rs` 14 项。
  - 违例与发球地点：`crates/sim/tests/rule_violations.rs` 16 项（后场八秒、回场、出界归属与边线发球点、罚球禁区记录/重罚/换边、前场犯规边线发球、命中后底线发球、末段暂停前场发球）。
  - 紧逼五秒：`crates/sim/tests/action_scenarios.rs::holding_the_ball_closely_guarded_for_five_seconds_is_a_violation`、`the_closely_guarded_timer_resets_and_spares_the_dribbler`。
  - 停表跳球：`crates/sim/tests/action_scenarios.rs::a_tie_for_the_ball_enters_a_held_ball_jump_ball`（参跳四人走到中圈后才掷球，无瞬移）。
  - 阵容与犯满：`crates/sim/tests/rotation.rs` 5 项。
  - 限制区计时：`crates/sim/tests/lane_clock.rs` 5 项（进攻三秒违例、及时离开不违例、防守三秒技术犯规、主动盯防不判罚、进攻方进入前场前不计时）。
  - 非法触球：`crates/sim/tests/illegal_touch.rs` 5 项（一臂之内伸手阻挡成立、合法间隔上的平行贴防不成立、够不到不成立、退开之后不再成立、活球里记为个人犯规）。
  - 背身五秒：`crates/sim/tests/back_to_basket.rs` 5 项（背身运球满五秒违例、面朝篮筐不违例、越过罚球线延长线不违例、收球终止计时、球被碰开终止计时）。
  - 双方犯规、恶意犯规与清晰路径：`crates/sim/tests/foul_scenarios.rs` 15 项；FIBA 破坏性犯规与 NBA 清晰路径犯规分别登记。
  - 暂停配额：`crates/sim/tests/timeout_scenarios.rs` 14 项（NBA 整场与第四节与末段额度、FIBA 整场与第四节额度、加时额度独立、活球只有持球方能叫、恢复比赛、按来源区分的暂停时长与活球请求占用完整档）。
  - 节次开始与交替拥有箭头：`crates/sim/tests/period_start_scenarios.rs` 7 项。
  - 罚球禁区与罚球人替换：`crates/sim/tests/free_throw_scenarios.rs` 6 项（含罚球离手后不判出手超时）。
  - 持球人推进：`crates/sim/tests/ball_handler_advance.rs` 2 项（绕行上限不改写推进目标、八秒内球过中线）。
  - 换人：`crates/sim/tests/substitution_scenarios.rs` 8 项。
  - 后场八秒与「踢球」重置：`crates/sim/tests/backcourt_clock_scenarios.rs` 4 项。
  - 比赛钟与进攻钟：`crates/sim/tests/clock_scenarios.rs` 13 项。
  - 出界与脚步：`crates/sim/tests/boundary_scenarios.rs` 11 项（含跳步：连续两次同一只脚触地）。
  - 跳球：`crates/sim/tests/jump_ball_scenarios.rs` 8 项、`crates/sim/tests/jump_ball.rs` 5 项。
  - 投篮与干扰球：`crates/sim/tests/shot_scenarios.rs` 10 项。
- 状态：22 项程序全部 `implemented`，没有缺失场景（`data/rules/rule_catalog.json`）。进攻三秒、防守三秒、发球五秒、非法触球四项各有生产消费路径与场景测试。

## D. 并行动作与执行证据

- 交付物：每名球员自己的 `ActionRuntime`、多人交互编号、防守动作运行时、动作生命周期事实。
- 代码：`crates/sim/src/actions/mod.rs`、`crates/sim/src/world.rs::advance_parallel_actions`、`crates/sim/src/resolve/`。
- 证据：
  - `crates/sim/tests/parallel_actions.rs` 4 项：同一 tick 内多人运行时并存且种类不同、出手期间防守动作并行、每个 `ActionStarted` 恰有一个终态、停表阶段终态不起手。
  - `crates/sim/tests/action_chain.rs` 5 项、`crates/sim/tests/assignment.rs` 2 项、`crates/sim/tests/continuity.rs` 2 项。
  - `crates/sim/tests/fake_protection.rs` 7 项、`crates/sim/tests/defensive_technique.rs` 8 项、`crates/sim/tests/dribble_technique.rs` 4 项、`crates/sim/tests/pass_technique.rs` 2 项。
- 状态：`World` 只保存逐人动作运行时，多人配合按 `interaction_id` 结算。

## E. 候选、战术与能力

- 交付物：候选生成、执行前重校验、技术选择、能力与倾向、防守响应。
- 代码：`crates/sim/src/policy.rs`、`crates/sim/src/shot_quality.rs`、`crates/sim/src/assignment.rs`、`crates/sim/src/defense.rs`、`crates/sim/src/value.rs`。
- 证据：
  - 动作场景：`crates/sim/tests/action_scenarios.rs` 34 项（含切入六种路线、挤过与绕过、回位、包夹与护球、欧洲步、后撤步）。
  - 决策与价值：`crates/sim/tests/policy_utility.rs`、`crates/sim/tests/value_invariants.rs`、`crates/sim/tests/decision_trace.rs`。
  - 情境：`crates/sim/tests/situational_emergence.rs` 5 项（含比分处于劣势方的末段主动犯规、砍罚球差的大个子）。
- 状态：16 个动作族均有生产路径与通过测试，状态全部是 `verified`；进攻三秒对应的「清空禁区」行为已经实现（`lane::clearing_required` 与 `lane::exit_target`），回归见 `crates/sim/tests/lane_clock.rs`。

## F. 事件消费者与交付验证

- 交付物：独立账本、协议投影、回放、CLI/WASM 一致性、场景矩阵、机制实验、性能与分配报告。
- 代码：`crates/validation/`、`crates/bball-wasm/`、`crates/cli/`、`crates/sim/src/stream.rs`、`crates/sim/src/session.rs`。
- 证据：`crates/validation/tests/event_ledger.rs` 7 项、`crates/cli/tests/interface_parity.rs` 3 项（NBA seed 211 与 FIBA seed 307 的 CLI/WASM 整场摘要一致）、`crates/sim/tests/checkpoint_disk.rs` 3 项、`crates/sim/tests/allocation_budget.rs` 1 项、`crates/validation/tests/five_dimension.rs` 3 项。
- 状态：账本与协议完成；`validate-core` 可产出认证、批次统计与分支覆盖工件。

## G. 校准与文档

- 交付物：参数校准、统计参考与来源更新、机制实验、独立验证、文档刷新。
- 代码与数据：`scripts/calibrate/fit_policy.py`、`scripts/calibrate/fit_tables.py`、`scripts/calibrate/freeze.py`、`data/calibration/*.json`。
- 证据：`crates/sim/tests/runtime_tables.rs`（运行时表加载与来源校验）、`python3 scripts/calibrate/freeze.py --check`（三张冻结表哈希）、`nba-sim validate-core`（四层判定、批次统计、分支覆盖与认证工件）、`nba-sim validate-core --mechanism`（属性机制实验写入 `mechanism_core.json`）。
- 状态：参数搜索与门禁证据由 `scripts/calibrate/` 下的脚本和 `scripts/refresh_gate_evidence.py` 生成；`baseline/fingerprint.json`、NBA/FIBA `validate-core` 工件、认证状态、峰值资源与全量测试汇总由 `scripts/final_validation.sh` 一次写出。判定数字以 `docs/dev/evidence/gap_closure_cycle/gate.txt`、`docs/dev/evidence/action_chain_validation/certification.json` 与 `docs/dev/evidence/performance.json` 为准。

## H. 动作动画与观测闭环

- 交付物：动作动画覆盖目录、完整动作状态帧、动作参与者与生命周期事件映射、2.5D 骨骼动画配方、二维战术板动作呈现及逐项自动验证。
- 代码与数据：`crates/protocol/src/action_recipe.rs`、`crates/sim/src/stream.rs`、`crates/debug-server/static/court3d.js`、`crates/debug-server/static/app.js`、`data/rules/action_animation_catalog.json`。
- 证据：`scripts/check_animation_catalog.py`、`scripts/check_action_runtime_catalog.py`、`node scripts/test_court3d.mjs`、`scripts/test_court_visuals.py`、`crates/sim/tests/action_animation_coverage.rs` 2 项、两类渲染器同帧断言与未知值负面对照。
- 状态：16 个动作族与 111 个动作条目各有生产候选、结算器、场景测试与渲染配方；动作身份、阶段、参与者与时间边界来自权威运行时。

## 最终验收覆盖

1. 动作目录覆盖收球步法与轴心脚、突破的三种变体、跳步收球与离散脚步、掩护发起、`ReboundJump` 消费者；16 个动作族的状态全部是 `verified`，`missing_evidence` 为空。
2. 权威状态与档案结构由 `docs/dev/current/full_simulation_plan.md` §3.2、§4 与 §5 给出：11 个策略族、整数时间、七种 `BallState`、`ControlState` 内部权威、`World::adjudicate` 单一写入口；结构回归由 `crates/sim/tests/architecture_decoupling.rs` 给出。
3. 干扰球的下行段、触板后与筐沿阶段各有场景测试（`crates/sim/tests/shot_scenarios.rs` 10 项），`PROC-GOALTENDING` 的场景清单完整。
4. 机制实验的 13 项属性判定为 `pass`：各项样本量按效应量给出（`REQUIRED_SEEDS`），`defensive_rebound` 使用最终参数向量对应的样本数，配对区间整段在零的一侧，`known_insufficient.json` 的登记表为空；逐项数值与区间写在 `docs/dev/evidence/action_chain_validation/mechanism_core.json`。
