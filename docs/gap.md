# NBA-Sim · 从第一性原理闭合 GAP 的整体设计

> 文档类型：目标设计与实施契约  
> 定位：把“能运行的篮球模拟原型”推进为“过程可信、可校准、可复现、可使用的篮球模拟引擎”  
> 适用范围：`crates/*`、`data/*`、`scripts/*`、CI、CLI、回放与评判工具  
> 上游契约：`charter.md`、`architecture.md`、`quality.md`、`attributes.md`、`tactics.md`、`design.md`  
> 状态纪律：本文档定义**应该如何解决问题**；现状、测试结果、完成度与失败证据只写入 `status.md` / `problem.md`

---

## 0. 设计结论

篮球模拟器的最小真相不是“球员移动得像不像”，也不是“终场比分落不落在区间内”，而是：

> 在给定规则、球员、战术和种子的条件下，系统能否生成一条**空间上可行、规则上合法、因果上完整、统计上可校准、可逐步重放**的比赛过程。

因此，本方案不以继续增加动作枚举、战术名称或 UI 卡片为第一目标，而以闭合下面这条链为第一目标：

```text
输入档案
  → 结构化比赛状态
  → 感知与战术机会
  → 球员决策意图
  → 执行时重校验
  → 物理事实
  → 篮球语义
  → 裁判裁决
  → 唯一状态转移
  → 因果事件账本
  → 回合/阶段评判
  → 校准证据与可复现回放
```

任何一环不可信，后面的统计、战术分析和 UI 都只能放大错误。

本设计解决以下类别的 GAP：

1. 真相源分裂、时间与球权不同步；
2. 事件没有稳定因果链，回合摘要依赖猜测；
3. 物理事实、篮球语义、裁判规则和决策逻辑互相越界；
4. 战术档案存在但执行仍是按索引跑固定点位；
5. 属性有字段和单调函数，但没有可靠的端到端行为响应；
6. 轮换、疲劳、教练和动态防守不足以形成完整球队；
7. NBA/FIBA 的规则差异被少数标量开关掩盖；
8. 评判器容易给出“高分假安全感”；
9. 静态守卫、黄金哈希和统计门存在形式达标风险；
10. 批量模拟、PBP 导入、资源使用和调试回放还不够可靠。

---

## 1. 术语、优先级和不可妥协项

### 1.1 术语

| 术语 | 定义 |
|---|---|
| **事实** | 已经发生、不可由下游改写的物理或裁决结果，例如“球在 tick 120 被点掉” |
| **意图** | 决策层希望执行的动作，不等于动作已经发生 |
| **机会** | 在当前状态下可供球员选择的动作集合，例如可传、可投、可突破 |
| **回合** | 一支球队连续拥有进攻控制权的生命周期；以显式 `possession_id` 标识 |
| **动作** | 一次有开始、执行、结束/取消状态的篮球行为，例如一次传球或一次投篮 |
| **阶段** | 比赛生命周期或回合子阶段，例如活球、罚球、发球、节间 |
| **账本** | 能从事件流独立重建的时间、球权、得分、犯规和动作记录 |
| **参考分布** | 带来源、版本和口径的数据档案；不是评判代码里的常数 |
| **证据包** | 一次模拟/评判/校准的完整可复现实验材料 |

### 1.2 优先级

所有设计、修复和取舍按以下顺序处理：

```text
状态与因果正确性
  > 物理与规则合法性
  > 可解释的行为差异
  > 真实分布拟合
  > 性能与存储
  > 展示与便利功能
```

性能优化不得改变前四项；UI 不得拥有比赛真相；统计门不得掩盖硬错误。

### 1.3 四条不可妥协项

1. **唯一事实源**：同一个事实不能同时由多个可写字段表示。
2. **守恒与因果**：比分、球权、时钟、犯规、回合和动作必须能对平。
3. **可复现**：同一输入包和种子必须得到同一事件序列与状态摘要。
4. **不伪造证据**：缺失、未知、解析失败和未适用必须显式表达，不能用默认值、空摘要或高分掩盖。

---

## 2. 第一性原理

### P1 · 状态是系统的真相，帧和摘要都是投影

引擎内部维护一个私有、可验证的 `World`。`StreamTick`、渲染帧、统计和回合摘要都从 `World` 与事实事件派生，不能反向修改它。

### P2 · 篮球过程服从守恒关系

至少要满足：

```text
每个得分
  = 一个合法得分结果
  = 一个有效出手/罚球/规则罚则
  = 一个有来源的球状态转移

每个失误
  = 一个失误原因
  = 一个失去控制权的事件
  = 一个合法的新回合起点

每个篮板
  = 一个未命中且可争抢的投篮结果
  = 一个可解释的落点
  = 一次合法控制权转移或继续进攻
```

“统计结果看起来合理”但账本无法对平，视为失败。

### P3 · 物理、语义、裁决、决策必须单向流动

```text
物理事实
  → 篮球语义
  → 联赛裁决
  → 状态转移/事件

比赛状态 + 能力 + 倾向 + 战术机会
  → 决策意图
  → 执行重校验
  → 物理执行
```

裁判不能修改空间事实；战术不能直接修改球员坐标；评判器不能干预模拟。

### P4 · 时间必须是可计算的，不是格式化出来的

权威时钟使用整数 tick 或整数微 tick；`f32` 只用于几何、概率和展示。回合时长由 tick 差值计算，禁止使用 `max(0.1)` 等显示层下限伪造经过时间。

### P5 · 决策是受约束的不确定选择，不是剧本解释器

战术档案可以声明机会、职责、优先级和约束，但不能保证某个球员在某个时刻必然跑到某个坐标、必然执行某个动作。真实行为来自能力、倾向、状态、规则和随机性共同作用。

### P6 · 校准参数和参考数据必须有来源

每个行为参数必须回答：

- 它表示什么物理/行为含义？
- 属于规则、能力、倾向、战术、教练还是评判参考数据？
- 使用什么单位或锚点？
- 由什么数据标定？
- 由哪个测试证明它有效？

“把文件加入白名单”不等于参数已经进入合法通道。

### P7 · 评判器必须先保证可审计，再计算指数

评判器首先回答“有没有足够证据”和“每条准则的适用机会是什么”，之后才计算缺陷率或综合指数。没有证据不等于通过。

### P8 · 可用性是实验闭环的一部分

一个研究者应该能够：

```text
加载输入
→ 运行固定种子
→ 得到事件/帧/评判工件
→ 从缺陷跳到回合、事件和决策 trace
→ 通过 override 做 A/B
→ 复现并比较结果
```

如果只能手改 Rust、目测一场比赛或从 stdout 猜结果，系统不算好用。

---

## 3. 目标系统架构

### 3.1 分层

```text
┌───────────────────────────────────────────────────────────────┐
│ Application                                                   │
│ CLI / debug-server / WASM / replay / evaluator               │
│ 只读消费快照、事件和评判工件                                  │
├───────────────────────────────────────────────────────────────┤
│ Orchestration: engine                                        │
│ World、阶段调度、事实归集、唯一状态转移、生命周期             │
├───────────────┬───────────────┬───────────────┬───────────────┤
│ decision       │ tactics       │ physics       │ semantics     │
│ 机会/效用/意图 │ 体系/适配/责任 │ 运动/碰撞/弹道 │ 篮球语义事实  │
├───────────────┴───────────────┴───────────────┴───────────────┤
│ officiating: 联赛裁决与罚则程序                              │
├───────────────────────────────────────────────────────────────┤
│ domain: 类型、规则、档案、时间、状态转换                      │
├───────────────────────────────────────────────────────────────┤
│ protocol: 版本化事件/帧/trace 契约                            │
└───────────────────────────────────────────────────────────────┘
```

依赖约束：

- `domain` 不依赖其他内部 crate；
- `physics` 只消费 domain 的物理输入；
- `decision` 只读感知快照、规则、能力、战术和状态；
- `semantics` 只解释物理事实；
- `officiating` 只消费语义事实和 `LeagueProfile`；
- `engine` 是唯一编排层；
- `invariants` 检查版本化协议和账本，不读取 engine 私有字段；
- `evaluator` 只读完整事件/帧流，不修改引擎状态。

### 3.2 权威 `World`

目标结构如下，字段默认私有：

```rust
struct World {
    pub(crate) time: TimeState,
    pub(crate) lifecycle: LifecycleState,
    pub(crate) ball: BallState,
    pub(crate) score: ScoreState,
    pub(crate) roster: RosterState,
    pub(crate) player_runtime: PlayerRuntimeStore,
    pub(crate) possession: PossessionLedger,
    pub(crate) action_ledger: ActionLedger,
    pub(crate) rules: GameRules,
    pub(crate) decision: DecisionRuntime,
    pub(crate) tactics: TacticsRuntime,
    pub(crate) physics: PhysicsWorld,
    pub(crate) pending_facts: FactBuffer,
    pub(crate) pending_events: EventBuffer,
    pub(crate) rng: RngStreams,
}
```

外部只能通过以下接口读取或发出命令：

```rust
pub fn snapshot(&self) -> ReadOnlySnapshot;
pub fn step(&mut self) -> Result<StreamTick, EngineError>;
pub fn apply_command(&mut self, command: LifecycleCommand) -> Result<(), EngineError>;
```

禁止：

- `pub possession`、`pub carrier_idx` 等外部可写真相字段；
- 物理层单独持有 `has_ball` 作为独立事实；
- UI、CLI 或测试直接改比分、球权和时钟；
- 通过测试后门绕过合法转换，除非测试显式标注为 `invalid_fixture`。

### 3.3 三种数据必须分开

| 数据 | 可变性 | 用途 |
|---|---|---|
| `WorldState` | 引擎内部可变 | 当前比赛真相 |
| `Snapshot` | tick 内只读 | 感知、决策、评判输入 |
| `Fact/Event` | 发布后不可变 | 因果账本、回放、统计、审计 |

任何阶段只能读取本阶段所需的 snapshot，并通过显式结果返回事实或意图，不能偷偷修改别的阶段状态。

---

## 4. 时间、坐标与数值纪律

### 4.1 权威时间模型

定义：

```rust
type TickIndex = u64;
type SimMicros = u64;

struct TimeState {
    tick: TickIndex,
    sim_time_us: SimMicros,
    period: u32,
    game_clock_ticks: u32,
    shot_clock_ticks: u32,
    phase_elapsed_ticks: u32,
}
```

`tick_seconds` 在规则档案中声明并在 setup 时冻结。所有时钟使用整数 tick：

```text
remaining_ticks -= 1
```

展示时才转换为秒并格式化。规则档案可以使用秒作为输入，但 setup 时统一编译成 tick，并保存编译后的规则哈希。

必须区分：

- 活球比赛时钟；
- 死球/罚球/暂停经过时间；
- 模拟总时间；
- 墙钟耗时。

回合节奏只使用规定的篮球比赛时间，不把暂停等待时间混入。

### 4.2 坐标模型

- 物理世界使用单一单位：英尺；
- 2D 球员位置、速度、加速度和球的 3D 位置分开表示；
- `PlayerData` 的 `height_cm`、`weight_kg`、`wingspan_cm` 只能经过 capability mapping 转为物理量；
- 坐标原点、进攻方向、篮筐位置和三分线由 `CourtGeometry` 定义；
- 任何从 normalized 坐标到英尺的转换只能经过一处函数；
- 输出帧可以舍入，内部状态和事件不得使用舍入后的展示值继续计算。

### 4.3 “瞬移”只允许是显式生命周期事实

换人入场、节间重新站位和跳球布置属于离散 placement，不是普通运动。

每次 placement 必须产出：

```text
PlacementStarted / PlacementApplied
actor
from / to
reason
phase
```

并让不变量在该阶段使用专门豁免，而不是把球员隐藏在帧中或伪造速度为零。

---

## 5. 球权与球轨迹：从多源字段改为正交状态机

### 5.1 设计决定

将“谁控制球”和“球怎么运动”分开，避免 `Held`、`Pass`、`Drive`、`ControlTransfer` 既表示轨迹又表示球权而产生矛盾。

```rust
enum BallControl {
    Controlled {
        player_id: PlayerId,
        team: TeamId,
    },
    Uncontrolled {
        last_touch_team: Option<TeamId>,
        last_touch_player: Option<PlayerId>,
    },
    Dead {
        reason: DeadBallReason,
        next_team: Option<TeamId>,
    },
}

enum BallMotion {
    AtPlayer { player_id: PlayerId, offset: Vec3 },
    Carrying { player_id: PlayerId, offset: Vec3 },
    Flight(FlightParams),
    Loose(LooseBallParams),
    Stationary { position: Vec3 },
}

struct BallState {
    control: BallControl,
    motion: BallMotion,
}
```

派生接口：

```rust
fn holder(&self) -> Option<PlayerId>;
fn possession_team(&self) -> Option<TeamId>;
fn position_at(&self, time: SimMicros) -> Vec3;
fn is_live(&self) -> bool;
```

`has_ball` 只能由 `holder() == Some(id)` 派生，物理实体不能独立写入它。

### 5.2 唯一状态转移入口

```rust
fn transition_ball(
    &mut self,
    cause: BallTransitionCause,
    now: TickIndex,
) -> Result<TransitionResult, EngineError>;
```

该函数必须原子完成：

1. 校验当前状态与触发事实是否允许该边；
2. 创建新球状态；
3. 同步回合账本；
4. 产生结构化事件；
5. 更新需要派生的缓存；
6. 返回下游不能修改的事实集合。

禁止任何其他函数直接给球权字段赋值。

### 5.3 关键状态边

| 当前 | 事实 | 下一状态 |
|---|---|---|
| Controlled | 传球释放 | Uncontrolled + Flight(Pass) |
| Flight(Pass) | 接球成功 | Controlled(receiver) + AtPlayer |
| Flight(Pass) | 被点掉 | Uncontrolled + Loose(PassTipped) |
| Flight(Pass) | 直接抢断 | Controlled(defender) |
| Controlled | 投篮释放 | Uncontrolled + Flight(Shot) |
| Flight(Shot) | 命中 | Dead(MadeBasket) |
| Flight(Shot) | 打铁触筐 | Uncontrolled + Loose(Rebound) |
| Loose | 球员控制 | Controlled(player) |
| Controlled/Uncontrolled | 违例/出界 | Dead(violation reason) |
| Dead | 发球开始 | Dead/InboundSetup |
| InboundSetup | 发球释放 | Uncontrolled + Flight(Inbound) |
| FreeThrow | 罚球命中/不中 | 下一罚、Dead 或 Loose(Rebound) |

所有状态转换表必须可穷举测试；缺少合法出口的状态边必须在 schema 校验和生命周期测试中暴露。

### 5.4 物理约束

#### 持球与突破

- `AtPlayer` 的球位置由球员位置加偏移派生；
- `Carrying` 的球位置仍由 driver 当前坐标派生；
- 偏移范数不得超过 `holder_leash`；
- driver 消失、离场或动作结束时，必须先转换到 `AtPlayer`、`Flight` 或 `Dead`，不能继续输出“球权属于某人但球在数十英尺外”。

#### 控制交接

建立 `ControlTransfer` 前计算端点距离和可用时间：

```text
required_speed = distance / duration
required_speed <= ball_max_speed - safety_margin
```

不可行时必须改变交接时刻/端点、改为松球争抢或拒绝该转换，不能事后让 L1 检查替引擎发现错误。

#### 弹道

球的轨迹参数在释放时冻结：起点、终点、起飞时刻、弧线、速度上限、碰撞候选。物理步进只采样，不修改已发布的事实。

---

## 6. 生命周期与主循环

### 6.1 唯一阶段序列

`step()` 只负责调度，禁止把业务逻辑重新塞回调度器：

```text
0  BeginTick
1  LifecyclePrelude
2  AdvanceClocks
3  ResolveRuntimeConstraints
4  BuildReadSnapshot
5  UpdateTacticalResponsibilities
6  GenerateActionOpportunities
7  DecideIntents
8  RevalidateAndStartActions
9  AdvancePhysics
10 ResolveBallistics
11 InterpretSemantics
12 ApplyOfficiating
13 CommitTransitionsAndLedgers
14 CheckInvariants
15 EmitTick
```

每阶段返回不可变事实或明确的 `PhaseOutcome`：

```rust
enum PhaseOutcome {
    Continue,
    ShortCircuit { reason: ShortCircuitReason },
    EndPeriod,
    EndGame,
}
```

任何 `ShortCircuit` 仍必须经过：

```text
事实归集 → 账本提交 → 不变量检查 → 帧输出
```

不能有“提前 return 后没有 summary/事件/不变量”的出口。

### 6.2 阶段写权限

| 阶段 | 可写内容 |
|---|---|
| 生命周期 | 生命周期状态、周期边界、死球等待 |
| 时钟 | 时钟计数，不写比分和球权 |
| 感知 | 无写权限 |
| 战术 | 战术责任和机会缓存，不写物理位置 |
| 决策 | 意图队列，不写球和比分 |
| 执行 | 动作生命周期，不直接裁决结果 |
| 物理 | 位置、速度、碰撞事实 |
| 弹道 | 球轨迹事实和候选到达 |
| 语义 | 接触、空间、出手语义 |
| 裁决 | 犯规、违例、罚则结果 |
| 提交 | 唯一状态转移、账本、比分 |
| 不变量 | 只读检查 |
| 输出 | 只读序列化 |

### 6.3 生命周期完备性

任何比赛都必须进入以下终态之一：

```text
GameEnd(regular)
GameEnd(overtime)
GameEnd(aborted_with_error)
```

测试必须断言 `is_finished()` 是由真实的 `GameEnd` 事实导致，而不是达到 tick 上限或回合上限。

---

## 7. 事件、动作和回合账本

### 7.1 事件封装

所有事件使用结构化 envelope，不以字符串作为主数据：

```rust
struct EventEnvelope {
    schema_version: u16,
    sequence: u64,
    tick: TickIndex,
    sim_time_us: SimMicros,
    period: u32,
    possession_id: Option<u64>,
    action_id: Option<ActionId>,
    parent_event_id: Option<EventId>,
    kind: EventKind,
    actors: Vec<EntityId>,
    payload: EventPayload,
    source: FactSource,
}
```

`kind` 使用稳定枚举或版本化字符串；字段改变必须升 schema 版本。

### 7.2 动作生命周期

每个动作必须有：

```text
IntentCreated
IntentAccepted / IntentRejected
ActionStarted
ActionFact(s)
ActionCompleted / ActionCanceled / ActionDowngraded
```

例如传球：

```text
PassIntentCreated
PassReleased(from, frozen_to, release_tick)
PassPathObserved(path, intercept_candidates)
PassReceived / PassTipped / PassIntercepted / PassDropped
PossessionChange(if any)
```

`PASS_TIPPED`、`PASS_DROPPED`、`STEAL` 是不同原因，不能在 summary 中统一伪装成 `TURNOVER_STEAL`。

### 7.3 回合生命周期

```rust
enum PossessionStatus {
    Started,
    Live,
    ShotInFlight,
    OffensiveReboundContinuation,
    FreeThrowSequence,
    Ended { result: PossessionResult },
}

struct PossessionLedgerEntry {
    id: u64,
    team: TeamId,
    start_tick: TickIndex,
    end_tick: Option<TickIndex>,
    start_clock: u32,
    end_clock: Option<u32>,
    actions: Vec<ActionId>,
    terminal_event: Option<EventId>,
    turnover_actor: Option<PlayerId>,
    shooter: Option<PlayerId>,
    passes: u32,
    result: PossessionResult,
}
```

回合结束必须由一个明确的 `PossessionEndCause` 触发：

- `MadeBasket`；
- `DefensiveRebound`；
- `Turnover`；
- `OffensiveFoul`；
- `PeriodEnd`；
- `GameEnd`；
- `PossessionAbortedWithError`，仅允许作为失败工件，不能作为正常比赛结果。

不再使用没有原因的 `UNATTRIBUTED_END` 作为正常终端事件。若无法归因，评判器必须输出 `IncompleteEvidence` 并让硬完整性门失败。

### 7.4 回合时长

```text
duration_ticks = end_tick - start_tick
active_duration = 活球 tick 中属于该回合的 tick 数
```

允许短于常见分布的回合，但必须保留真实 tick 差值。若同 tick 内发生释放、点掉和收球，事件顺序必须用 `sequence` 区分；不能把时长填成固定的 0.1 秒。

### 7.5 账本平衡检查

每场比赛生成以下平衡式：

```text
starting_possessions + possession_changes
  = ended_possessions + active_possession_at_end

score_delta
  = made_field_goals + made_free_throws + rule_awards

team_fouls
  = foul_events by team

shot_attempts
  = made_shots + missed_shots + blocked_shots + canceled_shots(if explicitly canceled)

turnovers
  = steals + tipped_passes_causing_loss + dropped_passes + violations + offensive_fouls + other_explicit_causes
```

任何不平衡都写入 `ledger_violations.ndjson`，不能仅打印 stderr。

---

## 8. L0–L4 质量检测网

### 8.1 L0：数据和协议完整性

- 每行事件/帧必须能解析；坏行立即报错并带行号；
- schema version、规则哈希、能力档案哈希、战术档案哈希必填；
- sequence 和 tick 单调；
- `possession_id`、`action_id`、`parent_event_id` 引用可解析；
- 不允许 evaluator 静默 `filter_map(...ok())` 丢行；
- 评判输入为空或证据不足时不得得满分。

### 8.2 L1：硬物理与状态不变量

包括：

- 在场球员人数；
- 球员边界、速度、加速度；
- 球速、球高、球与持球人 leash；
- 单一球权；
- 比分和时钟单调；
- 进攻时钟范围；
- 合法阶段组合；
- 篮板只能来自可争抢投篮；
- 罚球序列合法；
- 出界和发球位置合法。

所有阈值从当前帧携带的 `FrameRules` 读取，禁止审计器复制一份默认常量。

### 8.3 L2：事件因果和账本

每条规则都必须说明：

- 适用机会数；
- 通过数；
- 缺陷数；
- 证据缺失数；
- 责任子系统。

示例：

| 规则 | 必须证明 |
|---|---|
| `SCORE_CAUSALITY` | 每个比分增量对应 made shot/free throw/rule award |
| `TURNOVER_CAUSALITY` | 每个失误对应 steal/tip/drop/violation/foul 等原因 |
| `PASS_CORRIDOR` | release 目标、传球轨迹和接球位置使用同一事实 |
| `STEAL_GEOMETRY` | 抢断者在传球路径或物理抢球范围内 |
| `REBOUND_CAUSALITY` | 投篮不中、落点和抢篮板者顺序完整 |
| `POSSESSION_COVERAGE` | 每个开始回合最终有合法终端或明确失败 |
| `FREE_THROW_CHAIN` | 罚球次数、得分、篮板和重新发球一致 |

### 8.4 L3：行为真实性

按结果条件化评判：

- 回合时长 × 结果；
- 传球、突破、掩护、切入和出手构成；
- contest × 出手类型 × 球员能力；
- 时钟利用；
- 失误收益/代价；
- 协防后空位、换防后的错位和恢复；
- 轮换、体力和犯规压力。

L3 不得把单个罕见样本直接视为错误；必须结合比赛状态、时钟、比分、动作替代品和参考分布。

### 8.5 阶段豁免必须显式

发球人界外、替补席越界、换人入场、跳球站位和节间 placement 由阶段规则解释。禁止输出帧隐藏事实以迎合 L1。

---

## 9. 决策系统：从“直接给目标点”改为“机会—意图—执行”

### 9.1 感知快照

```rust
struct PerceptionSnapshot {
    tick: TickIndex,
    ball: BallPerception,
    players: Vec<PlayerPerception>,
    assignments: AssignmentGraph,
    clock: ClockPerception,
    tactics: TacticalContext,
}
```

快照内的距离、速度、contest、可达时间和对位关系必须由同一 tick 的事实派生，不能用一个阶段的新位置和另一个阶段的旧目标混合。

### 9.2 候选机会

候选包括：

```text
Hold / Dwell
Pass(receiver, endpoint)
Drive(lane, finish options)
Shoot(type, release zone)
Cut(target zone)
Screen(target, angle)
Roll / Pop
Help / Stay / Closeout / Switch / Recover
```

候选生成器只声明“可选项”和物理前置条件，不能直接执行。

### 9.3 效用

使用结构化项，禁止全局 IQ 或无语义的总乘数：

```text
utility(action) =
    tactical_value
  + skill_value
  + tendency_value
  + state_value
  + teammate_value
  - risk_value
  - fatigue_cost
  - rule_penalty

final_probability = softmax(utility / temperature)
```

其中：

- `skill_value` 只使用该动作相关能力；
- `tendency_value` 表达想不想做，不表达产量本身；
- `state_value` 使用比分、时间、对位、空间和比赛阶段；
- `risk_value` 必须同时体现收益和失败代价；
- 所有权重进入 `DecisionRules`，公式形状留在代码；
- 不同动作可以使用不同温度和候选剪枝规则，但参数有明确语义。

### 9.4 意图和执行重校验

意图必须携带：

```rust
struct Intent {
    id: ActionId,
    created_tick: TickIndex,
    actor: PlayerId,
    action: ActionKind,
    expected_context: ContextDigest,
    expected_targets: Vec<EntityId>,
    geometry_claim: GeometryClaim,
    utility_trace: UtilityTrace,
}
```

执行时重新检查：

- 球权仍属于 actor；
- 接球人仍在可达范围；
- 目标路径是否被封堵；
- 动作是否已被别的动作锁定；
- 规则阶段是否仍允许执行。

失败时只能：

```text
Cancel / Downgrade / Replan at next decision boundary
```

不能强行执行，也不能在 Execution 阶段偷偷产生新决策。

### 9.5 传球事实统一

传球 release 时冻结：

```text
from_pos
frozen_to_pos
receiver_id
release_tick
flight_duration
corridor_geometry
```

接球人由物理运动向 `frozen_to_pos` 收敛；弹道采样、接球判断和 evaluator 都读取同一 `PassReleased` 事实。禁止：

- 引擎用接球人当前位置采样；
- 事件保存旧目标点；
- 评判器再用第三个位置解释。

### 9.6 决策 trace 完整性

每个决策机会必须输出：

- 候选列表；
- 硬约束过滤原因；
- 软约束分数；
- 最终效用和概率；
- RNG stream 与 draw index；
- 选中、拒绝、降级或没有机会的原因。

“没有 trace”是审计缺陷，不自动当成没有决策。

---

## 10. 战术系统：数据化，但禁止把 JSON 变成剧本

### 10.1 战术档案的合法语义

战术档案只允许声明：

- 阵型空间约束；
- 槽位需求；
- 动作机会图；
- 触发条件；
- 选项优先级；
- 对防守方案的偏好；
- 球队级风险与节奏偏好。

档案不得声明：

- 指定 roster ID 的行为分支；
- “第 N 秒必传给某人”；
- 不受防守影响的固定动作序列；
- 只能靠枚举代码解释的未知字段；
- 以屏幕坐标硬编码整套回合脚本。

### 10.2 目标 schema

```json
{
  "schema_version": 2,
  "id": "high_pnr_v2",
  "formation": {
    "slots": [
      {
        "id": "handler",
        "requirements": [
          {"attribute": "ball_handling", "weight": 1.0},
          {"attribute": "decision_iq", "weight": 0.8}
        ],
        "space_constraint": "top_or_slot"
      },
      {
        "id": "screener",
        "requirements": [
          {"attribute": "strength", "weight": 0.8},
          {"attribute": "screen_frequency", "weight": 0.7}
        ],
        "space_constraint": "high_screen_area"
      }
    ]
  },
  "opportunity_graph": [
    {"kind": "screen", "actor": "screener", "target": "handler"},
    {"kind": "drive_or_pass", "actor": "handler", "options": ["roll", "pop", "corner_pass", "shoot"]}
  ],
  "defensive_counters": ["drop", "switch", "blitz"],
  "team_bias": {
    "pace": "rules.tactics.pace_bias",
    "risk": "rules.tactics.risk_bias"
  }
}
```

空间约束使用有名字的语义区域和几何函数，不能把所有行为退化成 `pos_hint: [x,y]`。

### 10.3 槽位适配

```text
fitness(player, slot)
  = Σ requirement_weight × capability(player, requirement)
  - conflict_penalty
  - fatigue_penalty
```

适配过程：

1. 校验档案字段和能力字段均已注册；
2. 按稀缺性排序槽位；
3. 使用确定性最大匹配；
4. 冲突时回溯；
5. 记录每个 slot 的适配分和未满足原因；
6. 没有合适球员时允许降级档案或返回 setup 错误，不能静默按索引绑定。

`HashMap<String, f32>` 不能作为唯一能力接口；能力字段使用枚举/注册表，未知属性在 setup 时拒绝。

### 10.4 防守责任图

防守体系必须输出可执行的图，而不是只输出字符串：

```text
primary assignment
help responsibility
screen coverage
switch partner
low-man responsibility
closeout target
recover target
```

对每个挡拆/切入事件，防守责任图可以发生变化。`switch`、`over`、`under`、`drop`、`hedge` 和 `recover` 每一个都必须有：

- 候选生成器；
- 物理目标；
- 语义事件；
- 执行器；
- 失败恢复路径；
- 行为扰动测试。

只有定义 enum 没有执行器，视为未实现。

### 10.5 战术验证场景

至少包含：

1. 同一高位挡拆对 drop/switch/blitz 产生不同候选；
2. 提高 screener 的力量和掩护倾向改变掩护质量与后续选择；
3. 提高 handler 的决策能力降低低质量强投和危险传球；
4. 弱侧协防增加护框收益，同时增加底角空位风险；
5. 换防发生后，新的对位责任随球员移动；
6. 改变档案而不改引擎代码即可得到不同过程。

---

## 11. 能力、倾向和物理映射

### 11.1 最终输入模型

`PlayerData` 是不可变档案：

```text
identity
physical: height_cm, weight_kg, wingspan_cm, age
attributes: normalized technical/athletic/mental abilities
tendencies: action preferences and risk preferences
```

运行态单独保存：

```text
current_stamina
fouls
minutes
workload
current_assignment
current_action
```

引擎不写回档案。

### 11.2 最终能力轴

保留并正式接线：

- 运动：`speed`、`acceleration`、`agility`、`strength`、`vertical`、`stamina`；
- 进攻：`ball_handling`、`passing`、`shooting_close`、`shooting_mid`、`shooting_three`、`free_throw`、`finishing`；
- 防守/篮板：`defense_perimeter`、`defense_interior`、`steal`、`block`、`offensive_rebound`、`defensive_rebound`；
- 心智：`decision_iq`、`off_ball_sense`、`defensive_iq`。

`defensive_iq` 只消费在：协防触发、换防判断、轮转恢复和责任保持；不能成为全局 IQ 乘数。

`roles` 从引擎本体移除。展示角色由属性和倾向的纯函数投影生成；教练指派的是战术槽位，不是球员身份。

### 11.3 最终倾向轴

```text
shoot_frequency
drive_frequency
pass_frequency
cut_frequency
screen_frequency
offensive_rebound_frequency
gamble_steal
block_aggressiveness
help_aggressiveness
physicality
transition_sprint
risk_tolerance
```

每个倾向必须绑定一个具体候选生成器、效用项或运动目标。倾向不直接等于产量。

### 11.4 Capability Mapping

能力映射集中在 `domain::capability`：

```rust
fn effective_max_speed(rules, attrs, physical) -> Speed;
fn effective_reach(rules, attrs, physical) -> Length;
fn effective_release_height(rules, physical) -> Length;
fn action_skill(action, rules, attrs, physical) -> ActionSkill;
fn action_preference(action, rules, tendencies) -> Preference;
```

规则：

- 不允许 `.max(0.5)` 造成低能力死区，除非它是有名称、有单位、有证据的规则 floor；
- 体格不直接进入概率公式；先映射成 reach、质量、释放高度等物理量；
- 每个动作只消费相关属性；
- 所有曲线形状和权重进入规则通道；
- 映射函数返回带语义的 newtype，减少厘米、归一值和英尺混算。

### 11.5 属性验证的三个层级

#### A. 映射级

证明属性改变了预期物理/概率量，包含边界、单位和有限性。

#### B. 控制场景级

固定所有其他变量、固定 RNG 序列或使用足够大的重复实验，验证：

- `free_throw` 改变罚球命中概率；
- `finishing` 改变对抗终结；
- `defense_interior` 主要改变禁区干扰；
- `defense_perimeter` 主要改变外线干扰；
- `passing` 改变执行失败而不是传球选择；
- `decision_iq` 改变候选选择质量而不是命中率；
- `steal` 改变抢断成功，而 `gamble_steal` 改变尝试频率和被过代价；
- `stamina` 改变疲劳衰减和换人时机。

#### C. 比赛级

在多种子、多场景、多回合下比较效应量和置信区间，不要求每一场的随机结果严格单调。

每个维度必须有“接线测试”和“断路负面对照”。负面对照必须真的替换生产映射或候选路径，测试在断路时失败，而不是单独测试一个人工恒函数。

---

## 12. 运动、接触和防守物理

### 12.1 运动模型

- 加速、制动、变向受属性和规则上限约束；
- 目标速度不能绕过最大速度；
- 球员只能在有明确 placement 事实时瞬移；
- 碰撞响应不得将球员推出界外，除非语义层产生出界/犯规事实；
- 非在场球员不进入在场几何不变量，但其 bench 位置不应污染事件流。

### 12.2 空间事实与篮球语义分离

physics 只输出：

```text
distance
relative_velocity
collision_normal
path_intersection
reachable_time
contact_impulse
```

semantics 再解释成：

```text
screen_contact
closeout
vertical_contest
charge_candidate
block_candidate
pass_lane_intersection
spacing_quality
```

officiating 最后根据规则和随机吹罚政策裁决：

```text
foul / no_call / violation / continuation / free_throw_program
```

### 12.3 接触和犯规

接触判定必须保留：

- 主动者；
- 被影响者；
- 接触点和时刻；
- 接触方向与强度；
- 动作上下文；
- 防守位置合法性；
- 是否在投篮/传球/掩护/争抢过程中。

“犯规率”不能只由一个全局常数生成。

### 12.4 物理场景测试

必须覆盖：

- Drive 持球 leash；
- ControlTransfer 速度可行性；
- 传球走廊与抢断者位置；
- 投篮释放点与最近防守者距离；
- 反弹落点与篮板争抢；
- 接触后球员速度/位置连续性；
- 球员离场和替补不污染场内检测。

---

## 13. 轮换、疲劳与教练

### 13.1 阵容真相

```rust
struct RosterState {
    active_roster: Vec<PlayerId>,
    on_court: [PlayerId; 5],
    bench: Vec<PlayerId>,
    unavailable: Vec<PlayerId>,
    rotation_plan: RotationPlan,
}
```

档案中的 `starters` 和 `active_roster` 是输入；`on_court` 是运行态；不能用 roster 顺序代替战术职责。

### 13.2 疲劳模型

疲劳是运行态状态：

```text
fatigue += sprint_load + acceleration_load + contact_load + jump_load
fatigue -= recovery_rate × dead_ball_time
performance_modulation = f(fatigue, stamina, action_type)
```

负荷数据随事件输出：分钟、冲刺距离、加减速次数、对抗次数、跳跃/落地次数、持球时间。

模型必须有上限、恢复和可校准参数，不能用单一“第四节乘数”。

### 13.3 换人决策

换人只在合法死球窗口执行。决策输入：

- 目标分钟和轮换段；
- 当前疲劳；
- 犯规风险；
- 对位收益；
- 阵容职责覆盖；
- 比分和比赛阶段；
- 教练倾向。

输出：

```rust
SubstitutionIntent {
    player_out,
    player_in,
    reason,
    expected_role,
    utility_trace,
}
```

候选替补按职责和能力匹配，不按 ID 字典序选择。没有合法替补时必须生成 `RosterConstraintViolation`，不能静默忽略。

### 13.4 教练策略

教练档案是偏好和阈值，不是时间脚本：

- 轮换频率；
- 暂停时机；
- 领先/落后风险偏好；
- 战术调整速度；
- 阵容偏好；
- 特定防守的应对优先级。

暂停和战术切换同样走机会—意图—执行流程，事件必须可审计。

### 13.5 轮换验收

- 提高 `stamina` 使球员在相同情境下更晚进入疲劳区；
- 提高 `substitution_tendency` 增加合法死球换人机会；
- 提高犯规数导致达到联赛上限后强制离场；
- 轮换后槽位、对位和球员运行态一致；
- 48 分钟比赛的分钟总和与球员上场事件相符。

---

## 14. 联赛规则与裁判裁决

### 14.1 规则档案必须覆盖“程序”，不能只覆盖数字

`LeagueProfile` 分为：

```text
ClockPolicy
ShotClockPolicy
PossessionStartPolicy
FoulClassificationPolicy
BonusPolicy
FreeThrowPolicy
OutOfBoundsPolicy
GoaltendingPolicy
TimeoutPolicy
SubstitutionPolicy
PeriodTransitionPolicy
GeometryPolicy
```

每个策略提供命名函数，而不是让 engine 到处比较阈值。

### 14.2 犯规程序

处理顺序固定为：

```text
RawContact
→ ContactKind / Severity
→ FoulClass
→ TeamControl / ShootingContext / BonusContext
→ PenaltyProgram
→ FreeThrow / Possession / DeadBall transitions
```

`PenaltyProgram` 至少表达：

- 是否罚球；
- 罚球次数；
- 是否继续比赛；
- 是否保留/转换球权；
- 是否进入 bonus；
- 是否个人犯满；
- 是否为球队控制球犯规；
- 是否为 and-one。

投篮犯规的罚球次数必须根据投篮类型和命中状态计算，不能统一取一个 `shooting_foul_free_throws`。

### 14.3 Bonus 边界

Bonus 使用“犯规发生前的球队犯规计数”判断：

```text
if prior_team_fouls >= bonus_threshold:
    apply_bonus_penalty
else:
    apply_normal_penalty
```

NBA 节末特例、FIBA 团队犯规、NCAA 半场 bonus/double bonus 必须由档案策略表示。不能用 `team_fouls >= threshold` 在所有联盟复用而不声明计数口径。

### 14.4 NBA/FIBA/NCAA 验收矩阵

每个 profile 必须有情景 fixture：

- 比赛开始/节末/加时；
- 进攻篮板时钟重置；
- 出界和发球；
- 两分、三分、命中加罚；
- 普通犯规、球队控制球犯规、bonus；
- 最后一罚命中/不中；
- 犯满离场；
- 跳球或交替拥有；
- 暂停和换人窗口。

验收不是“跑一场没有 Hard violation”，而是每个情景的预期程序和事件都正确。

---

## 15. 评判器、参考数据和校准闭环

### 15.1 评判结果类型

```rust
enum JudgmentStatus {
    Pass,
    Defect,
    NotApplicable,
    InsufficientEvidence,
}

struct Judgment {
    criterion: CriterionId,
    status: JudgmentStatus,
    severity: Severity,
    opportunity_count: u32,
    evidence_event_ids: Vec<EventId>,
    detail: String,
    attribution: Attribution,
}
```

`NotApplicable` 不进入分母；`InsufficientEvidence` 不算通过，触发证据覆盖门。

### 15.2 每条准则使用固定分母

评判器输出：

```text
opportunities
passes
soft_defects
hard_defects
insufficient_evidence
rate
confidence_interval(if batch)
```

例如失误率准则不是“全场只发一条 defect”，而是按场/队/回合机会计算偏差，并同时输出原始值、参考带、条件分组和样本量。

### 15.3 综合真实度指数的限制

综合指数不是通过门。若保留指数：

- 使用固定版本的准则权重；
- 权重按准则族归一，防止高频 Pass 稀释低频 Hard 缺陷；
- Hard 因果失败设置不可抵消的 floor；
- 证据不足使指数无效，而不是返回 1.0；
- 报告必须同时列出各准则缺陷率和证据覆盖率。

推荐报告结构：

```text
hard_gates
causal_coverage
pace_profile
shot_quality_profile
passing_profile
defense_profile
foul_profile
rotation_profile
performance_profile
```

### 15.4 评判器输入必须严格解析

`parse_stream` 规则：

- 任一坏行返回错误、路径和行号；
- schema version 不兼容立即失败；
- 事件引用缺失立即输出 `InsufficientEvidence` 并使完整性门失败；
- 空流、截断流和提前 EOF 都不是通过；
- 大流采用流式处理，不把整场所有 tick 无界读入内存。

### 15.5 参考分布数据契约

fixture 必须包含：

```text
fixture_version
league
source_ids
source_date_range
sample_count
extraction_method
missingness_policy
conditioning_dimensions
bands / quantiles / histograms
```

缺失的 PBP 字段保持未知，不使用“15 秒”“2 次传球”等看似观测值的默认值。转换器可以提供显式的 `imputation_policy`，但插补数据不能与真实观测混合进入同一分布。

### 15.6 PBP 转换

转换流程：

```text
raw PBP
→ schema normalization
→ game clock validation
→ possession reconstruction
→ event classification
→ action aggregation
→ missingness report
→ fixture generation
```

`possession_id` 必须真正参与重建；篮板、罚球、投篮、失误和换人不能直接各计一条回合。转换结果必须附带原始事件数、丢弃事件数、插补字段和置信等级。

### 15.7 校准协议

每轮校准必须产生一个证据包：

```text
manifest.json
rules_before.json
rules_after.json
fixture_version
seeds.json
engine_revision
baseline_stats.json
candidate_stats.json
judgments_before.ndjson
judgments_after.ndjson
attribution_diff.json
replay_hashes.json
resource_report.json
```

流程：

```text
选择 top 缺陷
→ 明确责任子系统
→ 只改一个参数族/一个机制
→ 先用 JSON override A/B
→ 跑固定种子矩阵
→ 检查硬门、覆盖率、各准则缺陷率
→ 再决定是否更新默认值
→ 如行为变化，更新黄金哈希并记录原因
```

禁止“改默认值—重编译—单场目测”。

---

## 16. 协议、CLI 与调试产品

### 16.1 统一输入输出

每次运行生成：

```text
run_manifest.json
stream.ndjson                # 可选逐 tick
facts.ndjson
violations.ndjson
judgments.ndjson
attribution_report.json
box_score.json
ledger_report.json
replay_digest.json
```

运行 manifest 固定：

- seed；
- engine revision；
- rules hash；
- roster hash；
- tactics hash；
- league id；
- protocol versions；
- command line；
- output mode。

### 16.2 CLI 目标

```text
nba-sim validate-input --setup setup.json
nba-sim simulate --seed 42 --rules rules.json --out run_dir
nba-sim batch --seeds 0..99 --scope full --mode summary --out dir
nba-sim evaluate --stream stream.ndjson --fixture nba.v2 --out report_dir
nba-sim replay --run run_dir --from-event EVENT_ID
nba-sim compare --run-a A --run-b B --by possession,criterion
nba-sim benchmark --ticks N --mode engine|stream|evaluate
nba-sim pbp-convert input.json --out fixture.json --league NBA
```

其中 `batch` 默认使用有界 summary/facts 模式；逐 tick 帧是显式选择，不允许默认把大型文件写入仓库或 `/tmp` 无生命周期管理。

### 16.3 回放和 UI

UI 只能消费：

- frame snapshot；
- event facts；
- decision trace；
- judgment/violation 引用。

点击缺陷必须能够跳转：

```text
criterion
→ possession_id
→ action_id/event_id
→ tick
→ snapshot
→ decision trace
```

UI 发现的值必须标注来源：事实、派生统计、评判结果或展示投影。

### 16.4 资源治理

- 大流采用流式写入和读取；
- 单场、单 seed、单 batch 有最大文件和 tick 预算；
- 运行开始前检查磁盘空间；
- 临时 `CARGO_TARGET_DIR` 和输出目录由 runner 管理并清理；
- 所有写入错误向上传播，不能 `let _ = writeln!()`；
- benchmark 分开测纯引擎、事件、序列化、评判和 UI 服务；
- CI 不直接运行无界全场输出。

---

## 17. 确定性和守卫体系

### 17.1 RNG 分流

使用确定性主种子派生命名 RNG stream：

```text
setup
lineup
decision.home
 decision.away
physics
officiating
presentation
```

一个模块增加随机抽样不得改变其他模块的 RNG 序列。所有集合迭代排序，禁止依赖 `HashMap` 的随机迭代顺序。

### 17.2 Replay digest

黄金哈希分三层：

1. **短 tick digest**：抓快速行为漂移；
2. **全场 event digest**：覆盖完整事件 envelope 和因果引用；
3. **状态 checkpoint digest**：每节/每回合保存权威状态摘要。

哈希 schema 明确列出字段，不哈希 UI 舍入字段；合理行为修复必须按证据更新对应层级，而不是只改测试常数。

### 17.3 常数守卫重写

禁止整文件白名单。新守卫应：

- 使用 Rust AST 或 token parser；
- 只允许规则构造器、数据档案和单位换算中的合法常数；
- 每个豁免绑定到文件、行/符号、原因和 reviewer；
- 禁止在 `match_engine.rs`、`tactics.rs`、`pipeline.rs` 等核心文件整文件豁免；
- 同时扫描浮点常数、整数行为阈值、字符串战术 ID 分支和球员 ID 分支。

负面对照：CI 测试临时注入一个行为常数，守卫必须失败；守卫脚本本身也要有测试。

### 17.4 能力链守卫

为每个维度登记：

```text
attribute
consumer symbol
observable
expected direction / tradeoff
fixture
negative control
```

测试替换生产消费者或参数通道后必须失败，不能只测试独立数学函数。

### 17.5 完整性守卫

- 截断 stream；
- 删除一个事件；
- 重排事件；
- 篡改一帧球员位置；
- 篡改比分；
- 重复一条 summary；
- 修改规则哈希。

这些反事实输入都必须被 L0/L1/L2 或 replay digest 捕获。

---

## 18. 测试体系和验收门

### 18.1 单元测试

覆盖：

- 类型 validate；
- 时间换算；
- 状态机所有合法/非法边；
- 罚则程序；
- capability mapping；
- 适配和匹配；
- 评判器准则；
- fixture parser；
- schema 版本。

### 18.2 属性测试和模型测试

使用一个简化参考状态机作为 oracle，随机生成：

- 传球/投篮/篮板/犯规/违例序列；
- 合法和非法阶段转换；
- NBA/FIBA 规则差异。

验证引擎不能产生：

- 双重球权；
- 无来源得分；
- 无篮板来源的篮板；
- 死球中活球动作；
- 时钟倒流；
- 不可达的状态卡死。

### 18.3 场景测试

每个篮球机制提供最小可控 fixture，固定球员、位置、时钟和 RNG：

- 传球走廊；
- 点传与直接抢断；
- Drive leash；
- 罚球链；
- and-one；
- 防守挡拆；
- 协防与恢复；
- 进攻/防守篮板；
- 换人和犯满；
- 节末和加时。

### 18.4 批量测试

分三种用途：

| 类型 | 目的 | 样本 |
|---|---|---:|
| Smoke | 完赛和 L1 快速检查 | ≥ 8 seeds |
| Regression | 当前版本硬门和账本 | ≥ 100 seeds |
| Calibration | 分布估计和置信区间 | 由 fixture 样本量决定 |

批量结果必须按场落盘或流式聚合，不能只输出一个 stdout 平均值。

### 18.5 性能测试

至少拆成：

```text
engine-only ticks/s
engine + facts ticks/s
engine + full frame ticks/s
stream encode MB/s
evaluator events/s
peak memory
allocation count
```

性能预算必须与输出模式绑定。默认无界输出不计入纯引擎吞吐声明。

### 18.6 必须通过的硬门

目标门不是“平均分不错”，而是：

1. 全场正常结束或明确失败；
2. L0 解析错误为 0；
3. L1 Hard violation 为 0；
4. `POSSESSION_COVERAGE` 为 100%；
5. 每个失误、得分、篮板、罚球均有因果来源；
6. replay digest 在相同输入下完全一致；
7. 评判器不吞错误、不以空证据给满分；
8. NBA 与 FIBA 情景测试按 profile 通过；
9. 能力和战术负面对照有效；
10. 资源预算不超限。

真实度分布门在硬门之后执行。任何硬门失败，都不能用综合真实度指数抵消。

---

## 19. 实施路线

顺序必须围绕依赖，而不是围绕文件数量。

### G0 · 证据和契约基线

内容：

- 冻结 schema、输入 manifest 和错误模型；
- 统一 `status.md` / `problem.md` 的证据口径；
- 建立严格 stream parser、资源 runner 和 evidence package；
- 修复 CI 中“只编译/只跑黄金哈希/白名单过宽”的假绿路径。

出口：任何测试报告都能绑定代码、输入、命令和输出。

### G1 · 权威时间、球权和生命周期

内容：

- `TimeState` 整数化；
- `BallControl + BallMotion`；
- 私有 `World`；
- 唯一 `transition_ball`；
- 完备状态边；
- 所有 short circuit 统一提交账本。

出口：模型测试覆盖所有状态边；历史 ControlTransfer、Drive leash 和死球卡死场景归零。

### G2 · 事件和账本闭环

内容：

- EventEnvelope、action lifecycle、possession ledger；
- 事件 ID 和 parent ID；
- summary 只从账本生成；
- 消除正常 `UNATTRIBUTED_END`；
- 得分/失误/篮板/罚球平衡器。

出口：100 seeds 的 L2 因果覆盖 100%，所有终端有合法原因。

### G3 · 物理和语义一致性

内容：

- 传球端点冻结；
- 统一接球和走廊事实；
- 控制交接速度预算；
- 持球球轨迹派生；
- contact → semantics → officiating 分层；
- L0–L2 严格检测。

出口：控制场景和负面对照通过，物理 Hard 与走廊因果门通过。

### G4 · 决策管线和能力接线

内容：

- opportunity/utility/intent；
- trace 100%；
- 执行重校验；
- capability mapping；
- 每维控制场景、比赛场景和断路测试。

出口：能力改变行为选择/执行/结果的证据，而不是只改变一个数学返回值。

### G5 · 战术和防守重构

内容：

- JSON schema v2；
- slot fill 和对位图；
- 战术只生成机会；
- switch/drop/hedge/over/under/recover 执行链；
- 移除按索引绑定和旧枚举主路径。

出口：新增战术只增加数据档案；改变档案或能力可产生可解释过程差异。

### G6 · 阵容、疲劳和教练

内容：

- 运行态 roster；
- 正常换人、死球窗口、犯满、替补不足；
- 负荷与疲劳；
- 教练轮换、暂停和战术调整。

出口：全场上场时间、换人事件和体能账本对平；阵容深度和轮换倾向有响应。

### G7 · 联赛规则闭环

内容：

- 规则策略拆分；
- 犯规分类和 penalty program；
- FIBA/NCAA 规则差异；
- 逐情景档案测试；
- 物理规则与语义规则分轴。

出口：每个 profile 的情景矩阵通过，不能只凭一场全场模拟宣称完成。

### G8 · 真实数据和校准

内容：

- PBP 规范化和缺失性报告；
- 条件参考分布；
- evaluator 分母/证据模型；
- 校准 evidence package；
- train/validation/holdout 分离。

出口：top 缺陷连续多个独立校准周期收缩，且没有通过放宽门或改变分母制造改善。

### G9 · 产品化和性能

内容：

- 流式 batch/evaluate/replay；
- 缺陷时间轴；
- 资源治理；
- 性能分层 benchmark；
- native/WASM 回放一致性。

出口：研究者无需修改引擎代码即可完成配置、运行、比较、定位和复现。

---

## 20. 交付验收清单

### 状态与物理

- [ ] 没有外部可写的球权/比分/权威时钟字段；
- [ ] 所有状态边有合法性和出口测试；
- [ ] 权威时间为整数 tick；
- [ ] 持球 leash、球速和控制交接约束在生成端保证；
- [ ] 正常比赛不会靠 tick 上限结束。

### 因果与账本

- [ ] 每个事件都有 sequence、tick、ID 和 parent；
- [ ] 每个回合有显式开始和终端；
- [ ] `PASS_TIPPED`、`PASS_DROPPED`、`STEAL` 不混淆；
- [ ] `UNATTRIBUTED_END` 不作为正常通过结果；
- [ ] 得分、失误、篮板、罚球账本全部对平。

### 决策与战术

- [ ] 战术不直接写物理目标；
- [ ] 候选、意图和执行分离；
- [ ] 每个意图有 trace 和重校验结果；
- [ ] slot fill 不按 roster index；
- [ ] 防守动作有真实执行器和恢复链；
- [ ] 能力扰动和断路负面对照进入 CI。

### 规则与阵容

- [ ] 规则差异集中在 LeagueProfile 策略；
- [ ] 罚球、bonus、球队控制球犯规和 and-one 有独立程序测试；
- [ ] 普通轮换、疲劳、犯满、暂停和教练策略可观察；
- [ ] NBA/FIBA/NCAA 情景矩阵不依赖引擎特化分支。

### 评判和工具

- [ ] 解析错误不可静默丢弃；
- [ ] `NotApplicable` 与 `InsufficientEvidence` 独立；
- [ ] 每个准则有机会数和固定分母；
- [ ] 综合指数不能抵消硬门；
- [ ] PBP 缺失字段不被伪造默认值填充；
- [ ] batch、evaluate、replay、compare、benchmark 均可复现；
- [ ] 输出有 manifest、版本、哈希和资源报告。

### 质量与资源

- [ ] L0/L1/L2 硬门全绿；
- [ ] 多种子全场结束；
- [ ] replay digest 稳定；
- [ ] 常数守卫不再整文件豁免；
- [ ] CI 负面对照确实能使守卫变红；
- [ ] 文件大小、内存、构建目录和临时输出有边界。

---

## 21. 最终判断标准

当且仅当下面四件事同时成立，才可以称为“真正可用的篮球模拟引擎”：

### 1. 它不会说谎

状态、事件、摘要、评判器和 UI 对同一事实给出一致解释；缺失证据不会被写成通过。

### 2. 它会打篮球，而不是播放战术动画

球员能力、倾向、教练、战术和防守会在约束下产生选择；防守可以打断计划，动作可以降级，结果可以改变后续状态。

### 3. 它能被验证和改进

每个缺陷都能定位到回合、事件、决策和责任子系统；每次校准都可以 A/B、复现和比较。

### 4. 它能被别人使用

用户可以用档案和规则运行比赛、批量实验、回放过程、比较方案、导入参考数据，而不需要修改核心引擎代码。

这四项比“支持多少战术”“能输出多少帧”更接近篮球模拟器的真实完成度。
