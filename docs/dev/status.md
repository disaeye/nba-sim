# NBA-Sim · 当前实现状态

> 状态类型：当前快照，不是执行日志。
> 动作链内核（`crates/sim`）的设计、参数来源与四层判定见 `docs/dev/action_chain_engine.md`；本文其余部分描述的是旧引擎 `crates/engine` 的状态。
> 证据原则：本文件只写当前工作区可以由代码、测试或守卫复核的结论。历史轮次见 `docs/dev/cycles/`，原始实验见 `docs/dev/evidence/` 与 `docs/decisions.md`。
> 复核日期：2026-10-07。R1 已闭合：Shot 飞行逐 tick 扫掠篮筐/篮板首次接触（`collision_checked_until` 区间追踪），触筐/触板按实际接触点与入射速度反弹，干净命中在筐平面结算，事件因果链保持 SHOT_RELEASE → SHOT_TRAJECTORY_ARRIVAL → SHOT_CONTACT/SCORE/SHOT_MISS/REBOUND；同参数封盖成功/失败对照（`block.rs`）与投篮归因、出手时序、罚球、封盖回归全部通过；黄金哈希随行为变更重冻结为 `0x21a35155c80c6e6b`（v97，登记于 `golden_hash.rs`）。R5 走步与二次运球已接入事实链：步态模型（双脚位置/摆动脚）在物理层产生接地点事实，已运球持球人在投篮/传球起手时收球并确立轴心脚（`DRIBBLE_GATHERED` 事件），收球后超两步或轴心脚容差外再落地即 TRAVEL，再突破即 DOUBLE_DRIBBLE；动作窗口运动学锁定收窄到 Execution/FollowThrough（Preparation 蓄势段保持移动，投篮起手同时设停止目标），违例哨作废未离手的挂起出手；专项测试与 3 种子 ×1q 稀疏性回归（零哨声）通过。R12 验证基础设施已启动（validation.md 五层实施）：协议层扩展、引擎观测事实发射与 `crates/validation` 的裁决/准则/重建/统计基础模块、场景框架、runner 与 CLI `validate` 已完成并通过守卫；黄金哈希随事件流扩充重冻结为 `0x4f0aabafed2e4d05`（v81，纯物理轨迹经对照探针逐位一致），吞吐基准带下界下调到 5000 并附三次重复测量。R12 工件与存储全部就位：validation.md §10 全清单十三件（含 `game_metrics.parquet`、`instances.sqlite`、旧兼容视图同源生成），集成测试 14 项全绿。R12 五层验证已接入开发流程：`run-tests.sh validation` 闸门（测试 → CLI 快检 → 认证汇总，退出码 0）+ 分层使用规范（日常 tier1 / 提交 validation 模式 / 里程碑整场批量）+ 页面持续可见（/api/validation/layers 与 /api/validation/certification 第 3/4/5 层详单与认证总览）。**r40-r50 引擎分布修正批次净成果（20 场终验收 defect 33/pass 19/insufficient 7）**：Pace 100.558/100.614（Pass，20 场精确）、3P% 0.355/0.366、FG% 0.455/0.474、near FG% 0.453/0.443（带内）；rim FG% 0.585/0.598、ORB 0.314/0.250、C04 0.887/0.811、TOV 0.197/0.134（收敛中）；结构性修正 9 项：四区记账 shot_id、回合开始原因事实与推导统一（21→28 格）、篮板分工+落点物理（100% 落 15ft 内）、暂停闭环（TeamTimeout + 余额 + 恢复帧发布）、R5 脚步状态机双模式、出手效用结构（位置语义一致化 + pull-up 候选 + 期望分地形 + 近框解罚）、推进移动语义（走廊居中 + 接应 + ADV 持久化）、暂停解冻 bug 修复、putback 情境门（负面结果留档）。golden v82→v95 全部按纪律登记。正式判定跑 20 场口径（并行批量），认证三件套投递网页可读目录（.work/validation-view/），场景-规则映射登记与分支覆盖工件已落地（27 规则域三态对照，实测 sampled 16 / scenario_only 2 / uncovered 9），MECH 11 实验覆盖全部 10 个可扰动属性（首跑可执行、方向趋势与物理通路一致），引擎检查点分叉与校验专用扰动入口已落地，JOINT-C01/C05/Q01 参照值已由 rebuild_joint_reference.py 从冻结归档重建入登记，full 叶项 10。批次 STAT 等效判定覆盖 42 项逐指标 + 3 项构成组 TV + 转移矩阵逐格 + 出手距离 W1，实测抓到的真实引擎偏差已驱动 r40-r46 修正批次。R12 待办：STAT-I01 分钟表来源（boxscore 归档）、追踪数据类 6 叶项（无公开渠道，保持 InsufficientEvidence）、MECH 实验功效提升（更多检查点）、引擎分布修正遗留清单（implementation.md：C01 末段交互 0.341/0.451、rim 份额 0.458/0.319、TOV 0.197/0.134 残余违例源、转换防守路线建模（8 秒违例残差 ~4 次/场 vs 真实 ~0.3）、C04 差距归因 OREB possession 非出手终结；出手位置语义一致化、三分份额治理、8 秒推进移动语义、putback 情境区分（负面结果留档）已由 r47-r49 关闭）。

## 1. 结论

当前工作区有一条可运行的单场模拟链：固定种子可重放，事件带身份和父链，评判器区分通过、缺陷、不适用和证据不足，得分、球权、时间、犯规峰值和失误责任有账本检查。罚球路径由 `begin_free_throw_flight` 统一生成轨迹，强制结果与普通概率投篮都经由瞄准点与飞行几何解析；到达结算核对 finality 和逐次尝试，积分账本有集成断言。`free_throw` 11 项定向测试、`drive_geometry` 6 项、nba-officiating 5 项、领域球态边矩阵 3 项、physics backend contract 15 项、rim-contact 8 项、弹道单测 5 项已通过；传球五分支互斥（`pass_speed_continuity.rs` 5 项）与突破终局因果（`drive_geometry.rs` 6 项）通过。R2 已闭合：全量 16 种子 `stats_baseline`（总分/回合/三分/篮下占比带、`SHOT_PROFILE_ZONE_MIX` 构成、四路径真实出手、账本与 Hard 门）全部通过，中位数与区间见 `current/plan.md` R2 节。R3（动作生命周期 17 项）与 R4（防守责任图）已闭合。R6 倾向与个体评判、R7 来源复核的当前状态见门矩阵。现行黄金哈希 seed42 × 2000 为 `0x21a35155c80c6e6b`（v97，连续飞行碰撞与走步事实链重冻结），演进链与变更源登记于 `crates/engine/tests/golden_hash.rs`。

当前工作区不能宣称已经达到 `docs/charter.md` 的真实比赛目标。`docs/basketball.md` 定义了动作生命周期、结果权、防守责任、罚则和违例程序；实现尚未满足该规格。R1 已闭合：Shot 飞行逐 tick 扫掠篮筐/篮板首次接触，触筐/触板按实际接触点与入射速度反弹，干净命中在筐平面结算，事件因果父链经投篮归因测试验证；同参数封盖成功/失败对照已建立；投篮账本、箱体对账和有限信息接线的定向测试通过；八种子全帧球速包络测试通过。逐项范围见 `docs/dev/current/implementation.md`。R5 的干扰球、贴身五秒、背筐五秒，R9/R10/R11 与 R12 的待办门是当前缺口。

## 2. 门矩阵

状态含义：`verified` 只表示所列代码和测试范围存在；`partial` 表示实现或证据只覆盖规格的一部分。历史命令输出不在本表重复。

| 门 | 状态 | 当前代码事实 | 未闭合边界 |
| --- | --- | --- | --- |
| 确定性 | verified（测试范围） | `crates/engine/tests/golden_hash.rs` 冻结固定窗口哈希，并记录后续行为变更 | 黄金哈希只证明声明窗口内的输入输出稳定，不证明比赛真实 |
| L1 不变量 | verified（测试范围） | `crates/invariants` 检查发布帧；`crates/engine/tests/invariants.rs` 覆盖非法阶段迁移等回归 | 不能从单个测试范围外推到所有种子和所有联赛 |
| 事件身份 | verified（协议范围） | `crates/protocol/src/frame.rs` 的 `FrameEvent` 带 `event_id` 与 `parent_event_id`；validation 扩展新增 `tick_index`、八个业务身份字段与 `relations` 结构化关系（`#[serde(default)]`，旧流可解析） | 账本仍解析事件载荷，不是全部直接消费类型化事件 |
| 回合终结 | verified（类型范围） | `PossessionEndCause` 没有未归因变体，失误终结要求责任球员 | 责任字段与所有事件窗口的持续矩阵不在本快照重跑 |
| 账本 | verified（测试范围） | `crates/evaluator/src/ledger.rs` 五式：得分、球权、时间、犯规与失误责任。`check_foul_conservation` 按事件顺序重建个人累计、每节球队累计、犯满离场、bonus 罚则与逐次罚球，限值取自帧内 `FrameRules`；正负对照与 seed 1 全场对账通过 | 加时球队犯规累计未按档案区分；矩阵级 16 种子对账随 `stats_baseline` 门维护 |
| 评判模型 | partial | `Verdict` 有四态；`nba.v3` 新增 `SHOT_ZONE_MAKE_JOINT` 与 `LATE_Q4_SHOT_PROFILE`，参考带来自 2023-24 ShotChartDetail，按生产 `CourtGeometry::shot_zone` 分区复算；来源检查脚本通过，已有 21 项单元、26 项既有集成及 5 项 R7 集成测试通过记录；旧版仍兼容 | `nba.v3` 未设为默认版本；ADR-009、参考带独立审阅和归档完整性审查仍未完成；其他联合关系与比赛情境仍在盲区清单 |
| 统计带 | verified（测试范围） | `crates/engine/tests/stats_baseline.rs` 对 16 个全场种子断言统计带、五式账本、Hard 门、四路径覆盖与 evaluator 分区判定 | R6 当前全场矩阵：总分中位数 208.5，平均回合 245.4，三分命中率中位数 33.8%，篮下占比中位数 0.360 且 16/16 入带，中投份额中位数 0.159，账本和 Hard 门通过 |
| 出手结果 | partial | `PendingShotRelease` 保存命中概率与干扰强度，不保存命中或犯规布尔值；出手释放在动作窗口边界发生，封盖有独立 `BlockedShot` 事实；Shot 飞行逐 tick 扫掠篮筐/篮板首次接触，命中由实际触筐事实决定，反弹按接触点与入射速度生成；犯规由语义接触事实裁定；同参数封盖成功/失败对照已建立；八种子全帧球速回归已通过 | 干扰球（下落段球-人接触）与篮筐干涉程序未实现 |
| 球态 | partial | `crates/domain/src/flow.rs` 的 `BallState` 表达归属、飞行参数与动作阶段；转换有领域测试。物理层 `has_ball` 旁路已删除，持球人作为 `step_with_ball_holder` 显式输入传入；球态写入收敛到 `transition_ball_state` 唯一通道并由 `scripts/check_ball_state_writes.py` 守卫（ADR-007 accepted，不变量形态）；黄金哈希 seed42 × 2000 现冻结为 `0x21a35155c80c6e6b`（v97：连续飞行碰撞与走步事实链；纯物理轨迹经对照探针逐位一致见 golden_hash.rs 演进链） | 外部 `BallState` 保持单枚举接口，`BallControl`/`BallMotion` 双枚举字面重构未做（gap.md §5.1 允许稳定接口） |
| 阶段调度 | verified（测试范围） | `MatchEngine::step_inner` 按 `docs/architecture.md` §4.1 顺序调度：时钟先行，跳球/死球流程为条件激活分支位于罚球与节末之后；阶段函数以「写入状态组」注释显式声明所写组名；`PhaseOutcome` 表达短路；十个状态组由 `scripts/check_engine_state_groups.py` 守卫；每 step 恰好一次不变量检查（含短路 tick）有回归断言 | 阶段函数仍接收 `&mut MatchEngine`（ADR-014 accepted 裁定，字段级编译隔离不采用） |
| 执行重校验 | partial | `apply_decision_output` 位于决策之后、物理步进之前 | 投篮命中和犯规已在进入释放前回放路径之前抽签 |
| Play | partial | `advance_active_play` 进入主循环；偏好和抑制进入决策效用；命中规则经 `resolve_verb` 改槽位目标。`play_engine_integration.rs` 覆盖触发、追踪和目标改写 | 动词产出的是固定深度的目标偏移。掩护接触、挡拆覆盖和防守选择没有统一生命周期 |
| 防守 | verified（测试范围） | `DefenseResponsibility` 九种主责任、`DefenseResponsibilityChanged` 责任交接事实、`ScreenDefenseRules` 挡拆策略（沉退/延误/换防/挤过/绕过）；`defense.rs`、`defense_effect.rs`、`defense_responsibility_chain.rs` 与 `defense_rotation_response.rs` 验证闭合 | ADR-008 实施闭合 |
| 能力 | partial | `PlayerAttributes` 有 22 个归一维度，含 `shooting_near`。出手分区由 `CourtGeometry::shot_zone` 决定，决策和出手结算共用 | 规格要求每个保留维度都有行为响应和断路测试。当前测试集合没有逐维覆盖全部 22 维 |
| 倾向 | verified（测试范围） | `PlayerTendencies` 有 12 个字段；`tendency_perturbation.rs` 对各字段有行为响应测试，单调响应、断路负面对照和真实比赛指纹检查均通过；`individual.rs` 已有使用率、助攻父链、责任恢复和末节体能准则 | 全场 16 种子统计矩阵、四区出手构成、账本和 Hard 门通过；个体表现仍未按球员角色分组 |
| 阵容与疲劳 | partial | `roster.rs` 处理换人；`modulation.rs` 更新体力；`rotation.rs` 检查疲劳换人 | `tactics.md` 的轮换表、暂停、垃圾时间和临场换体系没有完整程序 |
| 联赛档案 | verified（测试范围） | `LeagueProfile` 参数化节时长、进攻时钟、个人犯满（NBA 6/FIBA 5）、bonus（两者均为单节第 5 次，charter §6.3 核验）、三分几何和交替拥有。`league_profile.rs` 与 `fiba_scenarios.rs` 覆盖这些程序；攻方三秒与回场程序按同一规则通道执行 | 干扰球、技术犯规没有程序；走步与二次运球程序已实现但未按联赛档案分开验证（NBA/FIBA 走步规则同源）；加时犯规累计与暂停配额未按联赛档案分开验证。NCAA 不在首批范围 |
| 动作模型 | verified（测试范围） | `ActionStarted`、`ActionPhaseChanged`、`ActionCompleted`、`ActionCancelled`、`ActionFailed` 覆盖 13 种动作类型；保证同一球员同一 tick 至多一个活动动作窗口，且完成、取消与失败路径均解除运动学锁定；`lifecycle.rs` 17 项全过 | 无球与持球动作细分进入独立候选生成仍在扩展中 |
| 出手分布 | verified（测试范围） | 评判按四区统计：篮下、近筐、中投、三分；16 种子全场全过 `SHOT_PROFILE_ZONE_MIX` | 结合突破、切入接球、补篮与转换四路径事实，全部 16 种子均进入 `nba.v2.json` 的 `rim_share_of_fga` 参考带 |
| 球员身份 | partial | `PlayerData` 有六类位置和赛前攻防角色；投影写入 `RenderPlayer`，`identity_projection.rs` 检查整场稳定；`individual.rs` 有球队内使用率、助攻父链、防守责任和末节体能准则 | 身份字段仍未进入行为，个体准则不按角色分组 |
| 文档守卫 | verified（脚本范围） | `scripts/check_docs.py`、常数、阈值、身份、状态组和行数守卫有负面对照，并接入 CI | 守卫证明所列规则，不证明规格全部实现 |
| 验证基础设施（R12） | partial | `crates/validation`（`nba-validation`）：四态裁决模型、五层目录全量准则注册表、规则/统计档案、清单类型、严格流读取与实例/关系索引、独立重建器（箱体/回合/时钟/配对）、机会提取器、MOVE/TACT 事实层、统计比较器（配对 bootstrap、Bonferroni、Wasserstein-1、TV 距离）、JOINT 观测、机制实验框架、场景框架与 22 个必需场景、runner 与全部十三件工件（validation.md §10 全清单：`game_metrics.parquet` arrow/parquet 库写入、`instances.sqlite` 实例+关系查询存储、`ledger_comparison.json`、旧兼容视图同源生成等）、CLI `validate`（退出码 0/1/2/3）；集成测试 14 项（含截断拒绝、双结算检测、关系审计、parquet 读回、SQLite 查询、兼容视图同源一致性、配对 bootstrap 数学边界）全绿；协议层流首尾部类型就位。引擎侧发射 `StatAttribution`、`ShotAttemptResolved`、`ControlEstablished`、责任生命周期、时钟/节末事实与运动意图（到期清算，驻留任务不产生逐 tick 流水）；流尾未完成实例数经只读访问器取真实值 | STAT/JOINT 正式判定依赖的参考数据与等效容差未登记（数据准备任务，阻塞 RealismValidated）；机制实验检查点 API 为已登记缺口 |

## 3. 当前未完成工作

按 `docs/dev/gap.md` 的依赖顺序，当前执行队列是：

1. 补齐干扰球、贴身五秒与背筐五秒（个人犯规、球队罚则、逐次罚球、攻方三秒、回场、走步与二次运球已闭合；R1 已闭合）；
2. R9 战术三层（对位和换防 `assign_matchups`、体系档案化、防守 id 统一）；
3. R10 球员输入（`wingspan_cm`/`effective_reach`/`release_height` 与剩余独立映射、`domain::scouting` 展示投影）；
4. R11 评判闭环（动作完整、罚则完整、跳球与交替拥有准则）；
5. R7 收尾（ADR-009、参考带独立审阅、归档导入管线审核）；
6. R12 验证基础设施（validation.md 五层实施）：集成测试、旧工件兼容视图、`game_metrics.parquet` 与 SQLite 实例存储、文档收尾（协议层、引擎观测事实发射、验证器基础模块、场景框架、runner 与 CLI `validate` 已完成，见门矩阵「验证基础设施」行与 `docs/dev/current/implementation.md` R12 节）。

R1 与 R5 走步/二次运球的代码入口见 `docs/dev/current/plan.md` §2 与 §6。

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
- 剩余：干扰球、贴身五秒、背筐五秒没有程序（干扰球需先补飞行中球-人接触物理事实）；走步与二次运球已接入步态接地点事实与收球链（见门矩阵与 implementation.md R5 节）；加时累计口径未按档案区分（FIBA 计入第 4 节）。
- 松球停滞（seed 11 p123 曾 40 秒空转）：活球松球争抢优先派发——最近代表在一切战术微操（切割残留、被过恢复、槽位目标）之前被派 `REBOUND_CRASH`，每队至少一名未运动学锁定的代表。
- 板凳耗尽：候选（同队不在场且未犯满）为空时引擎发布流内事实 `BENCH_DEPLETED retain:<player>`（每球员仅一次），账本据此豁免「犯满未离场」与「犯满后继续犯规」——真实规则允许替补耗尽时犯满者留场。
- 松球接触：松球语境下的接触判罚按 `ContactPolicy::loose_ball_foul_multiplier`（0.15）缩减——争抢对冲属普通争抢，真实 loose ball foul 每场约 1-2 次。
- 犯规量校准：`SemanticRules::contact_foul_candidate_speed_ratio` 0.58→0.64（相对速度 12.8→14.1 ft/s 才算犯规候选），每场犯规 49→38 量级、FTA 54→40 量级、2P 命中率 0.33/0.30/0.275→0.32/0.29/0.265。16 种子矩阵：中位总分 227.5（带 [140,230]）、3P% 中位 36.8、篮下占比中位 0.325（16/16 带内）、回合数均值 251、账本 16/16 零违规、Hard 门全清。

## 4. 证据入口

- 原始实验和历史实测：[`evidence/problem.md`](evidence/problem.md)
- 跨周期差距和关闭条件：[`gap.md`](gap.md)
- 稳定里程碑顺序：[`roadmap.md`](roadmap.md)
- 已结束周期：[`cycles/`](cycles/)

## 5. 动作链内核判定（当前批次）

`nba-sim validate-core --seed 1 --games 32 --mechanism` 的当前结论是 `insufficient_evidence`，退出码 3：分布层十七项全部在带宽内，机制实验十项通过、一项登记为已知不足、零缺陷。

| 层 | 当前值 | 基准 | 判定 |
|---|---|---|---|
| 得分 | 233.0 | 227.6 | 通过 |
| 回合 | 204.1 | 198.2 | 通过（带宽 ±5%） |
| 几何违规 | 0 | 0 | 通过 |
| 语境违规 | 0 | 0 | 通过 |
| 逐场账本失败 | 0 | 0 | 通过 |

机制实验里 `speed`（−8.5%）与 `acceleration`（−3.8%）属于接线证据，度量直接取自推进时长公式；其余九项属于产物证据。`shooting_mid` 的中投出手份额只变化 +0.06%，登记在 `data/calibration/known_insufficient.json`，未登记的新不足项按缺陷处理并返回退出码 1。

部分比赛（面板的 `1q` 范围）只判几何、语境与账本三层；分布层需要整场比赛。
