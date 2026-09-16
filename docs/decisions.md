# NBA-Sim · 设计决策登记

> 本文档登记影响多个契约、具有取舍关系、且需要保留理由的设计决策。
> 它不是当前状态报告，也不是实施计划；当前实现状态见 `docs/dev/status.md`，实施计划见 `docs/dev/current/plan.md`。

## 0. 决策治理

一条内容只有在同时满足以下条件时才登记为决策：

- 存在两个以上合理方案；
- 选择会影响多个模块或后续迁移成本；
- 未来读者需要知道“为什么不是另一个方案”。

决策状态：

- **accepted**：已经成为设计约束；契约文档只保留结论；
- **proposed**：正在评估，不能被实现当作稳定契约；
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

防守档案不能只改变展示字段或站位标签。防守方案必须通过责任分配、轮转、施压、协防或执行重校验改变可观测行为；中性档案必须能复原行为中性基线。

最小执行范围由开发计划决定，完整责任图不是当前契约的隐含承诺。

规范落点：`tactics.md` §3–§5；验收引用 `quality.md` §6。

### ADR-005 · 名册顺序不是身份

**状态：accepted**

数组顺序、ID 中的序号和隐式索引不能决定球员能力、角色、首发、持球人或行为。球员身份来自档案，行为差异来自能力、倾向、规则和比赛状态。

规范落点：`attributes.md` §2、`tactics.md` §1–§4；守卫：`scripts/check_no_index_identity.py`。

### ADR-006 · 开发阶段不等于完成状态

**状态：accepted**

G/M/D/L/F/Round 是不同层级的计划或执行编号，不是完成度。任何阶段只有在其门的全部证据存在时才能标记完成；历史执行记录不能作为当前完成状态。

当前状态唯一来源：`docs/dev/status.md`。当前待办唯一入口：`docs/dev/current/plan.md`。

## 2. 待决策事项

### ADR-007 · 是否完成 BallControl × BallMotion 的内部迁移

**状态：proposed**

目标方案已在 `docs/dev/gap.md` §5 定义。接受前必须证明：

- 外部 `BallState` API 不发生无授权行为漂移；
- 所有归属、最后触球和死球责任信息仍可从一个权威状态派生；
- 迁移后不存在旧旁路字段继续承载真相。

### ADR-008 · 完整防守责任图的范围

**状态：proposed**

当前已接入防守方案对几何/部分结果的影响，但 `switch/drop/hedge/recover` 的完整意图—执行—事实链尚未形成统一证据包。是否把完整责任图纳入下一周期，由本周期 `D9`（防守责任链）与 `D11`（情景矩阵）的验收结果决定。

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
   契约。
3. **唯一已知消费者是战术 planner 的进攻几何参考点**（`plan_possession_targets_with_rules`
   的 `carrier_idx` 实参）。无 handler 球态（`ControlTransfer` / `LooseBall` / `RimRebound`
   期间的阵地与攻防转换布置）下，planner 以 `ball_pos_3d`（球的实际位置）为参考，
   不假设存在持球人；这与 D9.1 已删除的"按持球人槽位硬编码几何"一致——几何参考
   应跟随球，不跟随一个虚构的持球人。
4. 由此 `carrier_idx` 成为纯冗余：handler 永远可从 `BallState` 派生，下标投影不再是
   任何读取方的输入。删除是上述语义落地后的机械结果，而非独立的取舍。

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
2. 架构设计中宣称的“13 阶段窄签名函数式管线”无法彻底落地；
3. 并行与向量化计算受阻。

裁定：**将 MatchEngine 解构为纯数据世界（World）与无状态系统管线（Systems）**：
1. **纯数据世界（Component World）**：世界仅包含扁平的纯数据组件，包括 `Transform`（坐标/朝向/速度）、
   `Kinematics`（加速度/抓地力/动量）、`PlayerState`（体能/犯规/士气）、`BallComponent`（三维弹道与归属）、
   `MatchClock`（游戏时钟与进攻时钟）和 `GameFlowLedger`（比分与账本）。
2. **纯函数式系统（Stateless Systems Pipeline）**：主循环每个 tick 严格执行以下阶段，
   阶段之间仅通过窄数据契约流动：
   - `PerceptionSystem`：空间 Voronoi 拓扑与防守压迫密度计算；
   - `DecisionSystem`：基于意图评价的动作决策（纯函数，无状态突变）；
   - `PhysicsSystem`：刚体运动、动力学积分与弹道解算；
   - `OfficiatingSystem`：规则越界裁决、犯规吹罚与账本守恒审计；
   - `EventDispatcher`：不可变事件流与渲染帧打包。
3. **迁移策略**：采取渐进式解耦。`MatchEngine` 降级为单纯的外部协调器（Facade），
   内部逐步将各逻辑块抽离为独立 crate（`crates/engine-core`、`crates/physics`、`crates/decision`），
   保证每一步重构均有 16-seed 黄金哈希与 G-STATS 基准守卫。

规范落点：`docs/architecture.md` §4；执行入口：`docs/dev/current/plan.md`（D22–D28）。

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
