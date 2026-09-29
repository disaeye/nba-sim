# 规格与代码差距清单

> 文档类型：规格与代码差距基线，含本次 R1 和篮板来源定向复核。
> 对照范围：`docs/basketball.md`、`docs/tactics.md`、`docs/attributes.md`、`docs/quality.md`、`docs/protocol.md`、`docs/architecture.md`。
> 证据来源：工作区类型、事件、实现和已运行的定向测试。本清单不把测试名称存在当成行为已经符合规格。
> 覆盖边界：42 条总量与原统计来自先前只读核对；本次复核 R1 相关代码、篮板来源和节间时间定向测试，没有逐行检查全部引擎实现。新的 16-seed 矩阵仍在运行，结果未取得。原数量不能视为本次复核后的最新总数。当前 R1–R11 的逐项验收范围见 [`implementation.md`](implementation.md)。

状态含义：

| 状态 | 含义 |
| --- | --- |
| implemented | 所列符号覆盖该条规格的当前范围 |
| partial | 类型、参数或一部分路径存在，规格要求的事实没有全部发布 |
| missing | 规格要求的类型、事件或程序没有找到 |

## 1. 比赛过程

| 规格 | 状态 | 当前证据 | 未实现事实 |
| --- | --- | --- | --- |
| 动作生命周期 | implemented | `ActionStarted`、`ActionPhaseChanged`、`ActionCompleted`、`ActionCancelled`、`ActionFailed`、`WindowTransition`、`ActionTimeWindow`；`lifecycle.rs` 覆盖 13 种动作完整生命周期、单 tick 互斥与运动锁定解除 | 无 |
| 出手结果权 | partial | `PendingShotRelease`、`execute_shot`、`resolve_shot_arm`、独立 `BlockedShot`；哨响不截断在飞出手：罚球队列在到筐结算消费，命中计分即加罚、不中发 `SHOT_MISS`，被封盖的犯规出手不落松球 | 命中在飞行时长结束后抽取，未验证对应实际篮圈接触；投篮犯规在该时刻按防守距离抽取，未关联实际接触；缺少同参数封盖与未封盖对照 |
| 传球结果权 | partial | `BallState::Pass` 不含 `receive_success`；`PassReceived`、`PassDropped`、`PassTipped`、`PassIntercepted` 在到达或接触时发布；哨响作废在飞传球时补发 `PASS_DROPPED` 终结并经因果槽指向该 PASS 事件 | 出界由通用出界转换处理，尚未证明它与其他传球结果互斥并连到传球因果链 |
| 突破结果权 | partial | `BallState::Drive` 不含终局字段；`DriveOutcome` 在窗口边界发布 | 突破是否形成终结及犯规仍含概率抽取，犯规没有绑定实际身体接触事实；结果父链目前指向 `DRIVE_INITIATED`，缺少决定性接触父事实 |
| 松球收控 | partial | `LooseBall`、`LooseBallSecured`、可收速度门、弹开优先；活球松球争抢优先派发（最近代表在一切战术微操之前奔赴球点，切割残留/被过恢复/槽位目标均不拦截争抢；每队至少一名未锁定代表） | 无 |
| 篮板和争球 | partial | `ReboundContest`、`LooseBallSecured`、`try_resolve_rebounder`；松球回收进攻篮板已发布 `REBOUND`，并链接到 `SHOT_TRAJECTORY_ARRIVAL`；防守篮板也发布对应事实 | 同时满足控制条件时仍二选一；`JumpBallTriggered` 没有引擎调用；RimRebound 直接篮板路线的父链尚未覆盖全部路径 |
| 防守主责任 | implemented | `DefenseResponsibility`（九种主责任枚举）、`DefenseResponsibilityChanged`、`DefenseBreakdown`；`defense.rs` 验证责任转移事件链 | 无 |
| 挡拆覆盖 | implemented | `ScreenDefenseRules`（沉退、延误、换防、挤过、绕过）、`TargetAssignment` 责任标记；`defense.rs` 与 `defense_responsibility_chain.rs` 验证方案分化与基线恢复 | 无 |
| 接触和犯规 | partial | `ContactKind`、`GameEvent::Contact`、`GameEvent::Foul`（`FoulKind` 八种 + `FoulPenalty` 结构化罚则） | `NoCall` 不存在；引擎当前只发布 `personal`/`shooting` 两种，其他种类没有产生路径 |
| 犯规账本 | implemented | `check_foul_conservation` 重建个人累计、每节球队累计、犯满离场、bonus 与逐次罚球；限值取自帧内 `FrameRules`；板凳耗尽由流内 `BENCH_DEPLETED retain` 事实豁免（犯满者留场与继续犯规均合法）；正负对照 8 项测试 | 加时球队犯规累计未按档案区分 |
| 违例和暂停 | partial | `EIGHT_SECOND_BACKCOURT`、`FIVE_SECOND_INBOUND`、`SHOT_CLOCK_VIOLATION`、`OUT_OF_BOUNDS`、`THREE_SECOND_LANE`（攻方三秒）、`OVER_AND_BACK`（回场，含发球例外与事前传球阻断） | 走步、二次运球、干扰球、贴身五秒、背筐五秒没有程序（前两者需运球/合球阶段事实，干扰球需飞行中球-人接触物理）；防守三秒不在 charter 程序表；暂停配额没有程序 |
| 联赛程序字段 | partial | `LeagueProfile` 参数化节时长、进攻时钟、个人犯满（NBA 6/FIBA 5）、bonus（两者均为单节第 5 次）、三分几何和交替拥有；加时进节清零球队犯规（charter §6.3 NBA 独立限额口径） | 加时犯规累计口径未按档案区分（FIBA 计入第 4 节）、进攻时钟重置表、暂停配额、转换路径犯规、触筐后干扰开关没有字段与程序 |
| 六类账本 | partial | `check_ledger` 五式 | 罚球不是独立账本；球权检查不覆盖跳球和交替拥有 |

## 2. 战术

| 规格 | 状态 | 当前证据 | 未实现事实 |
| --- | --- | --- | --- |
| 进攻体系数据化 | partial | `TacticalSetSpec`、`TacticalSet` | JSON 档案提供槽位，Rust 枚举仍参与构造和规划 |
| 六种进攻体系 | partial | `data/tactics/` 两份档案 | `off_transition_push`、`off_delay_attack`、`off_post_split`、`off_drag_screen` 没有合法档案 |
| 槽位填充 | partial | `fill_slots`、`fill_slots_or_roster_order` | 失败时回退名单顺序，没有规格里的冲突回溯 |
| 防守体系 | partial | `DefensiveSystem`、`data/defense/schemes.json` | 规格字段在结构体中，运行档案使用另一套 `DefenseRules` 字段 |
| 防守标识 | partial | `DefensiveScheme`、`DefensiveTactic` | 三套 id 和六套 id 并存，阵容只使用后者 |
| Play 规则 | implemented | `select`、`evaluate_active_play`、`decide_on_ball_with_play`、`resolve_verb` | 触发、偏好、抑制和动词进入效用、约束、目标偏置 |
| Play 空间通道 | partial | `play_actions.rs` 的动词步长常量 | 档案没有绝对坐标；步长仍是代码常量 |
| 情境、轮换、暂停、教练 | partial | `SituationalTactics`、`RotationEntry`、`CoachProfile`、`CoachStrategy::evaluate` | 情境、轮换表、暂停和教练偏好没有消费点；垃圾时间只放宽体力阈值 |
| 对位和换防 | missing | `plan_possession_targets_with_geometry` | `assign_matchups` 不存在；初始对位按名单顺序；换防只改目标字符串 |
| 只加 JSON | partial | `TacticalSet::from_id`、`DefensiveTactic::from_id`、`builtin_play_specs` | 进攻和防守都要改枚举；内建 Play 列表写明三份文件 |

## 3. 球员输入

`PlayerAttributes` 有 22 个能力字段，`PlayerTendencies` 有 12 个倾向字段。

| 输入 | 状态 | 当前证据 |
| --- | --- | --- |
| `speed`、`acceleration`、`agility`、`strength` | implemented | `capability.rs` 有对应函数，`attribute_perturbation.rs` 有响应测试 |
| `ball_handling`、`free_throw`、`finishing`、`defense_interior`、`steal`、`decision_iq`、`off_ball_sense` | implemented | 有映射函数和响应测试 |
| `height_cm`、`weight_kg`、`age` | partial | 字段存在；`wingspan_cm`、`effective_reach` 不存在 |
| `vertical`、`stamina`、`passing`、`block`、`defense_perimeter` | partial | 字段存在；没有对应的独立映射或响应测试 |
| `shooting_close`、`shooting_near`、`shooting_mid`、`shooting_three` | partial | 字段进入分区结算；`capability.rs` 和扰动测试没有独立覆盖 |
| `offensive_rebound`、`defensive_rebound` | partial | 防守卡位和进攻篮板采样有测试；前场篮板没有独立映射函数 |
| 12 个倾向 | implemented（定向测试范围） | `PlayerTendencies` 与 12 项规格一致；`tendency_perturbation.rs` 有单调响应、断路负面对照及真实比赛扰动测试 | 全场四区分布与 R6 个体评判已通过 `stats_baseline`、`nba-evaluator` 和 tendency 测试 |
| 四大展示块和复合投影 | missing | `domain::scouting` 不存在 |

## 4. 系统和评判

| 规格 | 状态 | 当前证据 | 未实现事实 |
| --- | --- | --- | --- |
| 球权与球运动正交 | implemented | 物理层 `has_ball` 旁路已删除，持球人作为 `step_with_ball_holder` 的显式输入传入；`holder`、`possession_team`、`is_live`、位置均为权威球态的派生视图（gap.md §5.1 不变量形态，外部 `BallState` 接口保持稳定） | 无 |
| 单一球态写入 | implemented | `transition_ball_state` 唯一写通道；`scripts/check_ball_state_writes.py` 守卫断言赋值、字面量初始化与 `set_ball_state` 调用只出现在授权文件，含负对照；测试钩子经显式命名的 `*_for_test` 通道 | 无 |
| 阶段窄写权限 | partial | 阶段函数按 ADR-014 以「写入状态组：…」注释在签名处显式声明所写组名，跨组逐一列出；`&mut self` 形态由 ADR-014 accepted 裁定保留，由 `scripts/check_engine_state_groups.py` 守卫状态组结构 | 字段级编译期写隔离（ADR-014 已最定不采用） |
| 物理、语义、裁决顺序 | partial | 身体碰撞走 `drain_contacts`、语义解释、裁决 | 出手、突破、篮板、传球拦截在物理和语义阶段之前或之外决定 |
| 阶段调度 | implemented | `step_inner` 按 `architecture.md` §4.1 顺序调度：时钟先行，跳球/死球流程为条件激活分支位于罚球与节末之后；节间计时归时钟阶段，`dead_flow_phase` 只负责转场判定 | 无 |
| 不变量 | partial | `InvariantChecker::check_tick`；`step()` 包装器对含短路在内每个输出帧执行校验，含终局哨兵帧；每 step 恰好一次校验有回归断言 | 引擎内 Hard 违反记录后继续推进，阻断由 CLI 退出码与 violations 工件承担 |
| 确定性 | implemented | `golden_hash.rs` | 只证明冻结窗口，不证明规格完成 |
| 参数通道 | implemented | `GameRules`、常数和阈值守卫 | 守卫只覆盖已登记范围 |
| 执行重校验 | implemented | `revalidate_intent`、`INTENT_DOWNGRADE_RATE` | 重校验不改变已经写入的出手和突破结果 |
| 动作和责任评判 | partial | `evaluate_stream`、`individual.rs` | 已有使用率、助攻父链、防守责任恢复和末节体能个体准则；动作生命周期完整性仍待评判 |
| 证据不足 | implemented | `Verdict::InsufficientEvidence`、`individual.rs` | 个体准则覆盖事件归属不全、助攻父链事实缺失、无责任事实和无帧体能投影 |
| 多联赛 | partial | `league_profile.rs`、`fiba_scenarios.rs` | 测试覆盖现有计时、几何、bonus 和交替拥有，不覆盖第 1 节缺少的程序字段 |

## 5. 数量边界

上表共 42 条：14 条 implemented，26 条 partial，2 条 missing。

本次 R8 复核更新了球权与球运动正交、单一球态写入、阶段窄写权限、阶段调度与不变量五行（物理层持球旁路删除、唯一写通道守卫、阶段顺序重排与逐 step 校验回归）；剩余 missing 为 R9 的 `assign_matchups` 与 R10 的 `domain::scouting`。其余行仍为先前基线。`docs/dev/gap.md` §20 的勾选范围比本清单窄，两份文件同时保留。

## 7. R1 与篮板来源复核

- `BallState::Pass` 已删除 `receive_success`；`BallState::Drive` 已删除 `successful`、`finish_made`、`fouler_id`；投篮挂起与飞行球态不保存命中和犯规结果。
- 命中与触筐结果由到筐几何位置（`classify_shot_arrival`）与篮圈几何判定，反弹起点绑定至实测触筐点；犯规由语义接触事实经裁决层判定并链接前置接触事件。
- 传球接住、掉球、点拨、抢断和出界已实现互斥结算，并增加对应各分支与多种子互斥的针对性回归测试；结果父级统一指向触发它的 PASS 事件。
- 突破终局在窗口结束或提前终结门出现时裁定，已移除突破终结处的独立概率犯规抽取；结果事件 `DRIVE_REACHED` 与 `DRIVE_STOPPED` 已链接至触发它的 `DRIVE_INITIATED` 父节点。突破终结空间基准采用 driver 中心坐标，出手位置按 `drive_finish_extend_ft` 配置向篮筐推进。`ShotRelease` 已加入结构化 `ShotCreationSource`、来源事件 ID 与转换上下文 ID；事件发布根据来源类型设置因果父节点并校验事件类别。单场路径审计中 cut-reception SHOT_RELEASE 曾缺失父节点，修正后一次 seed 1 运行记录 183 次出手、57 次篮下，rim-share 为 0.311；但 evaluator 的 `SHOT_PROFILE_ZONE_MIX` 因中投占比 0.459 判为 Defect，且复测出现过箱体 FGA 与 SHOT_RELEASE 相差 1 次。来源完整性和分布门仍未通过，R2 统计分布尚未验收。
- `events.rs` 中犯规事件已链接到前置的 CONTACT 接触事实父节点，自由球与罚球维持既有因果链接。
- 罚球准备使用 `FreeThrowSetup` 三维轨迹状态，按三维起终点距离计算持续时间，到达罚球线后发布 `BallPlacementApplied`；连续罚球重新建立准备轨迹。罚球飞行入口统一由 `begin_free_throw_flight` 从概率和可选强制结果生成目标点及速度受限时长；强制命中直瞄篮筐中心，强制未中目标在筐环接触区域。准备态与飞行态都会校验活动犯规账本中的 shooter、剩余罚球次数和 `is_final`；飞行到达时以 checked attempt 序号记录事件并更新尝试数、命中数、剩余次数和比分。`free_throw` 测试已增强为逐次核对 attempt 序号、结果、箱体与比分，并要求末罚未中生成以结算接触点为起点的 `RimRebound`。
- 新增裁定回归确认 `ShootingContactCandidate` 即使相对速度低于一般犯规速度门，也进入基于现有 `ContactPolicy` 的投篮犯规裁决。该定向测试与 `free_throw` 11 项通过。此前矩阵有 6 个种子罚球为 0。
- 松球路径现对进攻与防守篮板均发布 `REBOUND`，因果父级取已有 `SHOT_TRAJECTORY_ARRIVAL`；事件发布在控制转移状态建立后识别回收人，并保存进攻篮板事件 ID。进攻篮板重置配置的进攻时钟，补篮持球人在选择其他动作时保留来源上下文，且其可行候选仅剩投篮。篮板来源/时钟测试 2 项、来源时间窗测试 5 项、补篮候选测试 1 项通过。来源位置核验使用球员实际位置和统一 `shot_zone`；转换终结仍带有效转换事件上下文。
- 节间时间测试确认 `QuarterEnd`/`Halftime` 停止推进比赛时间，节间结束进入 `DeadBall` 发球程序后时间继续推进。首轮全场重跑发现节末 `Held` 球态导致下一节发球程序没有重建，修正后 seed 1 已完成。接球决策现由 `PASS_RECEIVED` 事实触发即时决策，并由 `decision_wiring` 验证后续正常决策间隔。seed 1 来源与构成回归 `rim_attempt_composition` 已通过：195 次出手与箱体统计一致，篮下 76 次（占比 0.390）、近筐 31 次、中投 27 次、三分 61 次，突破、空切接球、补篮与转换四条攻框路径均有生产事件记录且因果父链完整，evaluator `SHOT_PROFILE_ZONE_MIX` 判定为 Pass。全量 16 种子 `stats_baseline` 全部通过：总分中位数 210.5 落在 [140, 230] 门槛，三分中位数 36.6%，篮下占比中位数 0.371 且全部 16 种子进入参考带（16/16），全部 16 种子通过 `SHOT_PROFILE_ZONE_MIX`，四路径均有真实出手，零账本违规且零 Hard 缺陷。R2 篮下出手分布与来源因果事实闭合。

## 6. R7 来源复核

`nba.v3` 使用的固定来源是 2023-24 NBA 常规赛 ShotChartDetail 归档，fixture 声明 1,230 场、218,701 次出手；归档 SHA-256 为 `303e71f967c199568b345cd73a75e595f1f99532cca3e6ce59afb286b63f840d`。来源固定到 `ismayc/shot-quality-study` commit `f4001f944d4aba1ba63e3d70edb3d6135ba576fe` 的 `data/shotdetail_2023.tar.xz`；数据说明、提取器及 2023-10-24 至 2024-04-14 比赛日期共同确认赛季范围。NBA.com 的 2023-24 3PA/FGA 基准为 39.5%；归档 SHOT_TYPE 测得 39.4854%，CourtGeometry 几何测得 39.4845%。ShotChartDetail 坐标以十分之一英尺为单位、以进攻篮筐为原点，`LOC_X` 横向取值、`LOC_Y` 从底线向前场增加。将其映射为 `x = 88.75 - LOC_Y/10`、`y = 25 + LOC_X/10` 后，所有坐标半径向下取整均与归档 `SHOT_DISTANCE` 一致；四区现按生产 `CourtGeometry::shot_zone` 几何复算。R7 参考带现用生产 Rim/Near/Mid/Three 边界重算。几何分类和 SHOT_TYPE 有 22 次差异：来源核验分别比较几何三分率与官方基准，逐场分区为避免将歧义三分纳入中投而保留两类标记出的三分。脚本核对归档哈希、比赛和出手数、唯一 `(GAME_ID, GAME_EVENT_ID)`、字段完整性、逐场区间、Q4 分母及参考分位数。出手来源事件或结果因果父链不完整时，R7 准则输出 `InsufficientEvidence`。

R7 的季节标记、比赛总量与官方三分率差异已消解，生产几何分区也已在当前复算脚本中覆盖。仍需独立审阅坐标变换和分区基准、检查完整归档的获取/导入复现流程，并完成 ADR-009；该公共归档仍是一份研究样本，官方季度率仅作总量合理性核验，不能确认每场数据完整性。默认继续采用 `nba.v2`，尚未登记真实性目标线。

## 7. 依赖顺序

代码补齐按 `docs/dev/current/plan.md` 的 R1 到 R11 执行，步骤见 `docs/dev/current/implementation.md`。第 1 节的结果权和账本是后续防守、违例、倾向评判的事实来源；统计带不能提前关闭这些条目。
