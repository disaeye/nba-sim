# 实施计划与当前执行状态

本文件记录当前实施范围、验收顺序与已核实证据。任务状态以会话中的 todo 清单为准；代码事实、测试结果和待办要求分别记载，避免把计划描述当作完成证据。

## 执行依赖

R1 结果事实与因果来源是 R2 篮下分布及 R8 球态重组的前置条件。R2 完成后执行 R3 动作生命周期，再执行 R4 防守责任图、R5 规则与犯规账本、R6 倾向与个体评判、R7 有来源的参考分布。R9 战术层依赖 R4，R10 球员输入依赖 R6，R11 评判闭环依赖 R3、R4、R5。

R1 细分任务按实际触筐事实、接触犯规事实、传球互斥及因果、突破决定事实、证据和文档同步的次序执行。任何结果都必须在其决定性事实发生时形成，并能由事件流回溯来源。

## R1 结果事实与因果来源

### R1.1 投篮结果

验收要求：从连续球路中的实际篮筐、篮板碰撞或篮筐平面通过事实判定命中、触筐未中或无接触偏出；以检测到的碰撞位置和速度生成反弹；投篮、接触、得分或未中、篮板之间保存同一投篮身份及父事件。

当前已核实：`BallState::Shot` 不再保存预先判定的命中结果；`SHOT_TRAJECTORY_ARRIVAL`、`SHOT_CONTACT` 已加入事件与因果链接；rim-contact 的反弹计算可接收接触位置。现有到达裁定仍只在 `tau >= 1` 的采样端点运行，篮板分支中的 backboard contact 仍使用预测 probe，故连续飞行碰撞权威尚未完成。八种子帧差分新增后记录到多类连续帧超出 85 ft/s：seed 2024/555 在 FREE_THROW 进入帧从先前持球或突破位置直接放置罚球点，另外还检测到 PASS、LOOSE_BALL、HELD 连续帧超限。罚球准备使用 `FreeThrowSetup` 三维轨迹从犯规球位移动到罚球点，时长按三维距离与球速预算计算；到达后发布 `BallPlacementApplied`，再开始罚球飞行。当前修改统一由 `begin_free_throw_flight` 依据当前 shooter、投篮概率、强制结果和最终罚球标记生成唯一罚球轨迹；强制命中瞄准筐中心，强制未中瞄准筐环接触区，普通罚球依据能力概率采样。准备态和飞行态均核对当前罚球 shooter、剩余次数与 `is_final`；到达结算以 checked attempt 序号记录事件，并更新尝试数、命中数、剩余次数和比分。新增集成断言核对连续罚球事件序号、逐次结果、箱体和比分，以及末罚命中后对方发球、末罚未中生成 `RimRebound` 并从检测到的接触位置开始。当前本轮复测通过 `cargo check --offline -p nba-engine --all-targets` 和 `free_throw` 集成测试（11 项）。`ball_ownership`（3 项）、`backend_contract`（15 项）、`rim_contact`（8 项）、physics 弹道单测（5 项）、罚球因果归属测试及 engine clippy 复测通过。`check_docs.py`、`check_max_source_lines.py`、`git diff --check` 与 `check_inline_constants.py` 均已通过；新增内联常数收编至配置数据通道，生产预算完成下调。八种子全帧球速回归测试 `all_adjacent_frames_respect_ball_speed_envelope` 10 项测试已全部通过。黄金哈希 seed42 × 2000 实测值为 `0xc8b4a2c005d473d8`，原锚点 `0x765d0062e099cdb5` 保持不变，全矩阵评判门尚未关闭。相关 todo 保持未完成。

### R1.2 犯规事实

验收要求：投篮或突破犯规必须引用实际非法 `Contact`，并保留出手身份；接触事实先于犯规发布；罚球不得截断仍在飞行的出手；删除投篮到达或突破结束时独立抽取犯规的路径。

当前已核实：已移除突破终结处的独立概率犯规抽取，犯规事件统一由语义层分类的接触事实经 `ResolutionLayer::resolve_semantic_contact` 裁定后发布；接触事实作为独立前置事件先于犯规发布，犯规事件通过 `causal_parent_of` 链接其对应的接触父事件；投篮动作过程中的非法身体接触分类为 `ShootingContactCandidate`，并进入投篮犯规裁定通道。罚球不截断在飞出手（R1.2 验收条款）：哨响时球已离手在飞或被犯规者本人的出手仍在动作窗口中，罚球程序入队且球按自己的弹道到筐结算（命中计分即加罚、不中发 `SHOT_MISS`），结算点消费队头启动罚球，不再出现已释放出手被死球凭空丢弃（无结果事实且命中分丢失）的路径；哨响时另有他人未离手的挂起出手则取消窗口并静默作废，不发布无结果的出手事实；在飞出手被犯规后被封盖的，封盖事实照发，球不落成松球而由罚球接管。犯满离场不再被死球归因载荷阻塞：哨响时持球人即归因人，强制换人先清除指向离场者的 `Dead.last_touch_player`，替补在犯满当 tick 即可执行。

### R1.3 传球结果与因果

验收要求：互斥表达接球、点掉、掉球、抢断和出界；接球必须有空间接近事实；结果父级关联唯一 PASS 事件；过渡期的球位与速度连续；接球能力差异可改变估计与结果。

当前已核实：传球结算路径实现了抢断、点拨、接住、掉球与出界的互斥裁定，单一传球尝试在终态至多触发一个结果分支；结果事件的父级统一指向触发它的 PASS 事件；接球成功后经速度受限的 `ControlTransfer` 轨迹平滑过渡至持球位（时长按三维路径长度计算，从地板球拾取不再超速），消除了位置跳变；哨响（违例或犯规判罚）作废在飞传球时在哨响位置补发 `PASS_DROPPED` 终结事实，父级经因果槽指向该 PASS 事件，事件流不再出现无结局的传球；`pass_speed_continuity.rs` 包含 5 项针对接球连续性、拦截互斥、点拨互斥、掉球互斥与多种子互斥的回归测试并全部通过；`decision_wiring.rs`（3 项）与 `pass_information.rs`（4 项）通过。

### R1.4 突破结果与因果

验收要求：突破终局由接触、位置和终结窗口事实决定，不在动作创建时预写概率结果；突分、急停跳投、受阻与攻框终结具备清晰的分支事实；结果事件的父级关联触发突破的 `DRIVE_INITIATED` 事件。

当前已核实：突破推进在动作窗口到达或提前终结门触发时按实际空间距离和技能判定攻框还是受阻停滞，已移除终结处的独立概率犯规抽取；突破结果事件 `DRIVE_REACHED` 与 `DRIVE_STOPPED` 均通过 `causal_parent_of` 链接其对应的 `DRIVE_INITIATED` 父事件；`crates/engine/tests/drive_geometry.rs` 包含 6 项针对路径绕行、防守阻断、对抗优势以及因果父链的集成测试并全部通过。

### R2 本轮复核

突破终结空间判定与出手位置现采用 `driver_pos`，终结点经 `drive_finish_extend_ft` 配置向篮筐推进。R2 统计回归已从生产 `SHOT_RELEASE` 的位置和对方篮筐几何计算分区，并要求箱体 FGA 与所有出手事件逐场一致。当前来源事实以 `ShotCreationSource` 标记；切入接球、进攻篮板和转换来源的 SHOT_RELEASE 父事件须分别指向匹配的 `PASS_RECEIVED`、进攻 `REBOUND` 和 `TRANSITION_STARTED` 事件，驱动急停出手关联突破事实。`ShotRelease.source_event_id` 由待释放状态保存，并用于事件发布时指定父事件；发布点核对父事件类型及关联球员。

最近完成的历史矩阵中，seed 1 有 180 次出手、67 次篮下（占比 0.372）、18 次近筐、71 次中投和 24 次三分，FGA 与箱体统计一致；篮下来源包含突破、切入接球和阵地战。`SHOT_PROFILE_ZONE_MIX` 因中投占比 0.394 超过 0.30 上限判为 Defect。该 16 种子矩阵各场篮下占比为 0.324–0.434、账本无违规，但全矩阵没有进攻篮板篮下出手，转换篮下出手只出现两次；该运行未通过新增构成断言。

本轮修复了弹框松球被收回时的篮板事实：进攻与防守均发布 `REBOUND`，父事件链接到出手到达事实；进攻篮板重置联赛配置的进攻时钟。进攻篮板持球人作其他决策时保留来源上下文，决策候选只留下投篮，以确保机会仍能产生补篮。来源判定按实际球员位置与 `CourtGeometry::shot_zone` 检查篮下分区；转换出手在篮下区域继承转换上下文；决策层为转换篮下出手提供加成并免除早出手惩罚。接球决策由 `PASS_RECEIVED` 事实触发即时决策，后续决策继续遵循正常决策间隔。命中率与犯规率基准已完成阶段校准，全量 16 种子 `stats_baseline`（`full_game_stats_within_baseline_band`）已全部通过：总分中位数 210.5（区间 [158, 251]，通过 [140, 230] 门槛）；平均回合数 247.5（通过 [172, 290] 门槛）；平均回合时长 14.40s（通过 [12.0, 20.0] 门槛）；三分命中率中位数 36.6%（通过 [30.0, 40.0] 目标带）；篮下出手占比中位数 0.371，全部 16 种子均进入 [0.25, 0.50] 参考带（16/16）；16 种子全部通过 evaluator `SHOT_PROFILE_ZONE_MIX` 构成判定；突破、切入接球、前场篮板与攻防转换四条路径在矩阵中均有真实出手；每场比赛的出手分区计数与箱体 FGA 严格一致；五式账本 16 种子全平（零违规）；Hard 门 16 种子全部通过。单种子来源审计 `rim_attempt_composition` 同步通过（FGA 195，rim 76 占比 0.390，`zone_verdict` 为 Pass）。R2 篮下出手分布与来源因果事实闭合。

## R2–R11 验收范围

### R2 篮下出手分布

突破终结、空切接球、前场篮板补篮、转换终结四条路径须产生可观测的篮下出手。使用 16 个全场种子核验 `rim_share_of_fga` 进入 `nba.v2.json` 参考带，并同时核对三分、罚球、比分账本及 Hard 门。

入口：`crates/evaluator/src/composition.rs`、`crates/evaluator/fixtures/nba.v2.json`、`crates/engine/tests/stats_baseline.rs`、`crates/engine/tests/defense_rotation_response.rs`。

### R3 动作生命周期

已完成动作开始、阶段变化、完成、取消、失败事件的定义与接入：
- `GameEvent` 增加 `ActionStarted`、`ActionPhaseChanged`、`ActionCompleted`、`ActionCancelled`、`ActionFailed`；
- `ActionCancellationReason` 包含 `RevalidationFailed`、`PreemptedByFoul`、`PreemptedByViolation`、`PossessionLost`、`Superseded`；
- `ActionFailureReason` 包含 `Blocked`、`BallStripped`、`IllegalScreen`、`Timeout`、`Contested`、`Interrupted`；
- `ActionType` 扩展 `ScreenRoll`、`ScreenPop`、`Cut`、`CutBackdoor`、`BoxOut`、`Putback`；
- `ActionTimeWindow` 接入对应物理窗口构造器；
- `MatchEngine` 接入 `start_action_window`、`cancel_action_window`、`fail_action_window`、`cancel_all_active_windows`；
- 保证同一球员同一 tick 至多一个活动未完成动作，若新动作启动则旧动作以 `Superseded` 取消；
- 所有动作终止路径（完成、取消、失败）均同步释放物理运动学锁定；
- 封盖事实发布同步触发 `fail_action_window(shooter_id, ActionFailureReason::Blocked)`；
- `lifecycle.rs` 17 项回归测试全部通过。R3 验收闭合。

### R4 防守责任图

已完成防守责任图与责任交接事实的定义与接入：
- `DefenseResponsibility` 包含 `PrimaryMatchup`、`Hedge`、`FightThrough`、`GoUnder`、`SwitchedMatchup`、`Help`、`Drop`、`Rotate`、`Recover` 九种主责任，保证每个防守人在任一 tick 有且仅有一个主责任；
- `GameEvent` 增加 `DefenseResponsibilityChanged` 与 `DefenseBreakdown`；
- `TargetAssignment` 扩展 `responsibility: Option<DefenseResponsibility>` 与 `offensive_player_id: Option<String>`；
- `ScreenDefenseRules` 引入 `OnBallScreenDefenseStrategy`（`StandardContest`、`FightThrough`、`GoUnder`、`Switch`）；
- `TacticalPlanner::plan_half_court` 生成对应的挡拆与弱侧责任标记；
- `MatchEngine` 在 `RuntimeObservations` 中追踪防守责任并在状态迁移时发布 `DefenseResponsibilityChanged`；
- 方案分化与基线恢复：`def_switch_heavy` 产生 `switched_matchup`，`def_drop_coverage` 产生 `drop`，`def_hedge_recover` 产生 `hedge`，`def_man_conservative` 保持基线；
- `defense.rs`、`defense_responsibility_chain.rs`、`defense_effect.rs`、`defense_rotation_response.rs` 全部通过。R4 验收闭合。

### R5 规则与犯规账本

犯规账本已按事件顺序重建：`check_foul_conservation` 从事件流独立重建个人累计、每节球队累计、犯满离场、bonus 罚则与逐次罚球对应关系。个人累计逐犯规对平 `personal_foul_count`；计入球队账的犯规按节累计，节关闭时必须等于帧计数器峰值，单犯规帧的 `period_team_foul_count` 必须能在帧计数器上观测到；`penalty.is_bonus` 必须等于球队累计达到阈值；每笔判罚的罚球必须以该犯规为因果父逐次执行，罚球人必须是被侵犯人，新的带罚球犯规或回合总结出现前上一判罚的罚球必须完成；犯满球员必须被换下、不得再犯规、不得再上场。限值（犯满上限、bonus 阈值）取自帧内 `FrameRules`（紧凑流首条携带、向前继承），检查器不自带副本。

违规程序：攻方三秒（限制区连续停留计时 + 责任人归因，冲筐类槽位行为改为前场时钟驱动的进-出曲线）与回场（前场建立判定 + 事前传球阻断 + 发球例外）已实现并有定向回归。违例责任人写入回合总结，满足 `TURNOVER_ACTOR_CONSISTENCY` 硬门。

逐次罚球：当前罚球程序进行中时，新判罚进入队列按序执行，不覆写进行中的程序（此前实现直接覆写，先判罚的罚球被静默丢弃，seed 1 实测两类账本不平衡）；命中的末罚在队列清空前不触发对方发球。犯满离场被持球状态阻塞时登记待换下，每 tick 重试直到执行。

联赛档案核验：NBA 与 FIBA 都从单节第 5 次球队犯规起进入罚则（charter §6.3），`LeagueProfile::fiba()` 的 bonus 阈值从 4 修正为 5。

剩余缺口：走步、二次运球、干扰球、贴身五秒、背筐五秒没有程序——前两者需要运球/合球阶段事实，干扰球需要飞行中球-人接触物理事实，物理层目前不产生这些事实；发球期间无球队控制的回场例外已实现，松球期间的前场建立沿用控球谓词。加时犯规累计（NBA 加时独立限额、FIBA 计入第 4 节）未按档案区分。

入口：`crates/evaluator/src/ledger.rs`、`crates/decision/src/constraint/`、`crates/engine/src/match_engine/phases.rs`、`events.rs`、`runtime_phase.rs`、`ball_flight/mod.rs`、`crates/engine/tests/violation_programs.rs`、`foul_programs.rs`、`free_throw.rs`、`league_profile.rs`。

### R6 倾向与个体评判

`PlayerTendencies` 已有 12 项字段，四项新增防守倾向已加入主客队档案；每项倾向均有行为消费点。`crates/engine/tests/tendency_perturbation.rs` 提供原先八项无行为消费倾向的单调响应、断路负面对照，以及逐项修改倾向后运行四个固定种子的真实比赛差异证明。`PlayerTendencies::validate` 同时检查全部 12 项数值范围。

`crates/evaluator/src/individual.rs` 已实现四条个体准则：球队内使用率按投篮出手、0.44 倍罚球出手和失误归属聚合；助攻父链只沿 `PASS_RECEIVED` 可重建；防守责任要求有离开主对位的责任事实且在同一回合恢复；第四节末段体能只读取携带球员投影的完整帧。使用率归属或流事件载荷缺失、助攻流缺少接球事实、责任迁移事实不足、或流中没有体能投影时均输出 `InsufficientEvidence`。评估测试（21 单元、26 集成）通过。

完整 16 种子 `stats_baseline` 复测通过：总分中位数 204.5，回合数均值 246.9，回合时长均值 14.40 秒，三分命中率中位数 38.5%，篮下出手份额中位数 0.333 且 16/16 入带，中投份额中位数 0.153；四区 `SHOT_PROFILE_ZONE_MIX`、账本及 Hard 门均通过。`DecisionRules.midrange_utility_bonus` 接入统一 `CourtGeometry::shot_zone` 的 Mid 区（基准 0.18，守住中距离占比逐种子下限 0.08），并由 `midrange_utility_bonus_changes_real_match_shot_distribution` 验证规则扰动会改变多种子真实比赛；`three_point_utility_multiplier` 为 0.65，`drive_rim_attack_bias` 为 1.3（守住篮下占比逐种子下限 0.25），`post_up_base` 为 3.6（背身仅在错位时胜出，保证背身与卡位系系数可触达真实行为）。本轮后随 R8 球位连续性与哨响连续性修复及决策重校准，seed42 × 2000 黄金哈希重冻结为 `0xca1d1cae20e46d0e`，由 `golden_hash` 定向测试核验。

入口：`crates/domain/src/data.rs`、`crates/engine/tests/tendency_perturbation.rs`、`crates/evaluator/src/individual.rs`、`crates/evaluator/fixtures/nba.v2.json`、`crates/engine/tests/stats_baseline.rs`。

### R7 有来源的参考分布

`nba.v3` 档案登记两项公开来源准则：四区投篮命中率分布、第四节最后 300 秒与此前 420 秒的三分出手份额差。来源为 2023-24 NBA 常规赛 ShotChartDetail，共 1,230 场、218,701 次出手；版本、归档哈希、统计窗口、生产分区几何、逐场 5–95 百分位方法和最小样本均记录于 fixture。研究归档固定为 `ismayc/shot-quality-study` commit `f4001f944d4aba1ba63e3d70edb3d6135ba576fe` 下的 `data/shotdetail_2023.tar.xz`，SHA-256 `303e71f967c199568b345cd73a75e595f1f99532cca3e6ce59afb286b63f840d`。日期范围为 2023-10-24 至 2024-04-14，和提交内 README 对该文件的 2023-24 赛季说明一致。NBA.com 公布 2023-24 3PA/FGA 为 39.5%；来源 SHOT_TYPE 测得 39.4854%，按坐标几何计算的三分率为 39.4845%。两种逐次分类有 22 次差异；区域准则保留两种来源标记的三分出手，独立来源核验分别报告两种比例。ShotChartDetail 坐标单位为十分之一英尺，原点位于进攻篮筐，`LOC_X` 为横向、`LOC_Y` 从底线向前场增大；坐标映射为 `x = 88.75 - LOC_Y/10`、`y = 25 + LOC_X/10` 后按生产 CourtGeometry `shot_zone` 阈值复算四区参考带。当前四区边界对左右镜像对称；同时校验坐标半径向下取整与每条 `SHOT_DISTANCE` 一致。Q4 晚段/早段最小样本分别为 9/15，每场符合评判样本要求。来源核验脚本检查归档哈希、数量、重复事件、缺失字段、NBA.com 基准、生产几何分区和全部参考分位数。评判缺少完整出手、结果父链或 Q4 时钟时输出 `InsufficientEvidence`。

本次修改后的 R7 定向评判 5 项、评估器 21 项单元测试与 26 项既有集成测试、Clippy、格式、文档守卫、内联常数守卫、差异检查和来源复算均通过。仍需独立审阅参考带和归档导入管线，并完成 ADR-009。`nba.v1`、`nba.v2` 与 FIBA 兼容性保持，`for_league("NBA")` 仍选 v2；ADR-009 接受前不登记真实度目标线。此归档是公开样本，不能单独确认赛事全集，也未覆盖盲区清单中的球队内部与其他情境关系。

入口：`crates/evaluator/src/fixture.rs`、`crates/evaluator/src/joint_situational.rs`、`crates/evaluator/fixtures/nba.v3.json`、`crates/evaluator/tests/joint_situational.rs`、`scripts/check_r7_reference.py`、`crates/evaluator/fixtures/blind_spots.md`.

### R8 球态与阶段权限

将球归属与飞行参数分离为权威球态和不可变飞行载荷；逐步移除物理层重复持球标记；按架构顺序调度阶段并收窄阶段写入权限。Hard 违规必须产出结构化工件并由调用方失败退出。

当前已实施并验证：

- 物理层删除 `PlayerPhysicsState.has_ball`、`SpatialPhysics::set_ball_holder` 与两个后端的旗标同步；`make_motion_proposals` 接收 `ball_holder_id` 显式输入，APF 队友排斥按该输入判定；主循环与三个短路路径的物理步进统一经 `step_with_ball_holder(dt, ball.holder_id())`。
- 约束层 `ConstraintContext` 增加 `ball_holder_id` 派生字段，出界判定改读权威球态持有人，不再依赖物理层缓存旗标；语义层 `SemanticEvaluator::contact`/`event_facts` 同样接收持球人参数；换人与轮换的持球拦截改为 `ball_holder_id()` 判定。
- 球态写入收敛：`BallRuntime.ball_state` 仅经 `set_ball_state` 写入，生产路径唯一调用点是 `transition_ball_state`（领域层转换表校验）；`scripts/check_ball_state_writes.py` 守卫断言赋值、字面量初始化与 setter 调用只出现在 `state.rs`/`write_entry.rs`/`test_hooks.rs`，含 `--self-test` 负对照，已接入 `run-tests.sh` 静态守卫门与 CI guards matrix。
- 阶段重排：`step_inner` 按 `architecture.md` §4.1 调度（时钟推进先行；跳球/死球流程为条件激活分支，位于罚球与节末之后）；节间计时移入 `clock_advance_phase` 的停表分支，`dead_flow_phase` 只负责转场判定；`was_tip_off` 守卫保持跳球期时间语义不变。
- 阶段写权限：每个阶段函数在文档注释中以「写入状态组：…」显式声明所写状态组，调度器内联段同格式标注；`&mut self` 形态按 ADR-014 accepted 裁定保留，状态组结构由既有守卫维护。
- 短路与哨兵校验：`step()` 包装器对每个输出帧执行 `check_tick`；流导出 `written_bytes == 0` 的终局哨兵帧改经 `step()` 走同一校验；新增 `invariant_checker_runs_on_every_step_including_short_circuits` 回归断言检查器计数与 step 数严格相等（覆盖跳球表现阶段与节间休息）。
- Hard 失败链路维持既有结构化工件：`ExportSummary.violations` + `ViolationTaxonomy`、CLI 侧 `.violations.ndjson` 账本与退出码 1；逐 tick 结构化结果经 `last_tick_violations()` 读取。

验证：nba-physics（5+15+1+8）、nba-domain（24+3+7+23+2+6）、nba-decision/semantics/officiating/invariants 全绿；engine 聚焦 `ball_state` 12、`pass_speed_continuity` 5、`free_throw` 11、`decision_wiring` 3、`lifecycle` 17、`invariants` 6、`violation_programs` 5、`foul_programs` 2、`drive_geometry` 6 全部通过；`golden_hash` 6/6（球位连续性与哨响连续性修复及决策重校准后重冻结为 `0xca1d1cae20e46d0e`，阶段重排与球态通道化本身逐位不变）；16 种子 `stats_baseline` 全部通过（总分/回合/三分/篮下占比带、五式账本、Hard 门与四路径覆盖断言）；workspace clippy `-D warnings`、`cargo fmt`、状态组守卫、球态写入口守卫、文档/行数/常数守卫与 `git diff --check` 通过。

入口：`crates/domain/src/flow.rs`、`crates/physics/src/movement/{mod,kinematics}.rs`、`crates/engine/src/match_engine/{mod,phases,runtime_phase,flow,roster,bookkeeping,state}.rs`、`crates/engine/src/match_engine/ball_flight/write_entry.rs`、`scripts/check_ball_state_writes.py`、`crates/engine/tests/invariants.rs`。

### R9 战术三层

进攻路径只读取 `TacticalSetSpec`；槽位填充失败返回明确错误；防守统一为 `DefensiveSystem`，初始对位由确定性的 `assign_matchups` 生成；Play 步长、轮换、暂停、比赛情境及教练档案均有生产消费点和扰动测试。

入口：`crates/domain/src/tactics.rs`、`crates/decision/src/tactics.rs`、`crates/engine/src/match_engine/tactics_phase.rs`、战术与防守集成测试。

### R10 球员输入

补充 `wingspan_cm`、`effective_reach`、`release_height` 和独立能力映射；每项提供单调响应与断路测试。明确区分引擎读取的球员资料和仅供界面展示的角色投影。

入口：`crates/domain/src/data.rs`、`capability.rs`、`crates/engine/tests/attribute_perturbation.rs`、规则完整接线测试。

### R11 评判闭环

待 R3、R4、R5 的事件语义稳定后，增加动作完整、防守责任完整、罚则完整、跳球及交替拥有准则。缺失终态、责任交接或罚则失衡必须生成带事件来源的 Defect，并进入归因报告和 Hard 门。

入口：`crates/evaluator/src/lib.rs`、`ledger.rs`、`composition.rs`、`crates/domain/src/event.rs`。

## 文档同步规则

代码变更、针对性测试、配置/参考数据与开发文档必须同一任务更新。`docs/dev/current/spec_gap.md` 的历史覆盖数字只代表它原本声明的有限范围；完成新的 gap 核验后才更新计数与状态。`docs/dev/status.md` 是当前状态文件；需同步维护 `docs/dev/current/plan.md`、本文件、`spec_gap.md`、`docs/README.md` 和 `docs/dev/README.md` 中的链接与任务状态。
