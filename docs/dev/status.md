# NBA-Sim · 当前实现状态

> 状态类型：当前快照，不是执行日志。
> 动作链内核（`crates/sim`）的设计与缺口清单见 `docs/dev/action_chain_engine.md`。
> 证据原则：本文件只写当前工作区可以由代码、测试或守卫复核的结论。旧引擎的记录保留在 `docs/dev/cycles/`、`docs/dev/evidence/` 与 `docs/decisions.md`，本文件不再复述。
> 复核日期：2026-11-04。
> 校准正在按 `full_simulation_plan.md` §12 G 进行；残差记录在 `docs/dev/current/implementation.md` 的校准一节。

## 1. 当前工作区状态

当前实施阶段按 `docs/dev/current/full_simulation_plan.md` 执行；阶段与证据的逐项对应见 `docs/dev/evidence/stage_gate_map.md`。`crates/sim` 是唯一比赛模拟实现，同一完成比赛的 CLI 与 WASM 入口已通过 seed 211 的摘要和投影事件对比。独立事件账本、会话检查点恢复与磁盘检查点、事件游标、规则场景矩阵与动作场景矩阵已有对应测试。

全局单动作 `World::active` 已移除：`World::advance_parallel_actions` 逐人推进每名球员自己的 `ActionRuntime`，多人配合由交互编号结算，防守动作另有 `CloseoutContest`、`StealAttempt`、`BlockJump` 三种运行时。并发性由 `crates/sim/tests/parallel_actions.rs` 4 项直接验证（多人运行时同一 tick 共存且种类不同、出手期间防守并行、每个 `ActionStarted` 恰有一个终态、死球终态不起手）。

动作目录的 16 个族全部有生产路径与通过测试，状态是 `verified`，`missing_evidence` 为空；传球八种技术、切入六种路线、突破三种变体（直接、变向、转身）、背身步法、跳步违例、欧洲步、后撤步、包夹与护球、挤过与绕过、回位均已接入候选与后果消费者。规则程序目录的 22 项程序全部是 `implemented`，没有缺失场景；新接入的有进攻三秒、防守三秒、背身五秒、发球五秒、非法触球、双方犯规、恶意犯规、清晰路径犯规、后场八秒的「踢球」重置，以及暂停配额、罚球禁区与罚球人替换、换人、节次开始与交替拥有箭头。

裁决已收拢到 `World::adjudicate` 一个提交入口（`PenaltyPlan`、`RestartPlan`、`ResumeContext`、`InboundLocation`、`ControlState`）；时钟有 `ClockState`、`ClockDirective`、`ClockController`；内核按职责分到 `clock/`、`program/`、`officiating/`、`actions/`、`control/`、`ledger/` 六个目录，旧路径经 crate 根重导出保持可用；`LeagueProfile` 的存储本身就是 `PeriodPolicy`、`ClockPolicy`、`FoulPolicy`、`TimeoutPolicy` 四个策略族（`#[serde(flatten)]` 让磁盘上仍是扁平键）。球的飞行是 `World.ball_flight`（种类 + 结算时刻）：四种飞行与比赛程序分存，`clock::advance` 按飞行种类决定哪个钟走。

完整工作区测试（99 个二进制）在最近一次运行中全部通过，基线指纹已按当前行为重记；32 场 `validate-core` 的 38 条分布判据全部在带内，认证 `pass`。权威状态与档案结构已解耦：`Phase` 不含飞行变体，`ControlState` 是一个结构体，`LeagueProfile` 按四个策略族存储且没有未声明的默认值（`crates/sim/tests/architecture_decoupling.rs` 6 项）。未闭合项：干扰球在篮框上方与碰板后的两种情形（清单见 `docs/dev/evidence/stage_gate_map.md`）。

## 2. 历史门矩阵：2026-10-21

以下矩阵是 2026-10-21 的历史记录，不描述当前工作区能力。各项 `verified` 只适用于当时记录的范围与测试。

状态含义：`verified` 只表示所列代码和测试范围存在；`partial` 表示实现或证据只覆盖规格的一部分；`未接线` 表示规格与数据已存在但没有任何消费点。

| 门 | 状态 | 历史代码事实 | 当时未闭合边界 |
| --- | --- | --- | --- |
| 确定性 | verified | `crates/sim/tests/baseline_fingerprint.rs` 覆盖 16 个种子的箱体、出手分区计数、回合结束原因分布与事实构成摘要，逐字段比对并报告差异路径 | 历史基线在 2026-10-30 工作区失配；完整工作区测试中，同种子重放通过，基线对比不通过 |
| 几何与语境合法性 | verified | 结算期逐动作记录；16 种子批次几何违规 0、语境违规 0 | 不覆盖未建模的动作 |
| 回合终结 | verified | `PossessionRecord.ended_by` 记录结束原因，`World::end_possession` 是唯一写入口 | 走步率判据未登记 |
| 账本 | verified | `crates/sim/tests/engine.rs` 断言球员得分合计等于队伍合计、得分与出手账本对平、罚球命中数与事实流一致 | 加时犯规累计未按档案区分 |
| 分布判定 | partial | `gate::evaluate` 21 条聚合判据加 18 条过程判据；32 场批次里 `zone_share_near`、`creation_share_cut`、`possessions_per_game`、`usage_top1_over_fifth` 四项越界 | 四项的成因见 §5，分别是出手种类生成、空切目标生成、回合节奏与名册层级，均不是现有系数能解决的 |
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
| 球员身份 | 仅投影 | `offensive_role` / `defensive_role` 随档案载入并进入 `StreamTick` 投影 | 不进入效用、概率与行为分支；角色的行为含义经 `PlayerTendencies::three_point_preference` 承载 |
| 联赛档案 | verified | `GameRules::league` 参数化节时长、进攻时钟、个人犯满、bonus 与三分几何 | 干扰球与技术犯规没有程序 |
| 动作模型 | verified | `ActionIntent` 加 `resolve` 分支覆盖 9 类动作 | — |
| 文档守卫 | verified | `scripts/check_docs.py` 扫 30 份文档 204 处引用；常数、阈值、身份与行数守卫接入 CI | 工作面豁免路径实在性检查，本文件引用已删路径不会变红 |
| 验证基础设施 | partial | `crates/validation` 提供判定批次、机制实验与四态裁决；当时的报告为 `defects_found` | 32 场训练组的 `shooting_fouls_per_game` 未达验收带；40 种子机制实验中 speed、acceleration、strength、defensive_rebound、decision_iq 仍为 `insufficient` |
| 性能预算 | verified | 实测单场 0.03 s、58,936 tick；稳态每 tick 堆分配 0，由 `crates/sim/tests/allocation_budget.rs` 在一千个以上稳态 tick 上逐 tick 计数 | `--stream` 仍逐帧序列化，单场 1.46 s；按回合回放是替代路径 |
| 观测视图 | verified | 2.5D 正交斜视视图 `crates/debug-server/static/court3d.js` 与二维战术板共用协议帧；`node scripts/test_court3d.mjs` 覆盖十名球员可见、位置与插值坐标误差 0、小人偏航与 facing 误差 0、资产前向对齐误差 0、背向镜头的球员全部显示背面、落地最低点 0.04 英尺与头高 6.12 英尺、卡通渐层材质 60 个、全场四角在镜头内、正交投影下同一高度尺寸一致（比值 1.0000）、平移限制在球场内；二维动效 `scripts/test_court_visuals.py` 以 `?court=2d` 运行通过 | 引擎不下发腾空高度与动作标签，一次性动作由 `eventType` 与 `events` 推导；小人朝向直接取 facing，固定镜头下背面会被遮挡 |

## 3. 历史快照中的未完成工作

此处的历史矩阵和旧未完成工作清单只用于追溯 2026-10-21 快照。最近一次完整工作区运行的失败及其原因记录于 `docs/dev/current/implementation.md`：过时的 `baseline_fingerprint_matches_recorded_artifact`；该运行早于 CloseoutContest 修改。当前 `events_per_game` 上限已按 32 场生产运行的实测范围调整，五维诊断测试 3/3 通过；指纹工件先于目前的内核行为，需要重新审查后才能继续作为基线。当前未完成项和最新校准状态以 `docs/dev/current/implementation.md` 及唯一权威实施方案 `docs/dev/current/full_simulation_plan.md` 为准。进攻 `ActionRuntime` 有部分实现；独立防守 CloseoutContest 已有真实执行路径与定向测试，完整并行动作调度仍未完成。当前完整校准未运行，历史校准数据仅适用于其注明的 2026-10-21 快照。

## 4. 证据入口

- 内核设计与缺口清单：[`action_chain_engine.md`](action_chain_engine.md)
- 跨周期差距与关闭条件：[`gap.md`](gap.md)
- 稳定里程碑顺序：[`roadmap.md`](roadmap.md)
- 已结束周期：[`cycles/`](cycles/)
- 基线指纹工件：`crates/sim/tests/baseline/fingerprint.json`（16 种子，随仓库提交，新 checkout 不依赖本地残留文件；记录方式见测试文件头部注释）
- 判定批次工件：`validate-core --out <目录>`

## 5. 历史判定批次：2026-10-21

下表中的命令与数据来自 2026-10-21 内核快照的历史验证批次，并非当前工作区最新运行。该批次有 1 项分布判定失败：`shooting_fouls_per_game` 为 16.531，目标为 19.597；其余 36 项分布与过程判定通过，几何违规、语境违规及账本失败均为 0。五维诊断通过。40 种子机制实验有 5 项 `insufficient`，认证为 `defects_found`。当前改动后的完整校准仍待重新运行。

| 指标 | 观测 | 基准 |
| --- | --- | --- |
| 得分 | 207.06 | 227.59 |
| 回合 | 198.78 | 198.20 |
| 投篮出手 | 0.945 次/回合 | 0.898 次/回合 |
| 罚球 | 46.66 次/场 | 43.33 次/场 |
| 投篮犯规 | 16.531 次/场 | 19.597 次/场 |
| 进攻篮板 | 26.78 次/场 | 依当批可争抢投失量计算 |

本工作区的 `shooting_fouls_per_game` 为 16.531，未达到验收区间下限 16.657。40 种子机制实验中，`speed`、`acceleration`、`strength`、`defensive_rebound`、`decision_iq` 的配对 bootstrap 区间未完全位于预期方向，均判为 `insufficient`。

