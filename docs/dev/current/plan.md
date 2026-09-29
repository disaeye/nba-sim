# 当前执行队列

> 文档类型：当前未完成工作的执行顺序。
> 已结束周期的设计和实测保留在 `docs/dev/cycles/` 与 `docs/dev/evidence/`。
> 当前可复核状态见 `docs/dev/status.md`。任务验收范围见 [`implementation.md`](implementation.md)。关闭条件见 `docs/dev/gap.md`。

## 1. 队列

```text
R1 结果权交回过程
  → R2 篮下出手分布
  → R3 无球与对抗动作生命周期
  → R4 挡拆防守责任图
  → R5 规则程序与犯规账本
  → R6 倾向规格与个体评判
  → R7 有来源的参考分布和真实度目标线
R1 → R8 球态与阶段权限
R4 → R9 战术三层
R6 → R10 球员输入
R3、R4、R5 → R11 评判闭环
```

后一项可以使用前一项已经发布的事实，不提前用统计带声明关闭。每项的类型、事实和测试入口见 `docs/dev/current/implementation.md`。

## 2. R1 · 结果权交回过程

R1 当前未关闭。投篮到达仍为端点分类，篮板路径尚未由连续碰撞事实完整驱动；传球接收的状态连续测试与有限信息测试已有局部通过记录。八种子 attribution 10 项测试全部通过：每一对相邻生产帧的球速包络测试 `all_adjacent_frames_respect_ball_speed_envelope` 全绿，出手结果事实与释放/到达/封盖事件严格对平（`shot_result_facts_reconcile_with_release_and_arrival_events`）。球位连续性已按同一物理口径修复三类单帧跳变：出手起点取球实际位置（突破终结的统计分区仍按向篮筐延伸的 finish_pos），传球携带实际出手球高 `from_z`，控制转移时长按三维路径长度计算。哨响连续性已实施（R1.2 验收条款）：罚球不截断在飞出手与被犯规者本人的挂起出手，到筐结算消费罚球队列；哨响作废在飞传球时补发 `PASS_DROPPED` 终结。罚球准备使用 `FreeThrowSetup` 三维轨迹，统一从 `begin_free_throw_flight` 生成罚球轨迹；`free_throw`（11 项）、`ball_ownership`（3 项）、physics backend contract（15 项）、rim-contact（8 项）、physics 弹道单测（5 项）、传球五分支互斥（`pass_speed_continuity.rs` 5 项）与突破终局因果（`drive_geometry.rs` 6 项）全部通过。黄金哈希 seed42 × 2000 冻结为 `0xca1d1cae20e46d0e`，随球位与哨响连续性行为修正及决策重校准同步重冻结并由 `golden_hash` 核验。详见 `implementation.md` 的 R1 证据。

规格要求见 `docs/basketball.md` §3。2026-09-25 起完成了球态预写结果字段移除、出手释放延迟、传球到达判定、突破窗口终局和黄金哈希更新。当前仍需把结果决定绑定到真实触筐与接触事实，补齐封盖对照、传球五结果互斥和事件因果父链；速度连续性仍需核验 PASS→HELD、STEAL 与 SHOT_RELEASE 等状态边界。具体复核见 `docs/dev/current/spec_gap.md` §7。以下为完整关闭条件：

1. 出手开始只保留出手人、出手位置、动作窗口和当时可见的防守事实；
2. 球离手后由弹道、封盖窗口和触筐事实产生命中、不中、封盖和犯规；
3. 传球在到达时产生接住、掉球、点掉或抢断；
4. 突破在接触和终结窗口结束时产生成功、失败、犯规和得分；
5. 固定种子下，决定结果的事实和结果事件保持可重放。

关闭证据：

- 每一对相邻生产帧均满足配置的球速上限，包括死球、罚球点布置和状态转换帧；
- 同一出手参数在封盖成功和封盖失败下得到不同球态；
- 结果事件的 `parent_event_id` 指向决定它的飞行、接触或封盖事件；
- 命中由实际触筐事实决定，投篮犯规由已判定的非法接触事实决定；
- 接球、掉球、点拨、抢断和出界结果互斥，并由对应的到达或接触事实决定；
- 突破终局由接触、位置和终结窗口事实决定，不在动作创建时预写概率结果；
- 现有得分账本继续对平；
- 黄金哈希按行为变更重新冻结，并写明变化来自结果决定点。当前冻结值为 `0xca1d1cae20e46d0e`，由 `golden_hash` 逐位核验；变化源为球位连续性与哨响连续性行为修正及决策重校准。

代码入口：

- `crates/engine/src/match_engine/execution.rs`
- `crates/engine/src/match_engine/state.rs`
- `crates/engine/src/match_engine/ball_flight/`
- `crates/engine/src/match_engine/block.rs`
- `crates/domain/src/flow.rs`
- `crates/engine/tests/shot_release_timing.rs`

## 3. R2 · 篮下出手分布

R2 已闭合。全量 16 种子 `stats_baseline`（`full_game_stats_within_baseline_band`）全部通过：总分中位数 204.5 落在 [140, 230] 门槛，区间为 [167, 232]；平均回合数 246.9 落在 [172, 290] 门槛；平均单回合持续时间 14.40s 落在 [12.0, 20.0] 门槛；三分命中率中位数 38.5% 落在 [30.0, 40.0] 目标带；篮下出手占比中位数 0.333，全部 16 种子均进入 [0.25, 0.50] 参考带（16/16）；16 种子全部通过 evaluator `SHOT_PROFILE_ZONE_MIX` 构成判定；突破、切入接球、前场篮板与攻防转换四条路径在矩阵中均有真实出手；每场比赛的出手分区计数与箱体 FGA 严格一致；五式账本零违规；Hard 门零缺陷。单种子来源审计 `rim_attempt_composition` 同步通过。决策基准：`drive_rim_attack_bias` 1.3 守住篮下占比逐种子下限 0.25，`midrange_utility_bonus` 0.18 守住中距离占比逐种子下限 0.08，`post_up_base` 3.6 使背身仅在错位时胜出并保住背身/卡位系系数的行为触达（`rules_complete_wiring` 14/14）。

关闭证据：16-seed 全场的篮下出手占比全部进入 `nba.v2.json` 的 `rim_share_of_fga` 带（16/16），同时总分、三分命中率、回合数与时长、账本、Hard 门及四路径覆盖全部通过。

## 4. R3 · 动作生命周期

R3 已闭合。接入动作开始、阶段变化、完成、取消与失败类型化事实：
- 发布事实：`ActionStarted`、`ActionPhaseChanged`、`ActionCompleted`、`ActionCancelled`、`ActionFailed`；
- 覆盖动作：掩护建立（`ScreenSet`）、顺下（`ScreenRoll`）、外弹（`ScreenPop`）、背切（`CutBackdoor`）、空切（`Cut`）、卡位（`BoxOut`）、补篮（`Putback`）及投传扑盖共 13 种动作；
- 互斥保证：同一球员在同一 tick 至多保留一个活动动作窗口，新动作发起时旧动作以 `Superseded` 原因自动取消；
- 锁定闭环：所有动作终止分支（完成、取消、失败）均同步解除物理层运动学锁定；
- 失败事实：封盖发布触发 `ActionFailed { reason: Blocked }` 并释放锁定；
- 回归验证：`crates/engine/tests/lifecycle.rs` 17 项全过。

## 5. R4 · 防守责任图

R4 已闭合。实施防守责任图与责任交接事实：
- 责任状态：定义 `DefenseResponsibility` 包含 `PrimaryMatchup`、`Hedge`、`FightThrough`、`GoUnder`、`SwitchedMatchup`、`Help`、`Drop`、`Rotate`、`Recover` 九种主责任，保证每个防守人在任一 tick 有且仅有一个主责任；
- 责任交接事实：发布 `GameEvent::DefenseResponsibilityChanged`，包含执行防守人、原责任、新责任、对位进攻人及触发动作；
- 挡拆覆盖策略：`ScreenDefenseRules` 引入 `OnBallScreenDefenseStrategy`（`StandardContest`、`FightThrough`、`GoUnder`、`Switch`），联动 `drop_depth_ft` 与 `hedge_distance_ft` 在战术规划层生成对应责任；
- 档案差异与基线：`def_switch_heavy` 产生换防责任（`SwitchedMatchup`），`def_drop_coverage` 产生沉退责任（`Drop`），`def_hedge_recover` 产生延误责任（`Hedge`），`def_man_conservative` 恢复基线；
- 测试验证：`defense.rs`、`defense_responsibility_chain.rs`、`defense_effect.rs`、`defense_rotation_response.rs` 全部通过。

## 6. R5 · 规则程序

依 `docs/basketball.md` §5 和 §6 补齐走步、三秒、干扰、个人犯规、球队罚则及逐次罚球。`check_foul_conservation` 改为重建个人累计、球队累计、离场、罚则和罚球结果。NBA 与 FIBA 的差异只来自 `LeagueProfile`。

当前状态：犯规账本重建（个人/球队累计、犯满、bonus、逐次罚球对账，限值取自帧内 `FrameRules`，板凳耗尽由流内 `BENCH_DEPLETED` 事实豁免）、逐次罚球队列（不再覆写进行中的程序）、犯满离场重试、攻方三秒与回场程序（含发球例外）、活球松球争抢优先派发已实现；FIBA bonus 阈值核验为单节第 5 次，加时进节清零球队犯规。走步、二次运球、干扰球、贴身五秒、背筐五秒仍缺程序：前两者需要运球/合球阶段事实，干扰球需要飞行中球-人接触物理事实，物理层不产生这些事实，需先补物理事实通道。

## 7. R6 · 个体身份

`PlayerTendencies` 含 12 个字段，涵盖出手、突破、传球、切入、掩护、前场篮板、切球、封盖起跳、协防、身体对抗、风险容忍和转换冲刺。每项均需从实际球员输入到行为结果通过真实比赛扰动验证，并提供切断通道的负面对照。个体评判包括球队内球员使用率、进球因果父链、同回合防守责任恢复和第四节末段体能；事件来源或球员归属不完整时输出 `InsufficientEvidence`。当前定向测试与 16 种子全场矩阵通过；中距离效用补偿经 `midrange_utility_bonus` 规则字段接入中投候选。

## 8. R7 · 评判基准

`nba.v3` 当前使用 2023-24 常规赛 NBA Stats ShotChartDetail 归档，包含 1,230 场与 218,701 次出手，文件 SHA-256 为 `303e71f967c199568b345cd73a75e595f1f99532cca3e6ce59afb286b63f840d`。归档提交固定为 `ismayc/shot-quality-study` `f4001f944d4aba1ba63e3d70edb3d6135ba576fe`；NBA.com 公布该季 3PA/FGA 为 39.5%，坐标几何复算为 39.4845%。`scripts/check_r7_reference.py` 现核对归档哈希、样本、唯一事件、坐标距离、生产几何四区分位数与 Q4 窗口分位数；几何三分与 ShotChartDetail `SHOT_TYPE` 有 22 次差异，脚本分别记录几何三分率并在分区样本中保留两类标记为三分的出手。本次修改后五项 R7 定向测试、评估器 21 项单元测试与 26 项集成测试、Clippy、格式、文档/常数守卫、差异检查和来源复算通过；缺失出手因果链或 Q4 时钟时保持 `InsufficientEvidence`。NBA 默认仍为 v2，R7 尚未闭合：仍需独立审阅生产分区参考带、审核归档导入转换管线并完成 ADR-009 审评；通过前不登记真实性目标线或切换默认版本。

## 9. R8 · 球态与阶段权限

R1 完成后实施。`BallState` 只保留归属，飞行参数进入不可变的 `InFlight`；物理层删除 `has_ball`。`step_inner` 按 `docs/architecture.md` §2 的顺序调度，阶段签名只列出写入的状态组。Hard 违反写入结构化工件并向调用方返回失败。

关闭证据：比赛推进中的球态变化只经过 `transition_ball_state`；出手、突破、篮板和传球拦截的结果发生在 `physics.step` 之后；短路路径仍执行不变量校验。

当前状态：物理层 `PlayerPhysicsState.has_ball` 与 `set_ball_holder` 旁路已删除，持球人作为 `step_with_ball_holder` 的显式输入传入，约束与语义层经 `ball_holder_id` 派生视图读取；球态写入收敛到 `transition_ball_state` 唯一通道，`scripts/check_ball_state_writes.py` 守卫（含负对照）断言赋值与初始化只出现在授权文件；`step_inner` 按 `architecture.md` §4.1 重排（时钟先行，跳球/死球流程为条件激活分支位于罚球与节末之后，节间计时归时钟阶段）；阶段函数在签名处以「写入状态组」注释显式声明所写组名（ADR-014 accepted 形态）；终局哨兵帧改经 `step()` 包装器校验，新增回归断言每 step 恰好一次不变量检查（含跳球与节间短路 tick）。阶段重排与球态通道化本身保持 seed42 × 2000 黄金哈希逐位不变；后续 R1 球位与哨响连续性修复及决策重校准重冻结为 `0xca1d1cae20e46d0e`。剩余：`BallControl` × `BallMotion` 双枚举字面重构未做（gap.md §5.1 允许外部 `BallState` 接口保持稳定，不变量形态已由守卫与派生视图满足，见 ADR-007 修订注记）。

## 10. R9 · 战术三层

R4 完成后实施。进攻只读 `TacticalSetSpec`，槽位填充失败返回错误而不回退名单顺序。防守只保留一份 `DefensiveSystem`，初始对位来自 `assign_matchups`。Play 步长进入档案。轮换、暂停、情境和教练档案各有消费点。

关闭证据：新增一个合法 JSON 体系不改枚举就能通过比赛配置校验；同一进攻输入下，两套防守档案的差异能沿责任事件回溯。

## 11. R10 · 球员输入

R6 完成后实施。补齐 `wingspan_cm`、`effective_reach`、`release_height`，并给 `vertical`、`stamina`、`passing`、四个出手分区、`defense_perimeter`、`block`、`offensive_rebound` 补独立映射和断路测试。`domain::scouting` 只提供展示投影，引擎不读取展示值。

关闭证据：每个新接入维度有单调响应；映射被切断后响应消失；展示值不进入效用、概率和黄金哈希。

## 12. R11 · 评判闭环

R3、R4、R5 完成后实施。`evaluate_stream` 增加动作完整、防守责任完整、罚则完整、跳球和交替拥有四类准则。证据不足保持独立裁决。

关闭证据：缺失终态、缺失责任交接或罚则对不上时，评判器产出 Defect；这些准则进入归因账本。
