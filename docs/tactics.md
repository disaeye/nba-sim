# NBA-Sim · 阵容与战术模块 · 数据契约设计

> 版本：v1.1（2026-09-01 对抗性审查修订：引用重建、效用代数与复合律声明、T1 验收互斥修复、阈值归属；历史见 status.md §6.3）
> 版本状态：**冻结层** = §1 设计原则、§3 分层架构、§4 耦合边界（T1–T8 全程不变；修订须升主版本并附证据）；**工作版本** = §2 类型与档案结构（随 §8 开放问题的接线证据演化）
> 定位：阵容管理与战术档案的**单一事实源**——球员档案（attributes.md）之上、引擎（design.md）之下的**球队组织层**：谁上场、打什么体系、体系如何适配球员
> 上游文档：`docs/charter.md`（宪章：C1 无硬编码、§5 能力涌现）；`docs/attributes.md`（属性契约：PA1 能力与倾向分离、PA7 机器面与展示面分离）；`docs/design.md`（架构与里程碑：M9 能力耦合）
> 适用范围：`crates/domain/src/data.rs`（TeamData/LineupConfig）、`crates/engine/src/setup.rs`、`crates/decision/src/tactics.rs`、战术 JSON 档案、轮换与指派管线

---

## 0. 为什么独立成文

1. **战术是组织，不是属性。** attributes.md 回答"一个球员是什么"，本文档回答"一组球员如何被组织成球队"——阵容（谁在场上）、战术（体系是什么）、适配（体系如何调用球员）。三者是组织关系，不是球员禀赋，不能塞进 PlayerData。
2. **现状是硬编码剧本，必须替换。** `crates/decision/src/tactics.rs` 的 `TacticalPlanner` 为每个 `TacticalSet` 枚举变体手写一套几何点位（`pg_spot`/`c_spot`/`screen_offset`…），slot 名（`"BallHandler"`/`"CornerSpacer"`）是显示字符串、球员按索引绑定——**这是宪章 C1 明文禁止的"预编排的回合脚本"**。设计必须给出从"枚举剧本"到"数据档案+能力适配"的替换路线。
3. **耦合面是三层架构的中间层，必须钉死。** 战术层消费球员档案（只读）、被引擎消费（经窄契约），同时向教练/经营层暴露决策接口。中间层的边界不清，则上下层一起腐烂。

**耦合面声明（本文档的宪法）**：战术层对球员的全部合法知识 = attributes.md §2 的类型（只读）；对引擎的全部合法输出 = §3.4 的战术指令（目标/角色/触发）；任何让战术认识 roster 身份（球员 id 分支）的改动违反宪章 C1，直接打回。

---

## 1. 设计原则

| # | 原则 | 一句话表述 | 守卫 |
|---|------|-----------|------|
| **TA1** | 战术是数据不是代码 | 战术体系由 JSON 档案声明（阵型、触发、优先级、分支），引擎认识"档案的类型"，不认识"某个具体战术" | 战术档案 schema 版本化；新增战术 = 新档案，零引擎改动 |
| **TA2** | 指派与执行分离 | 指派（slot 分配、对位、换人）是**离散决策**（回合级），执行（跑位、掩护、协防）是**连续行为**（tick 级）；前者产出指令，后者由 decision/physics 消费 | 指派器输出 TargetAssignment/RotationEvent，执行器零指派逻辑 |
| **TA3** | 角色是槽位不是身份 | 战术档案声明**槽位**（slot：需要什么样的球员），球员按能力填充；slot 是战术的输入接口，不是球员的本体身份 | 槽位需求 ↔ attributes.md §2.9 派生视图匹配；球员无 `role` 字段（attributes.md §2.7） |
| **TA4** | 适配是纯函数 | 阵容适配、slot 填充、对位指派 = `(roster, tactics, game_state) → assignment` 的纯函数；可复现、可扰动、可单测 | 适配器落 `domain::lineup`（零依赖）；随机性经种子注入 |
| **TA5** | 教练是策略不是脚本 | 教练决策（换人、暂停、战术切换）由**策略档案**驱动（何时换、换谁、换什么战术），不是预编排的比赛脚本 | CoachStrategy 消费 TeamTraits + 比赛状态；禁止硬编码时间/比分触发 |
| **TA6** | 战术与倾向共存不覆盖 | TeamTraits（球队意志）定基线，PlayerTendencies（个体偏好）在其上调制；两者是同层独立输入，各自生效、互不覆盖 | 效用管线中 TeamTraits 与 tendencies 作为独立加法项进入 BaseValue（各自权重归 DecisionRules），禁止合并字段或互相覆盖（architecture.md §5.1） |
| **TA7** | 无死档案字段 | 战术档案每个字段必须有 ≥1 条消费链（引擎行为响应）+ ≥1 条可观测的扰动响应链；死字段要么接线、要么删除 | 战术扰动测试（quality.md §6 / design.md §3.1 M9 扩展） |

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

**现状问题**：`LineupConfig.starters: [String; 5]` 硬编码首发、`bench: Vec<String>` 无结构、**无轮换机制**——`MatchEngine::with_setup` 载入段一次性把 starters+bench 载入物理世界后不再变动，48 分钟无换人（现状详见 `status.md` §3.1）。

| 字段 | 语义 | 类型 | 消费链 |
|------|------|------|--------|
| `active_roster` | 激活名单（12–15 人） | `Vec<PlayerId>` | 比赛初始化载入物理世界 |
| `starters` | 首发五人 | `[PlayerId; 5]` | 跳球与首节开局 |
| `rotation` | 轮换表：换人时机与对象 | `Vec<RotationEntry>` | 死球时触发换人 |
| `on_court` | 当前在场五人（运行态） | `[PlayerId; 5]` | 引擎持有，档案只读 |

`RotationEntry`（换人条目）：

| 字段 | 语义 | 示例 |
|------|------|------|
| `trigger` | 触发条件 | `TimeMark(period=1, clock=6:00)` / `Fatigue(threshold=0.75)` / `FoulCount(2)` |
| `out_player` | 换下球员 | `PlayerId` 或 `Slot(slot_id)`（`slot_id` 为战术档案 §2.2.1 的槽位标识） |
| `in_player` | 换上球员 | `PlayerId` |
| `priority` | 冲突时优先级 | `u8`（数值大者优先） |

**纪律**：
- 阵容是**离散决策**（TA2）——只在死球/暂停/节间触发，不打断连续比赛；
- 换人是**纯函数**（TA4）——`(rotation, game_state) → substitution_event`；
- 球员档案**不可变**（attributes.md §3.4）——轮换表只引用 `PlayerId`，不修改 PlayerData。

### 2.2 战术层（Tactics）

**现状问题**：`TacticalSet::HighPickAndRoll` 等 6 个枚举变体在 `TacticalPlanner::plan_possession_targets` 手写几何点位——每个战术一套 `pg_spot/c_spot/screen_offset` 硬编码，slot 名（`"BallHandler"`）是显示字符串，球员按 `carrier_idx` 索引绑定（现状详见 `status.md` §3.1）。**这是剧本，不是战术**。

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

**与现状的差异**：
| 现状（剧本） | 目标（档案） |
|-------------|-------------|
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
|------|---------|---------|
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

**现状问题**：`CoachStrategy`（`modulation.rs`）是硬编码的心理状态机（HotHand/Frustrated/Clutch），无换人/暂停/战术切换决策。

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

**现状问题**：`TacticalPlanner::plan_possession_targets` 返回 `Vec<TargetAssignment>`（目标位置 + slot 名字符串），`MatchEngine` 将其经 `set_player_target` 直接写入物理世界——**绕过 decision 管线**，球员无实时决策，只是执行剧本（现状详见 `status.md` §3.1）。

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
| 现状（剧本执行） | 目标（战术指导决策） |
|-----------------|---------------------|
| TacticalPlanner 直接生成 TargetAssignment | TacticalPlanner 生成**候选动作集** |
| match_engine 直接 set_player_target | DecisionSystem 效用评估 + 约束裁决 |
| 球员无选择，执行剧本 | 球员在战术框架内实时决策 |
| 扰动能力 → 行为不变（剧本短路） | 扰动能力 → 行为单调响应（涌现） |

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
               − soft_penalty − risk_penalty + morale_bias
```

---

## 6. 现状基线与差距

> 现状审计（剧本化、无轮换、无适配、无教练决策、原型表达力测试）已迁至 `docs/status.md` §3——本文档只保留设计契约。

---

## 7. 迁移路线（先档案化，再接线，后涌现）

**顺序原则**：先把硬编码剧本换成数据档案（T1–T3），再接适配层（T4–T5），最后接决策涌现（T6–T8）。档案化是行为中性重构，接线是行为扩展，涌现是行为质变——三层风险递增，必须分开验收。

| 步 | 内容 | 依赖 | 验收 |
|----|------|------|------|
| T1 | **战术档案 schema**：`OffensiveSystem`/`DefensiveSystem`/`SituationalTactics` JSON 类型落 `domain/data.rs`；现有 6 个 `TacticalSet` 枚举 → JSON 档案（行为等价） | 无 | serde 兼容；黄金哈希不变 |
| T2 | **战术加载管线**：`TacticalSet::from_id` → `TacticsLibrary::load(id)`；档案库目录 + 版本化 | T1 | 新增战术 = 新 JSON 文件，零引擎改动 |
| T3 | **slot 结构化**：`slots: Vec<Slot>`（含 `requirements` 能力需求）替换字符串数组 | T1 | slot 需求可表达（ball_handling > 0.7 等） |
| T4 | **适配层**：`domain::lineup` 纯函数模块（`fill_slots`/`assign_matchups`/`decide_substitution`） | T3, attributes.md T1 | slot 填充扰动测试过（扰动 ball_handling → 分配变化） |
| T5 | **轮换机制**：`RotationEntry` 类型 + 死球触发换人 + `SubstitutionEvent` | T4 | 48 分钟比赛有换人；扰动 stamina → 换人时机变化 |
| T6 | **教练策略**：`CoachProfile` 档案 + 换人/暂停/战术切换决策 | T5 | 扰动 substitution_tendency → 换人频率变化 |
| T7 | **战术接决策**：`TacticalPlanner` 生成候选动作集 → `DecisionSystem` 效用评估（替换直接 `set_player_target`） | T3, design.md M9 | 扰动能力 → 战术执行变化（涌现验证） |
| T8 | **对位指派**：`assign_matchups` 接入防守决策（替换 `MatchEngine` 战术目标直写中硬编码的对位目标） | T4, T7 | 扰动 defense_perimeter → 对位分配变化 |

> T1–T3 为行为中性重构：**黄金哈希必须不变**（与 T1 验收互为表里）；自 T4 起的行为性变更若影响默认行为，按 `design.md` §2 协议重冻结并附证据。

---

## 8. 开放问题登记

| # | 问题 | 现状 |
|---|------|------|
| OQ-1 | 战术档案的粒度：一份档案 = 一套完整体系 vs 可组合的原子动作（screen/cut/iso）+ 组合规则 | 暂定完整体系（实现简单）；若扰动测试发现战术同质化（所有球队打同样的体系），再引入组合规则 |
| OQ-2 | slot 适配分的权重与回溯阈值：每个 slot 档案声明 vs 全局统一表 | 暂定 slot 档案声明（灵活）；若标定困难，再收编全局表 |
| OQ-3 | 对位指派的动态性：回合内是否允许换防（switch on the fly） | 暂定回合开始时静态指派；若换防体系（switch_heavy）扰动测试失真，再引入动态换防 |
| OQ-4 | 教练 AI 的复杂度：规则驱动（if-then）vs 效用驱动（utility-based） | 暂定规则驱动（可解释）；若教练决策同质化，再引入效用驱动 |
| OQ-5 | 战术档案的版本化：战术库是否需要向后兼容（旧版本档案能否在新引擎跑） | 暂定不兼容（schema 升版本即废弃旧档案）；若战术库积累过多，再引入兼容层 |

---

## 9. 变更记录

> 按文档治理规则（`README.md` 修订规则 #4），本表已收敛至单一事实源：**变更记录见 `docs/status.md` §6.3**。
