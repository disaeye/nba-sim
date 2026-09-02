# NBA-Sim · 系统架构

> 版本：v1.1（2026-09-01 对抗性审查修订：引用重建、效用代数复合律与 RoleFit 移除、时序不变式表述修正）
> 定位：系统组织与模块交互的**架构契约**——分层、数据流、状态机、管线、决策、语义/裁决、多联赛
> 上游文档：`docs/charter.md`（愿景与目标宪章；其 §4 宪章条款镜像为本文档 §1.3）
> 关联文档：`docs/quality.md`（检测与评判体系）、`docs/attributes.md`（球员属性契约）、`docs/tactics.md`（阵容与战术契约）、`docs/status.md`（现状与差距）
> 适用范围：`crates/*` 全部子系统
> 修订纪律：本文档**零现状快照**——只写"应该是什么"，不写"现在是什么"；现状审计与差距矩阵一律去 `docs/status.md`

---

## 0. 文档目的

本文档回答三个问题：

1. **系统如何组织** —— 分层架构与依赖规则（§1–§2）；
2. **状态如何流转** —— 球权状态机、主循环阶段管线、决策系统（§3–§5）；
3. **如何支撑多联赛** —— 规则档案与约束分轴（§6）。

阅读顺序：§1（分层）→ §2（数据流）→ §3（球权状态机）→ §4（阶段管线）→ §5（决策）→ §6（语义/裁决/多联赛）。

---

## 1. 总体架构

### 1.1 设计原则

| 原则 | 一句话表述 |
|------|-----------|
| **P1 单一事实源** | 球的归属只能从一个字段推导，其余全是派生只读视图 |
| **P2 不变量即代码** | 每 tick 在引擎内校验物理/篮球底线，违反立即暴露（详见 quality.md） |
| **P3 显式阶段管线** | 主循环是显式阶段序列的调度，顺序由调度器与窄签名保证而非注释 |
| **P4 意图-执行重校验** | 动作在执行点重新经过约束过滤，快照只用于生成候选 |
| **P5 确定性优先** | 相同种子必须产生完全相同的世界，否则一切检测都失去意义 |
| **P6 校准通道唯一** | 一切影响行为的数值必经 `GameRules`；公式进代码、系数进规则、调参走 JSON 覆盖 |
| **P7 能力涌现** | 一切场上行为是 `(球员能力, 规则, 比赛状态)` 的纯函数；禁止剧本化战术与经验常数（宪章 C1） |
| **P8 逐回合评判** | 真实度判断必须落到每一个回合、每一个阶段（宪章 C2，详见 quality.md） |

### 1.2 分层视图

```
┌─────────────────────────────────────────────────────────────┐
│  应用层 (Application)                                        │
│  cli / debug-server / bball-wasm / 回放、统计与评判工具      │
│  - 只读快照 + 生命周期命令，不拥有比赛真相                    │
├─────────────────────────────────────────────────────────────┤
│  编排层 (Orchestration)        crates/engine                │
│  MatchEngine / MatchService                                 │
│  - 唯一认识所有子系统的层                                    │
│  - 只负责：阶段管线调度、事实归集、不变量触发、快照输出        │
│  - 不含任何物理公式、决策权重、犯规规则                       │
├──────────┬──────────┬──────────┬──────────┬─────────────────┤
│ 决策     │ 裁决     │ 语义     │ 物理     │ 不变量           │
│ decision │officiating│semantics│ physics │ invariants       │
│ 候选→约束 │接触→犯规 │物理→篮球 │刚体/弹道 │tick 底线校验     │
│ →效用→采样│违例→罚则 │意义解释  │空间查询  │                 │
├──────────┴──────────┴──────────┴──────────┴─────────────────┤
│  领域层 (Domain)               crates/domain                │
│  场地 / 时钟 / 阶段 / 事件 / 动作窗口 / 规则参数              │
│  - 只描述"比赛是什么"，不描述"比赛怎么打"                     │
│  - 依赖图最内层，零业务依赖                                   │
├─────────────────────────────────────────────────────────────┤
│  协议层 (Protocol)             crates/protocol              │
│  StreamTick / RenderFrame / FrameEvent / DecisionDebug      │
│  - 引擎对外的唯一数据契约，UI/回放/审计共用                   │
└─────────────────────────────────────────────────────────────┘
```

**依赖规则（编译期强制）**：
- 上层可依赖下层，下层**禁止**反向依赖；
- `domain` 不依赖任何其他内部 crate；
- `physics` 只依赖 `domain`；
- `decision` / `semantics` 可依赖 `domain` + `physics`（读快照）；
- `officiating` 另依赖 `semantics`（语义事实的消费方，与 §6.1 单向数据流一致）；
- `engine` 可依赖全部（含 `invariants`：`step()` 包装器每 tick 调用 `check_tick`，见 quality.md §1.1）；
- `invariants` 只依赖 `protocol`（校验输出帧，不接触引擎内部）；
- `protocol` 只依赖 `serde`。

### 1.3 红线宪章（`charter` §4 镜像，不可触碰）

以下条款源自 `docs/charter.md` §4，是本文档与一切实现的**不可修订红线**：违反任一条的改动 = 打回，无论动机多好。完整论证见 `charter`；本节只列工程执行细则与机械守卫——红线没有守卫只是愿望。

| 条款 | 内容 | 机械守卫 |
|------|------|---------|
| **C1 无硬编码** | 一切影响行为的数字必经规则/数据通道；一切行为是 `(能力, 规则, 状态)` 的纯函数，禁止剧本化战术与"让画面像"的经验常数 | 内联常数 grep 守卫（design.md §3.1 M7 阈值）+ 能力扰动测试（quality.md §6） |
| **C2 评判落地** | 真实度判断逐回合、逐阶段产出裁决；禁止以"多场模拟原始结果统计"作为真实性评判；仅允许把"裁决结果"聚合为真实度指数 | 回合/阶段评判工件逐条落盘（quality.md §2.2–2.4） |
| **C3 约束分轴** | 物理约束严格遵守且与联赛无关（可配置校准）；语义/规则约束按联赛档案 `LeagueProfile` 参数化（NBA/FIBA/NCAA），灵活可配 | LeagueProfile 切换验收（§6.3、design.md §3.1 M10） |
| **C4 确定性** | 相同种子 → 逐 tick 完全相同的世界；一切评判、归因、校准对比的前提 | 黄金哈希（quality.md §5）入库且必须常绿 |

> 条款与原则的映射：P6 是 C1 的数值通道，P7 是 C1 的行为要求，P8 即 C2，C3 落地为 §6.3 的规则档案参数化，C4 是 P5 的宪章级强化。

---

## 2. 核心数据流（单 tick）

```
         ┌──────────────────────────────────────────────┐
         │  MatchEngine::step()                         │
         └──────────────────────────────────────────────┘
                          │
   ┌──────────────────────┼──────────────────────┐
   ▼                      ▼                      ▼
[1] 时钟与生命周期     [2] 运行时约束求值     [3] 动作窗口推进
   current_time/shot      (24秒/出界/死球)      ActionTimeWindow
   _clock 推进             只读快照              锁/解锁运动学
   │                      │                      │
   └──────────────────────┼──────────────────────┘
                          ▼
                [4] 决策（活球，或死球发球就绪；到达决策间隔）
                   候选生成 → 硬约束过滤 → 软约束 → 效用 → 采样
                   输出 CandidateAction + DecisionTrace
                          │
                          ▼
                [5] 执行重校验（关键 · P4）
                   动作落地前再次过约束管线
                   若世界已变 → 降级/取消，不强行执行
                          │
                          ▼
                [6] 物理步进 physics.step(FixedDt)
                   刚体运动 + 碰撞 + 球弹道采样
                   产出 PhysicsFact / RawContact
                          │
                          ▼
                [7] 弹道裁决 ballistics resolve
                   传球到达/被断、投篮命中/打铁、篮板落点
                   球权状态转移（经唯一写通道 · P1）
                          │
                          ▼
                [8] 语义解释 semantics
                   RawContact → ContactKind/Severity
                   空间 → SpacingEvaluation
                          │
                          ▼
                [9] 裁决 officiating
                   语义接触 → Foul/NoCall
                   违例 → Turnover/FreeThrow
                          │
                          ▼
                [10] 阶段转换 phase transition
                    SubPhase / GameFlowState 迁移
                          │
                          ▼
                [11] 不变量校验 invariants.check_tick（关键 · P2）
                    物理底线 + 球权唯一 + 比分/时钟单调
                    违反 → 记录 + 导出报告（debug 可 panic）
                          │
                          ▼
                [12] 快照输出 build_tick() → StreamTick
```

> **时序不变式**：任何阶段不得修改它之前阶段已产出的事实（"事实" = `PhysicsFact` / `RawContact` / 已发布进事件流的 `GameEvent`）；球权状态可在多个阶段转移（执行、弹道裁决、罚球、发球等），但**只能经唯一通道 `transition_ball_state` 写入**（§3.3）；不变量校验 [11] 是最后的**校验**环节、不可绕过——其后仅允许无副作用的快照输出 [12]（EmitPhase）。

---

## 3. 球权状态机设计（P1 · 单一事实源）

### 3.1 设计：分离"归属"与"轨迹"，位置是派生量

```
BallState (归属 · 唯一事实源 · crates/domain)
├── Held { carrier: PlayerId }                       // 突破中的持球亦是 Held，
│                                                    // 突破由动作窗口（§4.2）描述
├── Transfer { from, to, kind }                      // 控球交接/发球递交
│     kind = ControlTransfer | InboundTransfer
├── InFlight { kind: FlightKind, flight: FlightParams }
│     FlightKind = Pass | Shot | FreeThrow | InboundPass | OutletPass
│     FlightParams = 起点/落点/弧线参数/起飞时刻（可闭式采样的参数集）
├── Loose { params: LooseParams }                    // 松球/篮板弹跳的参数集
├── Dead { reason: DeadReason, next: DeadTransition }
└── InboundSetup { inbounder: PlayerId, phase: InboundPhase }

BallTrajectoryKind (运动学采样参数 · crates/physics · 私有于执行)
  退化为采样函数的参数载体，不再承载归属语义
```

**位置是派生量**：`BallState` 不保存逐 tick 的位置/速度。任意时刻的球位置由飞行/松球参数**闭式采样**得出（时间的纯函数；`physics.step` 不触碰球）。因此：

- 每 tick 运动学**不写** `BallState`，`transition_ball_state` 只处理离散的归属/路由事件——"单一写入口"与物理步进不冲突；
- `InFlight{flight}` 与 `Loose{params}` 中的参数在飞行期间不可变；需要改变落点（如被抢断、打铁改弹）即是一次状态转移，不是参数改写。

### 3.2 派生视图（全部只读）

| 派生 | 实现 | 说明 |
|------|------|------|
| `carrier() -> Option<&PlayerId>` | 从 `BallState::Held` 派生 | 取代 `carrier_idx` 的读取方 |
| `has_ball(player) -> bool` | `carrier() == Some(player)` | 物理层不再持有独立标志 |
| `possession_team()` | Held→持球人队；InFlight/Loose→最后触球队 | 取代 `Possession` 字段的读取方 |
| `is_live() / is_dead()` | 从 BallState 变体派生 | 取代 `is_dead_ball` 布尔 |

**写入纪律（字段私有化 + 唯一 mutator + grep 守卫）**：
- `BallState` 只能被**一个函数** `MatchEngine::transition_ball_state(event) -> Vec<GameEvent>` 修改；
- 该函数是纯函数风格：`(当前 BallState, 物理/裁决事实) -> (新 BallState, 产出事件)`；
- 所有弹道到达、抢断、篮板、得分、出界都转化为对这个函数的调用；
- **强制机制**：字段必须私有化，杜绝外部直接赋值。注意 quality.md §2.1 不变量校验的是**输出帧**，绕过写入口的直接赋值产出的帧可能仍然自洽、查不出来——因此不变量**不是**写入纪律的强制手段；落地靠字段私有化 + grep 守卫（design.md §3.1 M2 验收）+ code review。测试代码若需绕过该入口，grep 守卫需声明测试豁免或提供测试专用构造器。

### 3.3 状态转换表（节选）

| 当前状态 | 触发事实 | 下一状态 | 副作用事件 |
|---------|---------|---------|-----------|
| Held(c) | 决策 Pass 执行 | InFlight{Pass} | PassReleased |
| InFlight{Pass} | 到达接球人 | Held(receiver) | PassCompleted |
| InFlight{Pass} | 防守者抢断 | Held(defender) + 球权翻转 | Steal + PossessionChange |
| InFlight{Pass} | 出界 | Dead{OutOfBounds} | Turnover |
| InFlight{Shot} | 命中 | Dead{MadeBasket} | Score(n) |
| InFlight{Shot} | 打铁触筐 | Loose (RimRebound) | ReboundAvailable |
| InFlight{FreeThrow} | 命中（非末罚） | Dead{FreeThrowMade} | Score(1)，准备下一罚 |
| InFlight{FreeThrow} | 命中（末罚） | Dead{MadeBasket} | Score(1) |
| InFlight{FreeThrow} | 不中触筐 | Loose (罚球篮板) | ReboundAvailable |
| Loose | 最近球员拿到 | Held(p) | Rebound/LooseBall secured |
| Held(c) | 24秒到 | Dead{ShotClockViolation} | Violation + PossessionChange |
| Dead{Foul} | 罚球程序开始 | Held(罚球者) | FreeThrowAttempt(i/n) |
| Dead{*} | 发球准备完成 | InboundSetup | InboundReady |
| InboundSetup | 发球传出 | InFlight{InboundPass} | InboundPass |
| （比赛开始） | 裁判抛球 | Loose (跳球) | TipOff |
| Loose (跳球) | 拨球控制 | Held(p) | Possession 确立 |

> 完整转换表作为 `domain` 的状态机数据，配合穷举测试（每个非法边都必须被拒绝）。

---

## 4. 主循环阶段化设计（P3 · 显式管线）

### 4.1 设计：阶段管线

将 `step()` 重构为**显式阶段序列的顺序调度**。每个阶段是独立函数，只接收它需要的字段 `&mut`（**不是**整个 `&mut World`）：

```rust
enum PhaseOutcome {
    Continue,                     // 进入下一阶段
    ShortCircuit(Vec<GameEvent>), // 本 tick 提前结束（如违例/节末）
}

// 示例签名：每个阶段只声明它需要的字段
fn clock_advance(clock: &mut ClockState, rules: &GameRules) -> PhaseOutcome;
fn decision_phase(sys: &mut DecisionSystem, clock: &ClockState,
                  ctx: &ConstraintContext, rng: &mut ChaCha8Rng) -> PhaseOutcome;
// 调度器（step 主体）解构 World、按固定顺序调用，静态分发。
```

> 阶段以对象列表组合且统一拿整个 `&mut World` 的签名无法兑现 §4.3 的借用隔离承诺（每个阶段都能改任何子结构）。故采用静态分发 + 窄签名；若未来退化为 trait 对象列表，必须重新评估写边界如何保证。

阶段序列（对应 §2 的 12 步；`FreeThrowPhase` 为死球罚球分支，条件激活，未出现在 §2 主数据流图中）：

```
ClockAdvancePhase
RuntimeConstraintPhase
ActionWindowPhase
FreeThrowPhase          (条件激活)
DecisionPhase           (条件激活：活球或死球发球就绪；到达决策间隔)
ExecutionPhase          (含执行重校验)
PhysicsStepPhase
BallisticsResolutionPhase
SemanticPhase
OfficiatingPhase
PhaseTransitionPhase
InvariantPhase          (最后的校验环节；其后仅 EmitPhase)
EmitPhase
```

### 4.2 收益

- **顺序错误变显式的调度器问题**：新增逻辑必须在调度序列中显式插入，无法"忘记"某步；
- **每阶段可单测**：构造所需字段，喂输入，断言 `PhaseOutcome`；
- **InvariantPhase 不可绕过**：它是调度序列固定最后一环，任何路径都经过它；
- **ShortCircuit 显式化**：取代分散的提前 `return` 出口。

### 4.3 World 结构拆分

`MatchEngine` 的字段按阶段归属拆成子结构：

```rust
struct World {
    clock: ClockState,           // current/game/shot clock, period
    ball: BallState,             // §3 唯一事实源
    flow: FlowState,             // game_flow + sub_phase + timers
    score: ScoreState,
    dead_ball: DeadBallState,    // inbound/free-throw 子状态
    physics: PhysicsWorld,       // 物理后端
    decision: DecisionSystem,
    events: Vec<GameEvent>,      // 本 tick 待发布
    // ...
}
```

每个阶段函数只取得它签名声明的 `&mut` 字段，调度器按签名解构 `World` 传参——越界修改是编译错误，用借用检查器替代纪律（该承诺依赖 §4.1 的窄签名静态分发）。

---

## 5. 决策系统设计（P4 · 意图-执行重校验）

### 5.1 决策管线

```
感知 → 候选生成 → 硬约束过滤 → 物理可行性 → 软约束惩罚
     → 效用评分 → softmax 个性化采样
```

效用公式（复合律权威声明——`tactics.md` §5 的战术效用展开是 BaseValue 因子的内部构成，不另立顶层代数）：

```
FinalUtility = (BaseValue + SkillBonus + TendencyBonus + ContextBonus + PreferenceBonus)
               × Feasibility × StaminaModulation
               − SoftPenalty − RiskPenalty + MoraleBias
```

- 乘性部分是**物理与可行性门**：Feasibility 为 [0, 1] 连续系数（几何/防守封堵，不可行直接乘零）；StaminaModulation 为体力衰减因子；
- 加性部分是**偏好与惩罚修正**：战术基值、个体技能加成、倾向偏好、情境偏好在 BaseValue 侧求和；软约束、风险与士气作为独立项修正；
- `RoleFit` 因子已随 roles 降级移除（`attributes.md` §2.7）：`PlayerData` 无 `role` 字段，效用管线禁止读取任何身份性 role；战术槽位适配只能作为 `TacticalFit` 输入由 `(attributes, tendencies)` 经 `tactics.md` §2.3 适配分派生。

> **涌现要求（P7）**：上式所有因子必须可从 `(PlayerAttributes/PlayerTendencies, DecisionRules, 当前状态)` 派生；`TacticalFit` / 任何权重禁止是与能力无关的硬编码常数。能力耦合与扰动验证见 `quality.md` §6。

### 5.2 执行点重校验

**问题**：决策基于 T0 快照，动作在 T0+Δ 执行，期间世界已变。

**设计**：

1. 决策产出的是**意图（Intent）**而非**命令**，携带决策时刻的关键上下文摘要（接球人位置、防守距离、快照 tick）；
2. `ExecutionPhase` 在意图落地时，将**当前** `ConstraintContext` 重新过一遍该动作的硬约束子集；
3. 若重校验失败：
   - 传球：接球人已不在走廊 → 降级为 `Dwell`（持球观察），意图标记作废；"重新决策"指**下一决策 tick**（DecisionPhase，§2 [4]）重新生成候选，不在执行阶段就地决策——保持阶段分离；
   - 投篮：防守者已封盖到位 → 按 `contest_intensity` 重新计算，而非用决策时刻的值；
4. 重校验结果记入 `DecisionTrace`，供 quality.md §2.2 评判器分析"多少动作在落地时被迫改变"。

### 5.3 决策可解释性

`DecisionTrace` 必须包含候选效用、约束标记、概率分布。这是回合评判（quality.md §2.2）与校准闭环（design.md §2）的基础，**必须保证每个决策都有完整 trace**，禁止出现"决策了但没有 trace"的路径。

---

## 6. 语义与裁决层设计

### 6.1 分层职责

| 层 | 输入 | 输出 | 不含 |
|----|------|------|------|
| physics | 刚体/速度/碰撞 | `RawContact`, `PhysicsFact`, 弹道到达 | 任何篮球概念 |
| semantics | `RawContact` + 比赛上下文 | `ContactKind`(Screen/Block/Charge/...), `SpacingEvaluation`, `ShotEvaluation` | 犯规判定 |
| officiating | `SemanticContact` + 规则 | `Foul` / `NoCall` / `Violation` / 罚则 | 物理计算 |

**单向数据流**：physics → semantics → officiating，禁止反向。

### 6.2 裁决的确定性

- 犯规判定必须是**确定性函数** `f(contact_context, rules) -> Foul|NoCall`，随机性只体现在"是否吹罚可吹可不吹的边际接触"，且用种子 RNG；
- 所有裁决结果记入事件流，可回放重建。

### 6.3 规则档案与多联赛（宪章 C3）

**约束分轴**：两类约束的配置粒度不同，不可混用：

- **物理约束**（人体运动极限、球飞行与碰撞、最小间距）：严格遵守、全局可配置校准，但**与联赛无关**——FIBA 球员和 NBA 球员受同一套人体物理；
- **语义/规则约束**（计时结构、犯规政策、罚球与 bonus、违例尺度、几何）：必须按 `LeagueProfile` 参数化。语义层与裁决层是纯函数 `f(接触/空间事实, LeagueProfile)`——同一接触事实在不同档案下可以有不同判罚（边际吹罚尺度、bonus 触发、犯规上限）。

**LeagueProfile 差异表（首批三联赛）**：

| 档案字段 | NBA（当前基线） | FIBA | NCAA（男） |
|---|---|---|---|
| 计时结构 | 4×12 min | 4×10 min | 2×20 min |
| 进攻时钟 | 24 s，前场板重置 14 s | 24 s，重置 14 s | 30 s |
| 个人犯满 | 6 犯 | 5 犯 | 5 犯 |
| 球队犯规罚则 | 单节 bonus（第 5 次犯规起） | 单节 bonus（第 4 次犯规起） | 半场 bonus / 双 bonus |
| 三分线 | ~23.75 ft（底角 22） | 6.75 m | 6.75 m |
| 交替拥有 | 节间轮换 | 交替拥有箭头 | 交替拥有箭头 |

**落地路径**：design.md §3.1 M7 先把散落的语义/裁决阈值收编进规则体系；M10 再抽出 `LeagueProfile` 独立类型，把现有 NBA 默认值变成它的一个实例，并补 FIBA 验收场次。

**验收（design.md §3.1 M10）**：切换联赛 = 切换一份规则档案（数据），计时/几何/犯规政策/罚球程序全部随之生效，引擎代码路径零改动。在此之前，任何"看起来只影响 NBA"的规则逻辑也必须直接写成档案参数，禁止先写死后迁移。

---

## 7. 领域层设计（crates/domain）

### 7.1 职责边界

只承载**篮球本体词汇**，不包含任何"怎么打"的逻辑。

### 7.2 核心类型

| 类型 | 职责 | 关键约束 |
|------|------|---------|
| `CourtGeometry` | 场地尺寸、篮筐位置、三分线、区域划分 | 构造时校验几何合法性（篮筐在界内等） |
| `GameRules` | 全部可调参数（时钟、物理上限、权重、阈值） | `validate()` 在 setup 时强制执行 |
| `DecisionRules` | 决策子系统参数（效用权重、约束阈值、采样个性化） | 独立 `validate()`，由 `decision` 消费；作为 `GameRules` 嵌套组注入 |
| `PlayerData` 系（`data.rs`） | `PlayerAttributes` 能力向量 + `PlayerTendencies` + 球队级 `TeamTraits` | 能力是行为差异的唯一合法来源（P7）；被 decision/physics/officiating 消费；**本体契约（分类学/值语义/锚点/迁移路线）以 `docs/attributes.md` 为单一事实源**；涌现要求见 quality.md §6 |
| `LeagueProfile`（M10） | 联赛规则档案：计时结构、进攻时钟与重置、犯规政策与 bonus、几何、语义阈值 | 收拢为档案（§6.3） |
| `GameFlowState` | 宏观生命周期（TipOff/LiveBall/DeadBall/FreeThrow/QuarterEnd/Halftime/Overtime/GameEnd） | 转换由 `engine` 驱动，此处仅定义 |
| `SubPhase` | 回合内子阶段（Initiation/ActionExecution/ShotAttempt/FlightAndRebound/DeadBallReset） | 与 GameFlowState 正交 |
| `BallState`（见 §3） | 球的宏观归属状态 | **唯一事实源** |
| `GameEvent` | 领域事实（得分/犯规/违例/接触/阶段转换） | 值对象，不可变 |
| `ActionTimeWindow` | 一个动作从发起到结束的时间窗口 | 驱动运动学锁定 |
| `FixedDt` | 固定步长 | 全引擎唯一时间推进单位 |

### 7.3 设计要点

- **规则参数集中（P6 的基石）**：所有影响行为的魔法数字（最大速度、最小间距、球速上限、效用权重、犯规阈值、节奏参数）必须收敛到 `GameRules`，禁止散落在子系统里写死。这是统计校准（design.md §2）的前提。
- **事件即事实**：`GameEvent` 是引擎对外的**事实**而非"日志"。回放、审计、统计全部从事件流重建，不依赖引擎内部字段。

---

## 8. 附录

### 8.1 与其他文档的关系

- `charter.md`（v1.0 立宪）是**目标宪章**：要做出什么、什么算好、红线与成功判据。本文档是**系统组织契约**：模块如何分层、状态如何流转。映射：`charter` §4 宪章条款 → 本文档 §1.3；C1 → P6/P7；C3 → §6.3；C4 → P5；
- `quality.md` 是**检测与评判体系**：不变量、真实度评判、工具链、性能预算——本文档的架构如何被观测与验证；
- `attributes.md` / `tactics.md` 是**数据契约**：球员/阵容/战术的分类学与值语义——本文档的领域层消费它们；
- `design.md` 是**执行路线**：里程碑、校准协议、验收标准——本文档的架构按那个顺序落地；
- `status.md` 是**现状与差距**：本文档的架构现状如何、差距在哪。

### 8.2 术语表

| 术语 | 定义 |
|------|------|
| **事实源 (SoT)** | 某一信息的唯一权威存储，其余为派生 |
| **意图 (Intent)** | 决策产出但尚未执行的动作，携带决策上下文 |
| **执行重校验** | 意图落地时基于当前世界重新过约束 |
| **ShortCircuit** | 阶段管线中本 tick 提前结束的显式信号 |
| **LeagueProfile** | 联赛规则档案（计时/犯规/几何/语义阈值），多联赛配置的载体（§6.3） |

### 8.3 关键 crate 依赖图

```
domain ◄── physics ◄── semantics ◄── officiating
  ▲         ▲            ▲
  │         │            │
  └─────────┴── decision ┘
                ▲
                │
              engine ──► protocol ◄── invariants
                ▲
        ┌───────┼────────┐
      cli  debug-server  bball-wasm
```

补边：`engine ──► invariants`——`step()` 包装器每 tick 调用 `check_tick`（quality.md §1.1）。
