# NBA-Sim · 阵容与战术模块 · 数据契约设计

> 定位：球员档案之上的球队组织契约，定义阵容、战术档案、槽位适配、对位、轮换和教练策略的语义。
> 上游契约：`docs/charter.md`、`docs/attributes.md`；决策与物理边界见 `docs/architecture.md`。
> 实现状态、迁移顺序和验证范围见 `docs/dev/status.md`、`docs/dev/gap.md` 和 `docs/dev/current/plan.md`。
> 修订纪律：本文档只写目标 schema 和稳定边界，不写当前实现、历史轮次或执行命令。

---

## 0. 为什么独立成文

1. **战术是组织，不是属性。** attributes.md 回答"一个球员是什么"，本文档回答"一组球员如何被组织成球队"——阵容（谁在场上）、战术（体系是什么）、适配（体系如何调用球员）。三者是组织关系，不是球员禀赋，不能塞进 PlayerData。
2. **战术不能退化为硬编码剧本。** 档案声明机会、职责、触发和约束；球员通过能力适配槽位；决策层仍然拥有实时选择权。任何把具体球员、固定坐标或回合顺序写成不可变代码的实现都违反 C1。
3. **耦合面是三层架构的中间层，必须钉死。** 战术层消费球员档案（只读）、被引擎消费（经窄契约），同时向教练/经营层暴露决策接口。中间层的边界不清，则上下层一起腐烂。

**耦合面声明（本文档的宪法）**：战术层对球员的全部合法知识 = attributes.md §2 的类型（只读）；对引擎的全部合法输出 = §3.4 的战术指令（目标/角色/触发）；任何让战术认识 roster 身份（球员 id 分支）的改动违反宪章 C1，直接打回。

---

## 1. 设计原则

| # | 原则 | 一句话表述 | 守卫 |
| --- | ------ | ----------- | ------ |
| **TA1** | 战术是数据不是代码 | 战术体系由 JSON 档案声明（阵型、触发、优先级、分支），引擎认识"档案的类型"，不认识"某个具体战术" | 战术档案 schema 版本化；新增战术 = 新档案，零引擎改动 |
| **TA2** | 指派与执行分离 | 指派（slot 分配、对位、换人）是**离散决策**（回合级），执行（跑位、掩护、协防）是**连续行为**（tick 级）；前者产出指令，后者由 decision/physics 消费 | 指派器输出 TargetAssignment/RotationEvent，执行器零指派逻辑 |
| **TA3** | 角色是槽位不是身份 | 战术档案声明**槽位**（slot：需要什么样的球员），球员按能力填充；slot 是战术的输入接口，不是球员的本体身份 | 槽位需求 ↔ attributes.md §2.9 派生视图匹配；球员无 `role` 字段（attributes.md §2.7） |
| **TA4** | 适配是纯函数 | 阵容适配、slot 填充、对位指派 = `(roster, tactics, game_state) → assignment` 的纯函数；可复现、可扰动、可单测 | 适配器落 `domain::lineup`（零依赖）；随机性经种子注入 |
| **TA5** | 教练是策略不是脚本 | 教练决策（换人、暂停、战术切换）由**策略档案**驱动（何时换、换谁、换什么战术），不是预编排的比赛脚本 | CoachStrategy 消费 TeamTraits + 比赛状态；禁止硬编码时间/比分触发 |
| **TA6** | 战术与倾向共存不覆盖 | TeamTraits（球队意志）定基线，PlayerTendencies（个体偏好）在其上调制；两者是同层独立输入，各自生效、互不覆盖 | 效用管线中 TeamTraits 与 tendencies 作为独立加法项进入 BaseValue（各自权重归 DecisionRules），禁止合并字段或互相覆盖（architecture.md §5.1） |
| **TA7** | 无死档案字段 | 战术档案每个字段必须有 ≥1 条消费链（引擎行为响应）+ ≥1 条可观测的扰动响应链；死字段要么接线、要么删除 | 战术扰动测试（quality.md §6 / protocol.md §2.1 M9 扩展） |

---

## 2. 分类学（目标 schema v1）

### 2.0 总览

```
球队组织层（本文档）
├── 阵容（Lineup · 谁上场）
│   ├── 激活名单    active_roster: Vec<PlayerId>（12–15 人）
│   ├── 轮换表      rotation: Vec<RotationEntry>（换人时机与对象）
│   └── 当前在场    on_court: [PlayerId; 5]（运行态，引擎持有）
├── 战术（Tactics · 打什么体系）
│   ├── 进攻体系    offensive_system: OffensiveSystem（阵型 + 动作序列）
│   ├── 防守体系    defensive_system: DefensiveSystem（对位 + 协防 + 挡拆策略）
│   └── 特殊情境    situational: SituationalTactics（关键时刻、最后两攻、领先/落后）
├── 适配（Fit · 体系如何调用球员）
│   ├── 槽位填充    slot_fill: (slot_requirements, roster) → assignment
│   ├── 对位指派    matchup: (offensive_set, defensive_roster) → defensive_assignment
│   └── 轮换决策    rotation_decision: (fatigue, foul, matchup, score) → substitution
└── 教练（Coach · 谁做决定）
    ├── 策略档案    coach_profile: CoachProfile（换人/暂停/战术切换倾向）
    └── 临场调整    in_game_adjustment: (game_state) → tactic_override
```

### 2.1 阵容层（Lineup）

| 字段 | 语义 | 类型 | 消费链 |
| ------ | ------ | ------ | -------- |
| `active_roster` | 激活名单（12–15 人） | `Vec<PlayerId>` | 比赛初始化载入物理世界 |
| `starters` | 首发五人 | `[PlayerId; 5]` | 跳球与首节开局 |
| `rotation` | 轮换表：换人时机与对象 | `Vec<RotationEntry>` | 死球时触发换人 |
| `on_court` | 当前在场五人（运行态） | `[PlayerId; 5]` | 引擎持有，档案只读 |

`RotationEntry`（换人条目）：

| 字段 | 语义 | 示例 |
| ------ | ------ | ------ |
| `trigger` | 触发条件 | `TimeMark(period=1, clock=6:00)` / `Fatigue(threshold=0.75)` / `FoulCount(2)` |
| `out_player` | 换下球员 | `PlayerId` 或 `Slot(slot_id)`（`slot_id` 为战术档案 §2.2.1 的槽位标识） |
| `in_player` | 换上球员 | `PlayerId` |
| `priority` | 冲突时优先级 | `u8`（数值大者优先） |

**纪律**：

- 阵容是**离散决策**（TA2）——只在死球/暂停/节间触发，不打断连续比赛；
- 换人是**纯函数**（TA4）——`(rotation, game_state) → substitution_event`；
- 球员档案**不可变**（attributes.md §3.4）——轮换表只引用 `PlayerId`，不修改 PlayerData。

### 2.2 战术层（Tactics）

**目标 schema**：战术档案 = **阵型（Formation）+ 动作序列（ActionSequence）+ 触发条件（Triggers）** 的数据声明。

#### 2.2.1 进攻体系（OffensiveSystem）

```json
{
  "id": "high_pnr_v1",
  "name": "高位挡拆",
  "formation": {
    "slots": [
      {"id": "handler", "pos_hint": [25.0, 25.0], "requirements": {"ball_handling": 0.7}},
      {"id": "screener", "pos_hint": [30.0, 25.0], "requirements": {"strength": 0.7, "screen_frequency": 0.6}},
      {"id": "corner_left", "pos_hint": [5.0, 5.0], "requirements": {"shooting_three": 0.6}},
      {"id": "corner_right", "pos_hint": [5.0, 45.0], "requirements": {"shooting_three": 0.6}},
      {"id": "dunker", "pos_hint": [10.0, 25.0], "requirements": {"finishing": 0.7}}
    ]
  },
  "sequence": [
    {"action": "screen", "actor": "screener", "target": "handler", "duration_sec": 1.5},
    {"action": "drive_or_pass", "actor": "handler", "options": ["drive", "pass_roll", "pass_pop", "pass_corner"], "decision_point": true}
  ],
  "triggers": {
    "preferred_score_range": [0.4, 0.8],
    "pace_multiplier": 1.0,
    "counter_to_defense": ["def_drop_coverage"]
  }
}
```

**关键字段**：

- `formation.slots`：阵型槽位 + **能力需求**（TA3）——slot 是战术的输入接口，不是球员身份；
- `sequence`：动作序列——每个动作声明 actor（slot id）、类型（screen/cut/drive/pass/shoot）、目标、时长；`decision_point: true` 标记需要 decision 管线实时裁决的分支；
- `triggers`：触发条件——比分范围、节奏调制、对特定防守体系的克制关系。

**规范对照**：

| 禁止的剧本化模式 | 契约要求的档案模式 |
| ------------- | ------------- |
| `TacticalSet::HighPickAndRoll` 枚举变体 | `offensive_system.id = "high_pnr_v1"` JSON 档案 |
| `pg_spot/c_spot` 硬编码几何 | `formation.slots[].pos_hint` 档案字段 |
| `slots = ["BallHandler", "CornerSpacer", ...]` 显示字符串 | `slots[].requirements` 能力需求（数值区间） |
| 球员按 `carrier_idx` 索引绑定 | `slot_fill` 纯函数按能力匹配（§2.3.1） |
| 新增战术 = 改枚举 + 写代码 | 新增战术 = 写 JSON 档案 |

#### 2.2.2 防守体系（DefensiveSystem）

```json
{
  "id": "switch_heavy_v1",
  "name": "大量换防",
  "base_scheme": "man_to_man",
  "on_ball": {"pressure": 0.8, "contest_height_penalty": 0.2},
  "help": {"help_aggressiveness": 0.6, "rotation_speed": 0.7},
  "screen_defense": {"strategy": "switch", "switch_threshold": 0.8, "mismatch_tolerance": 0.6},
  "matchup_rules": [
    {"condition": "opponent_handler.shooting_three > 0.75", "action": "deny_catch"}
  ]
}
```

**关键字段**：

- `base_scheme`：基础阵型（man_to_man / zone_23 / zone_32 / box_and_one…）；
- `on_ball/help/screen_defense`：防守三层的策略参数（压力、协防倾向、挡拆策略）；
- `matchup_rules`：对位规则——条件（对位者能力阈值）→ 动作（deny_catch/sag_off/double_team）。

**复合律（与 attributes.md §2.6/§2.8 对齐）**：防守强度存在三层输入——`DefensiveSystem`（结构与档位参数：触发阈值、压力半径）、`TeamTraits`（球队执行强度基线）、`PlayerTendencies`（个体在其上调制）。三层为独立输入，禁止任何一层覆盖或吞并另一层；进入效用的部分按 `architecture.md` §5.1 作为独立加性项/阈值参数生效，档位参数走规则通道。

#### 2.2.3 特殊情境（SituationalTactics）

| 情境 | 触发条件 | 战术覆盖 |
| ------ | --------- | --------- |
| 关键时刻 | 第四节/加时 + 分差 ≤5 | 降低节奏、提高单打权重、缩短轮换 |
| 最后两攻 | 节末 + 进攻时间 < 24s | 预设最后一攻战术（quick_hitter / iso_clear_out） |
| 领先/落后 | 分差 > 10 + 时间 < 5min | 领先：压节奏、磨时间；落后：抢三分、犯规战术 |

**纪律**：特殊情境是**战术覆盖**（override），不是独立体系——触发时临时替换 offensive/defensive_system 的特定字段，不替换整个档案。

### 2.3 适配层（Fit）

**核心问题**：战术档案声明 slot 需求（"需要一个 ball_handling > 0.7 的持球者"），阵容里有 5 名球员——**谁填哪个槽位**？

#### 2.3.1 槽位填充（Slot Fill）

```rust
pub fn fill_slots(
    slot_requirements: &[SlotRequirement],  // 战术档案声明
    roster: &[PlayerData],                   // 球员档案（attributes.md）
    rng: &mut impl Rng,
) -> Result<SlotAssignment, FitError>
```

**算法**（贪心 + 回溯）：

1. 对每个 slot，计算 roster 中每名球员的**适配分**（fitness score）：

   ```
   fitness(player, slot) = Σ(weight_i × min(1, player.attr_i / slot.requirement_i))
   ```

   其中 `attr_i` 是 slot 需求的能力维度（ball_handling/shooting_three/…），`weight_i` 是该维度的权重（slot 档案声明）；
2. 贪心分配：按 slot 的**稀缺性**（满足需求的球员数）排序，先填充最难满足的 slot；
3. 冲突回溯：若某 slot 无可用球员（适配分 < 该 slot 档案声明的 `min_fitness`；未声明时取适配层全局默认值——全局表的归属随 OQ-2 一并裁定），回溯调整先前分配；
4. 输出：`SlotAssignment = HashMap<SlotId, PlayerId>`。

**扰动测试**（TA7）：

- 扰动某球员的 `ball_handling`：该球员被分配到 handler slot 的概率应单调变化；
- 扰动 slot 需求阈值：分配到该 slot 的球员能力分布应单调变化。

#### 2.3.2 对位指派（Matchup）

```rust
pub fn assign_matchups(
    offensive_set: &OffensiveSystem,     // 对方进攻体系
    defensive_roster: &[PlayerData],     // 我方防守阵容
    defensive_system: &DefensiveSystem,  // 我方防守体系
) -> DefensiveAssignment
```

**输入**：对方 5 名进攻球员的 slot 分配（`SlotAssignment`）+ 我方 5 名防守球员 + 防守体系档案；
**输出**：`DefensiveAssignment = HashMap<DefenderId, OffensivePlayerId>` + 协防责任（help assignments）。

**算法**（匈牙利匹配 / 贪心）：

1. 构建代价矩阵 `cost[defender][offensive_player]`：

   ```
   cost = w1 × |defender.defense_perimeter - offensive.shooting_three|
        + w2 × |defender.defense_interior - offensive.finishing|
        + w3 × |defender.speed - offensive.speed|
        + w4 × mismatch_penalty(defender.height, offensive.height)
   ```

2. 最小化总代价（匈牙利算法，或贪心近似）；
3. 应用 `defensive_system.matchup_rules` 覆盖（如 `deny_catch` 条件触发）；
4. 输出对位 + 协防责任图（谁是第一协防、谁是轮转补位）。

#### 2.3.3 轮换决策（Rotation Decision）

```rust
pub fn decide_substitution(
    rotation: &[RotationEntry],       // 轮换表
    game_state: &GameState,           // 比赛状态（时间、比分、犯规）
    player_states: &[PlayerState],    // 球员运行态（耐力、犯规数）
    coach_profile: &CoachProfile,     // 教练策略
) -> Option<SubstitutionEvent>
```

**触发优先级**（高 → 低）：

1. **强制换人**：犯规数 ≥ 6（NBA）/ 受伤 / 被罚下；
2. **战术换人**：轮换表的 `trigger` 条件满足（时间/疲劳/对位劣势）；
3. **教练临场**：`coach_profile` 的换人倾向 + 比赛状态（落后时提前上主力、垃圾时间上替补）。

**输出**：`SubstitutionEvent { out: PlayerId, in: PlayerId, reason: SubstitutionReason }`，引擎在死球时执行。

### 2.4 教练层（Coach）

**目标 schema**：

```json
{
  "id": "coach_popovich_v1",
  "name": "波波维奇",
  "substitution_tendency": 0.7,
  "timeout_tendency": 0.5,
  "tactic_adjustment_tendency": 0.6,
  "preferred_lineups": [
    {"context": "closing", "lineup": ["pg1", "sg2", "sf3", "pf4", "c5"]}
  ],
  "preferred_tactics": {
    "default": "high_pnr_v1",
    "vs_zone": "five_out_motion_v1",
    "trailing_late": "quick_three_v1"
  }
}
```

**关键字段**：

- `substitution_tendency`：换人倾向（0=极少换人、1=频繁轮换）；
- `timeout_tendency`：暂停倾向（0=不叫暂停、1=频繁暂停）；
- `tactic_adjustment_tendency`：战术调整倾向（0=一套战术打到底、1=频繁切换）；
- `preferred_lineups`：偏好评分阵容（closing lineup = 关键时刻阵容）；
- `preferred_tactics`：偏好评分战术（默认 + 对特定防守的克制战术 + 特殊情境战术）。

**决策逻辑**（TA5）：

- 换人：`rotation_decision` 的触发条件 × `substitution_tendency` 调制；
- 暂停：对方得分高潮（连续得分 ≥ 8）× `timeout_tendency` 调制；
- 战术切换：对方防守体系变化 × `tactic_adjustment_tendency` 调制 + `preferred_tactics` 查表。

---

## 3. 分层架构（战术在引擎中的位置）

```
┌─────────────────────────────────────────────────────────┐
│  球队组织层（本文档）                                     │
│  ├── 阵容（Lineup）：谁上场                              │
│  ├── 战术（Tactics）：打什么体系                         │
│  ├── 适配（Fit）：体系如何调用球员                       │
│  └── 教练（Coach）：谁做决定                             │
└─────────────────────────────────────────────────────────┘
                        ↓ 战术指令（TargetAssignment / RotationEvent）
┌─────────────────────────────────────────────────────────┐
│  决策层（architecture.md §5）                               │
│  └── DecisionSystem：候选生成 + 效用评估 + 约束裁决       │
└─────────────────────────────────────────────────────────┘
                        ↓ 动作意图（CandidateAction）
┌─────────────────────────────────────────────────────────┐
│  物理层（architecture.md §6.1）                             │
│  └── PhysicsWorld：运动学 + 碰撞 + 球轨迹                 │
└─────────────────────────────────────────────────────────┘
```

**边界**：

- 战术层 → 决策层：`TargetAssignment`（目标位置 + 速度 + 动作类型）+ `RotationEvent`（换人事件）；
- 决策层 → 物理层：`CandidateAction`（Shoot/Drive/Pass/Cut/Screen…）；
- 物理层 → 战术层（反馈）：`GameState`（时间、比分、犯规、耐力）→ 触发轮换/暂停/战术调整。

---

## 4. 与属性的耦合边界（输入契约的执行细则）

```
PlayerData（attributes.md §2）──► 适配层（domain::lineup 纯函数）──► 战术指令
                                        ▲
TeamTraits / CoachProfile / Tactics JSON（本文档 §2）
```

- **适配层集中**：`球员能力 → slot 适配分`、`对位代价矩阵`、`轮换决策` 收敛为 `domain::lineup` 纯函数模块（零依赖可单测，engine/decision/officiating 共用）；
- **战术权重归档案**：slot 需求的权重、对位代价的权重、换人触发的阈值全部走战术 JSON 档案；本契约只定义字段语义，不定义权重数值；
- **禁止事项**：
  1. 战术层任何分支依赖球员 id（C1）；
  2. 子系统内散落适配逻辑（必须经 `domain::lineup`）；
  3. 战术档案直接当代码用（`TacticalSet` 枚举 + 硬编码点位）；
  4. 运行期修改战术档案（战术是不可变输入，临场调整 = 切换到另一份档案）。

---

## 5. 与决策层的耦合（战术如何变成行为）

**目标流程**：

```
战术档案（OffensiveSystem）
  → slot_fill 分配槽位（谁打哪个位置）
  → TacticalPlanner 生成候选动作集（每个 slot 的 CandidateAction 集合）
  → DecisionSystem 效用评估（球员能力 × 倾向 × 战术权重 × 比赛状态）
  → 约束裁决（constraint.rs：可行性检查）
  → 最优动作（CandidateAction）
  → PhysicsWorld 执行
```

**关键差异**：

| 禁止的剧本执行模式 | 契约要求的指导决策模式 |
| ----------------- | --------------------- |
| TacticalPlanner 直接生成固定目标 | TacticalPlanner 生成**候选动作集** |
| 编排层直接写物理目标 | DecisionSystem 效用评估 + 约束裁决 |
| 固定脚本决定球员动作 | 球员在战术框架内实时决策 |
| 能力扰动被脚本短路 | 能力扰动产生可解释响应 |

**候选动作集生成**：

```rust
pub fn generate_candidates(
    slot: &SlotAssignment,           // 槽位分配
    sequence: &[ActionStep],         // 战术动作序列
    game_state: &GameState,          // 比赛状态
    player: &PlayerData,             // 球员档案
) -> Vec<CandidateAction>
```

**示例**（高位挡拆）：

- handler slot：`[Drive, Pass(screener), Pass(corner_left), Pass(corner_right), Shoot]`
- screener slot：`[Roll, Pop, Screen]`
- corner_left/right slot：`[CatchAndShoot, Drive, Pass]`

每个候选动作的**战术基线效用**由战术档案声明（`sequence[].utility_weight`）。下列展开不是最终效用的独立公式，而是 `architecture.md` §5.1 效用式中 BaseValue 因子的内部构成——最终复合律以该节为唯一权威：

```
base_value (战术) = tactical_base (档案：sequence[].utility_weight)
                  + skill_utility (attributes.md：能力 × 权重)
                  + tendency_utility (attributes.md：倾向 × 权重)
                  + context_utility (architecture.md §5.1：比分、时间、对位经调制层)

FinalUtility = (base_value + preference_bonus) × feasibility × stamina_modulation
               − soft_penalty − risk_penalty + morale(action family)
```

`morale(action family)` 是士气标量与**动作族权重**的乘积（`ModulationRules.morale_*_affinity`）：
终结与突破为正权的族在自信高时提升，组织观察为零权的族在自信高时降低。
士气不能作为全候选共享的加性常数——采样是 softmax，共享项在归一化中抵消，
阶参数则对选择分布零影响。

---

## 6. 设计演化边界

本契约定义目标 schema 和职责，不规定某一周期的迁移顺序。迁移必须遵循：先建立档案类型，再建立纯函数适配，最后接入决策与执行；行为中性重构和行为变化分开验收。具体依赖和出口条件见 `docs/dev/gap.md` 与 `docs/dev/current/plan.md`。

---

## 7. 开放问题登记

| # | 问题 | 决策状态 |
| --- | ------ | ------ |
| OQ-1 | 战术档案的粒度：完整体系 vs 可组合原子动作 | 先以完整体系表达；若组合需求成为独立契约，再登记新决策 |
| OQ-2 | slot 适配分的权重与回溯阈值 | 优先由 slot 档案声明；全局默认只能作为显式规则档案 |
| OQ-3 | 对位指派的动态性 | 由防守档案声明是否允许回合内换防，并由责任链验证 |
| OQ-4 | 教练 AI 的复杂度：规则 vs 效用 | 先保持可解释的规则/档案接口；需要效用选择时另行决策 |
| OQ-5 | 战术档案的版本化 | schema 版本变更必须声明兼容策略；兼容层不是隐式要求 |

---
