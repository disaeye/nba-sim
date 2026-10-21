# NBA-Sim · 当前实现状态

> 状态类型：当前快照，不是执行日志。
> 动作链内核（`crates/sim`）的设计与缺口清单见 `docs/dev/action_chain_engine.md`。
> 证据原则：本文件只写当前工作区可以由代码、测试或守卫复核的结论。旧引擎的记录保留在 `docs/dev/cycles/`、`docs/dev/evidence/` 与 `docs/decisions.md`，本文件不再复述。
> 复核日期：2026-10-21。

## 1. 结论

当前工作区有七个 crate：`domain`（规格与数据档案）、`sim`（动作链内核）、`protocol`（帧与流类型）、`validation`（判定与机制实验）、`cli`（`sim-core` 与 `validate-core`）、`bball-wasm`（浏览器适配）、`debug-server`（观测页面）。

`crates/sim` 是唯一的模拟实现。固定种子可重放；`World::step` 的每 tick 顺序固定为时钟 → 运动 → 动作到期结算 → 阶段推进 → 决策。结算产出状态变化与事实，事实进入 `GameReport.events`。

内核的结构性能力已经就位，逐项见 §2 与 `action_chain_engine.md` §9：战术层由 `params.tactics` 加载并由 `tactics::update_play_window` 驱动，防守几何由 `defense::solve_defense_targets` 求解，过程级判定有十八条判据进 `gate::evaluate`。剩余缺口是聚合层的校准，见 §3。

## 2. 门矩阵

状态含义：`verified` 只表示所列代码和测试范围存在；`partial` 表示实现或证据只覆盖规格的一部分；`未接线` 表示规格与数据已存在但没有任何消费点。

| 门 | 状态 | 当前代码事实 | 未闭合边界 |
| --- | --- | --- | --- |
| 确定性 | verified | `crates/sim/tests/baseline_fingerprint.rs` 覆盖 16 个种子的箱体、出手分区计数、回合结束原因分布与事实构成摘要，逐字段比对并报告差异路径 | 指纹只证明同输入稳定，不证明比赛真实 |
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
| 验证基础设施 | partial | `crates/validation` 提供判定批次、机制实验与四态裁决 | 机制实验中 `stamina` 登记为 `mechanism_change` |
| 性能预算 | verified | 实测单场 0.03 s、58,936 tick；稳态每 tick 堆分配 0，由 `crates/sim/tests/allocation_budget.rs` 在一千个以上稳态 tick 上逐 tick 计数 | `--stream` 仍逐帧序列化，单场 1.46 s；按回合回放是替代路径 |

## 3. 当前未完成工作

P0–P5 六个阶段与文档状态层同步已经结束，正文归档在 `docs/dev/cycles/20261021_engine_rebuild/`。剩余的是聚合判定校准，按 `docs/protocol.md` §1 第 7 条单独成一次改动：

1. 近筐份额：让突破在 5–14 英尺稳定产生抛投，属于出手种类生成的结构改动；
2. 空切份额：把空切接进目标生成，而不是调基础分；
3. 回合数：节奏由动作时长与决策间隔决定，不在效用权重上；
4. 使用率层级：按原型改写 `data/roster/` 的 `shoot_frequency`。

## 4. 证据入口

- 内核设计与缺口清单：[`action_chain_engine.md`](action_chain_engine.md)
- 跨周期差距与关闭条件：[`gap.md`](gap.md)
- 稳定里程碑顺序：[`roadmap.md`](roadmap.md)
- 已结束周期：[`cycles/`](cycles/)
- 基线指纹工件：`.work/baseline/fingerprint.json`（16 种子，已被版本控制忽略；记录方式见测试文件头部注释）
- 判定批次工件：`validate-core --out <目录>`

## 5. 判定批次

`nba-sim validate-core --seed 1 --games 32` 的当前结论是 `defects_found`：几何违规 0、语境违规 0、账本失败 0，二十一条聚合判据里 `zone_share_near`、`creation_share_cut`、`possessions_per_game`、`usage_top1_over_fifth` 四项越界，十八条过程判据里 `usage_top1_over_fifth` 越界。

| 指标 | 观测 | 基准 |
| --- | --- | --- |
| 得分 | 229.0 | 227.59 |
| 回合 | 209.8 | 198.20 |
| 近筐份额 | 0.139 | 0.191 |
| 空切份额 | 0.035 | 0.045 |
| 使用率比值 | 1.390 | 2.100 |
| 单场换人 | 21.4 | ≥ 8 |
| 战术激活率 | 0.705 | 0.65..0.85 |
| 背身中锋三分份额 | 0.012 | ≤ 0.04 |
| 定点射手三分份额 | 0.650 | ≥ 0.35 |

这四项各有具体成因，都不是现有系数能解决的：

- `zone_share_near`：模型里落到 5–14 英尺的出手只来自背身勾手与突破抛投，而抛投在时钟充裕时被 `too_contested_early` 挡掉；`post_base` 从 0.13 抬到 0.28 能把近筐份额推到 0.173，代价是背身份额越界。要同时满足两者，需要让突破在近筐带稳定产生抛投，属于出手种类生成的结构改动。
- `possessions_per_game`：回合数 208.8 比基准高 5.3%，差在回合时长；`dribble_base` 从 0.35 扫到 0.75 都改不动它，说明节奏由动作时长与决策间隔决定，不在效用权重上。
- `creation_share_cut`：`cut_base` 从 0.055 抬到 0.07 能把空切份额推过下界，代价是 `steals_per_game` 与 `shot_points_by_clock` 同时越界；两项之间的取舍需要把空切单独接进动作链的目标生成，而不是调基础分。
- `usage_top1_over_fifth`：轮换把八人出场时间拉平后，队内使用率层级被压平；要重建层级需要按原型改写名册的 `shoot_frequency`，而 `data/roster/` 不在本次改动范围之内。
