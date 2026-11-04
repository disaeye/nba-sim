# 阶段交付与证据对应

> 本文件按 `full_simulation_plan.md` §12 的 A–G 阶段逐项列出交付物、代码路径、通过测试与未闭合项。测试身份写作 `测试文件::测试函数`，与 `data/rules/rule_catalog.json`、`data/rules/action_catalog.json` 中的登记一致。

## A. 规则与覆盖目录

- 交付物：规则条款登记、程序目录（22 项）、动作能力目录（16 个动作族）、NBA 2025-26 与 FIBA 2026 v1.1 档案身份、来源文档。
- 数据：`data/rules/rule_catalog.json`、`data/rules/action_catalog.json`、`data/rules/league_nba.json`、`data/rules/league_fiba.json`。
- 来源记录：`docs/dev/evidence/rule_catalog_sources.md`。
- 证据：`crates/domain/tests/rule_catalog.rs` 两项（程序与动作目录结构、场景身份唯一性、通过测试身份必须与证据集合一致；动作族的 `missing_evidence` 目前为空）。
- 状态：目录结构完成；程序与违例的实现状态逐条记录在 `rule_catalog.json` 的 `implementation`、`passing_scenarios`、`missing_scenarios`、`scenario_tests`。

## B. 状态与时间基础

- 交付物：`Clock`（整数 tick 与仿真时间）、`Phase` 时钟推进表、`Fact` 事件身份与序号、`World::step` 的固定步长循环、控制与球权状态。
- 代码：`crates/sim/src/state.rs`、`crates/sim/src/clock.rs`、`crates/sim/src/world.rs`。
- 证据：`crates/sim/tests/frame_stream.rs`（帧流因果 ID）、`crates/sim/tests/event_sequence.rs`（事件序号与游标缺口）、`crates/sim/tests/interface_determinism.rs`（重复运行与检查点续跑一致）、`crates/validation/tests/event_ledger.rs`（事件派生账本）、`crates/sim/tests/runtime_tables.rs`（传球飞行消耗两个时钟、罚球飞行停钟）。
- 状态：时钟与阶段推进完成；停表后果已经收拢到 `World::adjudicate` 一个提交入口（`StoppageCause`、`PenaltyPlan`、`RestartPlan`、`ResumeContext`、`InboundLocation`、`ControlState`）。

## C. 程序与判罚

- 交付物：开球与跳球、发球、罚球、暂停、换人、违例、犯规罚则、节末与加时。
- 代码：`crates/sim/src/world.rs`（跳球、发球、时段收尾）、`crates/sim/src/resolve/{foul,freethrow,rebound,shot}.rs`、`crates/sim/src/timeouts.rs`、`crates/sim/src/endgame.rs`、`crates/sim/src/rotation.rs`。
- 证据：
  - 跳球与交替拥有：`crates/sim/tests/jump_ball.rs` 5 项。
  - 罚球与计数：`crates/sim/tests/engine.rs`、`crates/sim/tests/rules_flow.rs` 11 项。
  - 规则矩阵：`crates/validation/tests/rule_matrix.rs` 10 项。
  - 违例与发球地点：`crates/sim/tests/rule_violations.rs` 9 项（后场八秒、回场、出界归属与边线发球点、罚球禁区记录/重罚/换边、前场犯规边线发球、命中后底线发球、末段暂停前场发球）。
  - 紧逼五秒：`crates/sim/tests/action_scenarios.rs::holding_the_ball_closely_guarded_for_five_seconds_is_a_violation`、`the_closely_guarded_timer_resets_and_spares_the_dribbler`。
  - 停表跳球：`crates/sim/tests/action_scenarios.rs::a_tie_for_the_ball_enters_a_held_ball_jump_ball`（参跳四人走到中圈后才掷球，无瞬移）。
  - 阵容与犯满：`crates/sim/tests/rotation.rs` 5 项。
  - 限制区计时：`crates/sim/tests/lane_clock.rs` 5 项（进攻三秒违例、及时离开不违例、防守三秒技术犯规、主动盯防不判罚、进攻方进入前场前不计时）。
  - 非法触球：`crates/sim/tests/illegal_touch.rs` 5 项（一臂之内伸手阻挡成立、合法间隔上的平行贴防不成立、够不到不成立、退开之后不再成立、活球里记为个人犯规）。
  - 背身五秒：`crates/sim/tests/back_to_basket.rs` 5 项（背身运球满五秒违例、面朝篮筐不违例、越过罚球线延长线不违例、收球终止计时、球被碰开终止计时）。
  - 双方犯规、恶意犯规与清晰路径：`crates/sim/tests/foul_scenarios.rs` 11 项。
  - 暂停配额：`crates/sim/tests/timeout_scenarios.rs` 9 项（NBA 整场与第四节与末段额度、FIBA 整场与第四节额度、加时额度独立、活球只有持球方能叫、恢复比赛）。
  - 节次开始与交替拥有箭头：`crates/sim/tests/period_start_scenarios.rs` 7 项。
  - 罚球禁区与罚球人替换：`crates/sim/tests/free_throw_scenarios.rs` 5 项。
  - 换人：`crates/sim/tests/substitution_scenarios.rs` 7 项。
  - 后场八秒与「踢球」重置：`crates/sim/tests/backcourt_clock_scenarios.rs` 4 项。
  - 比赛钟与进攻钟：`crates/sim/tests/clock_scenarios.rs` 10 项。
  - 出界与脚步：`crates/sim/tests/boundary_scenarios.rs` 11 项（含跳步：连续两次同一只脚触地）。
  - 跳球：`crates/sim/tests/jump_ball_scenarios.rs` 8 项、`crates/sim/tests/jump_ball.rs` 5 项。
  - 投篮与干扰球：`crates/sim/tests/shot_scenarios.rs` 8 项。
- 状态：22 项程序全部 `implemented`，没有缺失场景（`data/rules/rule_catalog.json`）。进攻三秒、防守三秒、发球五秒、非法触球四项各有生产消费路径与场景测试。

## D. 并行动作与执行证据

- 交付物：每名球员自己的 `ActionRuntime`、多人交互编号、防守动作运行时、动作生命周期事实。
- 代码：`crates/sim/src/action.rs`、`crates/sim/src/world.rs::advance_parallel_actions`、`crates/sim/src/resolve/`。
- 证据：
  - `crates/sim/tests/parallel_actions.rs` 4 项：同一 tick 内多人运行时并存且种类不同、出手期间防守动作并行、每个 `ActionStarted` 恰有一个终态、死球终态不起手。
  - `crates/sim/tests/action_chain.rs` 5 项、`crates/sim/tests/assignment.rs` 2 项、`crates/sim/tests/continuity.rs` 2 项。
  - `crates/sim/tests/fake_protection.rs` 7 项、`crates/sim/tests/defensive_technique.rs` 8 项、`crates/sim/tests/dribble_technique.rs` 4 项、`crates/sim/tests/pass_technique.rs` 2 项。
- 状态：全局单动作已移除；`World` 没有 `active` 字段，只有逐人推进。

## E. 候选、战术与能力

- 交付物：候选生成、执行前重校验、技术选择、能力与倾向、防守响应。
- 代码：`crates/sim/src/policy.rs`、`crates/sim/src/shot_quality.rs`、`crates/sim/src/assignment.rs`、`crates/sim/src/defense.rs`、`crates/sim/src/value.rs`。
- 证据：
  - 动作场景：`crates/sim/tests/action_scenarios.rs` 29 项（含切入六种路线、挤过与绕过、回位、包夹与护球、欧洲步、后撤步）。
  - 决策与价值：`crates/sim/tests/policy_utility.rs`、`crates/sim/tests/value_invariants.rs`、`crates/sim/tests/decision_trace.rs`。
  - 情境：`crates/sim/tests/situational_emergence.rs` 5 项（含落后方末段主动犯规、砍罚球差的大个子）。
- 状态：16 个动作族均有生产路径与通过测试，状态全部是 `verified`；进攻三秒对应的「清空禁区」行为已经实现（`lane::clearing_required` 与 `lane::exit_target`），回归见 `crates/sim/tests/lane_clock.rs`。

## F. 事件消费者与交付验证

- 交付物：独立账本、协议投影、回放、CLI/WASM 一致性、场景矩阵、机制实验、性能与分配报告。
- 代码：`crates/validation/`、`crates/wasm/`（`bball-wasm`）、`crates/cli/`、`crates/sim/src/stream.rs`、`crates/sim/src/session.rs`。
- 证据：`crates/validation/tests/event_ledger.rs` 5 项、`crates/sim/tests/interface_parity`（CLI 与 WASM 整场摘要与全部投影事件一致，seed 211）、`crates/sim/tests/checkpoint_disk.rs` 3 项、`crates/sim/tests/allocation_budget.rs` 1 项、`crates/validation/tests/five_dimension.rs` 3 项。
- 状态：账本与协议完成；`validate-core` 可产出认证、批次统计与分支覆盖工件。

## G. 校准与文档

- 交付物：参数校准、统计参考与来源更新、机制实验、独立验证、文档刷新。
- 代码与数据：`scripts/calibrate/fit_policy.py`、`scripts/calibrate/fit_tables.py`、`scripts/calibrate/freeze.py`、`data/calibration/*.json`。
- 证据：`crates/sim/tests/runtime_tables.rs`（运行时表加载与来源校验）、`python3 scripts/calibrate/freeze.py --check`（三张冻结表哈希）、`nba-sim validate-core`（四层判定、批次统计、分支覆盖与认证工件）、`nba-sim validate-core --mechanism`（属性机制实验写入 `mechanism_core.json`）。
- 状态：校准已执行。参数已从写死系数搬入表中（封盖手部半径、投篮犯规率、篮板伸手折减与加成、包夹权重、回场判定带宽、抛投区价值、突破落点偏移）；机制实验 `--mechanism` 逐项输出结论，当前通过项与未通过项记在 `docs/dev/current/implementation.md` 的机制实验一节；32 场 `validate-core` 上 38 条分布判据全部在带内（得分 232.5、回合 200.2、几何违规 0、语境违规 0、账本失败 0），认证为 `pass`；残差与已试杠杆记在 `docs/dev/current/implementation.md` 的校准一节。

## 未闭合项汇总

1. 动作目录的技术细分已补齐（收球步法与轴心脚、突破的三种变体、跳步收球与离散脚步、掩护由掩护人自主发起、`ReboundJump` 的仿真层消费者），16 个动作族的状态全部是 `verified`，`missing_evidence` 为空。
2. 权威状态与档案结构已解耦：`Phase` 不含飞行变体，`ControlState` 是结构体，`LeagueProfile` 按四个策略族存储且没有未声明的默认值（`crates/sim/tests/architecture_decoupling.rs` 5 项）。
3. 干扰球的三种情形（下落段、碰过篮板之后、球在筐沿上）都已接入生产路径并各有场景测试（`crates/sim/tests/shot_scenarios.rs` 10 项），`PROC-GOALTENDING` 的场景清单没有缺失项。
4. 机制实验的 13 项属性全部判定为 `pass`：每一项按它自己的效应量定样本量（`REQUIRED_SEEDS`），`defensive_rebound` 跑 640 个种子，相对变化 +0.0198、配对区间 [+0.0090, +0.0307] 整段在零的一侧，`known_insufficient.json` 的登记表因此清空。
