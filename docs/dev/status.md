# NBA-Sim · 当前实现状态

> 状态类型：当前快照，不是执行日志。
> 证据原则：本文件只写当前工作区可以由代码、测试或守卫复核的结论。历史轮次见 `docs/dev/cycles/`，原始实验见 `docs/dev/evidence/` 与 `docs/decisions.md`。
> 复核日期：2026-09-24。本快照没有在本次复核中重跑 16-seed 矩阵或全量测试套件。

## 1. 结论

当前工作区有一条可运行的单场模拟链：固定种子可重放，事件带身份和父链，评判器区分通过、缺陷、不适用和证据不足，得分、球权、时间、犯规峰值和失误责任有账本检查。

当前工作区不能宣称已经达到 `docs/charter.md` 的真实比赛目标。`docs/basketball.md` 现已定义动作生命周期、结果权、防守责任、罚则和违例程序；实现尚未满足该规格。传球和突破结果仍在动作开始时写入球态，投篮结果权的工作区改动尚未完成；篮下出手分布、无球动作、防守责任、犯规程序、个体身份和情境评判仍有明确缺口。

## 2. 门矩阵

状态含义：`verified` 只表示所列代码和测试范围存在；`partial` 表示实现或证据只覆盖规格的一部分。历史命令输出不在本表重复。

| 门 | 状态 | 当前代码事实 | 未闭合边界 |
| --- | --- | --- | --- |
| 确定性 | verified（测试范围） | `crates/engine/tests/golden_hash.rs` 冻结固定窗口哈希，并记录后续行为变更 | 黄金哈希只证明声明窗口内的输入输出稳定，不证明比赛真实 |
| L1 不变量 | verified（测试范围） | `crates/invariants` 检查发布帧；`crates/engine/tests/invariants.rs` 覆盖非法阶段迁移等回归 | 不能从单个测试范围外推到所有种子和所有联赛 |
| 事件身份 | verified（协议范围） | `crates/protocol/src/frame.rs` 的 `FrameEvent` 带 `event_id` 与 `parent_event_id` | 账本仍解析事件载荷，不是全部直接消费类型化事件 |
| 回合终结 | verified（类型范围） | `PossessionEndCause` 没有未归因变体，失误终结要求责任球员 | 责任字段与所有事件窗口的持续矩阵不在本快照重跑 |
| 账本 | partial | `crates/evaluator/src/ledger.rs` 检查得分、球权、时间、犯规和失误责任，共五式 | `check_foul_conservation` 只比较犯规事件数与单节球队犯规峰值，不重建个人犯规、bonus 和罚则 |
| 评判模型 | partial | `Verdict` 有四态；`ASSIST_PROFILE` 固定返回 `InsufficientEvidence`；构成准则有固定分母 | `crates/evaluator/fixtures/blind_spots.md` 登记的联合分布、情境、个体和序列仍无准则 |
| 统计带 | partial | `crates/engine/tests/stats_baseline.rs` 对 16 个全场种子断言统计带、五式账本和 Hard 门 | 最近一次全矩阵的原始数字只存在于历史证据；本快照不引用未重跑的结果。篮下占比带见 `gap.md` G6a |
| 出手结果 | partial | `PendingShotRelease` 在 `execute_shot` 写入 `is_made`、`fouled`、飞行时间和弧顶；释放时刻只回放这些字段。封盖有独立 `BlockedShot` 事实 | 球路、封盖和触筐不重新决定这颗球是否命中。`docs/blind_spots.md` 第 4 条登记此缺口 |
| 球态 | partial | `crates/domain/src/flow.rs` 的 `BallState` 是归属、飞行参数和预定结果的联合类型；转换有领域测试 | `BallControl` 与 `BallMotion` 正交迁移仍是 ADR-007 proposed |
| 阶段调度 | partial | `MatchEngine::step_inner` 按具名阶段顺序调用；`PhaseOutcome` 表达短路；十个状态组由 `scripts/check_engine_state_groups.py` 守卫 | 阶段函数仍接收 `&mut MatchEngine`。ADR-014 记录了未改成字段级窄签名的原因 |
| 执行重校验 | partial | `apply_decision_output` 位于决策之后、物理步进之前 | 投篮命中和犯规已在进入释放前回放路径之前抽签 |
| Play | partial | `advance_active_play` 进入主循环；偏好和抑制进入决策效用；命中规则经 `resolve_verb` 改槽位目标。`play_engine_integration.rs` 覆盖触发、追踪和目标改写 | 动词产出的是固定深度的目标偏移。掩护接触、挡拆覆盖和防守选择没有统一生命周期 |
| 防守 | partial | `data/defense/schemes.json` 的部分参数进入目标几何，`defense_effect.rs` 检查几何和结果分布；势能场、滞回、协防混合和弱侧标签有单元测试 | 挤过、绕过、换防、延误、沉退、恢复没有完整的触发、责任、失败和反事实证据。ADR-008 仍是 proposed |
| 能力 | partial | `PlayerAttributes` 有 22 个归一维度，含 `shooting_near`。出手分区由 `CourtGeometry::shot_zone` 决定，决策和出手结算共用 | 规格要求每个保留维度都有行为响应和断路测试。当前测试集合没有逐维覆盖全部 22 维 |
| 倾向 | partial | `PlayerTendencies` 有 8 个字段。`attributes.md` 登记 12 个倾向 | `gamble_steal`、`block_aggressiveness`、`help_aggressiveness`、`physicality` 不在结构体中。`cut_frequency`、`screen_frequency` 进入档案和派生角色，没有决策或运动消费点 |
| 阵容与疲劳 | partial | `roster.rs` 处理换人；`modulation.rs` 更新体力；`rotation.rs` 检查疲劳换人 | `tactics.md` 的轮换表、暂停、垃圾时间和临场换体系没有完整程序 |
| 联赛档案 | partial | `LeagueProfile` 参数化节时长、进攻时钟、个人犯满、bonus、三分几何和交替拥有。`league_profile.rs` 与 `fiba_scenarios.rs` 覆盖这些程序 | 走步、三秒、干扰球、技术犯规和完整罚则账本没有按联赛档案分开验证。NCAA 不在首批范围 |
| 动作模型 | partial | 持球候选为投、突、背身、传、发球、试探、停顿、推进。动作窗口有准备、执行、随挥 | `PostMoveKind` 无决策消费点。运球和跳投细分主要改变动作名。无球九种动作没有独立候选生成器 |
| 出手分布 | partial | 评判按四区统计：篮下、近筐、中投、三分。`nba.v2.json` 带有四区占比带 | 四区占比带未在本次合并中重跑。文档禁止单独用效用斜率凑这些数字 |
| 球员身份 | partial | `PlayerData` 有六类位置和赛前攻防角色；投影写入 `RenderPlayer`，`identity_projection.rs` 检查整场稳定 | 身份字段不进入行为。个体表现评判仍未按身份聚合 |
| 文档守卫 | verified（脚本范围） | `scripts/check_docs.py`、常数、阈值、身份、状态组和行数守卫有负面对照，并接入 CI | 守卫证明所列规则，不证明规格全部实现 |

## 3. 当前未完成工作

按 `docs/dev/gap.md` 的依赖顺序，当前执行队列是：

1. 出手、传球和突破的结果改由飞行、接触和封盖事实决定；
2. 在新的结果通道上闭合篮下出手分布；
3. 把已声明的无球和对抗动作接成可取消的动作生命周期；
4. 完成挡拆到防守选择的责任图；
5. 补齐走步、三秒、干扰球和个人犯规罚则账本；
6. 对齐 12 个倾向规格，并补个体与情境评判；
7. 用有来源的参考分布重标定真实度目标线。

第 1 项的当前代码入口是 `crates/engine/src/match_engine/execution.rs` 的 `execute_shot` 和 `crates/engine/src/match_engine/state.rs` 的 `PendingShotRelease`。

## 4. 证据入口

- 原始实验和历史实测：[`evidence/problem.md`](evidence/problem.md)
- 跨周期差距和关闭条件：[`gap.md`](gap.md)
- 稳定里程碑顺序：[`roadmap.md`](roadmap.md)
- 已结束周期：[`cycles/`](cycles/)
