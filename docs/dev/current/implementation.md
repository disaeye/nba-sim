# 当前实施进度

> 唯一权威实施范围、依赖和验收条件见 [`full_simulation_plan.md`](full_simulation_plan.md)。本文件只记录进度、已完成验证和当前残差，不另立方案。阶段与证据的逐项对应见 [`../evidence/stage_gate_map.md`](../evidence/stage_gate_map.md)。
>
> 本文件只写当前工作区可以由代码、测试或守卫复核的结论。

## 当前目标

按 `full_simulation_plan.md` 的顺序完成 NBA 与 FIBA 完整比赛模拟内核。用户已确认增加 H 阶段：补齐两类界面的球员动作动画。

H 阶段方案、接口与代码实现已全部完成。在协议层定义并导出 `RenderAction`，`RenderPlayer` 增加 `active_actions` 字段，完成序列化测试；在模拟引擎 `crates/sim/src/stream.rs` 中接入在场球员活动动作与防守状态，权威投影生成 `active_actions`，通过协议流测试；在 2.5D 球场 `crates/debug-server/static/court3d.js` 中基于 `player.active_actions` 调度 11 个骨骼动画片段与配方，移除全局事件和距离推测执行者的机制；在二维战术板 `crates/debug-server/static/app.js` 中基于 `player.active_actions` 呈现动作姿态、腾空指示与发力标记。端到端测试与文档守卫全部通过。

## 阶段进度

| 阶段 | 状态 | 当前记录 |
| --- | --- | --- |
| H. 动作动画与观测闭环 | 已交付 | 协议定义、引擎权威投影、两套呈现视图驱动改造与自动化测试全部完成。协议新增 `RenderPlayer.active_actions: Vec<RenderAction>`，包含稳定动作标识符、交互标识符、事实来源、动作族、动作细分变体、角色、阶段、时间区间、参与者与目标。2.5D 与二维战术板统一消费该状态，两套呈现视图保持一致，且全套测试与文档守卫均通过。 |
| 计划文档整理 | 已完成 | `full_simulation_plan.md` 是唯一权威方案；`plan.md` 是入口，本文件仅记录进度。 |
| 规则与能力目录 | 已完成 | 规则程序目录 22 项全部 `implemented`，没有缺失场景；动作目录 16 个族全部 `verified`，`missing_evidence` 为空。NBA 2025-26 与 FIBA 2026 v1.1 的来源记录在 `docs/dev/evidence/rule_catalog_sources.md`。目录里的每一条证据都指向真实存在的测试：`crates/domain/tests/rule_catalog.rs::every_catalog_evidence_points_at_a_real_test` 逐条去源码里找那个函数。 |
| 权威状态与计时 | 已完成 | 时钟有 `ClockState`、`ClockDirective`、`ClockController`，所有改钟都经控制器；停表后果统一走 `World::adjudicate`（配 `StoppageCause`、`PenaltyPlan`、`RestartPlan`、`ResumeContext`、`InboundLocation`、`ControlState`）。`Phase` 不含任何飞行变体，球的飞行记在 `BallFlight` 上，飞行不替换比赛程序。`ControlState` 是一个结构体，表达控球者、控制球队、最后触球人与其球队、控制建立时刻、前场控制、飞行与死球标志与本回合是否触筐。`LeagueProfile` 的存储本身就是 `PeriodPolicy`、`ClockPolicy`、`FoulPolicy`、`TimeoutPolicy` 四个策略族（`#[serde(flatten)]` 让磁盘上仍是扁平键），档案少一个键就读不出来。解耦的三条结构要求各有直接回归：`crates/sim/tests/architecture_decoupling.rs` 5 项。 |
| 比赛程序与规则裁决 | 已完成 | 22 项规则程序全部有生产消费路径与场景测试。已接入的有：开场与加时跳球、NBA 常规节交替发球与 FIBA 交替拥有箭头、发球五秒、进攻三秒、防守三秒、背身五秒、非法触球、双方犯规、恶意犯规、清晰路径犯规、后场八秒与回场、后场八秒的「踢球」重置、跳步违例、紧逼五秒、出界归属与发球地点分类、罚球禁区违例与罚球人替换、暂停配额三层、换人与犯满离场、教练复核与挑战、干扰球、赛前名单检查、加时与换边、最后一罚命中后的底线发球。 |
| 并行动作系统 | 已完成 | 全局单动作 `World::active` 已彻底移除：`World::advance_parallel_actions` 逐人推进每名球员自己的 `ActionRuntime`，多人配合由 `World::interactions` 按交互编号结算，`DefensiveActionRuntime` 另有 `CloseoutContest`、`StealAttempt` 与 `BlockJump` 三种防守动作。持球人、无球进攻球员与防守人各自持有调度与候选集。并发性由 `parallel_actions` 4 项直接验证。 |
| 决策、战术与能力 | 已完成 | 16 个动作族全部有生产路径与通过测试。技术细分已接入候选、执行与后果：运球七种技术按防守间隔与时钟选择并改变球的暴露程度；突破三种变体（直接、变向、转身）按防守人的站位选，效果是过掉他的机会与丢球风险不同；传球八种技术各有自己的轨迹与接球判定；切入六种路线；背身步法各有空间效果（上下步与撤步转身把防守人顶开）；跳步违例按连续两次同一只脚触地判定；欧洲步、后撤步、包夹与护球、挤过与绕过、回位均已接入；收球后贴身持球五秒判违例。 |
| 事件、回放与完整验证 | 已完成 | 事件派生球员与球队统计账本、稳定事件序号、带历史缺口检测的游标接口、SessionService 内存检查点恢复与磁盘检查点（写入种子、tick 与计数指纹，恢复时重放并校验，分叉就地报错）。完整比赛入口测试比较 CLI 与 WASM 的整场摘要和全部投影事件（seed 211）。当前 99 个测试二进制全绿。 |
| 校准与文档验收 | 已完成 | 32 场（种子 1..33）门禁 38 条判据全部在带内、认证 `pass`；机制实验在 13 项属性上跑配对 bootstrap，`defensive_rebound` 按登记的 640 个种子判。冻结三表未改动，文档守卫通过。 |

## 本轮新增验证证据

本轮改动均在工作区完整测试套件下验证：`bash .work/run_parallel.sh`（`cargo test --workspace --all-targets`，99 个测试二进制，4 进程并行）全绿；每进程 2 GiB 地址空间上限。

- `crates/sim/tests/lane_clock.rs` 5 项：进攻三秒违例、及时离开限制区不违例、防守三秒技术犯规、主动盯防的防守人不判罚、进攻方进入前场前不开始计时。
- `crates/sim/tests/illegal_touch.rs` 5 项：一臂之内伸手阻挡成立、合法间隔上的平行贴防不成立、够不到的防守人不成立、退开之后不再成立、活球里记为防守方的个人犯规。
- `crates/sim/tests/back_to_basket.rs` 5 项：背身运球满五秒判违例、面朝篮筐不违例、越过罚球线延长线不违例、收球终止计时、球被碰开终止计时。
- `crates/sim/tests/foul_scenarios.rs` 11 项：两分与三分投篮犯规的罚球数、加罚、进攻方犯规不计入球队单节犯规、双方犯规保留球权、罚则状态下的松球犯规、技术犯规保留球权、清晰路径犯规、两级恶意犯规与被逐。
- `crates/sim/tests/timeout_scenarios.rs` 9 项：整场额度、第四节额度、末段额度、加时额度独立、活球时只有持球方能叫暂停、死球时两方都能叫、暂停之后比赛恢复。
- `crates/sim/tests/period_start_scenarios.rs` 7 项：NBA 第二节发球、FIBA 后续节次的交替拥有箭头、NBA 加时跳球、箭头逐节推进、节次开始的发球权归属、节间休息停表、第一节的球权记录。
- `crates/sim/tests/free_throw_scenarios.rs` 5 项：进攻方提前进区、防守方提前进区、禁区外不记、罚球人离场后由队友顶上、罚球出手时限。
- `crates/sim/tests/substitution_scenarios.rs` 7 项：犯满离场、技术犯规不计入个人与球队累计、活球不换人、换人接管站位、刚下场的人要坐够休息时间、账本记录。
- `crates/sim/tests/backcourt_clock_scenarios.rs` 4 项：后场停满八秒判违例、防守人踢球后重新计时、过中线停表、传球越过中线同样停表。
- `crates/sim/tests/clock_scenarios.rs` 10 项：传球飞行两钟都走、投篮飞行只走比赛钟、罚球飞行两钟都停、死球与发球期间两钟都停、新的控制权把进攻钟设回二十四秒、触筐的前场篮板补到十四秒、没有触筐的前场篮板不动、进攻钟归零判违例、哨响停表、发球程序期间停表。
- `crates/sim/tests/boundary_scenarios.rs` 11 项：发球五秒违例、发球计时从程序开始算、进攻方碰出界交出球权、防守方碰出界进攻方保留球权、投篮出手之后球飞出界不记失误、收球后再运球判违例、合法恢复不算二次运球、移动轴心脚判走步、两步之内的收球步合法、第三步判走步、跳步违例。
- `crates/sim/tests/jump_ball_scenarios.rs` 8 项、`crates/sim/tests/jump_ball.rs` 5 项：拍球的人必须是跳球员之一、合法拍球的高度、停表跳球等人到位才掷球、撮球之前两钟不走、双方违例之后跳球重来且跳球员不变、交替拥有箭头的初始值、NBA 不使用箭头、跳球是它自己的比赛程序、加时以新的跳球开始。
- `crates/sim/tests/shot_scenarios.rs` 10 项：弧内出手记两分、弧外出手记三分、底角出手记三分、被盖之后球成活球、投篮犯规之后罚球、筐上方下落的球被碰到判干扰球、上升段的球可以合法封盖、筐柱圆柱段之外的球不算干扰球、碰过篮板之后防守人再碰球判干扰球、球坐在筐沿上被碰到判干扰球。
- `crates/sim/tests/action_variants.rs` 7 项：突破变体写进突破事实、变体不改落点但改过掉防守人的机会、变体按几何选、背身步法的空间效果、对位防守人挑站位姿态、姿态发布一次变化事实、贴身的防守人施压而不是滑步。
- `crates/sim/tests/architecture_decoupling.rs` 5 项：`Phase` 不含飞行变体、飞行种类独立于比赛程序、`ControlState` 带着全部控制字段、联赛档案按四个策略族存储、两个档案声明同一组字段、档案少一个键读不出来。
- `crates/domain/tests/rule_catalog.rs` 3 项：程序与动作目录的结构与场景身份唯一性、目录里的每一条证据都指向真实存在的测试。
- `python3 scripts/check_docs.py` 与 `git diff --check` 通过；冻结的三张表哈希未变（`scripts/calibrate/freeze.py --check`）。

## 既有验证证据

先前相关验证通过：

- `cargo check --workspace`：工作区编译检查通过。
- `cargo test --workspace --no-run`：全部测试目标编译通过；最大常驻内存约 488 MiB，构建任务为单作业。
- `cargo test -p nba-sim-core --test jump_ball --test runtime_tables --test rules_flow -- --test-threads=1`：覆盖 NBA 开场与加时跳球、NBA 第二和第三节交替发球、FIBA 交替拥有发球箭头推进、两钟、罚球停钟、末罚篮板、暂停恢复及 100 场阵容连续性。
- `cargo test -p nba-sim-core --test ball_physics -- --test-threads=1`：球速上限回归通过；扫掠手部接触拦截回归断言防守人身份和接触事件。
- `cargo test -p nba-sim-core --test finishing -- --test-threads=1`：包含常规第一、第二节换向后的双队三分线分类。
- `cargo test -p nba-sim-core --test finishing shooter_zone_matches_offensive_half_after_period_direction_switch -- --exact --test-threads=1`：通过；`shooter_zone` 在常规第一、第二节按实际进攻方向识别双方出手分区。
- `cargo test -p nba-domain --test rule_catalog --test game_flow_transitions --test rules_geometry -- --test-threads=1`：目录结构、场景身份唯一性、动作场景测试证据完整性校验。

以上仅为相关回归，不代表完整工作区行为测试通过。

## 校准记录适用范围

校准阶段已执行：`scripts/calibrate/fit_policy.py` 按职责分组调 `policy.json` 的权重，目标是 `shot_profile.json` 的区域与创造方式份额，以及 `possession.json` 的回合数、每回合出手、失误率、罚球与得分。引擎侧原先写死的系数已陆续搬进参数表，可被标定。每次拟合后用 `validate-core` 记录残差；历史批次（2026-10-21）的数字仅作参考。

### 当前残差（32 场，种子 1..33）

全部 38 条分布判据在带内。下表是同一批种子的实测值与基准值。

| 指标 | 观测 | 基准 |
| --- | --- | --- |
| possessions_per_game | 205.938 | 198.197 |
| points_per_game | 233.500 | 227.591 |
| blocks_per_game | 10.500 | 9.655 |
| shooting_fouls_per_game | 17.156 | 19.597 |
| fta_per_game | 43.906 | 43.334 |
| oreb_per_game | 27.500 | 27.663 |
| fga_per_possession | 0.937 | 0.898 |
| turnover_per_possession | 0.149 | 0.144 |
| steals_per_game | 14.656 | 16.121 |
| and_one_per_game | 2.219 | 2.500 |
| free_throw_percentage | 0.795 | 0.780 |
| zone_share 篮下 | 0.310 | 0.316 |
| zone_share 近筐 | 0.189 | 0.191 |
| zone_share 中距离 | 0.077 | 0.072 |
| zone_share 三分 | 0.424 | 0.421 |
| creation_share drive | 0.241 | 0.220 |
| creation_share cut | 0.042 | 0.045 |
| creation_share post | 0.055 | 0.064 |
| creation_share pull_up | 0.195 | 0.190 |
| creation_share putback | 0.049 | 0.045 |
| creation_share transition | 0.131 | 0.121 |
| creation_share set | 0.286 | 0.316 |

### 机制实验

`nba-sim validate-core --mechanism` 逐项把单个属性从 0.10 抬到 0.95，比同一批种子的基线，配对 bootstrap 重采样 2000 次。能力扰动的真实效应量在 1%–5%，40 个种子的区间会跨零，因此样本至少 160 个。

每一项按它自己的效应量定样本量，而不是所有属性用同一个数：160 个种子对效应量只有 2% 的属性不够，跨零不是「没效应」，是「样本不够」。样本量写在 `crates/validation/src/mechanism_core.rs` 的 `REQUIRED_SEEDS` 里，因此 `defensive_rebound` 跑 640 个种子，其余属性跑 160 个。

13 项属性的判定结果写在 `docs/dev/evidence/action_chain_validation/mechanism_core.json`，认证状态写在同目录的 `certification.json`。`defensive_rebound` 在 640 个种子上的相对变化是 +0.0282，配对区间 [+0.0174, +0.0387]，整段在零的一侧，因此这一项判定为 `pass`，`known_insufficient.json` 的登记表已经清空，认证工件的 `certification_status` 是 `pass`。它的效应量本来就只有 2%：篮板归属主要由站位与球弹向决定，防守篮板属性改变伸手半径，很少改变赢家——这正是它需要 640 个种子的原因。

## 当前残差

以下一项是当前内核明确的残差，逐项标注范围；它是方案的已知残差，不是未开工的阶段。

1. 五维诊断里 `events_per_game` 的上界是一条量程守卫，不是分布目标：它要挡住的是事件日志失控（引擎另有一层 50 万条事实的硬上限），因此上界跟着引擎的事实粒度一起抬。2026-11-04 这一轮把限制区计时、发球五秒、非法触球、突破变体、背身步法、对位站位姿态与干扰球的三种情形都接上了生产路径，每一类都发布自己的事实，因此每场的事实数从约 9600 涨到约 10500，上界抬到 13000 并记在 `data/diagnostics/five_dimension_bands.json` 的 `band_rationale` 里。

## 后续工作

方案 §12 的 A–G 阶段均已交付并有直接测试。干扰球的三种情形（下落段、碰过篮板之后、球在筐沿上）都已接入生产路径并各有场景测试，`PROC-GOALTENDING` 的场景清单因此没有缺失项。后续工作随规则与档案的扩展继续。
