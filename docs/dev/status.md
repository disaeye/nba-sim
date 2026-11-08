# NBA-Sim · 当前实现状态

> 状态类型：当前快照。
> 动作链内核（`crates/sim`）的设计与缺口清单见 `docs/dev/action_chain_engine.md`。
> 证据原则：本文件只写当前工作区可以由代码、测试或守卫复核的结论。旧引擎的记录保留在 `docs/dev/cycles/`、`docs/dev/evidence/` 与 `docs/decisions.md`，本文件不再复述。
> 复核日期：2026-11-08。
> 当前实施按 `docs/dev/current/full_simulation_plan.md` 的 A-H 工作包推进；最终状态与证据在行为冻结后统一记录。

## 1. 当前工作区状态

当前实施阶段按 `docs/dev/current/full_simulation_plan.md` 执行；阶段与证据的逐项对应见 `docs/dev/evidence/stage_gate_map.md`。`crates/sim` 是唯一比赛模拟实现，同一完成比赛的 CLI 与 WASM 入口已通过 seed 211 的摘要和投影事件对比。独立事件账本、会话检查点恢复与磁盘检查点、事件游标、规则场景矩阵与动作场景矩阵已有对应测试。

全局单动作 `World::active` 已移除：`World::advance_parallel_actions` 逐人推进每名球员自己的 `ActionRuntime`，多人配合由交互编号结算，防守动作另有 `CloseoutContest`、`StealAttempt`、`BlockJump` 三种运行时。并发性由 `crates/sim/tests/parallel_actions.rs` 4 项直接验证（多人运行时同一 tick 共存且种类不同、出手期间防守并行、每个 `ActionStarted` 恰有一个终态、死球终态不起手）。

动作目录登记 16 个族与 111 个动作条目；当前运行时动作类型、协议投影与两类渲染器按 `ActionSpecTable` 逐项接入，最终覆盖证据在行为冻结后生成。规则程序目录的 22 项程序按版本、条款、场景矩阵与生产消费者审计；`known_current_gaps` 在审计通过后清空。当前已接入的程序包括进攻三秒、防守三秒、背身五秒、发球五秒、非法触球、强制暂停、节末出手计时复核、后场八秒的「踢球」重置、暂停配额、罚球禁区与罚球人替换、换人、节次开始与交替拥有箭头。

裁决按 `docs/dev/current/full_simulation_plan.md` §5.3 收拢到 `World::commit`：`World::step` 收集 `RawIncident`，`AdjudicationBatch` 生成完整计划，提交入口原子写入球态、控制、程序、时钟、罚则、动作运行时与账本。`ClockState` 使用整数微秒，`ClockController` 是时钟写入口；内核按职责分到 `clock/`、`program/`、`officiating/`、`actions/`、`control/`、`ledger/` 六个目录。`LeagueProfile` 按 11 个策略族保存，加载时缺字段直接失败。`BallState` 与 `BallFlight` 分存，`clock::advance` 按飞行种类决定计时器推进。

最近一次完整工作区测试在 `.pi/tasks/review-tests.log` 记录 101 个测试二进制、零失败。`docs/dev/evidence/action_chain_validation/run.json` 的 32 场工件由 `63c04b99f533defb49742ce3d1f43ccefcc48f63` 生成，当前 `HEAD` 为 `264b2eed9c264ffde9a4d3146d441a3bf25c3cde`；最终校准工件在行为冻结后生成。`crates/sim/tests/architecture_decoupling.rs` 覆盖当前已实现结构；11 个策略族、完整回放清单和事件因果字段按 `docs/dev/current/full_simulation_plan.md` 实施。

## 2. 历史门矩阵：2026-10-21

以下矩阵是 2026-10-21 的历史记录，不描述当前工作区能力。各项 `verified` 只适用于当时记录的范围与测试。

状态含义：`verified` 只表示所列代码和测试范围存在；`partial` 表示实现或证据只覆盖规格的一部分；`未接线` 表示规格与数据已存在但没有任何消费点。

| 门 | 状态 | 历史代码事实 | 当时未闭合边界 |
| --- | --- | --- | --- |
| 确定性 | verified | `crates/sim/tests/baseline_fingerprint.rs` 覆盖 16 个种子的箱体、出手分区计数、回合结束原因分布与事实构成摘要，逐字段比对并报告差异路径 | 历史基线在 2026-10-30 工作区失配；完整工作区测试中，同种子重放通过，基线对比不通过 |
| 几何与语境合法性 | verified | 结算期逐动作记录；16 种子批次几何违规 0、语境违规 0 | 不覆盖未建模的动作 |
| 回合终结 | verified | `PossessionRecord.ended_by` 记录结束原因，`World::end_possession` 是唯一写入口 | 走步率按 `travel_per_game` 进分布判据，带宽由策略表给定 |
| 账本 | verified | `crates/sim/tests/engine.rs` 断言球员得分合计等于队伍合计、得分与出手账本对平、罚球命中数与事实流一致 | 加时犯规累计未按档案区分 |
| 分布判定 | partial | `gate::evaluate` 的聚合判据与过程判据共 39 条；16 场批次里 `zone_share_near`、`creation_share_putback`、`fta_per_game`、`steals_per_game` 四项越界，其余全部在带内 | 四项分别来自出手分区、补篮来源以及犯规与抢断抽样流的耦合；几何与语境违规为零 |
| 出手结果 | verified | `shot_make.json` 按出手类型与距离分箱；干扰以 `contest_sensitivity` 与 `contest_center` 两个标量做 logit 位移 | 无 closest-defender 分档数据源 |
| 球态与阶段 | verified | `BallState` 与 `Phase` 是封闭枚举；阶段推进集中在 `World::advance_phase` | — |
| 决策策略 | verified | `policy::choose_action` 以 `policy.json` 权重做线性效用，按正权重比例抽样；掩护后首个决策步把候选收敛到路线应答集；`Shoot` 需通过按分区最高可得分的归一化时钟门槛；抽样前按 `truncation_ratio` 剔除劣势候选 | 意向驻留尚未实现 |
| Play / 战术 | verified | `params.tactics` 加载七份体系档案与九份剧本，启动期校验 `then.slot`；`tactics::update_play_window` 每回合最多叫一次战术，`policy::choose_action` 消费 `carrier_preferences` 与 `inhibitions`，窗口结束发 `PlayWindowEnded`；实测激活率 0.756、窗口配对差 0 | 规则里的 `then` 动词尚未驱动目标生成 |
| 防守 | partial | `defense::solve_defense_targets` 按势能场求解五名防守人的目标点：对位羁绊、下沉、协防引力三项叠加；`PossessionState.matchup` 记录对位表，只在换防时交换；掩护路线回写进 `DefenderView.route` 并被决策层消费；`contest_for` 对钻过掩护与转换期防守未落位分别折减干扰 | 势能场尚未实现 X-Out 补位、真空吸力与滞回双阈值 |
| 协防引力场 | verified | `defense::solve_defense_targets` 读取 `world.rules.tactics.defense.potential_field`；四条行为判据与零堆分配均有测试（`crates/sim/tests/potential_field.rs`、`defense_allocation.rs`） | 真空吸力与 X-Out 阈值未接入 |
| 能力 | partial | 22 个能力维度；`shooting_*`、`strength`、`ball_handling`、`agility`、`defense_perimeter`、`height_cm` 进入效用与结算 | 逐维断路测试未覆盖全部 22 维 |
| 倾向 | partial | 13 个字段中 9 个进入生产路径；投射倾向 `three_point_preference` 同时作用于出手效用与站位伸缩 | `cut_frequency`、`offensive_rebound_frequency`、`risk_tolerance`、`transition_sprint` 在 `crates/sim` 里零引用 |
| 阵容与疲劳 | verified | `rotation::update_stamina` 双向计算体能（在场消耗、场下与停表恢复）；`rotation::substitute_if_needed` 按四条判据换人（犯满离场、体能阈值、首发犯规危机、垃圾时间）；`rotation::record_foul` 登记犯满 | 换人不区分对位与阵容搭配；实测单场换人 21.9 次、最长出场 22.1 分钟/场 |
| 残局 | verified | `endgame::win_probability` 给出解析胜率；`shot_constraint` 在末节按分差与剩余时间给出三分必投/压时间两种约束，残局指令优先于路线应答集；`intentional_foul` 在落后与砍人两种情形下判定故意犯规，最后两分钟只对持球人下手 | 胜率模型未进普通时段的决策权重 |
| 球员身份 | 仅投影 | `offensive_role` / `defensive_role` 随档案载入并进入 `StreamTick` 投影 | 角色的行为含义经 `PlayerTendencies` 承载，代码不按角色分支；`crates/domain/tests/roster_roles.rs` 把每类角色的最低行为要求写成判据 |
| 联赛档案 | verified | `GameRules::league` 参数化节时长、进攻时钟、个人犯满、bonus 与三分几何 | 干扰球与技术犯规没有程序 |
| 动作模型 | verified | `ActionIntent` 加 `resolve` 分支覆盖 9 类动作 | — |
| 文档守卫 | verified | `scripts/check_docs.py` 扫 30 份文档 204 处引用；常数、阈值、身份与行数守卫接入 CI | 工作面豁免路径实在性检查，本文件引用已删路径不会变红 |
| 验证基础设施 | partial | `crates/validation` 提供判定批次、机制实验与四态裁决；当时的报告为 `defects_found` | 32 场训练组的 `shooting_fouls_per_game` 未达验收带；40 种子机制实验中 speed、acceleration、strength、defensive_rebound、decision_iq 仍为 `insufficient` |
| 性能预算 | verified | 实测单场 0.03 s、58,936 tick；稳态每 tick 堆分配 0，由 `crates/sim/tests/allocation_budget.rs` 在一千个以上稳态 tick 上逐 tick 计数 | `--stream` 仍逐帧序列化，单场 1.46 s；按回合回放是替代路径 |
| 观测视图 | verified | 2.5D 正交斜视视图 `crates/debug-server/static/court3d.js` 与二维战术板共用协议帧；`node scripts/test_court3d.mjs` 覆盖十名球员可见、位置与插值坐标误差 0、小人偏航与 facing 误差 0、资产前向对齐误差 0、背向镜头的球员全部显示背面、落地最低点 0.04 英尺与头高 6.12 英尺、卡通渐层材质 60 个、全场四角在镜头内、正交投影下同一高度尺寸一致（比值 1.0000）、平移限制在球场内；二维动效 `scripts/test_court_visuals.py` 以 `?court=2d` 运行通过 | 引擎不下发腾空高度与动作标签，一次性动作由 `eventType` 与 `events` 推导；小人朝向直接取 facing，固定镜头下背面会被遮挡 |

## 3. 当前新增工作

H 阶段按 `docs/dev/current/full_simulation_plan.md` 的验收条件实施。协议层定义 `RenderAction` 与 `RenderPlayer.active_actions`；`crates/sim/src/stream.rs` 投影权威动作状态；`crates/debug-server/static/court3d.js` 与 `app.js` 读取同一 `ActionSpecTable`。`data/rules/action_animation_catalog.json` 与目录守卫登记 16 个动作族的动作变体；111 个条目的独立测试、两类渲染器同帧断言、未知值负面对照与回放定位在行为冻结后验证。

同时关闭了物理机制与规则层面的关键缺口：
- 伤病替换：`World::trigger_injury` 建立对抗伤病事件，并在活球中自动停表换人（`crates/sim/tests/substitution_scenarios.rs::an_injured_player_is_removed_and_replaced_by_bench`）。
- 教练挑战实质改判：依据复核事实证据改判并翻转球权，成功时不扣除暂停（`crates/sim/tests/challenge.rs::a_successful_challenge_overturns_ruling_and_retains_timeout`）。
- 防守连续链条与协防高级特性：封盖与抢断动作未成功时赋予失衡失位窗口，突破层感知并提升过人成功率；协防求解器接入滞回双阈值与弱侧 X-Out 补位轮转算法（`crates/sim/tests/potential_field.rs`）。
- 非投篮犯规生产路径：接入持球停步合球后的抱人犯规与篮板球卡位争夺推人犯规（`crates/sim/tests/foul_scenarios.rs`）。端到端测试与文档守卫均通过。
- 暂停与规则分类进生产路径：NBA 档案登记强制暂停触发点与三档暂停时长，`program/timeouts.rs::enforce_mandatory` 在死球时停表并给两队记账，`request_with_origin` 按来源与节次读档案时长（`crates/sim/tests/timeout_scenarios.rs`）。`FoulKind::Disruptive` 与联赛档案的 `foul_kinds`、`officials` 两节由 `challenge.rs::is_reviewable` 接进挑战入口与节末自动回放入口，节钟归零后的进球按出手瞬间记录的比赛钟裁定，钟响后离手的得分取消（`crates/sim/tests/replay_scope.rs`）。FIBA 破坏性犯规经 `apply_disruptive_foul` 按 Article 37 记 `FoulKind::Disruptive`、两罚与前场发球（`crates/sim/tests/foul_scenarios.rs::a_fiba_disruptive_foul_is_booked_separately_from_a_flagrant_foul`）。
- 封盖起跳后的应对链：滞空防守人面前，进攻方按对抗意愿造接触或按敏捷度侧移，两者都让封盖不可能发生（`crates/sim/tests/closeout_chain.rs`）。
- 赛前角色一致性：每一类进攻与防守角色的最低行为要求写成判据，并核对 ADR-020 的背身中锋与定点射手产出阈值与策略表一致；引擎对角色字段零读取，行为含义由 `PlayerTendencies` 承载（`crates/domain/tests/roster_roles.rs` 两个测试、`scripts/check_r6_tendency_wiring.py` 十三项生产引用守卫）。

## 4. 历史快照中的未完成工作

此处的历史矩阵和旧清单只用于追溯 2026-10-21 快照。当前工作树按 `docs/dev/current/full_simulation_plan.md` 的 A-H 工作包推进，稳定规格冲突已同步修订；最终状态、测试数量、门禁判据和校准工件在行为冻结后统一记录。

## 5. 证据入口

- 内核设计与缺口清单：[`action_chain_engine.md`](action_chain_engine.md)
- 跨周期差距与关闭条件：[`gap.md`](gap.md)
- 稳定里程碑顺序：[`roadmap.md`](roadmap.md)
- 已结束周期：[`cycles/`](cycles/)
- 基线指纹工件：`crates/sim/tests/baseline/fingerprint.json`（16 种子，随仓库提交，新 checkout 不依赖本地残留文件；记录方式见测试文件头部注释）
- 判定批次工件：`validate-core --out <目录>`
- 差距收敛周期的验证现场：`docs/dev/evidence/gap_closure_cycle/verification.json`（全量测试、文档守卫、动画目录守卫与门禁实测值）与 `docs/dev/evidence/gap_closure_cycle/gate.txt`（门禁逐条输出）

## 6. 历史判定批次：2026-10-21

下表中的命令与数据来自 2026-10-21 内核快照的历史验证批次，并非当前工作区最新运行。该批次有 1 项分布判定失败：`shooting_fouls_per_game` 为 16.531，目标为 19.597；其余 36 项分布与过程判定通过，几何违规、语境违规及账本失败均为 0。五维诊断通过。40 种子机制实验有 5 项 `insufficient`，认证为 `defects_found`。该历史批次的数值只用于追溯；最终校准工件在行为冻结后生成。

| 指标 | 观测 | 基准 |
| --- | --- | --- |
| 得分 | 207.06 | 227.59 |
| 回合 | 198.78 | 198.20 |
| 投篮出手 | 0.945 次/回合 | 0.898 次/回合 |
| 罚球 | 46.66 次/场 | 43.33 次/场 |
| 投篮犯规 | 16.531 次/场 | 19.597 次/场 |
| 进攻篮板 | 26.78 次/场 | 依当批可争抢投失量计算 |

本工作区的 `shooting_fouls_per_game` 为 16.531，未达到验收区间下限 16.657。40 种子机制实验中，`speed`、`acceleration`、`strength`、`defensive_rebound`、`decision_iq` 的配对 bootstrap 区间未完全位于预期方向，均判为 `insufficient`。

