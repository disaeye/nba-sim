# NBA-Sim · 设计决策登记

> 本文档登记影响多份规格、具有取舍关系、且需要保留理由的设计决策。
> 它不是当前状态报告，也不是实施计划；当前实现状态见 `docs/dev/status.md`，实施计划见 `docs/dev/current/plan.md`。

## 0. 决策治理

一条内容只有在同时满足以下条件时才登记为决策：

- 存在两个以上合理方案；
- 选择会影响多个模块或后续迁移成本；
- 未来读者需要知道“为什么不是另一个方案”。

决策状态：

- **accepted**：已经成为设计约束；规格文档只保留结论；
- **proposed**：正在评估，不能被实现当作稳定规格；
- **superseded**：被后续决策替代，只保留历史理由。

决策编号稳定不复用。修改已接受决策时，新增决策或明确标记为 superseded，不直接改写历史理由。

## 1. 已接受决策

### ADR-001 · 事件序列是比赛唯一真相

**状态：accepted**

比分、球权、时间、犯规和回合统计都必须能从事件序列独立重建。帧是观察投影，统计是派生结果，不能反过来成为比赛真相。

因此：

- 事件需要稳定的 `event_id`，跨 tick 唯一；
- 因果事件通过 `parent_event_id` 连接；
- 账本检查器独立于引擎状态维护；
- 无法归因的终结不是“未知但通过”，而是证据或完整性失败。

规范落点：`architecture.md` §2、`quality.md` §1–§3。

### ADR-002 · 球权归属与球运动最终正交

**状态：accepted**

球权归属（谁控制球、最后触球方、死球转移）与球的运动（持球偏移、飞行、松球、篮板）是两个不同维度。单一 `BallState` 仍可作为外部状态机接口，但其内部语义不得让运动字段伪装成归属事实。

迁移策略：先保持现有 `BallState` 的稳定外部接口，再逐步把 `BallControl` 与 `BallMotion` 的语义收敛到内部实现；迁移期间不得同时引入第二个权威球态。

规范落点：`architecture.md` §3；目标迁移方案：`docs/dev/gap.md` §5。

### ADR-003 · 校准必须服从因果证据

**状态：accepted**

统计进入参考带不是机制真实的充分条件。校准必须同时提供：

1. 分布证据；
2. 能力扰动或机制层证据；
3. 反事实场景证据（适用时）。

因此，聚合统计只能作为 sanity net 或回归信号，不能单独证明真实性。

规范落点：`charter.md` §5–§8、`quality.md` §2、`protocol.md` §1–§3。

### ADR-004 · 防守方案必须进入行为因果链

**状态：accepted**

防守档案不能只改变展示字段或站位标签。防守方案必须通过 `docs/basketball.md` §4 的责任分配、轮转、施压、协防或执行重校验改变可观测行为；中性档案必须能复原行为中性基线。

规范落点：`tactics.md` §3–§5、`basketball.md` §4；验收引用 `quality.md` §6。

### ADR-005 · 名册顺序不是身份

**状态：accepted**

数组顺序、ID 中的序号和隐式索引不能决定球员能力、角色、首发、持球人或行为。球员身份来自档案，行为差异来自能力、倾向、规则和比赛状态。

规范落点：`attributes.md` §2、`tactics.md` §1–§4；守卫：`scripts/check_no_index_identity.py`。

### ADR-006 · 开发阶段不等于完成状态

**状态：accepted**

G/M/D/L/F/Round 是不同层级的计划或执行编号，不是完成度。任何阶段只有在其门的全部证据存在时才能标记完成；历史执行记录不能作为当前完成状态。

当前状态唯一来源：`docs/dev/status.md`。当前待办唯一入口：`docs/dev/current/plan.md`。

### ADR-020 · 统一出手分区与赛前固定角色

**状态：accepted**

投篮分类只有一份判据：`domain::court::CourtGeometry::shot_zone`。分区为篮下小于 5 英尺、近筐 5 至 14 英尺、中投至少 14 英尺且位于三分线内、三分按联赛几何并优先于距离。分类事实取释放时刻出手点。`shooting_close` 对应篮下，`shooting_near` 对应近筐，`finishing` 保留对抗终结语义。命中基准分为 `shot_make_rim`、`shot_make_near`、`shot_make_mid`、`shot_make_3pt`。突破动作阶段仍由动作阈值表达，不承担出手分区。

球员档案新增 `position`、`offensive_role`、`defensive_role`。位置使用 Point、Combo、Wing、Forward、Big、Center 六类，依据长期阵容位置。攻防角色在赛前确定，整场不变；替补登场使用替补自己的配置。三者只进入身份和投影，不进入效用、概率或行为分支。回合内职责仍由实时动作、战术槽位和对位表达。

球员资料页按得分、组织、篮板、防守四块展示底层维度，不把同一块内的维度合并成一个数。

规范落点：`attributes.md` §2.3a、§2.7a、§2.7b、§2.9，`tactics.md` TA9，`basketball.md` §3.2。

## 2. 待决策事项

### ADR-007 · 是否完成 BallControl × BallMotion 的内部迁移

**状态：proposed**

目标方案已在 `docs/dev/gap.md` §5 定义。接受前必须证明：

- 外部 `BallState` API 不发生无授权行为漂移；
- 所有归属、最后触球和死球责任信息仍可从一个权威状态派生；
- 迁移后不存在旧旁路字段继续承载真相。

### ADR-008 · 完整防守责任图的范围

**状态：proposed**

`docs/basketball.md` §4 已定义对位、挤过、绕过、延误、换防、沉退、协防、补位和回位的责任事实。本决策不再决定规格范围；实现是否形成统一证据包由 `docs/dev/status.md` 记录。

### ADR-009 · 真实度目标线的重标定

**状态：proposed**

构成准则、证据覆盖和机制守卫稳定前，旧的真实度目标线不能直接继承。重标定必须基于新 fixture 版本、固定分母和盲区登记，不得用当前模拟输出反推参考带。

### ADR-010 · 无持球人球态下的归属语义

**状态：accepted**

背景：`carrier_idx` 删除（D8.1 → D15）被证实**不是等价重构**——`carrier_id()` 对
`Held` / `InboundReady` / `InboundTransfer` 之外的 7 种球态（Drive / ControlTransfer /
Pass / Shot / LooseBall / RimRebound / Dead）落入 `roster[possession][carrier_idx]` 回退，
即「上一次控制权的序号在当前球权队名单上的投影」；改为球态关联人后 8-seed
`total_p50` 由 213.5 漂移至 212.0（实证见 `docs/dev/evidence/problem.md` §25）。

裁定：**ball handler 是球态的函数，不是独立的可写字段**。无持球人球态下不存在 ball
handler，读取方得到 `None` 并走自己的无持球人分支；不得用名单下标投影出一个
"占位 handler"顶替。逐球态语义：

| 球态 | handler（`BallState::associated_player()`） | team possession（`BallState::possessing_team()`） |
| --- | --- | --- |
| `Held` / `Drive` | 持球人 / 突破人 | 持球人所在队 |
| `ControlTransfer` | 无（`None`） | 交接发起方（见下方衔接规则） |
| `Pass` | 接球人（`target_id`） | 接球人所在队 |
| `Shot` | 出手人（`shooter_id`） | 出手方 |
| `InboundReady` / `InboundTransfer` | 发球人 | 发球人所在队 |
| `LooseBall` / `RimRebound` | 无（`None`） | `last_touch_team`（最后触球方） |
| `Dead` | 进入死球时快照的 `last_touch_player`（可空） | 进入死球时快照的 `last_touch_team` |

规则与理由：

1. **控球归属（handler）与球队球权（team possession）是两个独立派生量**，都由
   `BallState` 载荷单一派生，互不顶替。无 handler 不等于无球权队——`LooseBall` /
   `RimRebound` / `Dead` 期间没有 handler，但仍有由 `last_touch_team` 决定的球权队。
2. **handler 缺省（`None`）必须由读取方显式处理**，不静默回退到名单序号。旧回退
   `roster[possession][carrier_idx]` 违反 ADR-005（名册顺序不是身份）——它让行为
   取决于球员在同队名单中的位置序号。`docs/dev/evidence/problem.md` §25.4 实测
   "从不跨队"只是当前转移序列的巧合（球权翻转后总是立即 `Held`），不是可依赖的
   可依赖的稳定约定。
3. **唯一已知消费者是战术 planner 的进攻几何参考点**（`plan_possession_targets_with_rules`
   的 `carrier_idx` 实参）。无 handler 球态（`ControlTransfer` / `LooseBall` / `RimRebound`
   期间的阵地与攻防转换布置）下，planner 以 `ball_pos_3d`（球的实际位置）为参考，
   不假设存在持球人；这与 D9.1 已删除的"按持球人槽位硬编码几何"一致——几何参考
   应跟随球，不跟随一个虚构的持球人。
4. 由此 `carrier_idx` 成为纯冗余：handler 永远可从 `BallState` 派生，下标投影不再是
   任何读取方的输入。删除是上述语义实施后的机械结果，而非独立的取舍。

对既有判断的修正：`docs/dev/status.md` 与 `docs/dev/evidence/problem.md` §25 原结论
"两个语义都不是显然正确的那一个，需先明确语义再重构"——本条即该语义裁定：采用
球态关联语义（`associated_player()` / `possessing_team()`），否定名单下标投影。
迁移期的行为变化（planner 几何参考点从"同队序号球员"改为"球位置"）是**有意的语义
修正**，须附 8-seed 矩阵与反事实证据，不按行为中性重构提交。

规范落点：`docs/architecture.md` §3（球权状态机）、`docs/dev/gap.md` §5；
实证依据：`docs/dev/evidence/problem.md` §25；执行入口：`docs/dev/current/plan.md` §4（D15）。

### ADR-011 · 从单体 MatchEngine 向纯数据管线与 ECS 架构演进

**状态：accepted**

背景：现有 `MatchEngine` 结构体已膨胀至 6,600+ 行，集成了时钟管理、名单物理、弹道积分、
战术槽位决策、裁判判定、事件发布与解说生成等全部职责。虽然 D14–D16 完成了只读快照与部分阶段隔离，
但核心主循环依然依赖于庞大的全局 `&mut self` 集中突变，导致：

1. 模块间隐式耦合深，单一字段修改难以做无副作用单元隔离测试；
2. 架构设计中宣称的“13 阶段窄签名函数式管线”无法完整实施；
3. 并行与向量化计算受阻。

裁定：**将 MatchEngine 解构为纯数据世界（World）与无状态系统管线（Systems）**：

1. **纯数据世界（Component World）**：世界仅包含扁平的纯数据组件，包括 `Transform`（坐标/朝向/速度）、
   `Kinematics`（加速度/抓地力/动量）、`PlayerState`（体能/犯规/士气）、`BallComponent`（三维弹道与归属）、
   `MatchClock`（游戏时钟与进攻时钟）和 `GameFlowLedger`（比分与账本）。
2. **纯函数式系统（Stateless Systems Pipeline）**：主循环每个 tick 严格执行以下阶段，
   阶段之间仅通过窄数据接口流动：
   - `PerceptionSystem`：空间 Voronoi 拓扑与防守压迫密度计算；
   - `DecisionSystem`：基于意图评价的动作决策（纯函数，无状态突变）；
   - `PhysicsSystem`：刚体运动、动力学积分与弹道解算；
   - `OfficiatingSystem`：规则越界裁决、犯规吹罚与账本守恒审计；
   - `EventDispatcher`：不可变事件流与渲染帧打包。
3. **迁移策略**：采取渐进式解耦。`MatchEngine` 降级为单纯的外部协调器（Facade），
   内部逐步将各逻辑块抽离为独立 crate（`crates/engine-core`、`crates/physics`、`crates/decision`），
   保证每一步重构均有 16-seed 黄金哈希与 G-STATS 基准守卫。

规范落点：`docs/architecture.md` §4；执行入口：`docs/dev/current/plan.md`（D22–D29）。
> 本条的「阶段函数接收字段级 `&mut` 切片」条款已由 ADR-014 根据实测数据修订；
> 其余要求（行为的纯函数性、不建与主引擎脱节的并行孤岛、渐进式解耦）继续有效。

### ADR-012 · 连续受限势能场动力学与空间 Voronoi 拓扑模型

**状态：accepted**

背景：现有移动与突破防守系统采用“离散目标槽位插值 + 经验概率骰子判定”范式。
球员根据预设战术插槽获得 `target_pos` 后做匀速或加减速位移，防守成功率依赖属性线性映射的骰子判定。
这种机制导致：

1. 球员位移生硬，缺乏真实身体惯性、变向制动距离与失位真实感；
2. 战术跑位死板依赖静态坐标，无法根据防守真实压迫形成动态空间拉扯（Spacing）。

裁定：**引入连续势能场动力学（Spatial Force Field）与沃罗诺伊空间分析（Voronoi Spacing）**：

1. **势能场动力学（Movement by Potential Field）**：
   - 球员移动受合力 $F = F_{attractor} + F_{repulsion} + F_{traction}$ 驱动；
   - 篮筐与无球空位为引力源，对位防守人构成具有朝向椭圆衰减的斥力阻力场；
   - 引入最大加速度、制动距离与变向抓地力（Traction Limit），超速或急停变向将受真实牛顿力学惯性惩罚；
2. **空间重力与沃罗诺伊面积（Voronoi Gravity）**：
   - 实时计算进攻球员拥有的 Voronoi 拓扑面积与局部防守压迫密度（Contestation Density）；
   - 战术决策（传球/突破/出手）不再依赖死板槽位，而是以“空间收益导数（Spacing Yield Gradient）”驱动，
     防守协防内缩必然自然导致外线 Voronoi 面积激增涌现空位。

规范落点：`docs/tactics.md`、`docs/architecture.md`；执行入口：`docs/dev/current/plan.md`。

### ADR-013 · 编排层巨石按职责划分模块

**状态：accepted**

背景：`crates/engine/src/match_engine.rs` 已膨胀至 6,700 行，把时钟、名单、弹道状态机、
动作执行、对抗裁定、球权转移、战术导航、事件发布、流导出与只读投影全部集中在
一个文件与一个 `impl` 块内。ADR-011 已裁定向纯数据管线演进，但文件级边界缺失
使“哪些代码属于哪个阶段”只能靠行号与注释判断，阶段划分无法单测也无法审阅。

裁定：**按职责把编排层划分为同一模块下的多个文件，保持行为逐字节不变**：

1. **模块边界以阶段与事实类型划分**，不以行数划分：
   - `mod.rs`：状态字段、构造、时钟与球态唯一写入口、`step()` 调度、只读访问器；
   - `tactics_phase.rs`：战术目标生成与移动导航；
   - `execution.rs`：动作执行与决策输出应用；
   - `contests.rs`：传球拦截、贴身切球、传球成败、篮板归属；
   - `transitions.rs`：球权与生命周期状态转移；
   - `events.rs`：事件裁决、强制项应用与因果链发布；
   - `receiver.rs`：接球人感知与领传几何；
   - `stream.rs`：流导出与资源治理；
   - `projection.rs`：只读快照与渲染帧，以及领域事实到协议事件的适配；
   - `construction.rs`：从 `MatchSetup` 与种子建立初始状态（`with_setup`）；
   - `accessors.rs`：只读访问器与 `sync_to_world`；
   - `test_hooks.rs`：40 个 `*_for_test` 受控写入钩子（隔离在一处便于审计）；
   - `ball_flight.rs`：弹道裁决（按球态逐 tick 推进球位、产出 `BallFlightOutcome`）
     与球态写入口（`transition_ball_state` / `sync_ball_holder` / `mark_receiver`）；
   - `roster.rs`：名册与换人（`forced_substitution` / `new_possession_pg` /
     `team_roster_ids` / `select_jumper_id`）；
   - `flow.rs`：阶段标签投影（`phase_type` / `ball_phase`）、宏观生命周期与子阶段
     迁移、作用域边界（`parse_scope*` / `set_scope` / `update_scope_completion`）、
     回合边界与阶参数（`complete_possession` / `is_clutch_situation` /
     `morale_bias_for`）、回合总结发射口（`emit_possession_summary`）；
   - `action_windows.rs`：动作窗口推进与运动学锁同步；
   - `decision.rs`：决策阶段的触发条件与上下文装配；
   - `bookkeeping.rs`：每 tick 收尾的持球人同步、体力推进、语义接触归集与事实抽取；
   - `runtime_phase.rs`：运行时约束求值、贴身切球、罚球结算、节末与终场短路；
   - `types.rs`：对外值类型（`MatchBoxScore` / `ExportSummary`）。
2. **子模块直接访问父模块私有字段**（Rust 隐私规则），因此划分不引入任何
   `pub` 字段或绕过不变量的新入口；`MatchEngine` 仍保持零 `pub` 字段。
3. **行为不变的判据是黄金哈希**：划分期间每一步都必须保持 `golden_hash` 通过，
   模块移动不得伴随任何逻辑改写。

规范落点：`docs/architecture.md` §4；执行入口：`docs/dev/current/plan.md`。

### ADR-014 · 编排层按命名状态组收敛访问面（修订 ADR-011 的阶段窄签名条款）

**状态：accepted**

背景：ADR-011 裁定向纯数据世界（World）加无状态系统管线演进，`architecture.md` §4.1/§4.3 给出
其具体形式：阶段函数接收它需要的字段 `&mut`，调度器解构 `World` 传参，目标是让越界修改成为编译错误。
ADR-013 已完成文件级划分，但未改变字段共享：全部子模块仍可访问 `MatchEngine` 的同一批私有字段。

对当前代码实测后的结论：

1. 82 个字段共 1,215 处 `self.<字段>` 引用；按「同一函数同时写入」为边，47 个字段存在多写入点、
   其中 37 个字段被 2 个以上模块共同写入（`pending_events`、`current_event`、`ball_pos_3d`、
   `shot_clock`、`receiver_estimate` 各被 5 个模块写入）；
2. 按「同一函数同时读取」为边，去掉 `step_inner` 与 `with_setup` 后 81/82 字段仍在单一连通分量内，
   即不存在可按读取关系自治划分的字段簇；
3. 按「同一函数同时写入」为边，59 个含写入的函数中 35 个只写单一字段簇，其余 24 个跨 2–7 簇，
   `step_inner` 单函数跨 7 簇；
4. 弹道裁决块（1,138 行）引用 25 个状态字段、调用 20 个引擎方法。

因此按字段 `&mut` 解构的窄签名管线在保持行为不变的前提下无法实施：它需要重写上述跨组函数体，
而重写的等价性不能由 `golden_hash`（只观测输出帧，不观测字段归属）证明。

裁定：

1. **状态核心是 `MatchEngine` 的命名状态组**（分组与字段清单见 `architecture.md` §4.3），
   不另立一份与它并行的权威状态。组字段以 `pub(crate)` 对同 crate 的模块可见
   （Rust 的字段可见性以模块为界，`state.rs` 的私有字段对它自己以外的模块不可见，
   因此 `pub(crate)` 是子模块能访问组字段的最小可见性），模块经 `self.<组名>.<字段>`
   具名路径访问；组结构体本身不 `pub`，外部 crate 拿不到它；
2. **窄签名的单位是状态组**：阶段与命令签名列出它写入的组名，跨组函数必须逐一列出，
   跨组写入从隐式变为显式；
3. **ADR-011 的「阶段函数接收字段级 `&mut` 切片」条款由本条修订**；ADR-011 关于
   「行为是 `(能力, 规则, 状态)` 的纯函数」「不建与主引擎脱节的并行孤岛」的要求仍然有效；
4. **`MatchWorld` 的定位是衍生视图**：它当前由 `sync_to_world` 逐 tick 写入，生产代码从不读取
   （唯一读取方是结果被丢弃的 `PerceptionSystem::evaluate`）。按 P1（单一事实源），
   权威状态保持在 `MatchEngine` 的状态组里；让感知结果进入行为因果链作为独立任务推进，
   其验收门是 `crates/engine/tests/wiring_proof.rs`，不以架构承诺代替。

规范落点：`docs/architecture.md` §4.1、§4.3；执行入口：`docs/dev/current/plan.md`（D22、D28）。

取舍代价：状态组不提供编译期的字段级写权限隔离，越组写入对本模块代码仍可编译。
替代手段是签名显式列出组名与 `scripts/check_engine_state_groups.py` 守卫
（断言 `MatchEngine` 零裸字段、断言组字段均为 `pub(crate)` 不泄出 crate、断言 `mod.rs` ≤ 400 行）。

### ADR-015 · 空间量的单一事实源

**状态：accepted（第 2、3 条已被后续 shadow world 移除超越，见文末修订注记）**

背景：空间计算在四个位置各自实现，`plan.md` §4.3 要求先裁定单一事实源再补进攻侧。
对四处实测后的职责与消费面：

| 位置 | 提供的量 | 消费面 |
| --- | --- | --- |
| `physics/src/spatial.rs` `SpatialGeometry` | 对位人距离与朝向、传球走廊、队友密度 | 经 `SpatialPhysics::openness` / `pass_corridor` 被 decision 与 semantics 大量读取 |
| `physics/src/perception.rs` `PerceptionSnapshot` | 球员级只读事实（到球/筐距离、最近对位人） | **零消费**（全仓无任何引用，只在 `lib.rs` 的 `pub mod` 出现） |
| `engine/src/world.rs` `PerceptionSystem` | 全场压迫密度、局部 Voronoi 开阔度近似、弱侧空位 | 引擎逐 tick 调用但结果被丢弃；行为测试 `match_world.rs` 验证其单调性 |
| `decision/src/potential_field.rs` `DefensePotentialFieldSolver` | 多体均衡位置与涌现防守动作 | `decision/src/tactics.rs` 的防守目标生成 |

裁定：

1. **球员级空间事实的唯一实现是 `SpatialGeometry`**（`physics` crate）：
   它已是 decision 与 semantics 的实际数据源，有自己的规则通道与行为测试。
   新增的球员级空间量一律加在这里，不得另建平行的球员级抽象；
2. **`physics/src/perception.rs` 删除**：它零消费，且其四个量中三个
   （到球距离、到筐距离、最近对位人）已由 `SpatialGeometry::get_openness` 与
   `nearest_opponent` 覆盖，保留它就是第四套平行模型；
3. **全场拓扑量的唯一实现是 `PerceptionSystem`**：压迫密度与开阔度是**全场**量
   （以十人为输入的聚合），不是球员级对位量，因此与 `SpatialGeometry` 不重叠。
   它先前不可用的根因是输入为空（`MatchWorld` 的球员数组从未填充），
   已由 `sync_to_world` 投影在册且在场球员修复；
4. **多体均衡求解的唯一实现是 `DefensePotentialFieldSolver`**：它输出的是
   防守目标与动作，属决策层的目标生成，与前三者的「事实描述」不同层。

规范落点：`docs/architecture.md` §4.3；执行入口：`docs/dev/current/plan.md`（D23）。

取舍代价：`PerceptionSystem` 仍是 `engine` 内的全场实现，而 `SpatialGeometry`
在 `physics`；两层不能共用一个球员级缓存。这是依赖方向的结果——`physics` 不能
依赖 `engine`，而全场聚合需要十人的完整视图。

> **修订注记（2026-09-21，shadow world 移除）**：第 2 条已执行（`physics/src/perception.rs` 删除）。第 3 条裁定的全场实现 `engine/src/world.rs::PerceptionSystem` 及其输入 `MatchWorld`/`sync_to_world`、行为测试 `match_world.rs` 已在 shadow world 移除中整体删除；全场防守拓扑（下沉护筐、空间真空、X-Out 轮转）不再是独立持久对象，而由第 4 条的 `decision/src/potential_field.rs::DefensePotentialFieldSolver` 在求解时从在场进攻球员位置即时涌现。因此第 3 条的「唯一实现」已由第 4 条吸收，`PerceptionSystem` 不再存在；第 1 条（`SpatialGeometry` 为球员级空间事实唯一实现）与第 4 条不受影响，仍然有效。

### ADR-016 · 接线扰动测试改用逐测试窗口，替代统一的 20000 tick 判据

**状态：accepted**

背景：`rules_complete_wiring.rs` 的判据「扰动一个规则系数，4 seed × 20000 tick，
≥3/4 指纹改变」在同一批授权行为变化（体力参数重校准、G6 换人系统）之后，
对三个小杠杆系数（`capability.risk_tolerance_gain`、`resolve.rebound.
boxout_distance_discount`、`resolve.rebound.putback_distance_discount`）
从通过降到 2/4。窗口加倍到 40000 tick 后仍是 2/4。
后续的飞行抛体化（v68）把 `modulation.morale_shoot_affinity`（情绪通道，
低频）也推入同一情形，并入本裁定的扩展判据。

测量：三个系数的消费链读取点（`contests.rs` 的拦截概率乘数与篮板距离折扣）
无任何改动；HEAD（改动前）14/14 通过。体力行为改变 → 换人改变 →
比赛轨迹的 RNG 抽样整体重排，这类「概率乘数上的小杠杆」本来就处在
2-3/4 的边缘可见区，轨迹重排把它们推到了 2/4。窗口加倍到 40000 tick
不恢复可见性；seed 扩到 6 后实测可见率：risk_tolerance 4/6、
putback ≥4/6、boxout 3/6（boxout 的最大可能杠杆受 boxout_bonus ≈ 0.225
限制在有效距离的 13.5%，归零已是最大杠杆）。

裁定：

1. 判据「≥3/4 @ 统一 20000 tick」在轨迹重排后不再能区分「已接线但杠杆小」
   与「未接线」，按文件纪律属于「判定标准需要重新裁定」，走本 ADR；
2. 三个失败系数的判定种子集从 4 扩到 6（42/1/7/100/999/31337），
   判据线定为 ≥3/6。它与 4-seed 判据 ≥3/4 的判别力等同：两种判据下
   「完全未接线」的失败形态都是指纹零改变（未接线系数在任何 seed 数下
   都是 0 改变）；≥3/6 线来自 6-seed 实测可见率，而非从 3/4 折算；
3. 该判据只适用于这四条测试；其余 10 条测试在 3/4 @20000 下保持原判据。
   两种判据在文件内并列，每条测试注明自己用的判据；
4. 禁止借本裁定放宽任何阈值或删除任何测试；若未来这三个系数的消费链
   被改动，须重新测量并回归 3/4 判据。

### ADR-017 · 球的全场物理约束以真实重力为基线（四步路线，第 1–3 步已实施）

状态：accepted

背景：打铁反弹的落点采样与入射完全无关（均匀距离 + 任意扇形方向），能量无
守恒；追问后盘点发现更深层的结构问题——传球/投篮/篮板飞行的 z 是「线性
插值 + 正弦弧」的动画函数，重力（32.17 ft/s²）只在松球路径生效；球在场上的
大部分时间不服从任何物理。

裁定（四步路线；第 1–3 步已实施，第 4 步待办）：

1. 第一步：飞行抛体化。传球/投篮/篮板飞行的 z(t) 全部由
   `ProjectileArc`（domain/projectile.rs）产生；飞行时长由「请求弧顶 +
   两端高度」闭式解出，受球速包络与动作窗口双重约束。正弦弧时代的六个
   零消费字段（pass_speed/inbound_pass_speed/shot_speed/ball_arc_multiplier/
   shot_arc_solve_iterations/rebound_min_arc_ft）删除，新增
   `pass_peak_distance_factor`。出手速度自然达到真实量级（25 ft 三分
   ≈ 37 ft/s）。松球路径本来就是真物理，不动。
2. 第二步：触筐物理。按入射方向采样触筐位置，反弹初速 = 入射 × 衰减、方向 =
   镜像反射 + 受控散射，落点由物理自然产生，加距离分布校准门。
   实施记录（2026-09-21，v69）：接触点在近筐沿受控扇形上采样（新增
   rim_radius_ft 与 4 个 rim_contact 参数），水平恢复系数随接触角从硬碰
   0.62 渐变到擦筐 0.35，竖直弹起 = 入射下落速度 × 0.35；距离采样时代
   的 9 个字段（rebound_short/long_min-max、angle_range、flight_base、
   flight_distance、distance_scale、rebound_peak）删除；罚球打铁与跳投
   打铁同一条反射链路；分布门（crates/physics/tests/rim_contact.rs）
   实测：近筐打铁 p50≈1.3 ft、中距≈5.9、三分≈9.4，混合出手 6 ft 内
   ≈0.63，方向回弹份额 ≈0.85；ORB% 从 0.538 向真实 0.245 靠拢到 0.433。
   出手弧顶校准（shot_peak_distance_factor 0.25→0.04）实测后暂缓：
   三分封盖率 3.7%→7.3%、3P% 中位 34.0→27.0 越带，出手时平均
   make_probability 三配置完全一致（0.307/0.308），封盖概率高度惩罚
   归零实验不改变封盖数——机制未明前不发布该行为变化，证据在
   rules.rs 的 shot_peak_distance_factor 注释。
3. 第三步：球-篮板/球-人碰撞几何（篮板作为矩形碰撞面、球穿人改为弹开）。
   实施记录（2026-09-21，v72「统一球员接触模型」）：(a) 篮板碰撞面——
   打铁按瞄准几何双通道路由（越过筐→触板镜像反弹、落点由抛体解出；
   否则维持近筐沿反射），板几何（距底线 4.0 ft、宽 6.0 ft、下沿 9.5 ft、
   上沿 13.0 ft、恢复系数 0.55）进规则通道；(b) 自由球-人碰撞——篮板飞行
   与地板球对球员身体（接触半径 = 人体 1.0 + 球 0.4 ft）弹开，摸高门控、
   同飞行同人只弹一次，快球（> 12 ft/s）不可半途收下，盖帽初速改为入射
   水平速度 × 0.45；(c) 传球碰撞化——删除出手时刻一次性拦截裁定与
   `Pass.intercept` 载荷，改为逐 tick 全防守者接触检测 + 结果分类（抢断/
   拨掉/轻擦，沿用 `intercept_*` 参数），高吊传受摸高门控。黄金哈希重冻结
   v72（0xb3685f078e05bfc9）；校准：乱战出手增多使三分 make_probability
   0.308→0.294、基准 0.34→0.36 回调（3P% 中位 31.0），ORB% 0.474→0.294
   （真实 0.245）。物理实现见 `crates/physics/src/ballistics.rs`、
   `crates/engine/src/match_engine/free_ball.rs`、`contests.rs`。
4. 第四步：分布校准门进评判器 + 盲区登记。

行为影响：黄金哈希重冻结 v68（0x820c9b27412c65cc）；比赛节奏整体后移
（长传弧顶抬高 → 首罚球 14792→21813 tick），8-seed 统计仍在带
（total_p50=172.5、3P% 中位 34.0）；接线判据的两处连锁按 ADR-016 既有
裁定处理（morale_shoot_affinity 并入扩展判据；risk_tolerance_gain 改用
饱和扰动 base=0.95/gain=0，杠杆比旧扰动强约 3 倍）。
第二步行为影响：黄金哈希重冻结 v69（0x7888d577ac33b69b）；8-seed 统计
在带（total_p50=165.0、3P% 中位 30.2）；morale_shoot_affinity 的指纹
可见度在触筐重排后降到 1/6（杠杆 = morale_bias ±0.05–0.12 × 0.35 ≈
±0.04，远小于候选间效用差），改用饱和量级扰动 3.0（±0.15–0.36）恢复
可观测。

### ADR-018 · 节间开场必须显式重建球的可取性；计时与违例要求控球

状态：accepted（2026-09-22）

背景：seed 14 全场在第 4 节开场出现「活球流程 + 停球状态」并存 201 tick，
无人能触球，后场计时按球位累加到 8 秒判出 `EIGHT_SECOND_BACKCOURT`，
责任人派生全空，产生 `turnover_player_id=null` 的失误终结——账本
`TURNOVER_CONSERVATION` 违规 + 评判 `TURNOVER_ACTOR_CONSISTENCY` Hard，
16-seed 矩阵以账本门失败退出。因果链（全部有帧级证据）：第 3 节末 0.3 秒
持球被拨掉（`BALL_POKED_LOOSE`，`last_passer_id` 已清）→ 节末
`settle_ball_for_period_break` 把在飞松球结算为停球（停表期间球也应是
停的，该步本身正确）→ 下一节开场只恢复流程、不重建球权程序。两个结构
缺陷叠加：(a) 节间开场没有球权程序，球态直接跨节携带；(b) 后场计时只看
「活球流程 + 球在后场」，不要求任何一方控制球。同类缺陷在 evidence
§32.14 有前科（当时成因未查）。

裁定：

1. 节间开场（`sync_game_flow`）必须显式回答「球现在谁能拿」：球已由在场
   球员持有（Held/Drive/InboundReady/InboundTransfer）时原样保留（常规
   路径行为不变）；其余状态按节末保留的进攻方进入显式发球程序
   （`begin_inbound`，从 `start_inbound_transition` 抽出的公共主体；
   不翻转球权、不重复回合结算）。`settle_ball_for_period_break` 保持
   不变——节末结算停球是对的，错在开场不重建。
2. 后场 8 秒的计时与违例都要求进攻方实际控球：松球、篮板、停球、投篮
   飞行不累加、不判违例；时钟层（`phases.rs`）与决策约束层
   （`ConstraintContext::offense_has_possession` +
   `eval_backcourt_clock_world`）用同一谓词，不另立第二口径。球在后场但
   无控制方时计时保持原值（控球中断暂停计数，重新建立控制后继续）。
3. 归因链以状态为唯一事实源：`LooseBall`/`RimRebound` 携带
   `last_touch_player` 载荷（构造点写入物理最后触球人），`Dead` 经停球
   结算继承；`current_turnover_player_id` 对松球态优先读载荷，旁路字段
   `last_passer_id` 仅作旧态回退。失误终结缺责任人不再可能静默产出：
   新增 L1 不变量 `LIVE_FLOW_DEAD_BALL`（活球流程下球必须可取）与
   `TURNOVER_ACTOR_MISSING`（失误终结必须带责任人），违反即该 tick
   Hard 缺陷。
4. 监控同步：账本五式与 Hard 门进入套件（`stats_baseline` 的 16-seed
   全场断言、`attribution` 的违例归因 16-seed 全场回归），不再只能靠
   人工跑矩阵发现此类缺陷。

行为影响：黄金哈希不变（v73 = `0xee0429df2bfdde79`，两处行为变更均在
节间/松球路径，不在开局窗口内；该常量由 fe5e233 重冻结但当时未登记，
v73 条目补登）。16-seed 矩阵修复前后对比：Ledger 1→0、Hard 缺陷 1→0、
Realism Index 0.000→0.998、中位总分 163→176、3P% 中位 30.3→32.3；
seed 14 全场帧流「活球流程 + 停球」201 tick → 0。吞吐无退化
（seed42 6530 / seed1 6365 ticks/s，带内）。实现见
`crates/engine/src/match_engine/{flow,transitions,phases}.rs`、
`crates/decision/src/constraint/{mod,evaluate}.rs`、
`crates/domain/src/flow.rs`、`crates/invariants/src/{lib,taxonomy}.rs`；
证据过程存档于 `output/review16/`（诊断帧流与逐 seed 归因报告）。

### ADR-019 · Play 规则模型：回合级规则化行为覆盖，与 System 分级

状态：accepted（2026-09-22）

背景：tactics.md §2.2 已裁定 System 级档案不设动作序列与触发条件（TA1），
回合级行为只剩体系语境与球员自主决策两层，缺一层可配置的「有组织回合行为」：
高位挡拆后的顺下、清空一侧后的单打、底角落位后的转移，这些是真实篮球里
教练组准备的行为面，目前只能散在效用管线的通用权重里，无法按回合语境启用。
直接引入动作序列会退回剧本（宪章 C1 禁止），需要一层新的形式化。

裁定（语义登记于 `docs/tactics.md` §2.2.4、§2.5）：

1. **两级 kind**：战术档案分 `System | Play` 两级。System 维持既有语义
   （阵型、槽位、能力需求，全场语境）；Play 是回合级规则化行为覆盖，
   由选板器在回合内按触发谓词启用，窗口化生效。两库经 kind 分流，互不混用。
2. **Play 的三个行为通道**：`carrier_preferences`（候选族效用加项）、
   `inhibitions`（soft 效用惩罚 / hard 候选移除）、`rules[].then.verb`
   （目标生成偏置与动作窗口类型）。三通道全部经效用管线与约束层生效，
   Play 不直接执行动作、不写坐标、不声明顺序；hard 抑制必须保留合法出路
   （抑制后候选集非空），由校验器拒绝违反者。
3. **谓词与动词封闭枚举**：触发谓词（球态/几何/场输出三类）、规则谓词、
   动作词表都是封闭枚举，档案字符串必须命中枚举，否则校验拒绝。
   几何谓词全部由 `SpatialGeometry`（ADR-015 单一事实源）派生。
4. **场输出谓词以滞回为前提**：消费势能场输出（`threat_ratio` /
   `void_ratio` / 涌现标签）的谓词与选板触发必须先过滞回稳定通道
   （双阈值 + 最小保持时间，`tactics.md` §2.5）；稳定态逐防守人维护，
   不新建持久世界对象（ADR-015 边界不变）。未经滞回的场量不得进入
   开关型判定，否则逐 tick 抖动会引发选板震荡。
5. **选板器是纯函数**（TA4）：`(playbook, context) -> Option<PlayActivation>`，
   每 tick 评估触发谓词，至多一个 Play 激活；窗口结束或回合终结后进入
   冷却。互斥关系不在首轮引入（OQ-6），校准显示需要时另行登记。

追加裁定（schema 与 selector/executor 对照后补足）：

6. **所有 `when` 非空并按 AND 求值**：trigger 和 rule 均拒绝空 `when`；触发项
   与规则条件每个决策 tick 重新求值，只有全部谓词为真才匹配。空列表的数学
   恒真语义不得成为档案的隐式无条件触发或规则。
7. **同一 Play 的多触发选择**：先在单个 Play 内选取命中项中谓词数最多者，
   同分取声明顺序首项；所选项决定 `window_seconds` 和 `cooldown_seconds`。
   不同 Play 仍按各自最高匹配谓词数排序，最高分同分时由注入随机数等概率
   选择一个 Play。
8. **偏好跨层累加**：Play 顶层偏好在窗口内持续生效，命中规则的偏好在当 tick
   条件成立时生效；相同 `DecisionActionFamily` 的生效 bonus 相加。各列表内部
   同族重复仍为 schema 错误。
9. **规则槽位冲突**：一个 Play 中多条规则若引用相同 `then.slot`，即使其 `when`
   当前看似互斥，也由 schema 校验拒绝。这样引擎每 tick 不会对同一目标槽位接收
   多个规则动作；不同槽位的规则可以同 tick 同时生效，按声明顺序返回结果。
10. **硬抑制检查边界**：schema 校验确认 hard 抑制未覆盖封闭动作族全集；该静态
    检查不保证未抑制族会在当前状态生成，也不保证其通过运行期约束。若实际候选
    集在 hard 抑制与约束筛选后为空，决策管线必须显式失败。引擎装配还负责检查
    `then.slot` 对当前 System 的引用存在性。
11. **JSON 表示**：示例遵循 `PlaySpec::from_json` 的 tagged enum 格式：`kind` 为
    小写 `play`；谓词使用 `{"predicate": "..."}` 对象，参数如 `r`、`side` 平铺在
    同一对象中。动作族与动词保持 Rust schema 的 PascalCase 字符串形式。

以上补充只裁定已批准 Play schema 与 #12 集成接口所必需的运行语义；PlayVerb 与
PlayPredicate 词汇表保持封闭，不增加动作能力。
