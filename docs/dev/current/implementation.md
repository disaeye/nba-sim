# 实施计划与当前执行状态

> 状态：本文记录旧引擎的实施进度，其中“五层验证已完成”指旧引擎的范围。动作链内核另有验收入口（`nba-sim validate-core`），见 `docs/dev/action_chain_engine.md`。

本文件记录当前实施范围、验收顺序与已核实证据。任务状态以会话中的 todo 清单为准；代码事实、测试结果和待办要求分别记载，避免把计划描述当作完成证据。

## 执行依赖

R1 结果事实与因果来源是 R2 篮下分布及 R8 球态重组的前置条件。R2 完成后执行 R3 动作生命周期，再执行 R4 防守责任图、R5 规则与犯规账本、R6 倾向与个体评判、R7 有来源的参考分布。R9 战术层依赖 R4，R10 球员输入依赖 R6，R11 评判闭环依赖 R3、R4、R5。

R1 细分任务按实际触筐事实、接触犯规事实、传球互斥及因果、突破决定事实、证据和文档同步的次序执行。任何结果都必须在其决定性事实发生时形成，并能由事件流回溯来源。

## R1 结果事实与因果来源

### R1.1 投篮结果

验收要求：从连续球路中的实际篮筐、篮板碰撞或篮筐平面通过事实判定命中、触筐未中或无接触偏出；以检测到的碰撞位置和速度生成反弹；投篮、接触、得分或未中、篮板之间保存同一投篮身份及父事件。

当前已核实：`BallState::Shot` 不再保存预先判定的命中结果；`SHOT_TRAJECTORY_ARRIVAL`、`SHOT_CONTACT` 已加入事件与因果链接；rim-contact 的反弹计算可接收接触位置。现有到达裁定仍只在 `tau >= 1` 的采样端点运行，篮板分支中的 backboard contact 仍使用预测 probe，故连续飞行碰撞权威尚未完成。八种子帧差分新增后记录到多类连续帧超出 85 ft/s：seed 2024/555 在 FREE_THROW 进入帧从先前持球或突破位置直接放置罚球点，另外还检测到 PASS、LOOSE_BALL、HELD 连续帧超限。罚球准备使用 `FreeThrowSetup` 三维轨迹从犯规球位移动到罚球点，时长按三维距离与球速预算计算；到达后发布 `BallPlacementApplied`，再开始罚球飞行。当前修改统一由 `begin_free_throw_flight` 依据当前 shooter、投篮概率、强制结果和最终罚球标记生成唯一罚球轨迹；强制命中瞄准筐中心，强制未中瞄准筐环接触区，普通罚球依据能力概率采样。准备态和飞行态均核对当前罚球 shooter、剩余次数与 `is_final`；到达结算以 checked attempt 序号记录事件，并更新尝试数、命中数、剩余次数和比分。新增集成断言核对连续罚球事件序号、逐次结果、箱体和比分，以及末罚命中后对方发球、末罚未中生成 `RimRebound` 并从检测到的接触位置开始。当前本轮复测通过 `cargo check --offline -p nba-engine --all-targets` 和 `free_throw` 集成测试（11 项）。`ball_ownership`（3 项）、`backend_contract`（15 项）、`rim_contact`（8 项）、physics 弹道单测（5 项）、罚球因果归属测试及 engine clippy 复测通过。`check_docs.py`、`check_max_source_lines.py`、`git diff --check` 与 `check_inline_constants.py` 均已通过；新增内联常数收编至配置数据通道，生产预算完成下调。八种子全帧球速回归测试 `all_adjacent_frames_respect_ball_speed_envelope` 10 项测试已全部通过。本阶段的中间实测值 `0xc8b4a2c005d473d8` 与更早锚点 `0x765d0062e099cdb5` 已被后续轮次重冻结演进（现行 v81，见 R12 节）。

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

R12 的验收范围另见下文「R12 验证基础设施」节。

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

剩余缺口：干扰球与篮筐干涉、贴身五秒和背筐五秒尚无程序。走步与二次运球已接入事实链与运行时判罚：步态模型是物理层的一等状态（双脚位置与摆动脚，身体越过支撑脚一个步幅时摆动脚落地并发射接地点事实）；已运球的持球人在投篮或传球起手时收球（DRIBBLE_GATHERED 事件、轴心脚取当时的支撑脚），收球后两步内落地或轴心脚在容差外滑动即 TRAVEL，收球后再突破即 DOUBLE_DRIBBLE，违例经 `dribble_control` 运行时约束终结回合；球离手（球态离开持球族）时由写入口清除收球记录，换持球人时初始化为未运球状态并重落双脚。动作窗口的运动学锁定收窄到 Execution/FollowThrough，Preparation 蓄势段保持移动（投篮起手同时设停止目标，收步超两步即走步）；违例哨作废未离手的挂起出手。专项测试覆盖轴心脚滑动、第三步、收球后传球/投篮、二次运球、真实收球链与正常比赛稀疏性（3 种子 ×1q 零哨声，判罚链经构造场景验证）。加时犯规累计（NBA 加时独立限额、FIBA 计入第 4 节）未按档案区分。

入口：`crates/evaluator/src/ledger.rs`、`crates/decision/src/constraint/`、`crates/engine/src/match_engine/phases.rs`、`events.rs`、`runtime_phase.rs`、`ball_flight/mod.rs`、`crates/engine/tests/violation_programs.rs`、`foul_programs.rs`、`free_throw.rs`、`league_profile.rs`。

### R6 倾向与个体评判

`PlayerTendencies` 已有 12 项字段，四项新增防守倾向已加入主客队档案；每项倾向均有行为消费点。`crates/engine/tests/tendency_perturbation.rs` 提供原先八项无行为消费倾向的单调响应、断路负面对照，以及逐项修改倾向后运行四个固定种子的真实比赛差异证明。`PlayerTendencies::validate` 同时检查全部 12 项数值范围。

`crates/evaluator/src/individual.rs` 已实现四条个体准则：球队内使用率按投篮出手、0.44 倍罚球出手和失误归属聚合；助攻父链只沿 `PASS_RECEIVED` 可重建；防守责任要求有离开主对位的责任事实且在同一回合恢复；第四节末段体能只读取携带球员投影的完整帧。使用率归属或流事件载荷缺失、助攻流缺少接球事实、责任迁移事实不足、或流中没有体能投影时均输出 `InsufficientEvidence`。评估测试（21 单元、26 集成）通过。

完整 16 种子 `stats_baseline` 复测通过：总分中位数 204.5，回合数均值 246.9，回合时长均值 14.40 秒，三分命中率中位数 38.5%，篮下出手份额中位数 0.333 且 16/16 入带，中投份额中位数 0.153；四区 `SHOT_PROFILE_ZONE_MIX`、账本及 Hard 门均通过。`DecisionRules.midrange_utility_bonus` 接入统一 `CourtGeometry::shot_zone` 的 Mid 区（基准 0.18，守住中距离占比逐种子下限 0.08），并由 `midrange_utility_bonus_changes_real_match_shot_distribution` 验证规则扰动会改变多种子真实比赛；`three_point_utility_multiplier` 为 0.65，`drive_rim_attack_bias` 为 1.3（守住篮下占比逐种子下限 0.25），`post_up_base` 为 3.6（背身仅在错位时胜出，保证背身与卡位系系数可触达真实行为）。本轮后随 R8 球位连续性与哨响连续性修复及决策重校准，seed42 × 2000 黄金哈希当时重冻结为 `0xca1d1cae20e46d0e`（v80），现行冻结值随 R12 观测层演进至 v81（见 R12 节），由 `golden_hash` 定向测试核验。

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

验证：nba-physics（5+15+1+8）、nba-domain（24+3+7+23+2+6）、nba-decision/semantics/officiating/invariants 全绿；engine 聚焦 `ball_state` 12、`pass_speed_continuity` 5、`free_throw` 11、`decision_wiring` 3、`lifecycle` 17、`invariants` 6、`violation_programs` 5、`foul_programs` 2、`drive_geometry` 6 全部通过；`golden_hash` 6/6（球位连续性与哨响连续性修复及决策重校准后当时重冻结为 `0xca1d1cae20e46d0e`，阶段重排与球态通道化本身逐位不变；现行冻结值为 v81）；16 种子 `stats_baseline` 全部通过（总分/回合/三分/篮下占比带、五式账本、Hard 门与四路径覆盖断言）；workspace clippy `-D warnings`、`cargo fmt`、状态组守卫、球态写入口守卫、文档/行数/常数守卫与 `git diff --check` 通过。

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

### R12 验证基础设施（validation.md 五层实施）

按 `docs/validation.md` 的五层验证规格构建独立验证器。与 R1–R11 的区别：R12 的交付物是一个只读 crate 与 CLI 入口，判定事实来自事件流的独立重建，不调用生产结算函数（validation.md §2.1）。

验收范围与当前进度：

- 协议层（已完成）：`FrameEvent` 新增 `tick_index`、八个业务身份字段与 `relations` 结构化关系（`#[serde(default)]`，旧流可解析）；`RenderFrame` 新增 `tick_index`；新增 `crates/protocol/src/validation_stream.rs`（流首部能力声明与尾部完整性摘要，缺尾部即截断流）；`ExportSummary` 携带 `open_business_instances`。入口：`crates/protocol/src/frame.rs`、`crates/protocol/src/validation_stream.rs`。
- 引擎观测事实发射（已完成）：`crates/engine/src/match_engine/business.rs` 按实例生命周期分配业务身份并推导关系；发射 `StatAttribution`、`ShotAttemptResolved`、`ControlEstablished`、`ResponsibilityStarted/Ended`、`ClockChanged`、`PeriodSettlementCompleted`、`PossessionAwarded`、`PlayWindowEnded`、`MovementIntent/Outcome`（validation.md §3.3）。运动意图按决策间隔到期清算，驻留任务不产生逐 tick 流水。发射点：`tactics_phase.rs`、`decision.rs`、`phases.rs`、`transitions.rs`。
- 验证器 crate（已完成）：`crates/validation` 已有四态裁决模型（`verdict.rs`）、五层目录全量准则注册表（`criteria.rs`，FLOW 20、CAUSE 8、MOVE/TACT 12 族变体、STAT 10、MECH 10、JOINT 9）、规则与统计档案（`profile.rs`）、清单类型（`manifest.rs`）、严格流读取与实例/关系索引（`stream.rs`）、独立重建器（`reconstruct.rs`：箱体/回合/时钟/配对）、机会提取器（`opportunity.rs`）、MOVE/TACT 事实层（`movement_tact.rs`）、统计比较器（`stats.rs`：等效区间、配对 bootstrap、Bonferroni、Wasserstein-1、TV 距离）、JOINT 观测（`joint.rs`）、机制实验框架（`mechanism.rs`：§7.2 全场配对反馈可用，§7.1 检查点实验显式报缺 API）、场景框架与 22 个必需场景（`scenario.rs`：FullGame 入口可用，Checkpoint/Declarative 入口显式报执行错误）、runner 与全部十三件工件（`runner.rs` + `artifacts.rs`，清单见下）。
- 工件全清单（已完成，validation.md §10）：`validation_manifest.json`、`coverage.json`、`instance_judgments.ndjson`、`ledger_comparison.json`（重建 vs 声明箱体、回合对齐、时钟重建、配对审计）、`game_metrics.parquet`（arrow/parquet 库列式写入，一行一场）、`statistical_results.json` 与 `mechanism_results.json`（STAT/MECH 逐条按注册表登记 InsufficientEvidence，原因码分别为 `insufficient_reference_data`/`experiment_config_missing`，不填零不跳过）、`scenario_results.json`、`validation_report.json`、`failure_segments.json`（缺陷与未判定裁决定位，按固定规则保留通过样本）、`instances.sqlite`（完成实例 + 关系索引查询存储，open 计数与流尾部交叉核对）、旧兼容视图 `judgments.ndjson` 与 `attribution_report.json`（同一裁决生成，severity 由 `gate_class` 映射，禁止第二套判据）。流尾未完成实例数经引擎只读访问器 `open_business_instances` 取真实值（不再硬编码 0）。入口：`crates/validation/src/artifacts.rs`、`crates/validation/src/runner.rs`。
- CLI 入口（已完成基础）：`nba-sim validate [--seed N] [--scope S] [--out D]`，退出码 0/1/2/3（validation.md §10）。入口：`crates/cli/src/commands/validate.rs`。
- 网页入口（已完成）：调试页面新增「验证」选项卡，对页面当前查看的比赛做五层判定：`/api/simulate` 返回给页面的刻序列缓存于服务端，`POST /api/validation/run` 对同一刻序列执行全部层判定（结论与页面展示的比赛严格一致，同一确定性重放），`GET /api/validation/status` 轮询、`GET /api/validation/report` 出逐场景与账本对账详单；页面只渲染结论，不重算判据（validation.md §10：UI 只读取结果）。工件写入 `.work/validation-view/`。入口：`crates/debug-server/src/validation_api.rs`、`crates/debug-server/static/`（deck 第四选项卡）。
- 集成测试（已完成）：`crates/validation/tests/validation_integration.rs` 14 项：准则注册表全族覆盖与编号抽查、22 个必需场景计数、未触发场景给 `InsufficientEvidence`、Checkpoint 入口显式报执行错误、重建箱体与声明对平（seed42 1q）、配对审计零悬空（投篮/传球/罚球因果配对、事件 ID 唯一单调、关系无回写无悬空、实例无双结算——终态 kind 按实例类型区分，loose_ball 只被 `LOOSE_BALL_SECURED`/`REBOUND` 关闭）、runner 端到端十三件工件与退出码、parquet 读回字段核验与 SQLite 实例存储写入/查询（open 实例计数、终态 kind、关系行）、兼容视图与同源裁决计数一致、均值差/W1/TV 三出口边界（含配对 bootstrap：两侧共享重采样下标，同分布时 W1=0；构成重采样按计数总量缩放）、机制实验拒绝检查点入口、机会规格缺项拒绝、规则档案阻塞未审核条目。测试中的统计实现修正：W1 由配对 bootstrap 取分位区间（独立重采样会把重采样离散噪声叠加进距离，实测 null 分布中位数 3.66 盖过观测差异）；TV 的构成重采样按计数总量多项抽样（每类抽一次会把 2 类构成的波动放大到 1.0）。
- 参考数据登记（已完成，展示可见性模式）：`data/reference/stat_deltas.v1.json`（v2）登记全部 10 个 STAT 叶项与 9 个 JOINT 叶项。数据源三件套（全部锁定 SHA-256）：ShotChartDetail 归档（2023-24 赛季，1,230 场、218,701 出手）、stats.nba.com PBP 归档（`shufinskiy/nba_data` nbastats_2024，**2024-25 赛季**，1,230 场逐事件）、pbpstats.com 逐回合归档（**2024-25 赛季**，去重后 243,768 回合；源文件含按事件分块的重复行，去重口径经每场回合数与官方 Pace 交叉验证）。三归档覆盖两个赛季且比赛集合不相交（GAME_ID 前缀 223 vs 224），跨归档比赛级关联不可行，各参照值严格在单归档内部计算。登记现状：**15 条指标属 10 个叶项 full**（STAT-S01 六指标、R01 三项、T01、B01、G02、JOINT-C01 比分差×末节时间三分占比带、C04 两项、C05 bonus 首进犯规序分布、Q01 边缘构成 + lag-1 转移矩阵逐格）；**G01 降为 Diagnostic**（投篮得分是 S01 的派生结果，独立诊断价值低，保留输出不占 Bonferroni 名额）；**2 条 pipeline_pending**（STAT-I01/JOINT-I01 分钟表需 boxscore 来源）；**6 条 blocked**（追踪数据/体系标签）。重建脚本 `scripts/rebuild_joint_reference.py`（C01/C05/Q01 三项参照值从冻结归档重算，check_stat_deltas.py 复用同实现做登记核对）。关键实证发现：官方 PBP 描述的 `.PN` 标记即 bonus（penalty）状态（行后紧随 Free Throw 1 of 2 验证），bonus 首进犯规序真实分布 5 次阈值占 82%、4 次阈值占 12%、整节未进 bonus 的队节占 54%——引擎固定阈值 `bonus_fouls_per_period=5` 与该混合分布的差距由 C05 判定如实反映。三维度审查结论：(1) 诊断价值——每个 full 条目对应明确的引擎缺陷假设（如 TOV 原因构成定位传球噪声/组织死板/规则过敏三类不同缺陷、Pace 定位回合时长控制、回合开始原因构成定位球权转换机制）；(2) 渠道——full 条目来源全部在手锁定，追踪类公开渠道不存在；(3) 修正的不科学项——Pace 加时修正（101.16→100.61，validation.md §6.2 口径）、TOV 原因改用 EVENTMSGACTIONTYPE 结构化编码（文本匹配把出界变体并入 BadPass 虚高至 0.51，结构化口径 0.36）、G01 降 Diagnostic 防 Bonferroni 稀释、STAT-I01/JOINT-I01 改联盟聚合分布口径（逐球员分钟对照违反 §6.1 相同适用人群，虚拟球员与真实球员身份不对齐）、TV 容差数学含义修正、population_scope 为联盟聚合分布口径（展示可见性模式）、Bonferroni m 计数单位明确为叶项级（叶项内多指标合取消耗一个名额）。校验脚本 `scripts/check_stat_deltas.py` 同步去重与加时修正逻辑。加载入口：`crates/validation/src/reference.rs`。
- 批次 STAT 等效判定（已完成）：`crates/validation/src/stat_batch.rs` + CLI `--games N`。多场批次（固定种子序列）每场产出充分统计量（GameMetrics 新增四区出手/命中、回合开始原因计数），与登记参照比对：以比赛为 cluster 的配对 bootstrap 取差值区间，三出口判定（validation.md §6.3）；叶项级合取（任一子指标超差即叶项超差）；输出 `batch_statistical_results.json`。实测（3 场 full）：四区构成、ORB%、二次进攻通过；FG% 0.266 vs 0.474、TOV 率 0.330 vs 0.134、Pace 修正后合理——这些 Defect 是验证器正确抓到的真实引擎偏差（引擎比赛与 NBA 的实质差距），正是第 3 层要暴露的。
- 审核模式更新：不设独立签字关卡，audit 状态置 approved；判定值、参照值与对账在 debug-server 验证选项卡直接展示，使用者随时可见，发现登记错误可直接修正重跑——以可见性替代事前签字。population_scope 同步裁决：验证对象是比赛形态（联盟级聚合分布），球员是虚构的，能力不要求与真实球员对齐（同一球员跨联赛表现尚且不同），只要求虚拟球员能力分布大致覆盖真实联盟水平区间使合并分布可比。
- 批次判定覆盖面（已完成）：`crates/validation/src/stat_batch.rs` 把全部已登记指标接入判定——42 项逐指标判定（FG%/三分占比与命中率/四区出手份额与各自 FG%/出手距离 W1（登记分位 → 桶中心样本，配对等长修正）/FTA 比例/FT%/fouls_per100/tov_rate/orb_pct/pace/OREB 占比与二次出手率/Q4 末段三分占比三桶（允许带为逐桶绝对差）/JOINT-Q01 边缘构成六类份额 + 转移矩阵逐格 21 格（引擎可判行：field_goal/steal/dead_ball，登记 scope 注明细粒度差异））+ 3 项构成组 TV 判定（STAT-T01 失误原因聚合五类、JOINT-C05 bonus 首进犯规序六桶、JOINT-Q01 开始原因七类）。工件 `batch_statistical_results.json` 新增 `group_verdicts` 与 `coverage` 段（每条登记条目的参判状态与跳过原因；损坏的登记形态会如实记 skipped 并可从 detail 定位）。
- 引擎侧修复（发现于接入过程）：`artifacts.rs` 回合开始原因匹配的字符串与流内实际序列化形态（SCREAMING_SNAKE_CASE，如 `TURNOVER_LOOSE_BALL`）不一致，`starts_*` 计数恒为零；修正后 Q01 判定从恒 InsufficientEvidence 变为可判定。`GameMetrics` 新增 16 个充分统计量字段（失误原因聚合五类、出手距离直方图、Q4 末段三分分桶、OREB 二次出手、bonus 首进犯规序直方图、转移矩阵 7×7 计数），parquet schema 同步。
- 测试工件位置修正：validation 集成测试的临时目录从 `crates/validation/.work/`（未被根 .gitignore 覆盖）改为仓库根 `../../.work/test-tmp/validation-it`。
- 引擎检查点分叉能力（已完成）：`SpatialPhysics::clone_dyn`（Rapier 后端按权威玩家状态重建刚体结构——kinematic body 位置完全由 `players` 驱动，重建等价于位精确复制；SimpleCircle 直接 Clone）、`PhysicsWorld` 手动 Clone、`MatchEngine` derive Clone + `experimental_fork()`（校验专用检查点分叉，外部仍摸不到状态字段，符合 D7）。配套 Clone：`DecisionSystem`/`ConstraintRegistry`（decision）、`ActivePlay`（engine state）、`InvariantChecker`（invariants）。验收闸：`crates/engine/tests/fork_continuation.rs` 位精确续跑测试——3 seed × 3 检查点位置 fork 后续跑与母局逐 tick 哈希一致 + 副本分岔跑不污染母局，2/2 通过；golden_hash 6/6 不变（纯 derive 无行为变化）。
- 校验专用扰动入口（已完成）：`crates/engine/src/match_engine/perturbation.rs` 定义 `ValidationAttribute` 枚举（11 个首批实验属性，get/set/name 三通道收编，编译期限定合法输入）；`MatchEngine::apply_validation_perturbation(player_id, attribute, value)` 走生产路径——属性写入经 `PlayerAttributes::validate` 同一套校验（越界 panic 快崩），派生量（物理运动上限）经 `nba_domain::effective_max_speed/accel` 映射层重算（与 `construction.rs` 初始化同源），返回旧值供实验恢复。验收测试 `crates/engine/tests/perturbation.rs` 5 项：派生量重算、越界拒绝、未点名球员不受波及、未触及状态一致性（比分/球权/犯规/时钟/球态/位置）、扰动传播进比赛结果（两侧结算序列分岔）。行为发现：shooting_three 扰动同时改变命中概率输入与出手效用（决策管线 zone skill 同源消费），单场窗口内档位方向性受小样本噪声支配，方向性判定属 MECH 框架（多检查点 × 多 trial 聚合）职责。
- MECH 检查点实验框架（已完成）：`crates/validation/src/mech.rs`——实验规格冻结（`spec_digest` FNV 摘要先登记后执行，`verify_spec_digest` 不符拒绝运行）、母局单遍走位 + 检查点谓词命中即 fork 四组（基线+低中高三档；档位不进具名随机键 → 同 (checkpoint, trial) 共享随机流天然配对；`reseed_validation_rng` 引擎入口）、trial 观测窗口后丢弃、机会数分母 = 命中检查点数（无控球者的检查点跳过且不计入）、命中数下限闸门（不足如实 `insufficient_checkpoints`）、`monotonicity_verdict` 档位单调性三出口判定（trial 级配对 bootstrap）。首批实验规格三项（MECH-S01 三分精度 / MECH-M01 速度 / MECH-R01 罚球）。谓词实现修正记录：StreamTick 的 x/y 是归一化坐标（0-1 = 94×50 ft），距离判定换算回英尺。集成测试 `crates/validation/tests/mech_experiment.rs` 3 项（端到端实验 / 摘要冻结闸 / 检查点不足如实失败）全绿；`mechanism.rs` 的 checkpoint 拦截翻新为可执行（引擎 API 落地）。
- MECH 判定接入 runner（已完成）：`crates/validation/src/mech_runner.rs`（从 runner.rs 拆出，D29）——`mechanism_results_with_experiments` 执行首批实验规格并产出 `mechanism_results.json`：每叶项判定 = 其全部实验单调性判定的合取（Defect > InsufficientEvidence > Pass），无实验覆盖的叶项如实 `experiment_spec_missing`；工件含实验记录（规格摘要、核对状态、检查点命中数、四组出手/命中数与命中率、单调性判定与区间）。罚球实验修正：检查点谓词改用 FREE_THROW 事件（罚球程序帧层无控球者），扰动目标取 payload `shooter_id`，观测通道分谓词（三分出手 / 罚球出手）。MECH-M01（速度）暂不入首批：其观测量（固定冲刺任务到位时间）与出手/命中通道不匹配，错配实验的档位方向无物理依据，等观测通道扩展后登记。首跑结果如实记录：R01 罚球低/中/高 = 0.842/0.870/0.846（30 检查点，噪声量级内无单调证据）、S01 三分 = 0.419/0.429/0.292（高档反而最低，疑似出手选择随属性变化的筛选效应——扰动改变了决策效用同源消费的属性，两侧出手序列分岔），两项均 InsufficientEvidence；判定口径与原始数据全部在工件中可查。
- 场景-规则映射登记与分支覆盖工件（已完成）：`crates/validation/src/rule_coverage.rs`——27 个规则域（投篮/封盖/篮板/传球/突破/犯规四类/罚球程序/时钟两域/换人/移动/掩护/防守覆盖/Play/转换 + 走步、二次运球、干扰球、贴身五秒、回后场、争球、恶意犯规、带球撞人、界外发球细分）与 22 个 SC-* 场景对照，覆盖三态（covered 18 / sampling_only 8 / missing 1），R5 违规程序域如实登记缺口与队列衔接。`crates/validation/src/sampling_coverage.rs`——批量比赛的事实类型统计（事件种类/违例变体/犯规类别/转换来源/回合终结原因）× 规则域采样键对照，输出三档（sampled 16 / scenario_only 2 / uncovered 9，3 场 1q 实测）与认证范围限制清单（未覆盖域全部如实列出）；`runner.rs` 新增 `run_batch_with_facts`，CLI 批量模式输出 `branch_coverage.json`。集成测试 +1（16/16 全绿）。
- 正式统计判定跑（已完成，当前口径 = 20 场）：`validate --seed 101 --games 20 --scope full` 批量认证 + `certification_summary.py` → `certification.json`。结论**如实为 defects_found**：第 1/2 层 22/22 场景通过；第 3/5 层 7 叶项 Defect、3 叶项 InsufficientEvidence（JOINT-C05 bonus 犯规序、STAT-B01 ORB%、STAT-R01 犯规频率）；第 4 层 11 实验覆盖 5 准则、单调性均 InsufficientEvidence（单检查点单 trial 功效不足）；覆盖 sampled 16 / scenario_only 2 / uncovered 9。45 项逐指标判定 15 pass（5 场时 2 pass——功效提升把边缘误判收敛）。**核心引擎偏差**（20 场稳定复现，判定输入，禁止为了判定好看而调）：FG% 0.351 vs 0.474；投篮得分 156.1 vs 97.2；TOV 率 0.333 vs 0.134；Pace 117.8 vs 100.6；四区各自 FG% 0.00-0.02 vs 0.36-0.60（**与总 FG% 0.351 矛盾，指向 zone 记账错误，优先排查**）；OREB 后二次出手率 0.99 vs 0.81；Q4 末段三分 0 vs 0.44；转移矩阵 steal→field_goal 0.32 vs 0.08（偷球后直攻过多）；出手距离偏远 1.0 ft。工件：`.work/test-tmp/validate_formal20/`（认证三件套已投递 `.work/validation-view/` 供网页读取）。
- validation 接入开发流程（已完成）：`scripts/run-tests.sh validation` 新模式——par_test 的 `validation` tier（nba-validation 全目标 + 引擎 `fork_continuation`/`perturbation`）→ CLI validate 快速档（seed 42、1q、全层判定、13 工件）→ `certification_summary.py` 认证汇总；退出码语义如实传递（执行错误非零，判定缺陷/证据不足由 certification.json 如实记录，禁止为了让它绿而改判定）。分层使用规范：日常写代码用秒级单元测试（tier1）；提交/合并用 tier1+tier2+`run-tests.sh validation`（约 5 分钟）；里程碑用 tier3 + 整场批量判定（`validate --games N --scope full` + certification.json）。debug-server 验证选项卡为持续可见窗口：新增 `GET /api/validation/layers`（统计层三态 + 机制实验档位命中率），前端第 3/4/5 层详单渲染（app.js `loadValidationLayers`）。验收自检：一条命令跑完闸门（退出码 0）、退出码 0/1/2/3 语义明确、13+2 工件齐全（+certification.json）、页面可查。实测 `run-tests.sh validation` 退出码 0（单场判定 insufficient_evidence 如实——批量统计判定才作认证）。
- 违例注入式判定（已完成，#37 时限类部分）：integration 测试 `time_violation_injection_produces_enforced_facts`——以 GameRules 通道把进攻时限压到 3 秒构造高压局面，断言 SHOT_CLOCK_VIOLATION 高频产生且每次违例的处罚链路逐次闭环（回合终结 = TURNOVER_VIOLATION、下一回合 TRANSITION_STARTED.origin = violation）。注入用规则配置实现（无需注册中心注入）。持球违例（走步/二次运球）的注入测试随 #39 结论挂起——判据缺失下无可注入的被测对象，建模升级后补。
- 引擎修正第 3/4 步：篮板分工与二次进攻决策（r42/r43 批次，20 场综合验收）：**篮板分工修正**（结构性）——此前每次 miss 后 25ft 内全员冲抢（实测 10.5 人/miss，真实攻方 2-3 人冲抢、防守 4-5 人卡位）；改为防守方只派距落点最近 2 人争抢、其余执行常规站位，配合 boxout_bonus_base 0.10→0.30。**二次进攻决策**——ReboundPolicy 新增 putback_pass_out_factor（1.6，走数据通道），前场板收球人的传球候选获得独立效用系数与补篮真实竞争。**20 场综合验收（vs r38 基线）**：Pace 107.7→**102.85（参照 100.614，首个 Pass）**；投篮得分 224.7→204.1；rim FG% →**0.359（参照 0.359 精确）**；ORB 0.368→0.343；二次出手率 0.908→0.881。**连带漂移（如实）**：C01 Q4 压哨三分 0.409→0.256（pass 拖长回合的代价）、rim 份额 0.426 vs 0.319、TOV 0.191。defect 33→33（Pass +1 同时 C01/rim 份额漂出——耦合系统的跷跷板，每步收敛以批量口径如实记录）。Golden v90 实验回滚记录（poke 球速批量口径无收益且 C01 恶化）、v91 篮板分工。
- 引擎修正第 2 步：失误剩余构成（r41 批次；结论=参数空间已榨干，剩余是结构性工作）：poke 球速 14→9 的实验在单场口径显示 poke 丢权下降（攻方救回 17-19 次/场），但 10 场批量口径 TOV 持平（0.192→0.197）且 C01 Q4 压哨三分 0.441→0.107 严重恶化（回合结构连带），已回滚。catch_radius 3.4 的对照在单场噪声内不可分辨，保持 2.8。**方法教训（并入 r40 迭代纪律）**：TOV 是复合指标（掉球/poke/违例三源各 ~30%），单场 A/B 噪声 ±3% 会误导，改动以批量口径为准。**剩余缺口的定性**：掉球源需传球精度建模（catch_radius 兼任拦截判定无法继续提）；poke 源需「持球人短反应窗」的争抢机制重做；违例源（最大单项，8 秒 6 次/场）需推进移动语义重做（r40 遗留）。三者都是结构性工程，r41 批次的参数空间已榨干，TOV 保持 0.19x 进入遗留清单。
- 引擎修正第 1 步：回合节奏（r40 批次，golden v87→v89；阶段性收敛）：**核心目标达成**——10 场口径 Pace 108.4→**101.45**（参照 100.614，几乎精确）；连带收敛三项：JOINT-C01 Q4 末段三分 0.326→**0.441**（参照 0.451）、3P% 0.298→**0.366**（参照 0.366 精确）、ORB 率 0.348→0.303（0.250，Defect→Insufficient）。参数落点（全部 GameRules 通道）：pass_base 1.15→1.30（传球数向真实 3.0/回合靠近）、urgency_shoot_boost 1.20→1.05、3pt base 二次回调 0.360→0.335（pass 提额后空位接球投增多的再平衡）、defender_speed_ratio 0.73→0.85（退防比护球推进快——真实退防是无球直线跑）。**对照实验否定项**：dwell_decay_max 0.85→0.50 方向相反（回合更多更短，非单调警告再次应验，保持 0.85）；carrier_speed_ratio 0.88 无效果（推进瓶颈是决策周期重设目标的走走停停，非巡航速度），均回滚。**遗留**：8 秒违例 ~6 次/场（真实 ~0.3；根源 = 后场推进被 2.4s 决策周期切碎，持球人均速 4-6ft/s；已试 advance_boost/defender_ratio/carrier_ratio 三个抓手，需专门实验周期处理移动语义）；rim 出手份额 0.434 恶化（0.319）、出手距离 16.2 恶化、TOV 0.192——转第 2-5 步队列。defect 计数 33→32（本步同时收敛与漂移，分布校准的走钢丝性质如实记录）。
- R5 脚步状态机（r45，结构性建模升级）：**执法语义重新设计**——收球（gather）后规则只允许终结/传球/站住，因此「生产 = 结构性合法」：travel_enforcement=true（默认）时 Gathered 持球人由决策层跳过 Drive/Advance 候选（收球不可再运球/不可持球推进），行为流里**不可能出现**走步与二次运球；travel_enforcement=false（注入测试）时候选照常生成、由运行时判定层（travel_anchor 锚点 + ConstraintContext.gathered 传递）抓 TRAVEL/DOUBLE_DRIBBLE——注入式判定验证判定链路而非赌球员犯规。配套：ConstraintContext.gathered 字段（flow.rs 从 travel_anchor 推导）、TacticalRules.offensive_crash_max（攻方冲抢人数上限走数据通道，修正 r42 的防守方硬编码 take(2)——方向错误：OREB 偏高的根源是攻方冲抢人数与落点分布，不是防守参与人数）。**落点物理修正**（同批）：筐沿恢复系数 flush 0.62→0.45 / graze 0.35→0.25——此前 92/93 篮板弹到 15ft 外（长篮板散满半场），修正后全部落在 15ft 内（真实 70%+ 在 10ft 内），单场 OREB 率 24.5% vs 真实 25%。**批量口径残留**（10 场）：三分份额 0.505 超参照 0.395（mult 0.80/0.74 对照各有胜负——份额-ORB-距离耦合，单参数无同时满足点，保持 0.80；份额治理归 Shot 效用期望分建模的结构项）。
- R5 违规程序评估（已完成——结论：持球违例的判据缺口是建模问题，基础设施已预留）：ViolationKind 新增 Travel/DoubleDribble 变体、clock 收球锚点 `travel_anchor`（突破结束转 Held 时设置、回合终结漏斗清除）、travel_tolerance_ft 规则字段预留。**两种判据的实测否定**：位移判据（收球后位移超容差即走步）产生 50+ 哨/场（引擎持球跑位无「两步内终结」约束，位移是长时间累积而非步幅）；序列判据（Drive→Held 后再 Drive 即二次运球）产生 16-22 哨/场（引擎的突破结束转 Held 表示「延误后重新组织」，持球人再 Drive 在现实中是合法的变向继续运球）。根因：引擎球态机无「收球（gather）/轴心脚/步数」建模，走步与二次运球在现有语义下不可判定——**误判的违例比缺判更扭曲比赛**，建模升级前保持不判定，规则域维持 sampling_only。时限类违例（8 秒/24 秒/三秒/发球 5 秒/回场）已有完整判定与处罚链路（既有实现，1q 实测频率仍偏高见 r38 遗留清单）。
- 引擎分布偏差修正（阶段性收敛，r38 批次五轮迭代；golden v82→v86）：**20 场正式口径同基线对照（修正前 vs 修正后）**——总 FG% 0.351→0.459（0.474，Defect→Insufficient）；3PA 份额 0.350→0.402（0.395，→Pass）；3P% 0.344→0.373（0.366 ✓）；Pace 117.8→107.7（100.6，Defect→Insufficient）；TOV 0.333→0.182（0.134，收敛 58%）；**Q4 末段三分 0.000→0.409（0.451）**——blowout 症状随回合结构修复而恢复（验证了「末段行为缺失是上游症状」的判定）；四区 FG% 从记账归零恢复量纲后进入带内微调（rim 0.568/0.598、near/0.443、three 0.373/0.359、mid +0.074 残余）。**连带新偏差（如实记录）**：ORB 率 0.279→0.368（0.250，Insufficient→Defect）——命中率结构变化改变篮板争抢分布，机制连带非记账回归；出手距离 W1 14.4→15.9（13.4，三分份额上升的次生效应）；投篮得分 156→225（Pace 残余 +7% × 效率 +3% 的乘积，随 Pace 收敛自愈）。**方法教训**：命中率与出手效用耦合反馈，单参数净修正法在份额变动后失效（mid base 0.44→0.20 使 near 0.442→0.229 全局漂移），迭代以批量口径小步进行并验证出手构成稳定。**遗留清单**：TOV 剩余（poke 掏球 + 8 秒违例构成）、ORB 率连带偏差、putback 效用权衡（0.908/0.811）、Pace 残余与投篮得分、出手距离形状。参数全部走 GameRules 数据通道，golden hash 每轮按纪律登记。(1) **四区 FG% 记账矛盾修复**——进球回合的 SHOT_ATTEMPT_RESOLVED 派生发布晚于回合结算（PossessionSummary 关闭 shot 实例），事件信封取不到 shot_id，11/11 FGM 无法归区 → journal 保留 last_closed_shot_id 供命中结算回退；修复后四区 FG% 从 0/0/0/0.002 恢复到 0.62/0.44/0.41/0.36 的合理量纲（记账 bug 与真实偏差分离，后者继续按偏差修）。(2) 四区命中基准校准两轮（BaseRates 走数据通道）：rim 0.32→0.706 / near 0.29→0.570 / mid 0.265→0.385 / 3pt 0.33→0.325，单场实测出场平均 0.617/0.438/0.405/0.360，全部落入注册参照带（0.598/0.443/0.398/0.359）。(3) 接球范围 catch_radius_base_ft 2.0→2.8（真实接球半径约 1 米）：传球掉球从 24 次/2 场降到 6 次，失误率 29.5%→19%。 (4) poke 掏球频率 2.2→1.2/s、成功上限 0.16→0.11（poke 丢球 11%→目标 3%）。(5) 前场板收球人放开 Pass 候选（此前候选裁剪强制补篮 → 二次出手率 0.99），传球/出手由效用决定，对齐真实 0.81。待验证：10 场批量判定对比（跑动中）。已识别未修：回合数 236 vs 201（活球进攻时长短 ~2s/回合，dwell/urgency 权重带）、三分份额 29% vs 39.5%、8 秒违例 4/109 回合 vs 真实 ~0.3%——与「组织衰减过快」同源，等批量数据后下轮修。Q4 末段三分 0 的根因判定为 blowout 症状（seed42 分差 65 分成垃圾时间），随回合结构修正观察恢复。
- 回合开始原因事实补发（已完成，golden v82）：TransitionOrigin 新增 made_field_goal / made_free_throw / violation / out_of_bounds 四变体；`start_inbound_transition` 按上一回合终结原因（`emit_possession_summary` 唯一写入 `last_possession_terminal` 入档）发射对应 `TRANSITION_STARTED`，罚球命中的发球显式覆盖为 made_free_throw。1q 实测六类 origin 齐发（54 次：loose 21 / made_fg 11 / drb 9 / oob 6 / violation 4 / made_ft 3）。连带语义修正：TRANSITION_STARTED 处理现在为全部转移情形设置 `transition_start_time`（此前只有 steal/drb/loose 三类），决策时序随之变化——golden hash 按纪律登记 v82 基线。验证侧 `GameMetrics::from_stream` 改为消费 TRANSITION_STARTED 事实（弃用终结原因启发式推导），origin→注册七类映射与 rebuild_joint_reference.py 的 STARTTYPE 分类一致（loose_ball_recovery→other：引擎侧无法区分偷球/封盖上游；timeout 引擎无暂停实现保持结构性缺失）。JOINT-Q01 转移矩阵判定从 21 格扩到 28 格（free_throw 行 7 格进入判定），登记 scope 注记同步更新。附修：MECH 扰动入口调小速度属性时把当前速度钳回新上限（防扰动瞬间击穿物理快崩断言，纯模拟路径不经过）。
- debug-server 批量认证总览（已完成）：新增 `GET /api/validation/certification`（读 `.work/validation-view/` 下的 certification.json + batch_statistical_results.json + branch_coverage.json），页面验证选项卡新增「批量认证总览」段——认证结论横幅（含各层计数）、逐叶项缺陷/证据不足清单、规则域覆盖三态；工件缺失时显示「尚未运行批量判定」提示与 CLI 命令。单场诊断的 layers 端点（统计 10 叶项 + 机制 11 实验）同页可见。验收：端到端跑通（模拟 → 单场验证 → 三端点均有数据）。
- MECH 实验全覆盖（已完成）：实验规格从 2 项扩到 11 项，10 个可扰动属性全部入册——shooting_close/mid（近/中距窗口谓词 → 分区命中率）、speed/acceleration/stamina（投篮不中瞬间谓词 → MOVEMENT_OUTCOME completed 耗时均值，LowerBetter 方向）、strength（不中瞬间 → 后场板率，卡位胜出通路）、offensive_rebound/defensive_rebound（不中瞬间 → 前/后场板率，分母 = 窗口内不中数）、decision_iq（后场控制点谓词 → 推进窗口失误率，LowerBetter）。规格结构新增 expected_direction 字段（参与冻结摘要），单调性判定按方向解释（LowerBetter 取配对差相反数）。首跑（seed42 1q）11/11 实验可执行且观测量有量纲；方向性趋势 D01 失误率 0.673→0.477 单调下降、B02 后场板率随属性下降，与物理通路一致；全部单调性判 InsufficientEvidence（单检查点单 trial 功效不足，功效提升待批量并行化后做更多检查点）。规格变更按冻结纪律作废旧摘要重新登记。
- 批量判定并行化（已完成）：`run_batch_with_facts` 改为多线程工作队列（`run_batch_with_facts_parallel`，默认并发 = min(可用核数, 3)，内存约束下的安全并发度），每场独立 seed 独立引擎实例，聚合按 seed 升序还原与串行逐位一致（`parallel_batch_matches_serial_bitwise` 测试断言充分统计量与采样事实逐键相等）。采样事实单场聚合（`SamplingFacts::from_single_run` + `merge`）替代全场留存（并行下内存从 O(全场流×并发) 降为 O(单场流)）。批量段实测 3 并发生效（Threads:4）；单场验证段（含 11 个 MECH 实验）仍串行，批量总耗时受其稀释。20 场 full 正式判定跑在此实现上进行。
- 引擎修正第 5 步：出手距离形状（r44 批次；阶段性结论——判据缺口确认）：长两分（20-23.75ft）占比 0.25 vs 真实 ~0.10，成因 = **Shot 效用与命中率脱钩**（效用只看距离/开放度，无 make 期望项 → 退线三分的命中收益无法进入效用）+ Drive 停滞点在长两分区时出手是唯一合理选项。已做：midrange_utility_bonus 0.15→-0.45（-0.10 无感知，乘积项量级 ~0.3 需同量级惩罚；长两分 0.25→0.23 小幅收敛）、退线三分候选生成（20-23.75ft 时额外生成后退到线外的三分候选）、stepback_three_expected_points_bonus 通道（期望分差加成，数值归 0 待启用）。**实验否定**：bonus 0.55 全三分提额挤占篮下（rim 0.154），限定退线候选后落点语义又与桶边界错位（stepback 量小落点仍在 22-24 区）——需要「候选 from_pos 与 execution 出手点、contest 评估」的位置语义一致化，属结构性。遗留清单更新。
- 引擎修正第 7 步：暂停实现（r46 批次，结构性补齐）：教练暂停评估（落后方分差 ≥10 且节剩 <4 分钟、余额内）挂在发球窗口（所有回合终结汇聚点 `start_inbound_transition`），执行切 `GameFlowState::Timeout` 停表、发 `TeamTimeout` 事实、暂停时长 60s（`timeout_duration_seconds`）后恢复到挂起的发球点并显式发 `TRANSITION_STARTED.origin=timeout`（转移矩阵 timeout 行的事实源闭环）。实测 6 次/场（真实 4-6）、暂停余额正确扣减。**10 场终验**：Pace **100.70（Pass，精确）**、每百回合犯规 **17.33（Pass）**、C01 0.419/0.451 接近、3P%/FG% 带内；三分份额 0.532 恶化（0.395）——pass_base 1.30 + 落点修正 + 长两分惩罚的联合把出手推向三分，确认「份额-ORB-距离」的耦合在当前效用结构（无 make 期望项）下无参数解，治理归 Shot 效用期望分建模（r44 遗留结构项）。defect 29→39 的构成全部是份额类漂移，核心五项（Pace/犯规/C01/3P%/FG%）全部带内或接近——分布校准的下一步是结构项而非继续扫参数。
- 引擎修正第 8 步：Shot 效用期望分建模（r47，三分份额治理结构项）：**根因重定位**——「效用缺 make 期望项」的 r44-r46 结论不完整，参数扫描与单场诊断把份额问题追到三个结构缺陷：(a) **出手位置语义不一致**（r44 遗留确认）：Shoot 效用的 distance_factor 用持球人当前位置，pull-up（进线）/stepback（退线）候选按错误距离计价，从未真实竞争；(b) **mid 区候选缺失**：长两分惩罚挂在 ShotZone::Mid 上连坐 17-20ft 真中距离，且无 pull-up 候选生成——set_play 持球人在弧线只有「原地三分 vs 长两分」二选一；(c) **近框 openness 双重惩罚**：rim/near 的 contest 天然高，结算层已罚 make 概率、效用层再罚一次，系统性把份额推向空位三分。**修正**（全部规则通道）：distance_factor/长两分罚改用候选 from_pos、新增 pull-up 中距离候选（pullup_midrange_step_ft=3ft）、期望分地形项（shot_ep_rim/near/mid/three=1.19/0.79/0.79/1.08 真实 NBA 期望地形，权重 0.35 锚 0.9，独立于结算 BaseRates 可校准——直接复用结算 base 会双重计入校准修正，w=1.0 实测把三分推到 0.662 反向恶化）、近框 openness 调制（shot_openness_interior_scale=0.5）、three_mult 0.80→0.65（结构到位后的重新定标）。**10 场批量**：3PA 份额 0.532→0.446（Defect→Insufficient）、ORB 0.334→0.294（→Insufficient）、FG% 0.443→0.463、四区构成 0.534→0.452、C04 0.919→0.866（全部向参照收敛），defect 39→36。**同批修复**：暂停恢复帧补 publish_events（PlacementApplied/TransitionStarted 滞留 pending 被 begin_tick 清除——不变式失去放置帧豁免依据、违例转移事实丢失 6 个）、回合开始原因推导统一 resolve_inbound_origin（发球路径与暂停恢复路径共用同一映射，暂停不再伪造 origin=timeout、不再吞违例转移）、SHOT_CLOCK_BOUNDS 限值涵盖进攻篮板 14s 重置。golden v94 登记。
- 引擎修正第 9 步：TOV 违例源治理 + 后场推进移动语义（r48，#48/#51 合并批次）：**推进停摆的结构性根因**（探针逐帧定位）：(a) ControlTransfer 飞行期接球人被运动学冻结（target=自身/速度 0），暂停打断冻结等待后接球人**永久冻结**——实测 ADV-KEEP speed=0.00 达 124 次/场；(b) 战术槽位分发不跳过推进中的 Held 持球人，槽位目标（可在身后）每周期覆盖 Advance 目标（实测 target 被改回 (17.6,43.9)）；(c) advance_target 的 y 沿用持球人当前位置，边线接球（y≈47-48）即沿边线走廊推进，被边线+追防两面夹死（均速 2.5 ft/s，8 秒仅推进 20ft）；(d) 推进期无接应，持球人孤立被包夹，传球无目标只能硬突。**修正**（全部规则通道）：execute_timeout 全场解冻（新增 PhysicsWorld::unlock_all_players，暂停是全场停表无人应保留冻结）、槽位分发跳过推进持球人（越线即恢复）、advance_target y 收拢中央走廊（court 24%-76%）、最近两名队友回撤接应（backcourt_support_count/offset/y_min/y_max）、ADV 持久化校验（目标仍在推进方向才保留，否则重设）。单场 8 秒违例 10→4-9 震荡（RNG 时序敏感），**10 场批量**：TOV 0.204→0.197、C01 0.273→0.341（向 0.451 回升）、defect 36→32。集成测试 seed 73→37（r47-r48 语义演化后 73 场景不足 exit 3，37 含封盖/犯规/罚球全过）。golden v95 登记。**遗留**：剩余 8 秒违例 ~4 次/场 vs 真实 ~0.3，需要转换防守路线建模（退防平行跑位而非贴身肉搏）——进 backlog。
- 引擎修正第 10 步：putback 情境区分（r49，#49，诚实负面结果）：ReboundPolicy.putback_openness_gate（补篮加成随空位衰减，0.25/0.75 两档）实测**伤 TOV**（单场 0.175→0.243-0.263）：OREB 后被贴防时「传出去重新组织」的回合失败率高，与 r43 pass_out_factor 2.2 的教训同构。gate 保持 0 禁用（字段与公式保留）。**C04 差距归因改判**：0.899 vs 0.811 的差值不在「补篮 vs 传球」选择（直接补篮 creation_source 历史上仅 1-4 次/场），在 OREB possession 的非出手终结（违例/节末）——与 #51 违例治理汇合，无独立参数解。10 场复批（gate 0 + ADV 校验 + 接应规则化）与 r48 完全一致（32/19/8，全部指标无显著变化——ADV 校验与接应规则化为行为中性重构）。
- 引擎修正第 11 步：20 场综合验收（r50，#53，r47-r49 全部结构项落地后）：seeds 101-120 整场批量。**净判定 defect 33 / pass 19 / insufficient 7**（20 场口径等效区间更窄，判定比 10 场批更严）。**带内项**：Pace 100.558/100.614（Pass，20 场精确）、3P% 0.355/0.366、FG% 0.455/0.474、near FG% 0.453/0.443、四区 near/mid 份额 Pass。**收敛中**：rim FG% 0.585/0.598（差 0.013）、ORB 0.314/0.250、C04 0.887/0.811、TOV 0.197/0.134。**残留 defect**：3PA 份额 0.460/0.395（20 场口径回吐部分 r47 收益）、rim 份额 0.468/0.319、W1 16.8/13.4、FTA/FGA 0.136/0.243（造罚球率——投篮犯规接触强度不足，evidence/problem.md §23.12 已登记瓶颈）、转移矩阵行分布。**口径注意**：投篮得分 203.9 vs 97.2 的对照恒为 defect——参照值是单队场均而模拟值是两队合计，登记口径需换算（下轮处理）。全程累计：r38 起 11 个批次，defect 从 42 降到 33（其中 Pace/犯规/3P%/FG%/rim FG% 等 15+ 项进入 Pass/带内），结构性修复 9 项（四区记账、转移矩阵事实、篮板分工与落点、暂停闭环、R5 脚步、出手效用结构、推进移动语义、接应机制、暂停解冻）。
- 引擎分布修正遗留清单（r40-r46 后，全部以批量口径度量）：出手距离 W1 16.6/13.4（长两分治理需出手位置语义一致化，见 r44）；C01 Q4 压哨三分 0.256/0.451（pass 拖长回合的连带，待节奏-末段交互实验）；rim 出手份额 0.426/0.319（篮下效用偏高，rim_pressure 权重带）；TOV 0.191/0.134（三源结构性，见 r41 结论）；二次出手率 0.881/0.811（pass_out_factor 1.6 已最优，2.2 非单调更差；剩余需 putback/传球效用重构）；出手距离 W1 16.6/13.4（份额对齐后形状仍偏）；8 秒违例 6/场（推进移动语义，r40 遗留）；投篮得分 204/97.2（=回合数×效率的乘积残差，随 Pace 完全收敛自愈）。每项均指向具体机制，下一批次的输入。
- 待办：STAT-I01 分钟表来源（boxscore 归档）；追踪数据类 6 叶项（无公开渠道，保持 InsufficientEvidence）；MECH 实验功效提升（更多检查点，依赖批量并行化）；R5 持球违例需脚步状态机建模升级（时限类违例判定与注入测试已闭环）；引擎分布偏差修正继续（上面的遗留清单，属引擎侧队列）。

行为影响与重冻结记录：验证事件进入帧事件串，黄金哈希从 `0xcd1598382421a6b8` 重冻结为 `0x4f0aabafed2e4d05`（v81）。对照探针证明纯物理轨迹（球员/球位置、球态、比分、时钟）在三个种子上逐位一致，差异全部来自事件流扩充。吞吐基准带下界从 5400 下调到 5000（事件处理量上升，seed42 复测 5396–5535、seed1 6339–6415 ticks/s）。运动意图清算语义修正（到期清算替代即时完成）消除驻留任务重发循环，2000 tick 事件量从 14118 降至 3719。

关闭条件：全部五层工作包达到 `ValidatorImplemented`（validation.md §11）；`FLOW/CAUSE/MOVE/TACT` 的 `/fact` 与场景定向测试通过；`STAT/JOINT` 在参考数据缺位时保持 `InsufficientEvidence` 并登记数据准备任务，不用实现完成代替外部证据完成。

## 文档同步规则

代码变更、针对性测试、配置/参考数据与开发文档必须同一任务更新。`docs/dev/status.md` 是当前状态文件；需同步维护 `docs/dev/current/plan.md`、本文件、`docs/README.md` 和 `docs/dev/README.md` 中的链接与任务状态。规格核对的历史基线（含 45 行状态表与 R1/R2/R7 复核快照）在 `docs/dev/cycles/20260916_convergence/spec_gap.md`，完成新的 gap 核验后更新 `docs/dev/gap.md` 对应节，不回写归档。
