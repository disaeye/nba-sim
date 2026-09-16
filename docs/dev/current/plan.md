# 当前周期计划 · 从第一性原理构建连续博弈与空间动力学引擎

> 文档类型：当前未完成工作的执行设计与落地计划。
> 规则：本文件不复述历史已归档 Round（D0–D21 见 `cycles/`）；任务关闭后将结论写入 `docs/dev/status.md`。
> 依赖入口：稳定设计见 `docs/architecture.md`、`docs/tactics.md`；架构决策见 `docs/decisions.md`（ADR-011, ADR-012）；当前状态见 `docs/dev/status.md`。
> 编号范围：本周期任务块使用 `D22–D28`。

## 0. 周期背景与总体目标

上一周期（`20260916_systematization`，D14–D21）已成功实现只读快照投影（EngineSnapshot）、彻底解耦 `carrier_idx`（ADR-010）、完成 step_inner 初步切分、实现防守方案 v2 初步接线、接回 clutch 与 drive 核心规则字段、建立 FIBA 交替拥有机制，并保持全套 45 套测试用例与 G-STATS 16-seed 统计基准全绿。

但在第一性原理的深度审视下，引擎依然存在深层次结构瓶颈：

1. **巨石单体债务（God Class）**：`match_engine.rs` 仍有 6,600+ 行，集中管理时钟、名单、弹道、规则判定与事件流，无法彻底落实无副作用的纯函数式阶段管线；
2. **离散概率与瞬移假象（Discrete Rolls & Teleportation）**：移动依靠目标坐标线性插值，突破与防守依赖经验骰子分支，缺少真实的身体加速度、惯性冲量与制动极限；
3. **战术空间僵化（Rigid Slot Spacing）**：跑位依赖静态坐标槽位插值，缺乏基于空间重力与防守压迫密度（Voronoi）的自主动态拉扯；
4. **动作微观时序缺失（Micro-Action Timings）**：投篮、传球与起跳缺乏细粒度运动学阶段划分，判定窗口粗糙。

**本周期总体目标**：彻底重构底层动力学与架构，推动引擎迈向**纯数据管线（ECS Dataflow）、连续势能场动力学（Force Fields）与空间几何拓扑（Voronoi Spacing）**驱动的现代化仿真系统。

## 1. 执行纪律与红线守卫

1. **零功能回退原则**：重构每一步必须保持 `./scripts/run-tests.sh` 45 套测试套件守卫全绿；
2. **统计健康带守卫**：16-seed 核心分布门必须保持在健康带内（`total_p50 ∈ [205, 225]`，`fg_pct ∈ [0.42, 0.52]`，`three_pct ∈ [0.30, 0.40]`）；
3. **因果单调性检验**：任何物理与决策参数的接入，必须附带单调性因果扰动测试与断路必红测试；
4. **渐进式迁移（Strangler Fig Pattern）**：新建核心模块，通过门面（Facade）模式逐步剥离 `MatchEngine` 内部逻辑，禁止一次性休克疗法式重写。

## 2. 任务依赖关系图

```text
D22 纯数据世界与系统管线解耦 (ECS Dataflow) ────────────► D28 周期出口与基准收敛
  │                                                            ▲
  ├─► D23 空间 Voronoi 拓扑与防守压迫感知 (PerceptionSystem)   │
  │     └─► D26 弱侧协防与防守责任链闭环 (Help Defense)         │
  │                                                            │
  ├─► D24 连续受限势能场动力学与惯性制动 (PhysicsSystem)        │
  │     └─► D25 细粒度微观动作链状态机 (Kinematics)             │
  │                                                            │
  └─► D27 零消费剩余字段处置与全规则闭环 ──────────────────────┘
```

---

## 3. D22 · 纯数据世界与系统管线解耦 (ECS / Pipeline Dataflow)

### 3.1 核心问题与设计目标

`MatchEngine` 单体持有过多内部可变状态。目标是将其职责解构为**纯数据存储（World）**与**无内部状态的执行管线（Systems）**，将主循环拆解为单向数据流。

### 3.2 架构设计

新建 `crates/engine/src/world.rs` 与 `crates/engine/src/systems/`：

- **`MatchWorld` 纯数据实体世界**：

  ```rust
  pub struct MatchWorld {
      // 刚体与运动学组件 (10 名场上球员)
      pub positions: [Vec2; 10],
      pub velocities: [Vec2; 10],
      pub facings: [Vec2; 10],
      pub physical_limits: [PhysicalLimit; 10], // max_accel, max_speed, traction
      // 比赛态与能力组件
      pub player_states: [PlayerRuntimeState; 10], // stamina, fouls, morale, fatigue
      pub attributes: [PlayerAttributes; 10],
      // 篮球动力学组件
      pub ball: BallComponent, // pos_3d, vel_3d, trajectory_kind, handler_id, last_touch
      // 时钟与账本组件
      pub clock: MatchClockComponent, // game_clock, shot_clock, period, possession_time
      pub ledger: MatchLedgerComponent, // score, team_fouls, timeouts, possession_arrow
      // 规则与配置
      pub rules: GameRules,
  }
  ```

- **纯函数式主循环管线**：

  ```rust
  // 单 tick 严格按阶段无环单向流动
  let perceptions = PerceptionSystem::evaluate(&world);
  let intentions  = DecisionSystem::decide(&world, &perceptions);
  let physics_res = PhysicsSystem::step(&mut world, &intentions, dt);
  let events      = OfficiatingSystem::arbitrate(&mut world, &physics_res);
  EventDispatcher::publish(&world, events);
  ```

### 3.3 验收门

- `MatchWorld` 独立编译并实现 `Clone` 与零堆分配快照；
- `step_inner` 代理至四大 System 执行；
- `match_engine.rs` 行数降低至 2,500 行以内；
- 45 套测试全绿，黄金哈希可追溯。

---

## 4. D23 · 空间 Voronoi 拓扑与防守压迫感知系统 (PerceptionSystem)

### 4.1 核心问题与设计目标

现有战术跑位依赖预设槽位绝对坐标（Static Slots），无球球员无法感知局部空间的空旷程度。目标是引入 **2D 约束沃罗诺伊图（Bounded Voronoi Diagram）** 与 **防守压迫密度（Defensive Contestation Density）**。

### 4.2 算法模型

1. **场上 Voronoi 面积拓扑**：
   在 $94 \times 50\text{ ft}$ 半场边界内，以 10 名球员位置为发生点（Seeds），生成几何多边形剖分。
   - 进攻球员 $i$ 的独立控制面积记为 $A_i$；
   - 当 $A_i > A_{threshold}$（例如外线单人控制面积 $\ge 120\text{ sq ft}$），标记为绝对空位（Open Window）；
2. **防守压迫度场函数（Contestation Field）**：
   对任意场上点 $\mathbf{x}$，防守压力由防守球员 $j$ 的距离与朝向投影衰减叠加：
   $$D(\mathbf{x}) = \sum_{j \in Def} \frac{w_j \cdot \max(0, \mathbf{f}_j \cdot (\mathbf{x} - \mathbf{p}_j))}{\|\mathbf{x} - \mathbf{p}_j\|^2 + \epsilon}$$
   其中 $\mathbf{f}_j$ 为防守者面朝向量，$w_j$ 为防守臂展与防守智商加权。

### 4.3 验收门

- 建立 `crates/decision/src/perception.rs`；
- 提供空位检测测试：当防守人收缩至油漆区时，底角进攻球员的空位评级单调增加；
- 传球决策接入空间增益梯度，外线空位出手率符合现代篮球特征。

---

## 5. D24 · 连续受限势能场动力学与惯性制动系统 (PhysicsSystem)

### 5.1 核心问题与设计目标

现有球员移动为目标点线性插值，突破与失位通过概率分支强行拉扯。目标是建立基于牛顿力学的**受限连续动力学模型**。

### 5.2 物理公式与动力学方程

球员加速度 $\mathbf{a}$ 由意图驱动力、环境斥力与抓地力衰减共同决定：
$$\mathbf{F}_{total} = \mathbf{F}_{drive} + \mathbf{F}_{spacing} + \mathbf{F}_{boundary}$$
$$\mathbf{a} = \frac{\mathbf{F}_{total}}{m}, \quad \|\mathbf{a}\| \le a_{max}(\text{agility, strength})$$
速度更新受地面最大静摩擦力（Traction Limit）约束：
$$\mathbf{v}_{t+1} = \mathbf{v}_t + \mathbf{a} \cdot \Delta t$$
$$\text{当 } \Delta \theta(\mathbf{v}, \mathbf{v}_{desired}) > 90^\circ \text{ 时触发急停变向，施加制动距离惩罚。}$$

### 5.3 验收门

- 建立 `crates/physics/src/kinematics.rs`；
- 消除任何坐标瞬移（瞬时位移速度超过物理最大极限 $v_{max} = 28\text{ ft/s}$ 即报警断言）；
- 变向制动与急停产生真实的减速滑行过程；
- 突破判定转为纯几何位置切入与接触抗衡，告别单一概率投掷。

---

## 6. D25 · 细粒度微观动作链状态机 (Action Kinematics)

### 6.1 核心问题与设计目标

投篮、盖帽与抢断判定缺乏物理时间窗口，导致前端呈现滑冰感，判定缺乏因果连贯性。

### 6.2 动作时序状态机

将投篮分解为 4 个不可逆物理阶段：

1. **合球阶段（Gather, 0.15s - 0.25s）**：双脚起跳步法调整，此时防守人可尝试切球抢断（Poke/Strip），不计投篮犯规；
2. **起跳升空（Elevation, 0.20s - 0.35s）**：重心向上积分，高度 $z(t)$ 抬升。此时身体接触判定为投篮犯规，盖帽判定窗口开启；
3. **最高点出手（Release, 0.05s）**：篮球脱手赋予初始抛物线初速度 $\mathbf{v}_0$，确定投篮品质与干扰修正；
4. **随摆与落地（Follow-through & Landing, 0.20s - 0.30s）**：球员下落恢复平衡，落地空间受规则保护（落地垫脚犯规判定）。

### 6.3 验收门

- 投篮动作链状态机在 `crates/domain/src/action.rs` 显式建模；
- 盖帽事件只能在 Elevation 阶段触发（落地后封盖必报守卫错误）；
- 前端 render frame 暴露 `action_phase` 字段，支持动作姿态同步。

---

## 7. D26 · 弱侧协防与防守责任链闭环 (Defensive Chain)

### 7.1 核心问题与设计目标

D17 仅打通了持球点 16 英尺内的挡拆对策（Drop/Switch/Hedge）。全场阵地战中的弱侧轮转（Low-man Help）、X-Out 轮转与底角补位依然断链。

### 7.2 责任转移矩阵

- **第一责任人（On-ball Contain）**：领防持球人；若被突破超车一个身位，触发协防呼叫；
- **第二责任人（Rim Protector / Low-man）**：禁区底角内线收缩护筐，放弃原对位人；
- **第三责任人（Weak-side Sink / X-Out）**：弱侧侧翼下沉同时兼顾底角与 45 度两人，执行轮转回防；
- 建立状态机交接超时与失误率判定，体能与防守智商直接决定轮转到位的时滞（Latency）。

### 7.3 验收门

- 突破成功时，弱侧防守人向篮下位移的响应率 $\ge 90\%$；
- 建立弱侧轮转与底角空位三分出手的反事实因果测试；
- 保持犯规率与内线终结比例在 NBA 基准带内。

---

## 8. D27 · 零消费剩余字段处置与全规则闭环

### 8.1 核心问题与设计目标

`UNIMPLEMENTED_RULE_FIELDS` 中仍残留 6 个未接线字段（`risk_tolerance`、`defensive_rebound_boxout_bonus` 等）。目标是彻底清空未消费清单，完成 100% 规则物理接线。

### 8.2 接线方案

1. `risk_tolerance` ➔ 注入 `PerceptionSystem`：高风险偏好球员倾向于尝试传球穿越高防守密度通道与赌博抢断；
2. `defensive_rebound_boxout_bonus` ➔ 注入卡位动力学：增强防守篮板人的斥力场半径与卡位阻力；
3. `offensive_rebound_putback_bias` ➔ 注入进攻篮板后的二次进攻即时出手倾向；
4. `help_defense_awareness` ➔ 决定 D26 协防触发的时滞帧数；
5. `post_defense_physicality` ➔ 决定低位背身对抗时的位移衰减系数；
6. `transition_leakout_chance` ➔ 决定投篮出手瞬间快下球员的起跑提前量。

### 8.3 验收门

- 清空 `UNIMPLEMENTED_RULE_FIELDS` 数组；
- 新增 `crates/engine/tests/rules_complete_wiring.rs`，对 6 个字段提供单调响应测试；
- 编译期静态断言确保 GameRules 无死字段。

---

## 9. D28 · 周期出口、全矩阵回归与归档

### 9.1 出口条件

1. **测试套件全绿**：`./scripts/run-tests.sh`（包含原有 45 套及新增测试）100% 通过；
2. **黄金哈希受控演进**：更新受控的黄金哈希快照，附带 16-seed 统计分布对比报告；
3. **架构度量达标**：`match_engine.rs` 瘦身成功，无任何单文件超过 3,000 行；
4. **文档治理闭环**：`python3 scripts/check_docs.py` 0 警告 0 错误，完成向 `status.md` 的成果转交。
