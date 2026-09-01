# NBA-Sim 硬核模拟引擎 · 系统 / 架构 / 详细设计

> 版本：v1.2（2026-09-01 复审修订；v1.0/v1.1 历史见 §14.4）
> 适用范围：`crates/*` 全部子系统
> 目标读者：引擎维护者、物理/决策/裁决子系统贡献者、QA 与回放工具作者

---

## 0. 文档目的与阅读方式

本文档是 NBA-Sim 模拟引擎的**重构基线设计**，回答四个问题：

1. **系统性问题是什么** —— 为什么当前模拟会产出"不符合物理/篮球逻辑"的结果；
2. **架构如何约束正确性** —— 用哪些结构性手段让"正确"成为默认路径而非靠人肉审查；
3. **如何快速识别偏差** —— 不变量、确定性回放、统计基线三层检测体系；
4. **如何安全地校准** —— 参数必须走什么通道、改默认值要附什么证据（v1.1 新增）。

阅读顺序建议：

- 管理者 / 新成员：§1 → §2 → §3 → §13（验收标准）；
- 引擎贡献者：§3 → §4 → §5 → §6 → §7 → §9 → §10；
- QA / 工具作者：§9 → §10 → §11 → §12。

本文档与 `docs/diag1.md`（MVP 愿景）互补：`diag1` 描述"要做成什么样"，本文档描述"如何用架构保证它做对了"。

---

## 1. 问题诊断：为什么模拟结果会"不对"

### 1.1 症状分类

当前模拟产出的非物理/非篮球结果，可归为四类：

| 类别 | 典型表现 | 根因层 |
|------|----------|--------|
| **物理不可能** | 两人同时持球、球穿过防守者、球员瞬移、球速超上限 | 球权状态多源、物理步进与弹道采样时序错乱 |
| **状态不自洽** | 死球时球员运球、节间球仍在飞、比分倒退、时钟回流 | 宏观生命周期与子阶段转换缺乏统一校验 |
| **叙事断裂** | "空位"投篮但防守者贴脸、传球给 3 米内无队友的方向、抢断者不在传球走廊 | 决策的上下文快照与执行时刻的世界状态脱节 |
| **统计失真** | 全场比分 200+、三分命中率 90%、失误率为 0、回合时长 2 秒 | 决策效用权重、概率分布、时间尺度未校准 |

### 1.2 五个根因（架构层面）

**R1 · 球/球权状态没有单一事实源（Single Source of Truth）**

球的归属同时编码在至少四个地方：
- `MatchEngine.ball_state: BallTrajectoryKind`（弹道枚举）
- `physics.players[id].has_ball: bool`（逐球员标志）
- `MatchEngine.carrier_idx: usize`（阵容索引）
- `MatchEngine.possession: Possession`（队伍球权）

任何一条代码路径只更新其中三个，就产生"两人持球"或"空气球"。`carrier_idx` 与 `ball_state` 的同步全靠调用者自觉。

**R2 · 正确性约束散落在事后脚本，不在引擎内**

物理上限（速度/加速度/间距/球速）曾在 `scripts/audit_stream.py` 里事后校验。引擎跑的时候不知道自己错了，等跑出 100MB ndjson 再用 Python 验尸，反馈环以"小时"计。且 Python 脚本自带一套限值常量（`MAX_PLAYER_SPEED=22.0` 等），与 `GameRules` 形成**双事实源**。

**R3 · 主循环是上帝函数，顺序靠人肉维持**

`MatchEngine::step()` 曾超过 2600 行（现状见 §5.1：已包外壳，`step_inner` 仍约 920 行），时钟推进、死球处理、运行时约束、动作窗口、决策执行、物理步进、弹道裁决、犯规判定、阶段转换交错在一起，多个提前 return 出口。任意一处顺序错误（比如先推进时钟再判节末）就产生穿越/死球运球类 bug，只能靠 code review 保证。

**R4 · 决策与执行之间存在"时间缝隙"**

决策系统基于 `ConstraintContext` 快照选择动作，但动作真正执行是在数个 tick 之后（传球飞行、突破过程）。这期间世界状态已变（防守者移动、接球人跑位），导致"决策时合理、落地时荒谬"。当前没有机制在**执行点**重新校验动作仍然成立。

**R5 · 校准常数硬编码在子系统内，调参靠改代码（v1.1 新增，2026-08 审计确认）**

子系统源码中存在约 **734 处**内联浮点常量（2026-09-01 复审审计；剔除 `domain/rules.rs` 参数表与 `domain/data.rs` 球员数据后。数字是快照，会随代码演化，**以审计命令为准**：`find crates -path '*/src/*' -name '*.rs' | xargs grep -ohE '[0-9]+\.[0-9]+' | wc -l`，再减去上述两个文件）：

| 重灾区 | 内联常量数 |
|---|---|
| `engine/match_engine.rs` | 108 |
| `officiating/resolution.rs` | 106 |
| `physics/movement.rs` | 91 |
| `officiating/config.rs`（游离于 GameRules 之外的第二个 config） | 64 |
| `decision/tactics.rs` | 51 |
| `decision/constraint.rs` | 50 |
| `semantics/lib.rs` | 49 |
| `decision/pipeline.rs` | 47 |

后果：**大部分引擎行为根本够不到校准旋钮**。调节奏时只能改 `GameRules` 里已参数化的少数几个值，或直接改子系统源码并重编译——不可批量、不可追溯、不可 A/B。2026-08-31 的回合节奏校准（`dwell_base 0.30→0.65` 等）就是以"改默认值→重编译→单场目测"的硬编码方式完成的，并被黄金哈希正确地拦截（见 §11.3 校准协议）。

### 1.3 设计原则（对应根因）

| 原则 | 对应根因 | 一句话表述 |
|------|----------|-----------|
| **P1 单一事实源** | R1 | 球的归属只能从一个字段推导，其余全是派生只读视图 |
| **P2 不变量即代码** | R2 | 每 tick 在引擎内校验物理/篮球底线，违反立即暴露 |
| **P3 显式阶段管线** | R3 | 主循环是显式阶段序列的调度，顺序由调度器与窄签名保证而非注释 |
| **P4 意图-执行重校验** | R4 | 动作在执行点重新经过约束过滤，快照只用于生成候选 |
| **P5 确定性优先** | 全部 | 相同种子必须产生完全相同的世界，否则一切检测都失去意义 |
| **P6 校准通道唯一** | R5 | 一切影响行为的数值必经 `GameRules`；公式进代码、系数进规则、调参走 JSON 覆盖（v1.1） |

---

## 2. 总体架构

### 2.1 分层视图

```
┌─────────────────────────────────────────────────────────────┐
│  应用层 (Application)                                        │
│  cli / debug-server / bball-wasm / 回放与统计工具            │
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
- `officiating` 另依赖 `semantics`（语义事实的消费方，与 §7.1 单向数据流一致）；
- `engine` 可依赖全部（含 `invariants`：`step()` 包装器每 tick 调用 `check_tick`，见 §8.1）；
- `invariants` 只依赖 `protocol`（校验输出帧，不接触引擎内部）；
- `protocol` 只依赖 `serde`。

> 2026-09-01 复审：以上依赖方向已经 `cargo tree` 验证全部成立（§13.3 架构验收）。

### 2.2 核心数据流（单 tick）

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

> **时序不变式**：任何阶段不得修改它之前阶段已产出的事实（"事实" = `PhysicsFact` / `RawContact` / 已发布进事件流的 `GameEvent`）；球权状态可在多个阶段转移（执行、弹道裁决、罚球、发球等），但**只能经唯一通道 `transition_ball_state` 写入**（§4.3）；不变量校验 [11] 永远最后、不可绕过。

---

## 3. 领域层设计（crates/domain）

### 3.1 职责边界

只承载**篮球本体词汇**，不包含任何"怎么打"的逻辑。

### 3.2 核心类型

| 类型 | 职责 | 关键约束 |
|------|------|---------|
| `CourtGeometry` | 场地尺寸、篮筐位置、三分线、区域划分 | 构造时校验几何合法性（篮筐在界内等） |
| `GameRules` | 全部可调参数（时钟、物理上限、权重、阈值） | `validate()` 在 setup 时强制执行 |
| `DecisionRules` | 决策子系统参数（效用权重、约束阈值、采样个性化） | 独立 `validate()`，由 `decision` 消费；作为 `GameRules` 嵌套组注入 |
| `GameFlowState` | 宏观生命周期（TipOff/LiveBall/DeadBall/FreeThrow/QuarterEnd/Halftime/Overtime/GameEnd） | 转换由 `engine` 驱动，此处仅定义 |
| `SubPhase` | 回合内子阶段（Initiation/ActionExecution/ShotAttempt/FlightAndRebound/DeadBallReset） | 与 GameFlowState 正交 |
| `BallState`（见 §4） | 球的宏观归属状态 | **唯一事实源** |
| `GameEvent` | 领域事实（得分/犯规/违例/接触/阶段转换） | 值对象，不可变 |
| `ActionTimeWindow` | 一个动作从发起到结束的时间窗口 | 驱动运动学锁定 |
| `FixedDt` | 固定步长 | 全引擎唯一时间推进单位 |

> 另注：`officiating::ResolveConfig`（裁决结算概率配置：基础命中率、接触政策、技能权重）目前游离于 `GameRules` 体系之外平行存在，是 M7 参数收编的并轨对象（§11.1）。

### 3.3 设计要点

- **规则参数集中（P6 的基石）**：所有影响行为的魔法数字（最大速度、最小间距、球速上限、效用权重、犯规阈值、节奏参数）必须收敛到 `GameRules`，禁止散落在子系统里写死。这是统计校准（§12）的前提。**当前状态（2026-09-01）：约 734 处子系统内联常量尚未收编（§1.2 R5），详见 §11.2 差距矩阵。**
- **事件即事实**：`GameEvent` 是引擎对外的**事实**而非"日志"。回放、审计、统计全部从事件流重建，不依赖引擎内部字段。

---

## 4. 球权状态机设计（P1 · 单一事实源）

这是修复"两人持球/空气球"的结构性方案。

### 4.1 问题

当前 `BallTrajectoryKind` 同时承担"球的运动学轨迹"和"球的归属"两个职责，且归属又冗余在 `has_ball` / `carrier_idx` / `possession`。

### 4.2 设计：分离"归属"与"轨迹"，位置是派生量

**现状基线（v1.2 补充）**：`crates/domain` 中已存在一个 `BallState` 标签枚举（`flow.rs`：Held/PassFlight/InboundTransfer/Drive/ShotFlight/Loose/Rebound/Dead），但它只是从 physics 的 `BallTrajectoryKind` 映射来的**无载荷投影**（`MatchEngine::ball_state_kind()`），不是事实源。M2 的演进路径：在现有枚举上**加载荷变体**（放弃 `Copy`），或以新类型替换后删除投影——不允许两个 `BallState` 长期并存。

```
BallState (归属 · 唯一事实源 · crates/domain)
├── Held { carrier: PlayerId }                       // 突破中的持球亦是 Held，
│                                                    // 突破由动作窗口（§3.2）描述
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

**位置是派生量（v1.2 关键澄清）**：`BallState` 不保存逐 tick 的位置/速度。任意时刻的球位置由飞行/松球参数**闭式采样**得出（现有实现：`BallisticsEngine::sample_ball_position`，时间的纯函数；`physics.step` 不触碰球）。因此：

- 每 tick 运动学**不写** `BallState`，`transition_ball_state` 只处理离散的归属/路由事件——"单一写入口"与物理步进不冲突；
- `InFlight{flight}` 与 `Loose{params}` 中的参数在飞行期间不可变；需要改变落点（如被抢断、打铁改弹）即是一次状态转移，不是参数改写。

### 4.3 派生视图（全部只读）

| 派生 | 实现 | 说明 |
|------|------|------|
| `carrier() -> Option<&PlayerId>` | 从 `BallState::Held` 派生 | 取代 `carrier_idx` 的读取方 |
| `has_ball(player) -> bool` | `carrier() == Some(player)` | 物理层不再持有独立标志 |
| `possession_team()` | Held→持球人队；InFlight/Loose→最后触球队 | 取代 `Possession` 字段的读取方 |
| `is_live() / is_dead()` | 从 BallState 变体派生 | 取代 `is_dead_ball` 布尔 |

**写入纪律（v1.2 修订：字段私有化 + 唯一 mutator + grep 守卫）**：
- `BallState` 只能被**一个函数** `MatchEngine::transition_ball_state(event) -> Vec<GameEvent>` 修改；
- 该函数是纯函数风格：`(当前 BallState, 物理/裁决事实) -> (新 BallState, 产出事件)`；
- 所有弹道到达、抢断、篮板、得分、出界都转化为对这个函数的调用；
- **强制机制**：字段必须私有化，杜绝外部直接赋值。注意 §8 不变量校验的是**输出帧**，绕过写入口的直接赋值产出的帧可能仍然自洽、查不出来——因此不变量**不是**写入纪律的强制手段；落地靠字段私有化 + grep 守卫（M2 验收，§13.1）+ code review。测试代码目前存在绕过该入口的直接赋值（如 `engine/tests/constraint_system.rs`），grep 守卫需声明测试豁免或提供测试专用构造器。

### 4.4 状态转换表（节选）

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

> 完整转换表作为 `domain` 的状态机数据，配合穷举测试（每个非法边都必须被拒绝）。罚球与跳球边为 v1.2 补入（代码中罚球流程已存在：`resolve_free_throw` / `start_free_throw_rebound`，v1.1 状态机未建模）。

---

## 5. 主循环阶段化设计（P3 · 显式管线）

### 5.1 问题

原 `step()` 是 2600+ 行上帝函数 + 8 个提前 return。**现状（2026-09-01）**：已包上 `step()` / `step_inner()` 外壳（外层做不变量检查与违反记录），但 `step_inner` 仍约 920 行、10 个 `return self.build_tick()` 提前出口，时钟/约束/决策/执行/弹道/阶段转换交错，顺序靠注释和人肉。

### 5.2 设计：阶段管线

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

> **v1.2 修订**：v1.1 曾给出 `trait Phase { fn run(&mut self, world: &mut World, ...) }` 的签名——阶段以对象列表组合且统一拿整个 `&mut World`，则每个阶段都能改任何子结构，§5.4 的借用隔离承诺无法兑现。故采用静态分发 + 窄签名；若未来退化为 trait 对象列表，必须重新评估写边界如何保证。

阶段序列（对应 §2.2 的 12 步；`FreeThrowPhase` 为死球罚球分支，条件激活，未出现在 §2.2 主数据流图中）：

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
InvariantPhase          (永远最后)
EmitPhase
```

### 5.3 收益

- **顺序错误变显式的调度器问题**：新增逻辑必须在调度序列中显式插入，无法"忘记"某步；
- **每阶段可单测**：构造所需字段，喂输入，断言 `PhaseOutcome`；
- **InvariantPhase 不可绕过**：它是调度序列固定最后一环，任何路径都经过它；
- **ShortCircuit 显式化**：取代 10 个分散的 `return self.build_tick()`。

### 5.4 World 结构拆分

`MatchEngine` 的 40+ 个平铺 pub 字段按阶段归属拆成子结构：

```rust
struct World {
    clock: ClockState,           // current/game/shot clock, period
    ball: BallState,             // §4 唯一事实源
    flow: FlowState,             // game_flow + sub_phase + timers
    score: ScoreState,
    dead_ball: DeadBallState,    // inbound/free-throw 子状态
    physics: PhysicsWorld,       // 物理后端
    decision: DecisionSystem,
    events: Vec<GameEvent>,      // 本 tick 待发布
    // ...
}
```

每个阶段函数只取得它签名声明的 `&mut` 字段，调度器按签名解构 `World` 传参——越界修改是编译错误，用借用检查器替代纪律（该承诺依赖 §5.2 的窄签名静态分发）。

---

## 6. 决策系统设计（P4 · 意图-执行重校验）

### 6.1 现有管线（保留）

```
感知 → 候选生成 → 硬约束过滤 → 物理可行性 → 软约束惩罚
     → 效用评分 → softmax 个性化采样
```

效用公式：

```
FinalUtility = BaseValue × Feasibility × TacticalFit × RoleFit
               − SoftPenalty − RiskPenalty + PreferenceBonus
```

### 6.2 关键增强：执行点重校验

**问题**：决策基于 T0 快照，动作在 T0+Δ 执行，期间世界已变。

**设计**：

1. 决策产出的是**意图（Intent）**而非**命令**，携带决策时刻的关键上下文摘要（接球人位置、防守距离、快照 tick）；
2. `ExecutionPhase` 在意图落地时，将**当前** `ConstraintContext` 重新过一遍该动作的硬约束子集；
3. 若重校验失败：
   - 传球：接球人已不在走廊 → 降级为 `Dwell`（持球观察），意图标记作废；"重新决策"指**下一决策 tick**（DecisionPhase，[4]）重新生成候选，不在执行阶段就地决策——保持阶段分离；
   - 投篮：防守者已封盖到位 → 按 `contest_intensity` 重新计算，而非用决策时刻的值；
4. 重校验结果记入 `DecisionTrace`，供 §9.2 叙事校验分析"多少动作在落地时被迫改变"。

### 6.3 决策可解释性

`DecisionTrace` 已包含候选效用、约束标记、概率分布。这是叙事校验（§9.2）和调参（§12）的基础，**必须保证每个决策都有完整 trace**，禁止出现"决策了但没有 trace"的路径。

---

## 7. 语义与裁决层设计

### 7.1 分层职责

| 层 | 输入 | 输出 | 不含 |
|----|------|------|------|
| physics | 刚体/速度/碰撞 | `RawContact`, `PhysicsFact`, 弹道到达 | 任何篮球概念 |
| semantics | `RawContact` + 比赛上下文 | `ContactKind`(Screen/Block/Charge/...), `SpacingEvaluation`, `ShotEvaluation` | 犯规判定 |
| officiating | `SemanticContact` + 规则 | `Foul` / `NoCall` / `Violation` / 罚则 | 物理计算 |

**单向数据流**：physics → semantics → officiating，禁止反向。

### 7.2 裁决的确定性

- 犯规判定必须是**确定性函数** `f(contact_context, rules) -> Foul|NoCall`，随机性只体现在"是否吹罚可吹可不吹的边际接触"，且用种子 RNG；
- 所有裁决结果记入事件流，可回放重建。

---

## 8. 观测与不变量体系（P2 · 快速识别偏差）

这是"快速识别模拟偏差"的核心。三层检测金字塔：

### 8.1 L1 · Tick 不变量（引擎内，每 tick）

**目标**：捕获物理不可能 + 状态不自洽（§1.1 前两类）。

实现：`crates/invariants`，只依赖 `protocol`，校验每帧 `StreamTick`。**当前共 20 条规则**（核心 16 条 + 因果图 4 条，2026-09-01 复审清点）：

| 规则 ID | 校验内容 | 类别 |
|---------|---------|------|
| `TEAM_ON_COURT_COUNT` | 双方在场人数各自严格等于 5 | 常识 |
| `BALL_SINGLE_HOLDER` | 至多一名 `hasBall=true` | 球权 |
| `BALL_HOLDER_MISMATCH` | `ball.holderId` 与 hasBall 标志一致 | 球权 |
| `BALL_HOLDER_ON_COURT` | 持球者必须在场 | 球权 |
| `BALL_HOLDER_EXISTS` | holderId 在球员列表中 | 球权 |
| `BALL_WITH_HOLDER` | 持球时球位置≈持球人位置（<3ft） | 球权 |
| `PLAYER_IN_BOUNDS` | 在场球员归一化坐标 ∈ [0,1]² | 物理 |
| `PLAYER_SPEED` | 位移/Δt ≤ max_speed + 容差 | 物理 |
| `PLAYER_SEPARATION` | 任意两人间距 ≥ min_sep×0.5（**唯一 Soft**） | 物理 |
| `BALL_SPEED` | 非持球球 3D 速度 ≤ ball_max + 容差 | 物理 |
| `BALL_HEIGHT_BOUNDS` | 球高度在合理区间 | 物理 |
| `SCORE_MONOTONIC` | 比分不倒退 | 状态 |
| `SCORE_DELTA_VALIDITY` | 单次得分增量合法（1/2/3） | 状态 |
| `CLOCK_MONOTONIC` | 同节内时钟不回流 | 状态 |
| `SHOT_CLOCK_BOUNDS` | 进攻时钟在 [0, 24] 区间 | 状态 |
| `SCORE_EVENT_WITHOUT_POINTS` | 得分事件必须伴随比分变化 | 因果 |
| `UNCAUSED_SCORE_DELTA` | 每次比分变化必须有因果事件 | 因果 |
| `REBOUND_AFTER_MADE_SHOT` | 命中投篮后不得出现篮板事件 | 因果 |
| `INBOUNDER_DEEP_IN_COURT` | 发球队员不得深入场地 | 因果 |
| `STEAL_MULTIPLE_HOLDERS` | 抢断不得产生多名持球者 | 因果 |

> 规则表为 2026-09-01 清点快照；新增规则只需在 `invariants` crate 添加实现（§13.3）。

**集成点**：`MatchEngine::step()` 包装器在每 tick 输出后调用 `check_tick()`，违反写入 `last_tick_violations` 并 eprintln；导出循环聚合为 `ExportSummary.violations`，CLI 据此以退出码 1 失败。

**严重级别**：`Hard`（物理不可能）/ `Soft`（不合理但可能合法，计数上报不阻断）。`Violation` 结构必须携带 `severity` 字段。**现状（2026-09-01）**：级别只有 `taxonomy.rs::severity_of_rule` 的事后映射（仅 `PLAYER_SEPARATION` 为 Soft），`Violation` 结构本身无该字段，工件行因此缺字段（§11.2 M1 缺口）。

**工件输出**：违反必须落到 `violations.ndjson`（`{tick, rule, severity, detail}`），作为偏差分类账（§9.4）与 debug-server 违规时间轴的数据源。**禁止只写 stderr。现状（2026-09-01）**：单场模式已落盘 `{out}.violations.ndjson`（CLI）；batch 模式只计数、不落工件（§11.2）。

**单一事实源**：限值一律取自帧内 `FrameRules`（来自 `GameRules`）。任何离线审计工具（含历史 Python 脚本）不得自带限值副本——`scripts/audit_stream.py` 的常量表必须退役，改由复用 invariants crate 的 Rust 审计 CLI 替代（§11.4 工具链验收）。**现状（2026-09-01）**：`check_tick` 自身对 `frame.rules` 缺失时用 `unwrap_or` 回退硬编码限值（22.0/85.0/3.6 等，`invariants/src/lib.rs`）——不变量 crate 内部就藏着第二套限值，必须移除；`frame.rules` 为必填字段，测试构造器可另设显式默认。

**阶段语义（v1.2 新增，必须实现）**：L1 规则必须定义在 DeadBall / InboundSetup / FreeThrow 等阶段下的行为（帧已携带 `phase` 与 `game_flow` 字段，但 `check_tick` 目前从不读取）：

- 发球人按规则站**界外**（`inbound_release_depth_ft` 推出边界），球被移到界外发球点——`PLAYER_IN_BOUNDS` / `BALL_SPEED` 等规则需要阶段豁免或替代判定，否则必然误报；当前不误报纯属引擎投影技巧，不是防护；
- **禁止投影规避**：不得通过导出帧时隐藏事实来迎合不变量。现状是引擎在 InboundReady/InboundTransfer 期间不导出持球者（`holder_id` 置空），"持球事实"在帧上消失——违反"事件即事实"（§3.3）。正确做法是不变量认识发球阶段并改判定，而不是帧撒谎；
- 分类账（`taxonomy`）中 `BENCH_DEEP_IN_COURT`、`BALL_TELEPORT` 已声明但从不发射（死条目）：要么实现——球被移动到界外发球点正是 `BALL_TELEPORT` 该管的场景——要么删除；
- 未来若引入死球站位重置（球员瞬移），`PLAYER_SPEED` 必误报，必须先补阶段豁免再引入重置。

### 8.2 L2 · 回合叙事校验（每回合结束）

**目标**：捕获叙事断裂（§1.1 第三类）。

每回合结束输出 `PossessionSummary`：

```rust
struct PossessionSummary {
    possession_id: u32,
    holding_sequence: Vec<PlayerId>,     // 持球序列
    decisions: Vec<DecisionTrace>,       // 每次决策的完整 trace
    shots: Vec<ShotEvaluation>,          // 出手质量分
    outcome: PossessionOutcome,          // 得分/失误/违例/节末
    duration_s: f32,
}
```

校验规则（示例）：
- 每次 `Shoot` 的 `contest_intensity` 必须与"出手瞬间最近防守者距离"一致（容差内）；
- 每次成功传球的接球人，在到达时刻必须在传球走廊可达范围；
- `STEAL` 事件中抢断者必须在传球走廊上；
- 回合时长 ∈ [1 tick, 24秒+进攻篮板延长时间]：篮球没有"规则最小"回合时长（发球即被抢断可 <1 秒），下界只是零/负时长的异常性检查（v1.2 修订）。

> **现状（2026-09-01）**：`PossessionSummary` 类型已存在（`domain/src/event.rs`）并随事件流发布，但为简化版字段（时长、传球数、终结事件、投篮/篮板/失误者、对抗强度等）——尚未包含逐次 `DecisionTrace` 序列、持球序列与出手质量列表；`possession_narrative` 测试仅断言事件计数，上述规则未落地（§11.2 M6）。

### 8.3 L3 · 统计分布基线（N 场批量）

**目标**：捕获统计失真（§1.1 第四类）。

- 批量运行种子矩阵（如 100 场），聚合每场统计到 JSON；
- 指标：比分分布、**投篮分解（2分/3分命中率、罚球率）**、失误率、回合时长分布、球员速度 p50/p99；
- 与 NBA baseline 或阶段门对比，漂移超带 → CI 失败。

**基线锚定哲学（v1.1 修订，重要）**：基线的职责是**奖励逼近目标、惩罚偏离目标**，不是"把当前实测值 ±25% 当锚"。结构必须是**设计目标带 + 阶段门**：

```
目标带（终态，固定不动）：total ∈ [190, 270]（两队和，48 分钟）
阶段门（逐级收窄；当前门以 `engine/tests/stats_baseline.rs` 为权威）：
  G1 [400, 700]   （第 1 轮实测 p50≈517）
  G2 [300, 500]   （第 2 轮实测 p50≈418.5）
  …（G3 中间门已过，轨迹见测试内注释）
  G4 [200, 310]   ← 当前所处（第 3 轮实测 p50≈334，进一步校准后入门；
                   seed 42 全场实测 252 分 / 202 回合 / 16.0s，黄金哈希 v8）
  G_final [190, 270] = 目标带
```

> **v1.2 更正**：v1.1 称当前所处 G1（p50=517），实际校准已推进至 G4。代码注释曾称 G4 为"终态目标带"——与本节冲突；**终态目标带以本节 [190, 270] 为权威**（310 分远超 NBA 现实），G4 只是倒数第二级门，需继续收窄。

**判定口径（v1.2 新增）**：总分取 **p50**，回合数与回合时长取**均值**（与当前测试实现一致）；判定种子数 **≥ 8**。当前基线测试仅 4 种子，弱于 §11.3 协议要求，需扩（§11.2）。三分命中率已在 batch 采集但尚未进门（目标带 30–40%，§13.2）。

规则：模拟统计落在当前阶段门内 → 绿；越出门外（无论方向）→ 红；逼近下一门 → 校准 PR 允许同时推进门。**禁止**把带设成"实测±任意百分比"——那会把已知失真固化成基线（2026-08-31 曾犯此错误并被当场抓包：校准取得进展反而导致测试失败）。

### 8.4 确定性保证（一切检测的前提）

- 固定步长 `FixedDt`、固定种子 `ChaCha8Rng`、禁止 `HashMap` 迭代序影响逻辑（所有迭代先排序）；
- **黄金 tick 哈希**：对标准种子运行，把每 tick 关键字段哈希串联，进 git。任何改动导致逐 tick 分歧 → CI 立即暴露；
- 黄金哈希抓"变了"，统计基线抓"错得离谱"。两者互补。

---

## 9. 观测与调试工具链（v1.1 新增为独立章节）

检测体系没有工具链就是空话。以下为可执行验收的组成部分：

### 9.1 批量跑批 CLI

目标形态：

```
nba-sim --rules rules.json --seeds 0..20 batch --out stats.jsonl
```

当前形态（2026-09-01；与目标形态的差异即工具链验收项，§11.4）：

```
nba-sim --rules rules.json --batch N [scope]   # 跑批：种子固定 1..=N
nba-sim [seed] [out.ndjson] [scope]            # 单场：seed 默认 42
nba-sim audit [path]                           # 离线不变量审计
```

- `--rules`：部分 JSON 覆盖 `GameRules`（依赖 `#[serde(default)]`），**校准的唯一入口**（P6）——已实现；
- `batch`：目标是种子矩阵批量跑、每场输出一行统计 JSON；当前为 `--batch N` 标志，种子固定 `1..=N`，每场流导出到 `/tmp/nba_batch_{seed}.ndjson`，聚合（总分/回合时长/3P% 中位数 + 违反计数）打印到 stdout——**无 `--seeds`/`--out`、不落违规工件**；
- 跑批必须 release 模式（见 §10 性能预算）。

### 9.2 PossessionSummary 与叙事校验器

（内容即 §8.2，工具形态为独立 analyzer：读 ndjson 流 + PossessionSummary，输出叙事违反清单。）

### 9.3 violations 工件

`violations.ndjson` 每行 `{tick, rule, severity, detail}`（现状：行内尚无 `severity`，字段缺口见 §11.2 M1）；analyzer 按 `rule` 聚合计数排序——80% 的"不像篮球"通常来自 2–3 个根因，分类账就是找根因的第一站。

### 9.4 debug-server 违规时间轴

回放 UI 加违规时间轴：加载 violations.ndjson，违规帧打点，点击跳转到该 tick 看场上态势。物理问题几乎都能 3 秒肉眼看出来，前提是违规检测先建好。

---

## 10. 性能预算（v1.1 新增）

**背景**：用户反馈"模拟有点慢"（debug 模式 ~950 ticks/s，1 节 24s）。性能从此是验收项，不是事后优化。

### 10.1 预算表

| 指标 | 预算 | 验证方式 |
|---|---|---|
| release 模式吞吐 | ≥ 20,000 ticks/s（基准机器） | `cargo run --release -- benchmark` |
| 1 节（~23k ticks） | ≤ 3s release | 同上 |
| 每 tick 堆分配 | 稳态为 0（复用缓冲；`build_tick` 的 Vec/String 复用） | 分配计数器或 heaptrack 抽查 |
| `String` clone / tick | 当前 `build_tick` 每球员 5+ 次字符串克隆 → 目标消除（球员 id/jersey 预分配 Arc/静态） | 代码审查 + 分配计数 |
| 批量跑批 | 100 场 × 全场 ≤ 30 min release | `batch` 计时 |

### 10.2 已知热点（2026-08-31 快查，未 profile 深挖）

- `build_tick()`：每 tick 克隆并 round 全部球员的 id/jersey/zone/action 等 String，排序 Vec；
- `modulation`：逐 tick `HashMap<String, _>` 全量遍历 + entry 插入；
- 战术规划每 tick 重建 `Vec<TargetAssignment>` 两次（主客队）。

### 10.3 纪律

- 性能优化 PR 不得改变行为——黄金哈希必须不变（哈希输入不含渲染舍入字段，纯数据结构优化的哈希天然稳定）；
- 若优化涉及浮点求和顺序等哈希敏感路径，须在 PR 中说明并重冻结哈希。

---

## 11. 工程落地规划

### 11.1 阶段划分

| 阶段 | 内容 | 风险 | 依赖 |
|------|------|------|------|
| **M1 不变量内建** | `crates/invariants` + step 包装 + 导出报告 + violations 工件 + severity 分级 | 低 | 无 |
| **M2 BallState 收敛** | §4 领域 BallState 枚举 + 纯函数转换表 + 派生视图替换 | 高（触碰多路径） | M1 |
| **M3 确定性基线** | 黄金 tick 哈希 + 阶段门统计基线 + **CI 化** | 低 | M1 |
| **M4 主循环阶段化** | §5 管线 + World 拆分 | 高 | M2、M3 |
| **M5 执行重校验** | §6.2 意图落地重校验 | 中 | M2 |
| **M6 L2/L3 校验** | PossessionSummary + 叙事校验 + 统计门 CI | 中 | M3 |
| **M7 参数收编（v1.1 新增）** | ~734 处子系统内联常量（§1.2 R5）→ `GameRules`/`DecisionRules`；`officiating/config.rs` 并轨；audit_stream.py 退役 | 中（行为必须逐批保持，黄金哈希锁） | M3 |

> 顺序原则：**先建检测网（M1/M3），再做高风险重构（M2/M4/M7）**。没有不变量 + 黄金哈希 + 阶段门三重兜底就动球权状态机或批量改参数是裸奔。

### 11.2 当前进度与差距矩阵（2026-09-01 复审快照）

| 里程碑 | 完成度 | 已达成 | 缺口 |
|---|---|---|---|
| M1 | ~90% | 20 条规则逐 tick 运行（核心 16 + 因果 4）；CLI 退出码；11 项负面对照（`invariants/tests/checker.rs`）；单场 violations 工件 `{out}.violations.ndjson`；多种子零违反 | `Violation` 无 `severity` 字段（仅 taxonomy 事后映射）；batch 不落违规工件；`check_tick` 兜底硬编码限值；L1 无阶段语义（§8.1 阶段语义） |
| M2 | ~45% | 写入口收敛（`transition_ball_state` 唯一，grep 验证）；`has_ball` 派生同步；行为保持 | **§4 核心未做**：归属仍在 physics 的 `BallTrajectoryKind`；`domain::BallState` 仅是无载荷标签投影（§4.2 演进路径）；纯函数转换表未建（罚球/跳球边亦未建模）；`carrier_idx` 仍是写入 fallback、`possession` 独立字段未降级派生 |
| M3 | ~70% | 哈希三件套齐备并冻结至 v8；阶段门推进到 G4 [200,310] 且绿（seed 42 实测 252 分/202 回合/16.0s）；锚定哲学已是"目标带 + 阶段门" | 无 `--seeds`/`--out` batch CLI（§9.1）；无 CI（`.github/` 不存在）；门判定仅 4 种子（协议要求 ≥8）；3P% 已采集未进门 |
| M4 | ~10% | `step()`/`step_inner()` 外壳（不变量检查 + 违反记录） | 管线静态分发、World 拆分、ShortCircuit 全部；`step_inner` 仍 ~920 行 / 10 个 `build_tick` 出口 |
| M5 | 0% | — | 全部：意图重校验 |
| M6 | ~20% | `PossessionSummary` 类型已存在并随事件流发布（`domain/src/event.rs`）；`possession_narrative` 测试存在 | 字段为简化版（无逐次决策 trace/持球序列/出手质量）；叙事校验器仅断言事件计数，§8.2 规则未落地；统计门无 CI |
| M7 | 0% | — | ~734 处内联常量收编（§1.2 R5）；config 并轨；脚本退役 |

### 11.3 校准协议（v1.1 新增——血的教训成文）

**前置（v1.2 注明）**：本协议依赖 §11.4 工具链验收中的 `--seeds`/`--out`/batch 违规工件；在其落地前，协议按 §9.1 的当前等价命令执行（跑批为 `--batch N`，种子固定 `1..=N`，聚合打印到 stdout）。

**谁可以改 `GameRules` 默认值？** 任何人，但必须走完整协议，缺一即打回：

1. **先跑基线**：改动前跑批记录统计快照（进 PR 描述）。当前命令：`nba-sim --batch 8 [scope]`（种子固定 1..=8）；目标形态落地后改用 `--seeds 0..7`（§9.1）；
2. **通道合法**：探参数用 `--rules override.json`（P6），确定值后才进 `rules.rs` 默认值——**禁止"改默认值→重编译→单场目测"循环**（2026-08-31 曾犯）；
3. **证据齐全**：PR 必须含 改动前后统计对比（≥8 种子）、落入哪个阶段门（§8.3）、黄金哈希重冻结 + 一行改动说明；
4. **单一职责**：校准 PR 不得混入行为/重构/性能改动；反之亦然；
5. **门推进**：统计持续逼近下一阶段门时，允许在同一 PR 推进门并注明依据；
6. **新公式纪律**：新决策公式/约束公式的所有系数必须先进 `GameRules`/`DecisionRules` 再写公式。**PR 中出现新的内联浮点行为常数 = 打回**（当前 `pipeline.rs` 的 `0.45`、`constraint.rs` 的 `0.25/0.10/0.02` 等即属此列，待 M7 收编）。

### 11.4 工具链验收（v1.1 从各里程碑抽出单列）

- [ ] `nba-sim --rules X --seeds A..B batch --out Y` 可用且 release 模式；
- [ ] violations.ndjson 工件由 CLI/引擎导出；
- [ ] `.github/workflows/ci.yml`：build + test（含黄金哈希）+ 阶段门统计（缩减种子集）；
- [ ] audit_stream.py 的限值常量删除或脚本退役。

---

## 12. 统计校准入口

所有物理/概率参数必须可从 `GameRules` 注入（含 `--rules` JSON 覆盖）。校准时：
1. 固定种子跑批量（§9.1）；
2. 对照阶段门（§8.3）；
3. 只调 `GameRules` 里的值 + 附协议要求的证据（§11.3），不改子系统代码。

---

## 13. 验收标准

### 13.1 功能验收（每阶段必须满足）

**M1 不变量内建**
- [ ] 每 tick 输出后自动执行全部 L1 规则；
- [ ] 违反时 `ExportSummary.violations` 非空，CLI 退出码为 1；
- [ ] 每条违反携带 `tick_index` + 规则 ID + **severity** + 可读描述；
- [ ] violations 落盘 `violations.ndjson`；
- [ ] 负面对照：注入两人持球/球人分离/比分倒退等场景必须全部捕获（现有 11 项，位于 `crates/invariants/tests/checker.rs`；引擎集成测试尚无注入用例）；
- [ ] 标准种子跑 1 节，零 Hard 违反（基线）。

**M2 BallState 收敛**
- [ ] 领域层存在 `BallState` 枚举，归属语义不再住在 physics；
- [ ] `BallState` 仅能被纯函数 `transition_ball_state` 写入（grep 无其他赋值）；
- [ ] `carrier_idx` / `has_ball` / `possession` 全部降级为派生只读；
- [ ] 状态转换表穷举测试：所有非法边被拒绝；
- [ ] 标准种子 + ≥5 额外种子全场，`BALL_*` 不变量零违反。

**M3 确定性基线**
- [ ] 相同种子两次运行，逐 tick 哈希完全一致；
- [ ] 黄金哈希入库且 **CI 上永远绿**（红的基线 = 关闭的检测网）；
- [ ] 统计基线为**目标带 + 阶段门**结构（§8.3），含投篮分解；
- [ ] 种子矩阵（≥ 20 场）批量统计可生成。

**M4 主循环阶段化**
- [ ] `step()` 主体 < 100 行，仅做阶段列表调度；
- [ ] 每个 Phase 有独立单元测试；
- [ ] InvariantPhase 在列表末端，无路径可绕过；
- [ ] 重构后黄金哈希与重构前一致（行为不变）。

**M5 执行重校验**
- [ ] 决策意图落地前重新过硬约束；
- [ ] 世界已变时动作被降级/取消而非强行执行，且记入 trace；
- [ ] 叙事校验中"落地改变"率可统计。

**M6 L2/L3 校验**
- [ ] 每回合产出 `PossessionSummary` 且通过叙事校验；
- [ ] 统计基线 CI 化，含三分命中率 30–40% 目标带。

**M7 参数收编**
- [ ] 子系统 src 非测试代码内联浮点行为常数 ≤ 20 处（现状 ~734，§1.2 R5；court 几何、单位换算等真常数除外）；
- [ ] `officiating/config.rs` 并入 `GameRules` 体系或成为其子结构；
- [ ] `audit_stream.py` 限值副本删除，审计由 Rust 工具承担；
- [ ] 每一批收编黄金哈希不变或按协议重冻结。

### 13.2 质量验收（持续）

| 指标 | 阈值 | 检测 |
|------|------|------|
| L1 硬违反 | 0 / 全场 | 不变量系统 |
| 两人持球/空气球 | 0 | BALL_* 规则 |
| 比分倒退/时钟回流 | 0 | SCORE/CLOCK 规则 |
| 决策 trace 覆盖率 | 100% | L2 |
| 相同种子确定性 | 逐 tick 完全一致 | 黄金哈希 |
| 全场总分（两队和） | [190, 270]（终态目标带） | 阶段门 |
| 三分命中率 | 30%–40% | 阶段门（先补采集） |
| 回合平均时长 | 14–22 秒 | 阶段门 |
| release 吞吐 | ≥ 20,000 ticks/s | §10 基准 |

### 13.3 架构与工具链验收

- [ ] `domain` 无任何对上层 crate 的依赖（`cargo tree` 验证）；
- [ ] 子系统无反向依赖（physics 不知 decision，semantics 不知 officiating）；
- [ ] 所有魔法数字可在 `GameRules` 中找到，无散落硬编码（≤20 处豁免清单）；
- [ ] 新增一条 L1 规则只需在 `invariants` crate 加一个函数，不改动引擎；
- [ ] batch CLI / violations 工件 / CI 三件套存在并绿。

---

## 14. 附录

### 14.1 与 docs/diag1.md 的关系

`diag1` 是 MVP 产品愿景（要有什么：两队、战术、回放、UI）。本文档是**工程正确性基线**（如何保证模拟结果可信）。两者共享同一 crate 分层；v1.1 新增的 `invariants` crate、BallState 状态机、阶段门校准协议、性能预算是对 `diag1` §3.2"比赛状态由引擎权威拥有"与"真实感优先"的具体落实。

### 14.2 术语表

| 术语 | 定义 |
|------|------|
| **事实源 (SoT)** | 某一信息的唯一权威存储，其余为派生 |
| **不变量** | 任何时刻都必须成立的物理/篮球底线条件 |
| **意图 (Intent)** | 决策产出但尚未执行的动作，携带决策上下文 |
| **执行重校验** | 意图落地时基于当前世界重新过约束 |
| **黄金哈希** | 标准种子逐 tick 关键字段的串联哈希，用于确定性回归 |
| **阶段门 (Stage Gate)** | 校准期间允许统计分布停留的走廊，随进展逐级收窄至目标带 |
| **叙事校验** | 对单回合事件序列的因果一致性检查 |
| **ShortCircuit** | 阶段管线中本 tick 提前结束的显式信号 |
| **校准协议** | 修改任何影响行为的默认值必须满足的流程（§11.3） |

### 14.3 关键 crate 依赖图

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

补边（v1.2）：`engine ──► invariants`——`step()` 包装器每 tick 调用 `check_tick`（§8.1），该依赖在 v1.1 图中缺失。

### 14.4 变更记录

| 版本 | 日期 | 变更 |
|---|---|---|
| v1.0 | 2026-08 | 初版：诊断、架构、M1–M6、验收标准 |
| v1.1 | 2026-08-31 | 审计修订：R5 硬编码根因与 P6 原则；§8.3 阶段门锚定哲学（替代"实测±%"）；§9 工具链单列；§10 性能预算；§11.2 差距矩阵；§11.3 校准协议；M7 参数收编；验收补 severity/violations 工件/CI/投篮分解/吞吐 |
| v1.2 | 2026-09-01 | 复审修订。**设计自洽修正**：§4.2 位置是派生量（闭式采样，消除与单一写入口的矛盾）+ 现有 `domain::BallState` 投影的演进路径；§2.2 球权写入改"通道唯一"（废止"只在 [7] 写入"）并为"事实"下定义；§5.2 阶段管线改窄签名静态分发（原 `&mut World` 签名无法兑现 §5.4 借用隔离）；§8.1 新增阶段语义条款（发球界外/死球豁免）+ 禁止投影规避 + 移除兜底限值要求；§4.4 转换表补罚球/跳球边；§4.3 写入纪律强制机制改为字段私有 + grep 守卫；§6.2 重决策语义明确为下一决策 tick；§2.2 [4] 决策门控含死球发球；§8.2 回合时长下界修正；§8.3 门梯更新（G4 当前、目标带 [190,270] 权威）+ 判定口径；§9.1/§11.3 CLI 现状与目标区分；§2.1/§14.3 依赖规则补 officiating→semantics、engine→invariants。**漂移刷新**：§8.1 规则表 11→20 条；§1.2 R5 常量 672→~734（附审计命令）；§11.2 差距矩阵全面刷新（M1~90%/M3~70%/M6~20%）；§5.1 step_inner 现状（~920 行/10 出口）；§3.2 补 DecisionRules/ResolveConfig；§13.1 负面对照 11 项及位置 |
