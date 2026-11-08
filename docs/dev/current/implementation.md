# 当前实施进度

> 唯一权威实施范围、依赖和验收条件见 [`full_simulation_plan.md`](full_simulation_plan.md)。本文件只记录进度、已完成验证和当前残差，不另立方案。阶段与证据的逐项对应见 [`../evidence/stage_gate_map.md`](../evidence/stage_gate_map.md)。
>
> 本文件只写当前工作区可以由代码、测试或守卫复核的结论。

## 当前目标

按 `docs/dev/current/full_simulation_plan.md` 的 A-H 工作包完成 NBA 与 FIBA 完整比赛模拟内核。当前阶段记录实施进度，最终状态与证据在行为冻结后统一记录。

H 阶段按 `docs/dev/current/full_simulation_plan.md` 的验收条件实施。协议层定义并导出 `RenderAction`，`RenderPlayer` 携带 `active_actions`；`crates/sim/src/stream.rs` 投影在场球员活动动作与防守姿态；两类球场读取同一 `ActionSpecTable` 与动作配方。未知枚举、缺失配方、目录项缺少验证均直接失败；最终覆盖证据由 `data/rules/action_animation_catalog.json`、浏览器脚本与 CI 守卫给出。

## 阶段进度

| 阶段 | 状态 | 当前记录 |
| --- | --- | --- |
| H. 动作动画与观测闭环 | 实施中 | `docs/dev/current/full_simulation_plan.md` H 规定权威动作状态、16 个动作族与 111 个动作条目的生产链、协议投影、两类渲染器和逐项自动验证。当前协议层已有 `RenderAction` 与 `RenderPlayer.active_actions`，`crates/sim/src/stream.rs` 已有活动动作投影；最终证据在行为冻结后写入目录守卫、浏览器脚本与 CI 工件。 |
| 计划文档整理 | 已完成 | `full_simulation_plan.md` 是唯一权威方案；`plan.md` 是入口，本文件仅记录进度。 |
| 规则与能力目录 | 实施中 | 规则程序目录 22 项按版本、条款、场景矩阵与生产消费者审计；动作目录 16 个族与 111 个动作条目按 `ActionSpecTable` 逐项接入，最终覆盖证据在行为冻结后生成。NBA 2025-26 与 FIBA 2026 v1.1 的来源记录在 `docs/dev/evidence/rule_catalog_sources.md`。目录里的每一条证据指向真实测试：`crates/domain/tests/rule_catalog.rs::every_catalog_evidence_points_at_a_real_test` 逐条核对测试函数。 |
| 权威状态与计时 | 已完成 | 时钟以整数微秒为唯一单位：`ClockState` 保存全部计时器与运行标志，`ClockDirective` 只有 `Start`、`Stop`、`Set`、`Restore` 四种形态，`ClockController` 是唯一改钟入口（其余模块读到的是秒数投影或微秒读数）。七种球态（持球、运球飞行、传球飞行、投篮飞行、松球、裁判控球、比赛用球离场）在 `nba_domain::match_model` 里给出，迁移表由 `BallBody::transition` 强制执行，非法边就地失败。`ControlState` 是控制权的唯一权威：控球人、控制球队、最后触球人与其球队、控制建立时刻、前场控制、触筐与触板身份、控制序列身份都在这一个结构体里，世界不再另存旁路字段。控制序列按控制球队划分，交接是原子事件。事件信封带程序身份、控制序列身份与来源事件。独立验证器只读事件流即可重建时钟与控制序列。走钟策略的唯一判定是比赛程序：死球程序（暂停、罚球、发球、节间）期间无论球是否还在空中，比赛钟都停着；只有活球阶段的飞行才改变走钟方式。推进前先按策略写入运行标志，推进只读运行标志。`crates/sim/tests/clock_scenarios.rs` 的三项回归钉住这三条。 |
| 比赛程序与规则裁决 | 实施中 | 22 项规则程序按方案补齐生产消费路径与场景矩阵；当前已接入的有：开场与加时跳球、NBA 常规节交替发球与 FIBA 交替拥有箭头、发球五秒、进攻三秒、防守三秒、背身五秒、非法触球、双方犯规、恶意犯规、清晰路径犯规、后场八秒与回场、后场八秒的「踢球」重置、跳步违例、紧逼五秒、出界归属与发球地点分类、罚球禁区违例与罚球人替换、暂停配额三层、换人与犯满离场、教练复核与挑战、干扰球、赛前名单检查、加时与换边、最后一罚命中后的底线发球。联赛档案按 11 个策略族组织：节次、时钟、进攻钟、犯规、罚球、发球、跳球、暂停、换人、违例与复核；每个族的字段都在档案里，缺字段读不进来。发球时限与发球准备时长、罚球出手时限与罚球间隔、进攻三秒与守方三秒、背身五秒、后场八秒都从档案读，引擎不再另存一份副本。未登记的违例在该档案下不成立，裁决入口就地失败；换人窗口由换人策略族给出。 |
| 并行动作系统 | 实施中 | 全局单动作 `World::active` 已彻底移除：`World::advance_parallel_actions` 逐人推进每名球员自己的 `ActionRuntime`，多人配合由 `World::interactions` 按交互编号结算，`DefensiveActionRuntime` 另有 `CloseoutContest`、`StealAttempt` 与 `BlockJump` 三种防守动作。持球人、无球进攻球员与防守人各自持有调度与候选集。并发性由 `parallel_actions` 4 项直接验证。 |
| 决策、战术与能力 | 实施中 | 16 个动作族与 111 个动作条目按 `ActionSpecTable` 逐项接入；当前技术选择覆盖运球七种技术、突破三种变体、传球八种技术、切入六种路线、背身步法、跳步违例、欧洲步、后撤步、包夹与护球、挤过与绕过、回位，以及收球后贴身持球五秒判违例。能力、倾向、战术与实时几何共同驱动候选、执行与后果。 |
| 事件、回放与完整验证 | 实施中 | 事件派生球员与球队统计账本、稳定事件序号、带历史缺口检测的游标接口、SessionService 内存检查点恢复与磁盘检查点按方案保存完整 `MatchSetup`、`WorldSnapshot` 与回放清单，恢复时校验版本与状态指纹。回放清单 `MatchManifest` 带着名单、首发、规则档案、种子与三个版本号（规则版本、校准版本、引擎模型版本），写到磁盘再读回来仍然是同一份输入。NBA 与 FIBA 档案从 CLI、WASM、debug-server 与会话检查点进入同一内核：CLI 的 `--league`、WASM 的 `setup_match_json`、debug-server 的 `POST /api/core/setup` 接受联赛档案与完整比赛设置。`FrameRules` 与 `StreamTick` 带规则版本、校准版本与引擎模型版本；`StreamTick.timers` 给出每个计时器的运行标志。完整比赛入口测试比较 CLI 与 WASM 的整场摘要和全部投影事件（seed 211，NBA；seed 307，FIBA）。最近一次完整工作区测试日志在 `.pi/tasks/review-tests.log`；最终测试数量在行为冻结后记录。 |
| 校准与文档验收 | 待最终验证 | 历史 32 场工件有 38 条判据并通过认证；当前门禁按 `docs/dev/evidence/gap_closure_cycle/gate.txt` 记录 39 条判据与 4 项越界，最终校准工件在行为冻结后生成。机制实验默认 160 个种子，`defensive_rebound` 按 640 个种子判定；冻结三表哈希与文档守卫按最终工件复核。 |

## 本轮新增验证证据

定向测试按工作包运行；行为冻结后运行一次完整工作区测试，测试数量、峰值常驻内存与退出码写入 `.pi/tasks/final-workspace-tests.log`。每个测试进程受固定地址空间上限约束。

- `crates/validation/tests/league_profiles.rs` 5 项：两个档案的 11 个策略族都填满内容、扁平策略族缺任一字段读不进来、嵌套策略族缺成员读不进来、缺策略族对象读不进来、未知字段读不进来。
- `crates/sim/tests/league_entry_points.rs` 6 项：NBA 与 FIBA 各跑完一整场、帧带三个版本号、帧带计时器运行标志、磁盘版与内嵌版校准版本一致、清单恢复自定义名单与联赛档案、校准版本不一致的清单被拒绝。
- `crates/cli/tests/interface_parity.rs` 3 项：NBA 整场 CLI 与 WASM 摘要一致、FIBA 整场 CLI 与 WASM 摘要一致、未知联赛标识就地失败。
- `crates/debug-server` 的 `session_setup_accepts_a_league_profile`：debug-server 接受 FIBA 档案，未知档案返回 400。
- `crates/sim/tests/checkpoint_disk.rs` 3 项：磁盘检查点恢复后跑出同一场比赛、分叉的检查点被拒绝、不支持的清单版本被拒绝。
- `crates/sim/tests/clock_scenarios.rs` 13 项：死球程序期间球在空中比赛钟仍然停、暂停期间球在空中比赛钟仍然停、一整场比赛消耗的比赛钟恰好等于四节时长之和，加上原有的十项走钟与重置场景。

- `crates/sim/tests/lane_clock.rs` 5 项：进攻三秒违例、及时离开限制区不违例、防守三秒技术犯规、主动盯防的防守人不判罚、进攻方进入前场前不开始计时。
- `crates/sim/tests/illegal_touch.rs` 5 项：一臂之内伸手阻挡成立、合法间隔上的平行贴防不成立、够不到的防守人不成立、退开之后不再成立、活球里记为防守方的个人犯规。
- `crates/sim/tests/back_to_basket.rs` 5 项：背身运球满五秒判违例、面朝篮筐不违例、越过罚球线延长线不违例、收球终止计时、球被碰开终止计时。
- `crates/sim/tests/action_animation_coverage.rs` 2 项：目录结构与 16 动作族全量覆盖测试、活动动作协议流权威字段测试。
- `crates/sim/tests/foul_scenarios.rs` 15 项：两分与三分投篮犯规的罚球数、加罚、进攻方犯规不计入球队单节犯规、双方犯规保留球权、罚则状态下的松球犯规、技术犯规保留球权、NBA 清晰路径犯规、两级恶意犯规与被逐、FIBA 破坏性犯规单独分类及两罚一掷、停步合球后的抱人犯规、篮板球争抢推人犯规。
- `crates/sim/tests/timeout_scenarios.rs` 13 项：整场额度、第四节额度、末段额度、加时额度独立、活球时只有持球方能叫暂停、死球时两方都能叫、暂停之后比赛恢复、NBA 强制暂停在第二节 6:59 以下首次死球触发、FIBA 无强制暂停、按来源与联赛档案读取完整/短暂/加时三档时长、活球请求占用完整档。
- `crates/sim/tests/replay_scope.rs` 4 项：节钟归零后离手的进球被取消、钟响前离手的进球保留、复核范围之外的档案不产生复核事实、改判成立时比赛钟从出手检查点恢复。
- `crates/sim/tests/challenge.rs` 9 项：挑战维持原判、二次挑战拒绝、窗口外拒绝、获利方不得挑战、无裁决拒绝、规则集默认额度、复核保持状态、依据证据改判推翻原判并翻转球权、复核范围之外的裁决不得受理。
- `crates/sim/tests/closeout_chain.rs` 4 项：滞空防守人面前的两条应对分支都可达、造接触分支只看对抗意愿门槛、侧移比例跟随敏捷度、同一次抽取可重放。
- `crates/domain/tests/league_profiles.rs` 4 项：NBA 不把破坏性犯规单列且不区分技术犯规等级、FIBA 单列并分级、两个档案都声明裁判权限与录像复核范围、两个档案的复核问题不同。
- `crates/validation/tests/event_ledger.rs` 7 项：生产比赛报告的账本守恒、FIBA 报告的账本守恒、每名球员的统计从事件流重算、封盖与投篮犯规的归属、挡拆手与统计口径不符时报错、时钟只凭事件流重建、控制序列只凭事件流重建且建立与结束成对。
- `crates/domain/tests/ball_ownership.rs` 5 项：七种球态的迁移矩阵与声明表逐一一致、非法边被拒绝、持球人只从真正持球的球态读出、只有三种飞行态算在空中、每种球态都有稳定名字。
- `crates/domain/tests/roster_roles.rs` 2 项：每一类进攻与防守角色的行为要求都被倾向字段满足，ADR-020 的背身中锋与定点射手投射判据与策略表一致。
- `crates/sim/tests/period_start_scenarios.rs` 7 项：NBA 第二节发球、FIBA 后续节次的交替拥有箭头、NBA 加时跳球、箭头逐节推进、节次开始的发球权归属、节间休息停表、第一节的球权记录。
- `crates/sim/tests/free_throw_scenarios.rs` 5 项：进攻方提前进区、防守方提前进区、禁区外不记、罚球人离场后由队友顶上、罚球出手时限。
- `crates/sim/tests/substitution_scenarios.rs` 8 项：犯满离场、技术犯规不计入个人与球队累计、活球不换人、换人接管站位、刚下场的人要坐够休息时间、账本记录、伤病产生与人员替换。
- `crates/sim/tests/potential_field.rs` 6 项：强侧底角防守人保持站位、弱侧低位人收缩护筐、突分接球人空间扩大、无掩护无过人对位稳定、弱侧 X-Out 轮转补位、防守扑空失衡被突破过掉。
该文件的 9 项证据见本轮新增验证证据中的首条记录；此处不重复登记。
- `python3 scripts/check_animation_catalog.py` 目录完整性守卫通过。
- `crates/sim/tests/backcourt_clock_scenarios.rs` 4 项：后场停满八秒判违例、防守人踢球后重新计时、过中线停表、传球越过中线同样停表。
- `crates/sim/tests/clock_scenarios.rs` 10 项：传球飞行两钟都走、投篮飞行只走比赛钟、罚球飞行两钟都停、死球与发球期间两钟都停、新的控制权把进攻钟设回二十四秒、触筐的前场篮板补到十四秒、没有触筐的前场篮板不动、进攻钟归零判违例、哨响停表、发球程序期间停表。
- `crates/sim/tests/boundary_scenarios.rs` 11 项：发球五秒违例、发球计时从程序开始算、进攻方碰出界交出球权、防守方碰出界进攻方保留球权、投篮出手之后球飞出界不记失误、收球后再运球判违例、合法恢复不算二次运球、移动轴心脚判走步、两步之内的收球步合法、第三步判走步、跳步违例。
- `crates/sim/tests/jump_ball_scenarios.rs` 8 项、`crates/sim/tests/jump_ball.rs` 5 项：拍球的人必须是跳球员之一、合法拍球的高度、停表跳球等人到位才掷球、撮球之前两钟不走、双方违例之后跳球重来且跳球员不变、交替拥有箭头的初始值、NBA 不使用箭头、跳球是它自己的比赛程序、加时以新的跳球开始。
- `crates/sim/tests/shot_scenarios.rs` 10 项：弧内出手记两分、弧外出手记三分、底角出手记三分、被盖之后球成活球、投篮犯规之后罚球、筐上方下落的球被碰到判干扰球、上升段的球可以合法封盖、筐柱圆柱段之外的球不算干扰球、碰过篮板之后防守人再碰球判干扰球、球坐在筐沿上被碰到判干扰球。
- `crates/sim/tests/action_variants.rs` 7 项：突破变体写进突破事实、变体不改落点但改过掉防守人的机会、变体按几何选、背身步法的空间效果、对位防守人挑站位姿态、姿态发布一次变化事实、贴身的防守人进入施压姿态。
- `crates/sim/tests/architecture_decoupling.rs` 9 项：比赛阶段不含飞行变体、飞行种类与比赛程序正交、控制状态携带权威控制字段、联赛档案按策略族存储、档案缺字段读不出来、七种球态不多不少、时钟指令恰好四种形态、改钟只有一个入口、每条事件都带因果身份。
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

### 历史残差（32 场，种子 1..33）

下表是 S8 历史 32 场工件的 38 条判据观测值与基准值。当前门禁按 `docs/dev/evidence/gap_closure_cycle/gate.txt` 记录 39 条判据；最终工件在行为冻结后生成。

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

`nba-sim validate-core --mechanism` 逐项把单个属性从 0.10 抬到 0.95，比同一批种子的基线，配对 bootstrap 重采样 2000 次。能力扰动的真实效应量在 1%–5%，40 个种子的区间会跨零，因此默认样本为 160 个。

每一项按自己的效应量定样本量：160 个种子对效应量只有 2% 的属性不够，跨零表示证据不足。样本量写在 `crates/validation/src/mechanism_core.rs` 的 `REQUIRED_SEEDS` 里，因此 `defensive_rebound` 跑 640 个种子，其余属性跑 160 个。

13 项属性的判定结果写在 `docs/dev/evidence/action_chain_validation/mechanism_core.json`，认证状态写在同目录的 `certification.json`。`defensive_rebound` 在 640 个种子上的相对变化是 +0.0282，配对区间 [+0.0174, +0.0387]，整段在零的一侧，因此这一项判定为 `pass`，`known_insufficient.json` 的登记表已经清空，认证工件的 `certification_status` 是 `pass`。它的效应量本来就只有 2%：篮板归属主要由站位与球弹向决定，防守篮板属性改变伸手半径，很少改变赢家——这正是它需要 640 个种子的原因。

## 当前残差

以下一项是当前内核明确的量程守卫，逐项标注范围；该项由方案登记并保持监控。

1. 五维诊断里 `events_per_game` 的上界是一条量程守卫，不是分布目标：它要挡住的是事件日志失控（引擎另有一层 50 万条事实的硬上限），因此上界跟着引擎的事实粒度一起抬。2026-11-04 这一轮把限制区计时、发球五秒、非法触球、突破变体、背身步法、对位站位姿态与干扰球的三种情形都接上了生产路径，每一类都发布自己的事实，因此每场的事实数从约 9600 涨到约 10500，上界抬到 13000 并记在 `data/diagnostics/five_dimension_bands.json` 的 `band_rationale` 里。

## 后续工作

方案 §12 的 A–G 阶段与 H 阶段按 `docs/dev/current/full_simulation_plan.md` 的最终验收条件实施；当前状态由 `docs/dev/status.md` 与最终证据工件给出。干扰球的下行段、触板后与筐沿阶段各有场景测试，`PROC-GOALTENDING` 的场景清单没有缺失项。
