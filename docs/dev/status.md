# NBA-Sim · 当前实现状态

> 状态类型：当前快照，不是执行日志。
> 证据原则：本文件只写当前工作区可以由代码、测试或守卫复核的结论。历史轮次见 `docs/dev/cycles/`，原始实验见 `docs/dev/evidence/` 与 `docs/decisions.md`。
> 复核日期：2026-09-29。R2 与 R6 全场统计矩阵、R6 倾向和个体评判已通过。R7 已增加 NBA v3 参考档案及两项联合/情境评判；2023-24 来源、官方 3PA/FGA 基准和 CourtGeometry 分区复算已核对，当前脚本通过。NBA 默认仍采用 v2；ADR-009、参考带独立审阅及归档完整获取与转换流程审核仍未完成。本次变更后的评估器 21 项单元、26 项既有集成和 5 项 R7 集成测试、Clippy、格式、文档守卫、常数守卫、参考复算与差异检查通过。R8 已实施：物理层持球旁路删除、球态唯一写通道守卫、阶段序列按 architecture.md §4.1 重排与逐 step 不变量校验回归；ADR-007 已按不变量形态接受；阶段重排后的 16 种子全场统计矩阵（总分/回合/三分/篮下占比带、五式账本、Hard 门与四路径覆盖）全部通过。R1 因果修复已实施并验证：出手起点取球实际位置、传球携带实际球高、控制转移按时三维路径时长，三类单帧球位跳变消除；哨响不截断在飞出手（罚球队列在到筐结算消费），哨响作废在飞传球时补发 `PASS_DROPPED` 终结；犯满离场不再被死球归因载荷阻塞。八种子 attribution 10 项全绿；黄金哈希随行为修正重冻结为 `0xca1d1cae20e46d0e`；决策基准重校准（`drive_rim_attack_bias` 1.3、`post_up_base` 3.6、`midrange_utility_bonus` 0.18）同时守住篮下与中距离逐种子下限并恢复背身/卡位系系数触达，16 种子矩阵复测通过。执行顺序和逐项验收范围见 [`current/implementation.md`](current/implementation.md)。

## 1. 结论

当前工作区有一条可运行的单场模拟链：固定种子可重放，事件带身份和父链，评判器区分通过、缺陷、不适用和证据不足，得分、球权、时间、犯规峰值和失误责任有账本检查。罚球路径由 `begin_free_throw_flight` 统一生成轨迹，强制结果与普通概率投篮都经由瞄准点与飞行几何解析；到达结算核对 finality 和逐次尝试，积分账本有集成断言。`free_throw` 11 项定向测试、`drive_geometry` 6 项、nba-officiating 5 项、领域球态边矩阵 3 项、physics backend contract 15 项、rim-contact 8 项、弹道单测 5 项已通过。R2 本轮松球篮板事件因果和 14 秒时钟回归 2 项、来源生命周期测试 5 项、节间与发球恢复测试 1 项、补篮候选测试 1 项、接球决策测试 3 项均通过；engine 全目标编译、strict clippy、格式、diff、文档、源文件行数和 inline-constant 守卫通过。新的 seed 1 来源与构成审计正在运行，新的 16 种子矩阵尚未运行；R2 验收未关闭。历史运行的 evaluator 区域分布存在缺陷，旧矩阵未覆盖篮下来源全路线。罚球路径核对 shooter、finality、逐次事件和账本。seed42 × 2000 哈希实测为 `0x53857b793b53951a`，原锚点 `0x765d0062e099cdb5` 保持不变，全矩阵评判门保持未关闭。

当前工作区不能宣称已经达到 `docs/charter.md` 的真实比赛目标。`docs/basketball.md` 定义了动作生命周期、结果权、防守责任、罚则和违例程序；实现尚未满足该规格。R1 已移除传球接球与突破终局的球态预写字段，将出手释放延迟至动作窗口边界，命中与触筐由到筐几何位置判定，犯规由物理接触事实裁定，传球五类结果互斥与突破结果因果父链已通过集成回归验证。投篮账本、箱体对账和有限信息接线的定向测试近期通过；八种子全帧球速包络测试全部通过。逐项范围见 `docs/dev/current/implementation.md`。篮下出手分布、无球动作、防守责任、犯规程序、个体身份和情境评判仍有明确缺口。

## 2. 门矩阵

状态含义：`verified` 只表示所列代码和测试范围存在；`partial` 表示实现或证据只覆盖规格的一部分。历史命令输出不在本表重复。

| 门 | 状态 | 当前代码事实 | 未闭合边界 |
| --- | --- | --- | --- |
| 确定性 | verified（测试范围） | `crates/engine/tests/golden_hash.rs` 冻结固定窗口哈希，并记录后续行为变更 | 黄金哈希只证明声明窗口内的输入输出稳定，不证明比赛真实 |
| L1 不变量 | verified（测试范围） | `crates/invariants` 检查发布帧；`crates/engine/tests/invariants.rs` 覆盖非法阶段迁移等回归 | 不能从单个测试范围外推到所有种子和所有联赛 |
| 事件身份 | verified（协议范围） | `crates/protocol/src/frame.rs` 的 `FrameEvent` 带 `event_id` 与 `parent_event_id` | 账本仍解析事件载荷，不是全部直接消费类型化事件 |
| 回合终结 | verified（类型范围） | `PossessionEndCause` 没有未归因变体，失误终结要求责任球员 | 责任字段与所有事件窗口的持续矩阵不在本快照重跑 |
| 账本 | verified（测试范围） | `crates/evaluator/src/ledger.rs` 五式：得分、球权、时间、犯规与失误责任。`check_foul_conservation` 按事件顺序重建个人累计、每节球队累计、犯满离场、bonus 罚则与逐次罚球，限值取自帧内 `FrameRules`；正负对照与 seed 1 全场对账通过 | 加时球队犯规累计未按档案区分；矩阵级 16 种子对账随 `stats_baseline` 门维护 |
| 评判模型 | partial | `Verdict` 有四态；`nba.v3` 新增 `SHOT_ZONE_MAKE_JOINT` 与 `LATE_Q4_SHOT_PROFILE`，参考带来自 2023-24 ShotChartDetail，按生产 `CourtGeometry::shot_zone` 分区复算；来源检查脚本通过，已有 21 项单元、26 项既有集成及 5 项 R7 集成测试通过记录；旧版仍兼容 | `nba.v3` 未设为默认版本；ADR-009、参考带独立审阅和归档完整性审查仍未完成；其他联合关系与比赛情境仍在盲区清单 |
| 统计带 | verified（测试范围） | `crates/engine/tests/stats_baseline.rs` 对 16 个全场种子断言统计带、五式账本、Hard 门、四路径覆盖与 evaluator 分区判定 | R6 当前全场矩阵：总分中位数 208.5，平均回合 245.4，三分命中率中位数 33.8%，篮下占比中位数 0.360 且 16/16 入带，中投份额中位数 0.159，账本和 Hard 门通过 |
| 出手结果 | partial | `PendingShotRelease` 保存命中概率与干扰强度，不保存命中或犯规布尔值；出手释放在动作窗口边界发生，封盖有独立 `BlockedShot` 事实；触筐与反弹由到筐几何位置裁定；犯规由语义接触事实裁定。八种子全帧球速回归已通过 | 尚未覆盖连续飞行中的细粒度触筐碰撞检测 |
| 球态 | partial | `crates/domain/src/flow.rs` 的 `BallState` 表达归属、飞行参数与动作阶段；转换有领域测试。物理层 `has_ball` 旁路已删除，持球人作为 `step_with_ball_holder` 显式输入传入；球态写入收敛到 `transition_ball_state` 唯一通道并由 `scripts/check_ball_state_writes.py` 守卫（ADR-007 accepted，不变量形态）；黄金哈希 seed42 × 2000 冻结为 `0xca1d1cae20e46d0e` | 外部 `BallState` 保持单枚举接口，`BallControl`/`BallMotion` 双枚举字面重构未做（gap.md §5.1 允许稳定接口） |
| 阶段调度 | verified（测试范围） | `MatchEngine::step_inner` 按 `docs/architecture.md` §4.1 顺序调度：时钟先行，跳球/死球流程为条件激活分支位于罚球与节末之后；阶段函数以「写入状态组」注释显式声明所写组名；`PhaseOutcome` 表达短路；十个状态组由 `scripts/check_engine_state_groups.py` 守卫；每 step 恰好一次不变量检查（含短路 tick）有回归断言 | 阶段函数仍接收 `&mut MatchEngine`（ADR-014 accepted 裁定，字段级编译隔离不采用） |
| 执行重校验 | partial | `apply_decision_output` 位于决策之后、物理步进之前 | 投篮命中和犯规已在进入释放前回放路径之前抽签 |
| Play | partial | `advance_active_play` 进入主循环；偏好和抑制进入决策效用；命中规则经 `resolve_verb` 改槽位目标。`play_engine_integration.rs` 覆盖触发、追踪和目标改写 | 动词产出的是固定深度的目标偏移。掩护接触、挡拆覆盖和防守选择没有统一生命周期 |
| 防守 | verified（测试范围） | `DefenseResponsibility` 九种主责任、`DefenseResponsibilityChanged` 责任交接事实、`ScreenDefenseRules` 挡拆策略（沉退/延误/换防/挤过/绕过）；`defense.rs`、`defense_effect.rs`、`defense_responsibility_chain.rs` 与 `defense_rotation_response.rs` 验证闭合 | ADR-008 实施闭合 |
| 能力 | partial | `PlayerAttributes` 有 22 个归一维度，含 `shooting_near`。出手分区由 `CourtGeometry::shot_zone` 决定，决策和出手结算共用 | 规格要求每个保留维度都有行为响应和断路测试。当前测试集合没有逐维覆盖全部 22 维 |
| 倾向 | verified（测试范围） | `PlayerTendencies` 有 12 个字段；`tendency_perturbation.rs` 对各字段有行为响应测试，单调响应、断路负面对照和真实比赛指纹检查均通过；`individual.rs` 已有使用率、助攻父链、责任恢复和末节体能准则 | 全场 16 种子统计矩阵、四区出手构成、账本和 Hard 门通过；个体表现仍未按球员角色分组 |
| 阵容与疲劳 | partial | `roster.rs` 处理换人；`modulation.rs` 更新体力；`rotation.rs` 检查疲劳换人 | `tactics.md` 的轮换表、暂停、垃圾时间和临场换体系没有完整程序 |
| 联赛档案 | verified（测试范围） | `LeagueProfile` 参数化节时长、进攻时钟、个人犯满（NBA 6/FIBA 5）、bonus（两者均为单节第 5 次，charter §6.3 核验）、三分几何和交替拥有。`league_profile.rs` 与 `fiba_scenarios.rs` 覆盖这些程序；攻方三秒与回场程序按同一规则通道执行 | 走步、二次运球、干扰球、技术犯规没有程序；加时犯规累计与暂停配额未按联赛档案分开验证。NCAA 不在首批范围 |
| 动作模型 | verified（测试范围） | `ActionStarted`、`ActionPhaseChanged`、`ActionCompleted`、`ActionCancelled`、`ActionFailed` 覆盖 13 种动作类型；保证同一球员同一 tick 至多一个活动动作窗口，且完成、取消与失败路径均解除运动学锁定；`lifecycle.rs` 17 项全过 | 无球与持球动作细分进入独立候选生成仍在扩展中 |
| 出手分布 | verified（测试范围） | 评判按四区统计：篮下、近筐、中投、三分；16 种子全场全过 `SHOT_PROFILE_ZONE_MIX` | 结合突破、切入接球、补篮与转换四路径事实，全部 16 种子均进入 `nba.v2.json` 的 `rim_share_of_fga` 参考带 |
| 球员身份 | partial | `PlayerData` 有六类位置和赛前攻防角色；投影写入 `RenderPlayer`，`identity_projection.rs` 检查整场稳定；`individual.rs` 有球队内使用率、助攻父链、防守责任和末节体能准则 | 身份字段仍未进入行为，个体准则不按角色分组 |
| 文档守卫 | verified（脚本范围） | `scripts/check_docs.py`、常数、阈值、身份、状态组和行数守卫有负面对照，并接入 CI | 守卫证明所列规则，不证明规格全部实现 |

## 3. 当前未完成工作

按 `docs/dev/gap.md` 的依赖顺序，当前执行队列是：

1. 出手、传球和突破的结果改由飞行、接触和封盖事实决定；
2. 在新的结果通道上闭合篮下出手分布；
3. 把已声明的无球和对抗动作接成可取消的动作生命周期；
4. 完成挡拆到防守选择的责任图；
5. 补齐走步、二次运球、干扰球、贴身五秒与背筐五秒（个人犯规、球队罚则、逐次罚球、攻方三秒与回场已闭合）；
6. 对齐 12 个倾向规格，并补个体与情境评判；
7. 用有来源的参考分布重标定真实度目标线。

R1 当前代码入口包括 `crates/engine/src/match_engine/execution.rs`、`crates/engine/src/match_engine/ball_flight/arms.rs`、`crates/engine/src/match_engine/events.rs` 与对应引擎测试。R1 的剩余验收项见 `docs/dev/current/plan.md` §2。

### R4 防守责任图闭合

实施防守责任图与责任交接事实：
- 责任状态：定义 `DefenseResponsibility` 包含 `PrimaryMatchup`、`Hedge`、`FightThrough`、`GoUnder`、`SwitchedMatchup`、`Help`、`Drop`、`Rotate`、`Recover` 九种主责任，保证每个防守人在任一 tick 有且仅有一个主责任；
- 责任交接事实：发布 `GameEvent::DefenseResponsibilityChanged`，包含执行防守人、原责任、新责任、对位进攻人及触发动作；
- 挡拆覆盖策略：`ScreenDefenseRules` 引入 `OnBallScreenDefenseStrategy`（`StandardContest`、`FightThrough`、`GoUnder`、`Switch`），联动 `drop_depth_ft` 与 `hedge_distance_ft` 在战术规划层生成对应责任；
- 档案差异与基线：`def_switch_heavy` 产生换防责任（`SwitchedMatchup`），`def_drop_coverage` 产生沉退责任（`Drop`），`def_hedge_recover` 产生延误责任（`Hedge`），`def_man_conservative` 恢复基线；
- 测试验证：`defense.rs` 4 项测试、`defense_responsibility_chain.rs` 5 项测试、`defense_effect.rs` 2 项测试、`defense_rotation_response.rs` 全部通过。R4 验收闭合。

### R5 规则与犯规账本

- 犯规账本：`check_foul_conservation` 按事件顺序重建个人累计、每节球队累计、犯满离场、bonus 罚则与逐次罚球对应（罚球因果父必须是指到它的判罚、罚球人必须是被侵犯人、判罚间不得遗留未执行的罚球、犯满球员必须被换下且不得再犯或再上）。限值取自帧内 `FrameRules`，检查器不自带副本。正负对照 8 项与 seed 1 全场对账通过。
- 逐次罚球：当前程序进行中的新判罚进入队列按序执行；命中的末罚在队列清空前不触发对方发球（修复判罚覆写导致的罚球丢失与账本不平衡）。
- 犯满离场：被持球状态阻塞的强制换人登记待换下并每 tick 重试。
- 攻方三秒：限制区（底线至罚球线 16×19 ft 矩形）连续停留计时，正运球或立即攻框的持球人按宽容条款不计数；冲筐类槽位行为（顺下/背切/下沉）改为前场时钟驱动的进-出曲线，掩护人基础位移出限制区；限制区内持球人的停车类候选（背身/试探/原地等待）被抑制。
- 回场：前场建立判定 + 事前传球阻断 + 发球例外（发球飞行无球队控制）；持球人战术目标不指向后场。
- 联赛档案：FIBA bonus 阈值核验为单节第 5 次（与 NBA 同，charter §6.3）；加时进节清零球队犯规（独立限额口径）。
- 转换终结归类：转换语境下的突破攻框按 TransitionFinish 归类（描述回合创建语境，优先于动作路径），并携带 TRANSITION_STARTED 因果父。
- 剩余：走步、二次运球、干扰球、贴身五秒、背筐五秒没有程序（需先补运球/合球阶段与飞行中球-人接触的物理事实）；加时累计口径未按档案区分（FIBA 计入第 4 节）。
- 松球停滞（seed 11 p123 曾 40 秒空转）：活球松球争抢优先派发——最近代表在一切战术微操（切割残留、被过恢复、槽位目标）之前被派 `REBOUND_CRASH`，每队至少一名未运动学锁定的代表。
- 板凳耗尽：候选（同队不在场且未犯满）为空时引擎发布流内事实 `BENCH_DEPLETED retain:<player>`（每球员仅一次），账本据此豁免「犯满未离场」与「犯满后继续犯规」——真实规则允许替补耗尽时犯满者留场。
- 松球接触：松球语境下的接触判罚按 `ContactPolicy::loose_ball_foul_multiplier`（0.15）缩减——争抢对冲属普通争抢，真实 loose ball foul 每场约 1-2 次。
- 犯规量校准：`SemanticRules::contact_foul_candidate_speed_ratio` 0.58→0.64（相对速度 12.8→14.1 ft/s 才算犯规候选），每场犯规 49→38 量级、FTA 54→40 量级、2P 命中率 0.33/0.30/0.275→0.32/0.29/0.265。16 种子矩阵：中位总分 227.5（带 [140,230]）、3P% 中位 36.8、篮下占比中位 0.325（16/16 带内）、回合数均值 251、账本 16/16 零违规、Hard 门全清。

## 4. 证据入口

- 原始实验和历史实测：[`evidence/problem.md`](evidence/problem.md)
- 跨周期差距和关闭条件：[`gap.md`](gap.md)
- 稳定里程碑顺序：[`roadmap.md`](roadmap.md)
- 已结束周期：[`cycles/`](cycles/)
