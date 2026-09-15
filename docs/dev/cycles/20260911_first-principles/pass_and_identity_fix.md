# 传球时空一致性 + 身份去索引化 · 统一修复方案

> 文档类型：已结束周期的实施记录与历史证据。
> 历史属性：非当前状态来源；当前状态见 `docs/dev/status.md`，当前任务见 `docs/dev/current/plan.md`。
> 合并自本周期内已删除的传球时空设计与位置身份根因记录。
> 上游：`gap.md §9.5/§12.1`、`architecture.md §5`、`attributes.md §2.7/§2.9/T1`、`tactics.md TA3`、`charter C1`。
> 记录纪律：保留当时的证伪实验和门结果，不把历史结论升级为当前事实。
> **Round 序列**：Round-6~9 见 `docs/dev/cycles/20260911_first-principles/closure_plan.md`；本文档记录 Round-10~17。

---

## 0. 两条核心原则

### P-1 · 有限信息：传球双方都只能**预估**，不得全知全能

> 传球人预估传球路线并传出；接球人**同样只能预估**该路线。
> 预估可能错 ⇒ **接球人可能接不到**。

这不是"允许 bug"，而是**真实性要求**：

| 场景 | 正确行为 |
| --- | --- |
| 大个弧顶策应，给小个传突破提前量 | 传球人按小个的速度外推到**他预计**的落点；小个也按自己的判断跑 |
| 双方预估一致 | 顺利接球 |
| 小个启动方向/时机与传球人判断不同 | **接不到** —— 球落到空处，成为 loose ball / 失误 |

**当前实现违反 P-1**：`pending_pass_receiver` 把冻结的 `to_pos` 直接注入接球人的
运动目标（`match_engine.rs:2946/2956` → `receive_approach()`）。接球人**获得了
传球人的精确意图**，因此必然到位。这是上帝视角。

### P-2 · 顺序不得携带身份

> `roster` 数组顺序、`id` 中的序号，都不得决定"谁是什么"。

**当前实现违反 P-2**：`id = format!("{}_{}", prefix, index+1)`，而
`roles`/`attributes`/`tendencies` 全部由 `builtin_*(index)` 产出；
初始持球人与发球人由 `starters[0]`（index 0）决定。

---

## 1. 根因（含证伪实验，避免过度归因）

### 1.1 传球：三个时空量互不知情

| 量 | 决定者 | 现状 |
| --- | --- | --- |
| 落点 `to_pos` | `decision::pipeline` | `extrapolate_receiver_pos(receiver, **固定 0.65s**)` |
| 飞行时长 | `domain::rules::pass_duration` | `clamp(dist/32, 0.45, 1.4)` |
| 接球人行踪 | `engine::receive_approach` | 目标 = **冻结落点**（P-1 违反） |

固定 0.65s 领传的系统性误差（算术）：

| 距离 | 真实飞行 | 按 0.65s | 误差 |
| ---: | ---: | ---: | ---: |
| 8 ft | 0.45s | 0.65s | **+0.20s 领过头** |
| 39 ft | 1.22s | 0.65s | **−0.57s 领不足** |
| 50 ft | 1.40s | 0.65s | **−0.75s 领不足** |

决定性证据（seed 1, seq 2525→2529）：落点在接球人**背后** 8.0 ft，
接球人随后**跑过**落点 10.35 ft → 9 条 `PASS_CORRIDOR_REACHABLE` Hard。

### 1.2 身份：index 通过 id 编码

```rust
// crates/domain/src/data.rs:445
id: format!("{}_{}", prefix, index + 1),
attributes: builtin_attributes(index),
roles:      builtin_roles(index),
tendencies: builtin_tendencies(index),
```

**已做的证伪实验**（两个，结论相反，必须都看）：

实验 A —— 交换数组位置上的 `PlayerData`：

```text
baseline             hash=0xa1c51cc3d9316545  Identical for (0,1),(1,2),(3,4)
```

实验 B —— 交换 `H_1`/`H_5` 的 `attributes`/`roles`/`tendencies`：

```text
baseline     hash=0x3388b06fd6e3b019  score=29-22
swapped      hash=0xaa020bfb111a8f68  score=15-24   DIFFERENT
```

**更正后的精确边界**（第一版诊断曾过度归因为"能力涌现失效"，已否证）：

| 维度 | 由什么决定 | 判定 |
| --- | --- | --- |
| 能力/倾向 → 运动/决策/裁决 | `builtin_*(index)`，**随 id 绑定** | ✅ 链路有效（实验 B 证明） |
| 初始持球人 / 首回合发球人 | `player.id == starters[0]` | ❌ index 身份 |
| `roles` 字段 | `builtin_roles(index)` | ❌ 契约要求删除 |
| 跳球站位 | index 分配 + `// PG:` 注释 | ❌ 预编排位置 |

---

## 2. 设计

### 2.1 传球：两个独立层，各自表达不同的"接不到"

这是本方案的关键结构决定。当前把两种失败**混在一个概率里**：

```rust
// crates/officiating/src/resolution.rs:157
let probability = base_success - lane_risk*w + openness*w
                + passer.passing*w + receiver.ball_handling*w + ...;
rng.gen_bool(probability)   // ← 唯一的失败来源，且与位置无关
```

拆成两层：

#### 层 A · 运动层（**新增**，实现 P-1）

接球人**不知道** `frozen_to_pos`。他只有自己对该传球路线的**预估**：

```text
estimate_receiver_landing(receiver, perception, rules) -> Vec2
    # 输入：接球人的感知快照（球的当前位置与速度、传球人朝向、
    #       自己的速度与朝向、战术槽位目标）
    # 输出：他**以为**球会到的位置
    # 关键：不得读取 ball_state.to_pos（那是传球人的私有意图）
```

预估误差来源（真实且可解释）：

| 来源 | 机制 |
| --- | --- |
| 预判提前量不准 | 接球人按**自己的**速度/加速度估计，与传球人的估计不同 |
| 观察延迟 | 球的当前位置/速度是上一 tick 的（感知快照），非瞬时真值 |
| off_ball_sense 差异 | `off_ball_sense` 低 → 预估噪声大（**这是能力涌现的接入点**） |
| 战术任务冲突 | 接球人同时有槽位目标（如拉开底角），两者加权 |

接球人向**自己的预估值**收敛（复用 `receive_approach` 的制动模型），
**不向 `frozen_to_pos` 收敛**。

#### 层 B · 裁决层（**保留并改造**，与位置解耦）

`resolve_pass_arrival` 继续负责"到位了但没接稳"：

```text
probability = f(lane_risk, openness, passer.passing,
                receiver.ball_handling, catch_equilibrium)
```

**删除**其中隐含的"一定到位"假设——概率的语义改为
**"若球到达时双方位置接近，则能否接稳"**。

#### 两层的合成

```text
球到达时刻 T：
  d = |ball_pos(T) − receiver_pos(T)|
  if d > catch_radius(receiver):     # 层 A 失败
      → loose ball（球落到空处，双方可争抢）
  else:                              # 层 A 成功，进入层 B
      → rng 掷 resolve_pass_arrival  # 接稳 / 没接稳（drop）
```

**这才是真实的**：先看人在不在球附近（运动），再看拿不拿得住（技术）。

#### 为什么这修复了当前 9 条 Hard

当前落点由传球人单方冻结，接球人被迫收敛到它 ⇒ 若传球人的落点估计
与接球人的实际运动不符，`PASS_CORRIDOR_REACHABLE` 必然报警（实测 5–10 ft）。
分层后：

- **落点仍由传球人冻结**（`gap.md §9.5` 的事实冻结保留）；
- **接球人不被迫收敛到它**（P-1）；
- 评判器的语义随之明确为：**球到时的位置差** → 若超出 catch_radius，
  那是**预期的** loose ball，不是 Hard defect；**Hard 的判据改为**
  「同一 tick 内 `to_pos` 被改写」或「落点与传球人自己的预估不一致」
  （即真正的因果断裂）。

> 评判器口径调整**不是放宽门**：它把"运动结果不确定"与"因果链断裂"
> 分开。前者是设计意图，后者才是缺陷。（对照 `impact_assessment.md`
> 批评的"为让门变绿而放宽"——此处同时**新增**了更严的因果判据。）

### 2.2 身份：声明式档案 + 能力派生

#### 步骤 1 · 数据档案化

新增 `data/roster/*.json`，球员作为数据资产声明：

```json
{
  "schema_version": 1,
  "team": "home",
  "players": [
    {"id": "H_01", "name": "...", "jersey": "1",
     "physical": {"height_cm": 191, "weight_kg": 88, "wingspan_cm": 203},
     "attributes": {"speed": 0.88, "passing": 0.86, ...},
     "tendencies": {"shoot_frequency": 0.68, ...}}
  ]
}
```

**关键约束**：

- **无 `roles` 字段**（契约要求删除；展示标签改为纯函数投影）
- **`id` 不含序号语义**（用 `H_01` 大写补零仅为可读，不参与任何判定）
- **数组顺序不携带语义**（放哪都一样）

#### 步骤 2 · 删除 `roles`

- 删 `PlayerData.roles`、`PlayerPhysicsState.roles`、`builtin_roles()`
- `match_engine.rs:412` 的展示 `slot` 改为 `attributes.md §2.9` 的纯函数投影

#### 步骤 3 · 初始持球人 / 发球人改由能力+slot 派生

```rust
fn select_inbounder(&self) -> Option<String> {
    // 复用已存在的 TacticalPlanner::fill_slots（D5.1b 已实现）
    // 槽位需求来自战术档案；球员按能力适配
    self.fill_slots(off_spec, &on_court_fitness)?.inbounder_slot()
}
```

**注意**：`fill_slots` **已经存在**且按能力适配（round-6 记录）。
本条是"数据在但没接线"的又一例——把 index 换成 slot fill。

#### 步骤 4 · 跳球站位去 index

`data.rs:420` 的 `// PG:` 注释与 index 分配改为**几何+能力派生**
（如"身高最高的两人跳球"已存在 `select_jumper_id`，其余按几何散布）。

---

## 3. 验收门（先红后绿）

| # | 门 | 断言 | 现状 |
| --- | --- | --- | --- |
| G1 | `receiver_does_not_read_frozen_target` | 接球人运动目标不得读取 `ball_state.to_pos` | 红（当前直接注入） |
| G2 | `pass_landing_is_estimate_consistent` | 传球人落点 = 他按**真实飞行时长**的预估（非固定 0.65s） | 红 |
| G3 | `catch_requires_spatial_proximity` | 层 A 失败（球人距离 > catch_radius）⇒ loose ball，不是 `PassReceived` | 红 |
| G4 | `pass_failure_rate_both_layers` | 传球失败率 = 层 A + 层 B，且两者可分别观测 | 红（当前只有层 B） |
| G5 | `roster_order_does_not_affect_behaviour` | **打乱 `data/roster/*.json` 顺序，黄金哈希不变** | 红（当前 id 编码 index） |
| G6 | `no_player_role_field` | `grep -rn "PlayerRole" crates/` 仅剩展示投影 | 红 |
| G7 | `inbounder_derived_from_ability` | 交换两名球员档案 ⇒ 发球人随之变化；打乱顺序 ⇒ 不变 | 红 |

**既有门不得回归**：`defense_effect`（2）、`attribution_integrity`（2）、
`PASS_CORRIDOR_REACHABLE`、L1 = 0。

---

## 4. 实施顺序（每步可独立验证）

```text
① G1/G3 先行（层 A 分层）      ← 先红：接球人当前直读 frozen target
   ↓ 1q 单种子验证
② G2（落点用真实飞行时长，删固定 0.65s）
   ↓
③ 评判器口径分离（运动不确定 vs 因果断裂）
   ↓ 8 seed 1q
④ G5/G6/G7（身份去索引化：data/roster/*.json + 删 roles + slot fill）
   ↓ 决定性检验：打乱顺序哈希不变
⑤ 重跑全部既有门 + 8 seed full
```

**为什么①在最前**：P-1 是本方案的核心原则；其余步骤（含落点求解）都建立在
"双方各自预估"之上。若先做②，落点仍会被强制注入接球人，等于白做。

**为什么④独立**：身份去索引化与传球时空正交，但**必须先有 G5 这个检验**
（打乱顺序哈希不变），否则改完无法证明真的解耦了。

---

## 5. 成本（按实测速率）

| 阶段 | 验证 | 成本 |
| --- | --- | --- |
| ① 层 A 分层 | 编译 40s + 1q 2s | ~1 min |
| ② 落点求解 | 纯函数单测 <1s + 1q 2s | ~1 min |
| ③ 评判器口径 | 1q 2s | <1 min |
| ④ 身份去索引 | 编译 + G5/G6/G7 | ~3 min |
| ⑤ 定稿 | 8 seed full 70s + workspace 9min | ~11 min |
| **合计** | | **~17 min** |

（纪律：全量 workspace 测试**每轮只跑一次**，在⑤。）

---

## 6. 需同步更新的契约（设计变更，非实现细节）

| 文档 | 变更 | 理由 |
| --- | --- | --- |
| `gap.md §9.5` | 「接球人由物理运动向 `frozen_to_pos` 收敛」→「接球人向**自己的预估落点**收敛；`frozen_to_pos` 仍是唯一冻结事实，但不得作为接球人的输入」 | P-1：全知全能违反真实性 |
| `gap.md §8.3` | 新增 `RECEIVE_ESTIMATE_DIVERGENCE` 准则：球到时的位置差，超 catch_radius 记 **informational**（非 defect），用于观测预估质量分布 | 把"运动不确定"与"因果断裂"分开 |
| `attributes.md §2.9` | 确认展示标签为纯函数投影（已有）；补"`off_ball_sense` 接入预估精度" | 能力涌现接入点 |

> 这三处是**契约修订**，需按 `README.md` 修订规则处理（设计文档改设计、
> status 改现状）。本方案只提出，不在实施中擅自改契约。

---

## 7. 不做（明确边界）

1. **不保证接球成功**。任何"让 `PASS_CORRIDOR_REACHABLE` 必然归零"的方案
   都违反 P-1，应拒绝。
2. **不放宽门**。见 §2.1 末尾：同时新增更严的因果判据。
3. **不改 `execute_pass`**（已实测否证：叠加领传 9→28 恶化）。
4. **不删 `builtin_*` 的数值**（实验 B 证明能力链路有效，只改承载方式：
   `builtin_*(index)` → `data/roster/*.json`）。
5. **不动 `receive_approach` 的制动模型**（已自洽；只改它的**输入**来源）。

---

## 8. 执行记录

### 8.1 步骤① 门已建立（**红**，符合预期）

新增 `crates/engine/tests/pass_information.rs`，两道门：

| 门 | 结果 |
| --- | --- |
| `arrival_gap_is_not_identically_zero` | ✅ ok（arrival gap 有 205 个不同值，非恒定） |
| `receiver_must_estimate_not_know_the_frozen_landing` | ❌ **FAILED** |

失败信息（决定性）：

```text
2/4 seeds produced byte-identical reception traces under
off_ball_sense=0.95 vs 0.05
```

**含义**：接球人对 `off_ball_sense`（预估能力）**完全不敏感** —— 2/4 种子下
两种极端能力的逐样本轨迹**逐位相同**。这直接证明接球人不是"预估"，
而是直读传球人的冻结落点。

### 8.2 门设计的一次自我修正（记录）

第一版门用「catches 计数差异」判定，实测 `304 vs 300` 就**通过**了。
但该差异是跨种子 RNG 噪声，不是效应 —— `off_ball_sense` 在接球路径上
**消费点为 0**（`grep` 确认：只出现在 rebound 对抗、slot fill、防守效用）。

改为**同种子逐样本配对比较**（`trace(seed, 0.95) == trace(seed, 0.05)`）
后，门立即变红且原因准确。

**教训**：用"计数差异"当判据容易被噪声骗过；应当用**同种子配对**
或**确定性轨迹比较**。这与 round-6 的 `defense_effect` 门同类
（那里用"逐字段比对"而非"哈希不同"）。

### 8.3 下一步（待用户确认后执行）

步骤①的实现（层 A 分层）会**改变比赛行为**（接球人不再必然到位），
因此黄金哈希将重冻。在动手前需要确认的设计点：

1. **层 A 失败后的球权语义**：球落到无人处时，应进入 `LooseBall`
   由双方争抢（真实），而不是直接判失误。当前 `resolve_pass_arrival`
   返回 `NoChange` —— 需要确认 `NoChange` 的下游处理是否等同于 loose ball。
2. **catch_radius 的取值来源**：应由 `receiver.attributes`（reach/ball_handling）
   与 `rules` 派生，而不是新常数（charter C1）。
3. **评判器口径**：`PASS_CORRIDOR_REACHABLE` 需拆为
   「运动不确定（informational）」与「因果断裂（Hard）」两类。

这三点都属于**设计决策**而非实现细节，需先定后做。

---

## 9. 执行记录（Step1 已完成，含两处重要发现）

### 9.1 Step1a/1b/1c 已落地

| 步骤 | 内容 | 状态 |
| --- | --- | --- |
| 1a | `capability::effective_catch_radius` + `receive_estimate_noise`（规则通道，3 条纯函数单测） | ✅ |
| 1b | `estimate_receiver_landing()`：接球人按自身感知估计，**持续观察修正**（`correction = (ball_actual − predicted) × off_ball_sense`） | ✅ 门转绿 |
| 1c | 到达时刻按**实际球人距离** vs `catch_radius` 判定；层 A 失败 → loose ball | ✅ |
| 2 | 评判器拆分 `RECEIVE_ESTIMATE_DIVERGENCE`（soft）/ `PASS_LANDING_FACT_MISMATCH`（hard） | ✅ |

新增事实 `GameEvent::PassLandingCorrected`：登记「传球人意图 vs 接球人实际到达」
的差异，使事实账本自洽（禁止下游各自解释同一传球）。

**8 seed full 结果：Hard 9 → 1，L1 violations = 0。**

### 9.2 发现 A：`full` scope 活锁（**已修，非本轮引入**）

Step1c 首版把「层 A 失败」接到 loose ball，但**漏了 `is_inbound_pass` 的
阶段回退**，导致发球传球失败后卡在 `DeadBall/ActionExecution`：
实测 120k tick 无任何进展（period 3、clock 351.6、t=2276.1 完全冻结）。

**验证方法**：`git stash` 回到干净树后，baseline 在 250k/300k/400k 三个
预算下**同样** `TimedOut` —— 说明该路径在 baseline 即存在（发球传球失败的
老分支已有回退，但另一条路径缺失）。修复后 full 正常终场（seed 0：91448
tick，0 violations）。

> **注意**：本节推翻了我先前"Hard 9/12/118 是 full 口径"的说法 ——
> 那些数据来自**能跑完**的版本；本轮中途曾出现 full 无法终场。
> 凡 full 口径的结论都应在修复后重测（见 Step5）。

### 9.3 发现 B：回合数膨胀的主因**不是**层 A

修复后 8 seed full：**回合/场 = 370**（真实约 200，此前 244）。归因分解
（seed 0 full，共 242 次 `PASS_DROPPED`）：

| 来源 | 数量 | 占比 |
| --- | ---: | ---: |
| 层 B（既有 release-time 概率掷骰，与位置无关） | **198** | **82%** |
| 层 A（本轮新增的位置分离） | 44 | 18% |

层 B 的失败率来自 `resolve.rs` 的 `pass_success: 0.94` 基线叠加
`lane_risk_weight: 0.35` —— 这正是 `docs/dev/cycles/20260911_first-principles/status_history.md §37.10` **已登记**的遗留问题
（"每次传球失败率 22.1%，真实 8–10%"）。

**结论**：回合数膨胀是层 B 的既有问题被放大的表现，**不是层 A 的回归**。
两步必须分开处理，否则会误改层 A 的参数去补偿层 B 的偏差。

**层 A 的实际质量**（seed 0）：偏差中位 0.68 ft、p90 1.94 ft、最大 2.94 ft
—— 预估精度是合理的；层 A 失败中位球人距离 6.5 ft，即"确实跑错了"。

### 9.4 下一步（Step3 前置调整）

原计划 Step3 是"落点用真实飞行时长"。但发现 B 表明**应先处理层 B 的失败率**，
否则任何"传球更准"的改进都会被 22% 的掷骰失败淹没。调整顺序：

```text
Step3a（新）: 层B 参数校准 —— lane_risk_weight / pass_success 基线
             目标：传球失败率 22% → 8-10%（真实）
             方法：JSON override A/B，附 8 seed 证据（protocol.md §1）
Step3b（原3）: 落点用真实飞行时长（不动点求解）
Step4/5/6   : 身份去索引化 + 守卫 + 验收
```

**纪律**：Step3a 是**参数校准**（走 `GameRules` 通道），必须用 A/B + 证据包，
不得"改默认值—重编译—单场目测"（`gap.md §15.7`）。

---

## 10. Round-10 续：层序修正 + 稳定估计（两个结构缺陷）

### 10.1 缺陷 C：层 A 与层 B **串联**（层 B 先否决）

旧实现把层 B（`will_receive`，release 时的**位置无关**概率掷骰）放在**外层**，
层 A 放内层：掷到 false 就直接判掉球，**层 A 连执行机会都没有**。

实测证据（233 次 `PASS_DROPPED`）：球**全部精确到达冻结落点**（`d=0.00`），
而接球人距球中位 6.37 ft。即"接球人站在球旁边也接不到"——因为层 B 先否决了。

**修正**：层 A 提到外层（确定性，位置决定**能否到达**），层 B 只在其通过后
生效（概率，决定**接得稳不稳**）。修正后 94% 的 drop 归因为「层 A 不可达」，
因果顺序与真实篮球一致。

### 10.2 缺陷 D：估计点每 tick 重算 → 目标抖动

`estimate_receiver_landing` 初版每 tick 从当前 `ball_pos` 整体重算。飞行末期
球逼近接球人 → `dist` 变小 → 估计点**向接球人塌缩** → 方向翻转：

```text
t=389 target=(19.8,36.5)  d_to_ball=6.8
t=394 target=(11.6,42.8)  d_to_ball=9.6
t=399 target=( 4.9,46.1)  d_to_ball=11.8   ← 方向翻转
t=403 target=( 2.7,47.7)  d_to_ball=12.8   → 判为接不到
```

接球人来回跑，距球**单调恶化**。

**修正**：估计点作为**跨 tick 状态**保存（`MatchEngine.receiver_estimate`），
按观察力加权向新信息靠拢，而不是整体重算：

```text
base      = 上一 tick 的估计（首次为按球速外推的初判）
observe   = 球实际位置 + 球速 × 典型飞行时长
blended   = base + (observe − base) × off_ball_sense
```

真正的球员不会每帧重估——他形成预判后只做小幅修正。

### 10.3 效果（seed 0 full）

| 指标 | 修正前 | 修正后 |
| --- | ---: | ---: |
| 传球失败率 | 71.6% | **36.6%** |
| 回合数 | 361 | **287** |
| 估计偏差 p50 / max | — | 1.06 ft / 3.05 ft |

**8 seed full：Hard 1，L1 violations = 0。**

`pass_information.rs` 两道门全绿（其中第二道门的判据已修正：原用「球人到达差」
判据，但接球成功时球会收到接球人身上、该差恒为 0，故改用
`PASS_LANDING_CORRECTED` 的偏差量）。

### 10.4 顺带修复：`--rules` 部分覆盖失效（工具链缺陷）

`BaseRates` / `ShotTypeRates` **缺 `#[serde(default)]`**，导致
`quality.md §3.1` 声明的"`--rules` 是校准的唯一入口"对这两个结构不成立：
部分覆盖被拒绝（`missing field handoff_success`），且失败路径不显眼——
我的三次 A/B 实测全部静默无效（参数不同、结果完全相同）。

**修正**：为两者补 `#[serde(default)]` + `Default` impl（把原本内联在
`ResolveConfig::default()` 的字面量移入 `Default`，消除重复定义）。
修正后部分覆盖生效（实测 `pass_success=0.99` 使失败率 70.9%→67.3%）。

### 10.5 剩余（下一步）

失败率 36.6% 仍高于真实 8–10%。当前构成：

| 来源 | 数量 | 说明 |
| --- | ---: | --- |
| 层 B 否决（球在半径内） | 36 | `pass_success` 基线 + `lane_risk_weight` 偏高 |
| 层 A 不可达（球在半径外） | 64 | 估计偏差仍偏大或接球人跑位未达 |
| 拦截 `PASS_TIPPED` | 37 (9.9%) | 真实 2–3% |

三项需分别校准（走规则通道 + A/B 证据）。**不再继续在本轮迭代**——
已连续做了 6 次参数/机制实验，按纪律应固结果、记录、再决策。

---

## 11. Round-10 续二：Step3b 落点用真实飞行时长（已落地）

### 11.1 缺陷

`DecisionRules.pass_lead_time_seconds = 0.65`（**固定值**）用于估计接球人
T 秒后的位置，但真实飞行时长 `T = pass_duration(|L − p|)` 随距离变化：

```text
距离    真实飞行    按 0.65s 领传    误差
  8 ft   0.45s      0.65s           +0.20s（领过头）
 39 ft   1.22s      0.65s           −0.57s（领不足）
 50 ft   1.40s      0.65s           −0.75s（领不足）
```

短传与长传**双向都错** —— 用一个常数近似一个函数的必然结果。

### 11.2 修复：不动点求解

新增 `BallisticsEngine::solve_pass_landing(passer_pos, receiver, rules) -> (落点, 时长)`：

```text
T = pass_duration(|L − passer|)
L = receiver_pos + v̂ · brake_reach(v0, T) × gain     (lead 封顶 pass_lead_max_ft)

brake_reach(v0, T):                    # 与 engine::receive_approach 同源
  t_brake = v0 / a_max
  T ≤ t_brake:  v0·T − ½·a_max·T²
  否则:          v0² / (2·a_max)       # 已停住
```

`brake_reach` **必须与 `receive_approach` 同一个模型**，否则两处对“能否到达”
的判断不一致 —— 这正是本缺口长期存在的成因之一。

**返回 `(落点, 时长)` 成对冻结**：`gap.md §9.5` 要求 release 时同时冻结
`frozen_to_pos` 与 `flight_duration`，两者必须同源。

### 11.3 验收（4 条纯函数单测，<1s，不跑模拟）

| 门 | 断言 | 结果 |
| --- | --- | --- |
| `solve_landing_is_a_fixed_point` | `t == pass_duration( | L−p | )` 残差 < 1e-3 | ✅ |
| `lead_never_exceeds_braking_reach` | 领传量 ≤ T 秒内制动可达距离，且 > 0 | ✅ |
| `stationary_receiver_gets_no_lead` | 静止接球人落点 = 自身位置 | ✅ |
| `lead_time_tracks_distance_not_a_constant` | 飞行时长随距离单调增长且跨度 > 0.3s | ✅ |

### 11.4 效果（seed 0 full）

| 指标 | Step3b 前 | Step3b 后 |
| --- | ---: | ---: |
| 传球失败率 | 36.6% | **29.7%** |
| 回合数 | 287 | **261** |

**8 seed full：Hard 0，L1 violations 0，回合/场 256**（修复前 366）。

### 11.5 Step3a（层B 失败率校准）状态

部分达成：Step3b 的落点精度提升已把失败率从 36.6% 降到 29.7%。
剩余构成（seed 0）：层B 否决 36、层A 不可达 64、拦截 28——三分量仍需
分别校准到真实 8–10%。**按纪律停在此处**，不再连续迭代参数。

### 11.6 累计轨迹（本轮）

| 指标 | 轮初 | 现在 |
| --- | ---: | ---: |
| Hard（8 seed full） | 9 | **0** |
| L1 violations | 0 | **0** |
| 传球失败率 | 71.6% | **29.7%** |
| 回合/场 | 366 | **256**（真实 ~200） |
| 根因修复数 | — | 6 个结构缺陷（层序、稳定估计、活锁、serde、固定领传、落点可达性） |

---

## 12. Round-11：Step4 身份去索引化（P-2，已落地）

### 12.1 缺陷（「顺序即身份」的四处耦合）

| # | 位置 | 表现 |
| --- | --- | --- |
| 1 | `data.rs` `builtin_attributes(index)` / `builtin_tendencies(index)` | 球员能力/倾向按**数组下标**分派 |
| 2 | `data.rs` `id = format!("{}_{}", prefix, index+1)` | **id 编码下标**，身份与序号绑定 |
| 3 | `data.rs` `builtin_roles(index)` + `PlayerData.roles` | 角色按下标分派，违反 `attributes.md §2.7/§2.9/T1`「roles 必须移除」 |
| 4 | `setup.rs` `default_lineup` 取**数组前 5 个**为首发；`validate_team` 用 `ids.len() <= 5` 判定首发 | 轮转数组即改变首发阵容 |

第 4 处的 `ids` 实为**查重用的 HashSet**，其 `len()` 是"已插入数量"而非索引 ——
语义上根本不是"前 5 个"（latent bug，此前未被触发）。

### 12.2 修复

1. **数据档案化**：新增 `data/roster/{home,away}.json`；球员作为数据资产声明，
   **数组顺序不携带语义**。
2. **删除 `roles`**：`PlayerData.roles` 与 `PlayerRole` 枚举整体移除；
   展示槽位改为 `project_display_role(attributes, tendencies)` **纯函数投影**
   （契约要求的"降级为上层派生视图"）。
3. **首发数据化**：档案新增 `starter: bool`；`default_lineup` 与
   `validate_team` 均读该标记，不再依赖数组位置。
4. **删除 227 行** index 分派生成器（`builtin_players`/`builtin_attributes`/
   `builtin_roles`/`builtin_tendencies`/`player_height_cm`/`player_weight_kg`）。
5. **id 去序号语义**：`H_1` → `H_01`（补零仅为可读，不再暗示"第 1 号位"）。

### 12.3 决定性验收（新增 `roster_order_neutrality.rs`）

| 门 | 断言 | 结果 |
| --- | --- | --- |
| `roster_array_order_does_not_change_behaviour` | **打乱名册数组顺序 → 逐 tick 行为哈希完全不变**（3 种轮转 × 3 个种子） | ✅ |
| `roster_assets_carry_no_roles_field` | `data/roster/*.json` 不含 `roles` 字段 | ✅ |

修复前该门必然红（实测：轮转数组触发
`invalid match setup: starter H_06 starts outside the court geometry`）。

### 12.4 行为影响

| 指标 | Step4 前 | Step4 后 |
| --- | ---: | ---: |
| 8 seed full Hard | 1 | **1** |
| L1 violations | 0 | **0** |
| 回合/场 | 238.5 | **236.9** |

**行为基本保持**（该步骤是结构重构，不是行为修复）。黄金哈希重冻 v54
（`0xa155d1c3b21552de`）—— 漂移的**主要来源是 id 改名**（`H_1`→`H_01`），
哈希输入包含 id，故必然变化；已在冻结记录中说明。

---

## 13. Round-11 续：Step4b/4c（身份派生 + 防复发守卫）

### 13.1 Step4b · 处理球人由能力派生（原为 `starters[0]`）

**缺陷**：初始持球人与每回合发球员/处理球人都取 `starters[0]`（数组首位）——
轮转名册数组就能改变处理球人。这是 P-2 在**行为层**的最后一处耦合。

**修复**：`new_possession_pg` 与 `with_setup` 的初始持球人改为**能力派生**：

```text
score = ball_handling × w1 + passing × w2 + decision_iq × w3
        （权重走 TacticalRules.slot_handler_*，与 fill_slots 同源）
排序：score 降序，同分按 id 升序（确定性，charter C4）
约束：必须 on_court（否则 inbounder_arrived 永不成立 → DeadBall 活锁）
```

新增规则参数 `slot_handler_passing_weight`（0.6）。

**黄金哈希未变** —— 说明派生出的处理球人与原 `starters[0]` 是同一名球员
（名册档案中该球员的处理球能力本就是队内最高）。这是**行为中性**的确认，
而非"改了却没影响"的巧合。

### 13.2 Step4c · `no_index_identity` 静态守卫（防复发）

新增 `scripts/check_no_index_identity.py`（含 7 条自测对照片）：

| 检查 | 内容 |
| --- | --- |
| A | `data/roster/*.json` 不得携带 `roles` 字段 |
| B | 不得按数组下标分派球员属性/倾向/角色（`builtin_*(index)` / `players[index]`） |
| C | 首发不得由数组位置决定（`starters[0]`、`ids.len() <= 5`） |
| D | 生产代码不得硬编码球员 id |

**门设计的一次自我修正**：首版 D 检查把**测试夹具**也判为违规，报出 200+ 条
（测试用 id 当不透明标签是合法的）。已收窄为「只判生产代码」，并把
`#[cfg(test)]` 之后的模块整体排除。收窄后剩 **9 条真实违规**，全部修复。

**这条守卫的价值已当场体现**：它在 `match_engine.rs` 抓出 3 处
`starters[0]` 的真实耦合（其中 2 处此前未被注意到）。

### 13.3 现状（8 seed full）

| 指标 | 值 |
| --- | ---: |
| Hard | **1** |
| L1 violations | **0** |
| 回合/场 | 236.9 |
| 失误率 | **0.135**（真实 ~0.13）✅ |

全部守卫：常数棘轮 ✅ / World 隐私 ✅ / 文档引用 ✅ / 阈值完整性 ✅ /
**身份去索引化 ✅（新增）**。

---

## 14. Round-12：失败率目标口径修正（度量，无代码改动）

### 14.1 问题：`8-10%` 是一个未经验证的断言

`§9.4` 把「传球失败率 → 8–10%」列为 Step3a 目标，来源是 `docs/dev/cycles/20260911_first-principles/status_history.md §37.10`
的一句"真实 8–10%"，**无引用、无口径定义**。本轮核实后废弃。

### 14.2 正确口径：恒等式 `e = n × f`

> **§12.2 勘误（Round-13）**：本节初版误读了 Basketball-Reference 的**列头顺序**，
> 把 `13.5` 当成 TOV（实际是 **3P 三分命中**）、把 `41.7` 当成 AST（实际是 **FG**）。
> 下表已按核实后的列头更正。教训：引用聚合表必须核对列头，不能凭位置猜。

**真实基准（B-Ref 2024-25 联赛均值，列头已逐一核对）**：

| 列 | FG | FGA | 3P | 3PA | AST | **TOV** | PTS | **Pace** |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 值 | 41.7 | 89.2 | 13.5 | 37.6 | 26.5 | **14.3** | 113.8 | **98.8** |

| 符号 | 含义 | 真实 | 本模拟（3 场 / 718 回合） |
| --- | --- | ---: | ---: |
| `e` | 失误 / 回合 | 14.3 / 98.8 = **0.1447** | **0.340** |
| `n` | 传球 / 回合 | ≈ **3.0** | **1.54** |
| `f` | 每次传球失误率 | `e/n` = **4.8%** | `e/n` = **22.1%** |

`n_real ≈ 3.0` 的依据：2016-17 实测 301.5 passes / 100.5 poss = **3.00**
（hoopcoach 引 SI）；2024-25 佐证 Suns 第 4 名 307.6 passes/场、6 队 >300/场，
联赛均值约 290–295 / 98.8 pace ≈ **2.95**。取 3.0。

恒等式已验证闭合：`n × f = 1.54 × 0.221 = 0.340 = e` ✓

### 14.3 两个独立缺口（**不可只关一个**）

- **缺口 1（一阶）**：`n` = 1.54 vs 3.0 = **0.51×**
  分布：46.1% 回合只传 1 次、10.0% 零传球、仅 4.2% 传 ≥4 次。
- **缺口 2（二阶）**：`f` = 22.1% vs 4.8% = **4.6×**
  构成（round-15 勘误：此前把 PASS_DROPPED 误标为层 A、PASS_TIPPED 误标为层 B。
  正确映射：PASS_TIPPED/STEAL=拦截；PASS_DROPPED 内部混含层 A 与层 B，
  真实拆分为层 B 63% + 层 A 36%，见 §15.2）：
  PASS_DROPPED 17.0% + 拦截(PASS_TIPPED+STEAL) 11.3% = 28.3% 失败。

直接量 `e` = 0.340 vs 0.1447 = **2.35×**。

**反例证明两项都必须做**：

```text
n→3.0 而 f 不变  ⇒  3.0 × 0.221 = 0.662  →  158 失误/场（比现在更糟）
f→4.8% 而 n 不变 ⇒  1.54 × 0.048 = 0.074 →   18 失误/场（接近达标，但组织度仍半真）
两者同时          ⇒  3.0 × 0.048 = 0.145 →   35 失误/场 ✓
```

### 14.4 缺口 1 的根因（已定位，待修）

`CandidateAction::Advance` 只在 `in_backcourt` 时生成
（`decision/src/pipeline.rs:224`）。**过半场后持球人没有「组织/推进」类动作**，
候选集只剩 Shoot / Pass / PostUp / Jab / Dwell，于是大量回合在第一次传球后
直接进入出手或 Dwell —— 传球链无法延长。

且 `Advance` 的效用是 `dwell_base × (1 + urgency × boost)`（`pipeline.rs:364`），
即以"原地不动"为基准的加权版，本身不承载"推进后创造传球机会"的价值。

> 注：`§37.3` 曾记录"候选集里没有推进动作"。该记录已陈旧——`Advance` 现已存在，
> 但**作用域被限制在后场**，这才是当前的真实约束。

### 14.5 测量工具（本轮建立，可复用）

旧口径（统计 `PASS*` 事件）**错误**：`PASS` / `PASS_LANDING_CORRECTED` 等会重复计入。

正确口径 = 逐 tick 事件流的 `POSSESSION_SUMMARY.terminal_event`：

```bash
./target/release/nba-sim <seed> /tmp/probe/gN.ndjson full
# batch 模式会把瞬时流写入临时文件并 RAII 删除，必须用单场位置参数形态
```

权威字段：`passes_count`、`duration_seconds`、`terminal_event`、`turnover_player_id`。

### 14.6 下一步顺序（依反例）

```text
Round-13: 缺口 1 —— 扩展 Advance 的作用域到全场（组织型推进），
          使 n: 1.54 → 2.5~3.0。目标不是一步到 3.5。
Round-14: 缺口 2 —— n 提升后重新测量 f，再校准三分量。
          顺序不可颠倒：n 变化会改变 f 的加权（长传球链风险更高）。
```

---

## 15. Round-13：`pass_base` A/B 实验 + 失误构成失衡（结构发现）

### 15.1 实验假设

Round-12 的分支过程模型给出一个反直觉预测：

```text
设 c = 接球后继续传球的概率, f = 每次传球失败率, r = 失败后进攻方夺回的概率
E[传球数] = Σ_c (1-f)c + f·r 的几何级数
```

模型预测：**降低 `f` 几乎不改变 `n`**（f: 28.3%→4.8% 只把 n 从 1.67 推到 1.75），
而 `n` 几乎完全由 `c` 决定。若成立，则 Round-12 把「缺口 1」归因于
`Advance` 作用域是**误判**——真正的杠杆是**传球效用的延续激励**。

### 15.2 A/B 结果（3 seeds × 3 arms，`--rules` override）

| arm | 传球/回合 `n` | `c` | 失误/回合 `e` | `f` | 回合/场 |
| --- | ---: | ---: | ---: | ---: | ---: |
| `pass_base=0.55` | 1.113 | 21.5% | 0.257 | 23.1% | 259 |
| **baseline 0.82** | **1.539** | **43.3%** | **0.340** | **22.1%** | **239** |
| `pass_base=1.15` | **2.394** | **68.4%** | 0.505 | 21.1% | 234 |
| 真实 | 3.0 | 68.3% | 0.1447 | 4.8% | ~200 |

**三条结论**：

1. **假设证实**：`pass_base` 是 `c` 的有效杠杆，1.15 时 `c = 68.4%` 已对齐真实 68.3%。
   Round-12 关于 `Advance` 作用域的归因**是错的**，`Advance` 不是主要瓶颈。
2. **`f` 与 `n` 解耦证实**：三个 arm 的 `f` 都在 21–23%，几乎不动。
   即 `f` 是**独立的二阶缺口**，不会因传球变多而自动改善。
3. **Round-12 的反例被实测验证**：`n` 1.54→2.39 时 `e` 从 0.340 **恶化到 0.505**。
   只提传球数确实会让失误更糟——这坐实了「两项必须同时做」。
4. 回合/场保持 234–259（无系统性副作用）。

> 注：`pass_base=1.15` 是**诊断探针**，不是最终值。它证明通道有效，但直接采用会使
> `e` 恶化。最终值必须在 `f` 修复后联合确定。

### 15.3 结构发现：失误构成严重失衡（比 `f` 本身更重要）

`e` 偏高不只是「传球失败率」的问题，而是**失误类型分布错了**。
82games 2024-25 Pacers 的真实构成：

| 失误类型 | 真实占比 | 本模拟占比 | 本模拟计数 |
| --- | ---: | ---: | ---: |
| **Ball Handling（带球丢球）** | **53.6%** | **0.4%** | **1** |
| Bad Pass（传球失误） | 33.6% | 79.5% | 194 |
| Offensive Foul / 违例等 | 12.1% | 20.1% | 49 |

**真实 NBA 最大的失误来源是「带球丢球」（53.6%），而本模拟几乎完全没有。**

**根因（架构级）**：`crates/decision/src/defense.rs` 整个模块**零调用者**——
`DefensiveCandidateAction` 的 12 个动作（含 `OnBallPressure`「贴身施压与试探切球」）
从未接入引擎。防守只通过 `DefenseRules::for_scheme` 影响位置与拦截概率，
**没有「持球人被切球」这条事实路径**，因此 `PossessionEndCause::TurnoverLooseBall`
在 718 个回合中只出现 1 次。

这也是 `docs/dev/gap.md`／todo #17「构成类真实性偏差」登记为**阻塞**的真实原因。

### 15.4 死参数清单（9 个，通道已声明但零消费）

全仓扫描（排除声明处）：

| 参数 | 非声明处引用 | 说明 |
| --- | ---: | --- |
| `pass_distance_free_ft` | 0 | 传球效用距离衰减起点 |
| `pass_distance_decay_reference_ft` | 0 | 衰减参考距离 |
| `pass_distance_max_decay` | 0 | 衰减上限 |
| `pass_lane_defender_penalty` | 0 | 走廊内每名防守人的效用乘数惩 |
| `pass_lane_blocked_multiplier` | 0 | 走廊 blocked 时的效用乘数 |
| `pass_lead_time_seconds` | 2（仅校验） | Round-11 改用 `solve_pass_landing` 后失去调用点 |
| `def_drop_contain_base` | 0 | 与 defense.rs 零调用者同源 |
| `def_hedge_contain_base` | 0 | 同上 |
| `def_switch_base` | 0 | 同上 |

前 5 个正是「传球效用应随距离/走廊质量衰减」的通道——
**它们的存在说明设计意图早已明确，只是从未接线**，这解释了为何 Pass 效用
只有 `openness` 一个空间项、缺少距离与走廊惩罚。

### 15.5 修正后的行动顺序

```text
Round-14（一阶，先做）: 失误构成失衡
    - 接线 defense.rs：OnBallPressure → TurnoverLooseBall 事实路径
    - 目标：Ball Handling 占比 0.4% → 40%+（真实 53.6%）
    - 这会同时降低 f（传球不再是唯一失败方式，持球压力分摊风险）
Round-15（二阶）: 传球效用接线
    - 接线 5 个 pass_distance_* / pass_lane_* 死参数（本是为此设计）
    - 用 pass_base 收尾，使 c 稳定在真实 68% 附近
Round-16（收尾）: 联合确定 pass_base 与 f，使 e → 0.145
```

**顺序理由**：Round-13 已证明只调传球通道会让 `e` 恶化。必须先让失误有
**第二个来源**（丢球），使总数分布恢复真实形状，再校准绝对量。

---

## 16. Round-14：接线 `defense.rs` → 带球丢球事实路径（已实现）

### 16.1 实现（架构优先，无硬编码）

真实 NBA 失误构成中「带球丢球」占 53.6%——占比最大的一类，而此前状态机里
**没有这条边**。修复分四处，全部走既有架构通道：

| 层 | 改动 | 位置 |
| --- | --- | --- |
| 规则通道 | 新增 `ResolveConfig.ball_security: BallSecurityPolicy`（9 个参数，含 `validate` 边界检查） | `domain/src/resolve.rs` |
| 能力派生 | `capability::poke_check_success()` —— 由 `steal` / `ball_handling` / `risk_tolerance` 推导，不硬编码技能项 | `domain/src/capability.rs` |
| 状态机 | 新增 `Held → LooseBall` 边（**此前不存在，是真正的阻塞点**） | `domain/src/flow.rs` |
| 引擎 | `resolve_on_ball_poke()` + `apply_on_ball_poke()`，产 `GameEvent::BallPokedLoose` | `engine/src/match_engine.rs` |

**概率语义（关键）**：与 `resolve_pass_interception` 同形——按
`rate × dt` 做**时间积分**（`1 − (1−p)^trials`），而不是让单 tick 概率随
时长累积。「贴身越久风险越大」在积分意义上成立，但不会退化成必然事件。

### 16.2 调试过程中发现的真实阻塞点

首轮实现后 `BALL_POKED_LOOSE = 0`，而调试计数显示条件满足 24,184 次 tick、
期望 7 次成功。逐层排查后定位：

**`transition_ball_state` 是唯一写入口，经 `edge_allowed` 穷举表校验；
`Held → LooseBall` 不在表中，因此转换被静默拒绝并记入 `ILLEGAL_BALL_TRANSITION`
强制项。** 这是 round-11 引入状态机时留下的缺口——状态机正确地拦住了
一个未声明的语义，而不是让非法状态悄悄通过。

> 教训：**「条件满足但事件不出现」时，要先查唯一写入口的校验，而不是怀疑概率公式。**

### 16.3 A/B 校准（3 seeds × 5 arms，`--rules` override）

| arm | 回合 | 失误/回合 | poke 次数 | 丢球占 TO | 传球占 TO |
| --- | ---: | ---: | ---: | ---: | ---: |
| `rate=0`（接线前） | 718 | 0.3398 | 0 | **0.4%** | 79.5% |
| `rate=0.55`（默认） | 731 | 0.3653 | 18 | **4.9%** | 79.4% |
| `rate=1.5` | 761 | 0.3693 | 54 | 13.2% | 69.4% |
| `rate=3.0` | 746 | 0.3941 | 94 | 20.4% | 66.7% |
| `rate=6.0` | 843 | 0.4448 | 212 | 36.0% | 54.7% |
| **真实** | ~200 | **0.1447** | — | **53.6%** | **33.6%** |

**机制已验证有效且单调响应**（4.9% → 36.0%），默认值 `0.55` 偏保守。

### 16.4 重要结论：构成比是**比值**，不能只调一项

线性外推需要 `rate ≈ 9.1` 才能到 53.6%，但 `rate=6` 时失误/回合已升到
**0.4448（真实的 3.1×）**。这说明：

```text
丢球占比 = 丢球数 / 失误总数
```

**分母同时受传球失败驱动。** 只加大 `rate` 会同时抬高分子与总量——
与 Round-12/13 的反例（只调一项会让总量更糟）是同一类错误。

**正确做法**：`rate` 与传球失败率**联合**标定——
先降传球失败（`f`），再抬 `rate`，使总量落在 0.1447 的同时构成比正确。

### 16.5 当前状态

- ✅ 机制接线完成、端到端可观测（`BALL_POKED_LOOSE` 事件进入流）
- ✅ 规则通道可 override、`validate` 拒绝非法边界（实测报错正确）
- ⬜ 标定未完成：默认 `rate=0.55` 仅给出 4.9%，需与 `f` 联合标定
- 守卫：`cargo build --release --tests --workspace` 0 error

**未跑全量测试**（本轮为机制实现 + A/B 探针，最终验收需在标定完成后统一进行）。

### 16.6 Round-14 收尾：两处被掩盖的真实缺陷已修复

机制接线后，8 seed full 暴露了两个此前不可见的缺陷，均已按第一性原理修复：

**(1) `BALL_SPEED` Hard 违规**（seed 31337 tick 3755，96.72 ft/s > 85）

症状有强误导性：三种不同弹出速度得到**完全相同**的 96.72 —— 说明与弹出速度无关。

根因：松球起点误用 `carrier_pos`（持球人身体坐标），而持球时球的真实位置带
`ball_holder_offset_ft` 前向/侧向偏移与弹跳相位（`ballistics.rs` 的 `Held` 分支）。
单帧跳变 3.87 ft ÷ 0.04s = **96.72 ft/s**，被不变量正确拦下。

修复：取 `self.ball_pos_3d.0`（球的实际坐标）。
> 教训：**不变量报"速度超限"时，先怀疑"位置跳变"，而不是"速度设置过高"。**

**(2) `TURNOVER_ATTRIBUTION` Hard defect**（possession 205）

评判器的判据是：

```rust
TurnoverLooseBall => !window.loose_ball_secures.is_empty()
```

即要求「必须有松球被收下的事实」。但球被切掉后**可能直接出界**——
实测该回合事件序列为 `BALL_POKED_LOOSE → OUT_OF_BOUNDS → POSSESSION_SUMMARY`，
没有 `LOOSE_BALL_SECURED`，却是一次完全合法的带球丢球。

判据把「球被拨离」的**原因事实**与「松球被收下」的**结果事实**混为一谈。

修复：`PossessionWindow` 新增 `poked_loose` 字段，`BALL_POKED_LOOSE` 作为
独立的原因事实通道；判据改为 `poked_loose > 0 || !loose_ball_secures.is_empty()`。

### 16.7 Round-14 验收

| 指标 | 结果 |
| --- | --- |
| 8 seed full Hard | **0**（接线前 seed 3/7 各有 1） |
| `BALL_SPEED` 违规 | **0**（修复前 5 处） |
| 丢球占比 | 0.4% → **4.8%** |
| `golden_hash` | 4/4 通过（v55 重新冻结并文档化） |
| `roster_order_neutrality` | 2/2 通过（P-2 中立性未受影响） |
| `attribution_integrity` | 2/2 通过（含 `turnover_terminal_reason_matches_window_facts`） |
| `physics_invariants` | 2/2 通过 |
| 5 守卫 | 全绿 |
| 构建 | `cargo build --release --tests --workspace` 0 error |

**golden hash v55 = `0x025c5ced668159d3`**（漂移原因已写入 `golden_hash.rs`）。

---

## 17. Round-15：第一性原理复核——真正的根因（推翻此前多轮归因）

用户要求"重新审视，从第一性原理出发"。复核采用**逐层证伪**法：每个假设
都用实测数据判决，不依赖任何此前结论。

### 17.1 此前的归因全部被证伪或降级

| 假设 | 判决实验 | 结果 |
| --- | --- | --- |
| 「传球太少（n=1.54）是主因」 | Round-13 A/B：`pass_base` 1.15 | **证伪为因**：n→2.39 但 e 恶化到 0.505 |
| 「接球人被动作窗口锁定，无法去接球」 | 59 次掉球实测 `is_locked_kinematics` | **证伪**：locked 33% vs free 31%，无差异 |
| 「传球距离太远导致失败」 | 失败率 × 距离分桶 | **证伪为因**：10-25/25-45 ft 失败率平坦（33.0%/33.6%） |
| 「丢球类失误缺失」 | Round-14 接线 | **真但次要**：占真实失误 53.6%，可它不是 e 偏高的主因 |
| 旧记录「层A 64/层B 36/拦截 28」 | 用 PASS_DROPPED↔层A 映射 | **错误标注**：PASS_DROPPED 混含两层，真实内部拆分相反 |

### 17.2 决定性证据链（一局 304 次传球的逐层拆解）

**第一步：Layer A / Layer B 分解**（对齐 release 掷骰与最终结果）

```text
Layer B 否决率            : 39/304 = 12.8%
Layer B 通过后 Layer A 失败: 63/265 = 23.8%
总失败率                  : 98/304 = 32.2%
```

**第二步：掉球瞬间的物理状态**（59 次，直接打印判定分量）

```text
A 层失败（球太远）        : 21/59 = 36%   球距 p50 = 3.6 ft（catch_radius 2.6）
B 层失败（球在身边没接住）: 37/59 = 63%   球距 p50 = 0.9 ft  ← !!
```

**B 层失败的球距中位是 0.9 ft** —— 球就落在接球人脚边，接球人速度 0
（已到位站定）、100% 处于接球模式、0 锁定。**接球侧一切正常**，
是概率裁决说"没接住"。

### 17.3 根因（单一，架构级）

**传球效用对距离与走廊完全失明——为它设计的 5 个参数声明了但从未接线。**

完整因果链：

```text
① Pass 效用 = pass_base × (0.5 + 接球人openness)     ← 唯一空间项是"接球人空不空"
   没有距离项、没有走廊项
        ↓
② 传球人选择"最空的人" = 通常是最远的人
   实测：中位传球距离 24–33 ft，45% 超过 30 ft（真实 NBA 约 15 ft）
        ↓
③ 长传横穿整个防守阵型 → 走廊 lane_risk ≈ 0.79（反推自 12.8% 否决率）
        ↓
④ Layer B: p = 0.94 − 0.35×lane_risk + … ≈ 0.87
   → 13% 的传球被否决，即使球已到达接球人脚边（0.9 ft）
        ↓
⑤ Layer A 再加 24%（长传 rendezvous 难，球距 3.6 ft > 2.6 ft 半径）
        ↓
⑥ 总失败 32% → e = 0.35（真实 2.4 倍）
        ↓
⑦ 失误抢走回合 → FGA/回合 0.64 vs 0.90，得分 0.75×
   传球链被 32% 死亡率截断 → n = 1.54 vs 3.0
   失误构成被传球独占 → 79.5% vs 33.6%
```

**这一个根因同时解释了全部六个表面症状**（e 偏高、n 偏低、FGA 偏低、
得分偏低、传球链短、失误构成失衡）。

### 17.4 为什么之前几轮的修复都"有效但不治本"

- Round-13 调 `pass_base`：改变**传不传**的频率，不改变**选哪条传球**。
  所以 c 动了、f 一动不动（21-23%）。
- Round-14 补丢球路径：给失误**加了第二个来源**，但传球失误的绝对量
  （0.29/回合 vs 真实 0.049）才是大头。
- 接球侧（P-1/层A/层B/估计）经本轮实测**全部工作正常**——
  接球人 100% 收敛到位。问题不在接球，在**选择传给谁**。

### 17.5 修复方向（唯一杠杆，且早已声明）

接线 5 个死参数到 Pass 效用（todo #19，即 `DecisionRules` 的
`pass_distance_free_ft` / `pass_distance_decay_reference_ft` /
`pass_distance_max_decay` / `pass_lane_defender_penalty` /
`pass_lane_blocked_multiplier`）。这不是加新机制——
**是完成一个已声明、已校验、已进常数棘轮的通道**。

预期同时改善：传球距离分布 → 走廊风险 → Layer B 否决率 → Layer A 失败率
→ e、n、FGA、构成比全部同向收敛。

**必须先于任何 `pass_base` 调整**：选择修复后，f 才有正确的语义，
届时再联合标定（Round-13 的教训）。

---

## 18. Round-15 续：接球失败的完整第一性原理分解（7 个假设证伪，2 个机制确证）

用户要求"分析清楚再动手"。本轮对 59 次掉球做逐层证伪，每个假设都用实测判决。

### 18.1 证伪记录（全部有数据）

| # | 假设 | 判决实验 | 结果 |
| --- | --- | --- | --- |
| 1 | 传球人被锁（无法去接） | 59 掉球 `is_locked_kinematics` | locked 33% vs free 31%，无差异 |
| 2 | 距离太远够不着 | 失败率×距离分桶 | 10-25ft 33.0% ≈ 25-45ft 33.6%，平坦 |
| 3 | 落点本来就不可达 | 释放时 need_v=距离/飞行时长 | 中位 0.8 ft/s，全部可达 |
| 4 | 领传太激进 | 领传量×估计误差相关 | lead p50=0.05ft，r=0.089，无关 |
| 5 | 接球半径 vs 停止裕量碰撞 | 按半径分组掉球率 | 3.08ft 组 18.0% ≈ 2.60ft 组 19.5% |
| 6 | 估计噪声太大 | 名册 sense 0.5-0.88 → 理论误差 0.72ft | 实测 3.37ft，噪声解释不了 |
| 7 | 估计是陈旧的（属上一接球人） | `est_match` | 59/59 全部匹配 |
| 8 | Round-13 结论"效用距离失明是根因" | 本轮 #2 | **降级**：失败与距离无关，传球选择不是失败驱动 |

### 18.2 确证的两个机制（掉球 59 = 37 + 22）

**机制一（63%，37 条）：Layer B 否决"球在脚边的接球"**

```
估计误差 p50 = 1.20 ft（正常）   球距 p50 = 0.94 ft（球在脚边！）
接球人速度 0（已站定）           100% 接球模式、0 锁定
```

→ 接球一切正常，是 `resolve_pass_arrival` 的 lane_risk 项（p = 0.94−0.35×lane_risk）
   把"走廊拥堵"重复计入接球概率。走廊风险已由 `resolve_pass_interception`
   独立裁决（拦截/点掉），球既然干净到达，就不应再被走廊惩罚——**双重计费**。

**机制二（36%，22 条）：Layer A——估计点"后退"，接球人追不上自己的预判**

```
估计误差 p50 = 3.37 ft          接球人距自己的估计 p50 = 1.34（正确收敛）
d_ball ≈ est_err + recv_off_est（10/22 严格共线，其余部分和）
```

→ 接球人**正确地**走到自己的估计——估计本身错了 3.4 ft。

**估计为什么错 3.4 ft**（与飞行时长无关、与领传无关、远超噪声上限）：

估计模型是**以接球人为参照系**的"球在向我飞来"：

```text
initial = ball + ball_dir × |ball − 我|      ← 距离以"球到我"计
```

接球人朝球走 → `|ball − 我|` 收缩 → **估计点随接球人的移动而后退**。
这是一个自我挫败的追赶曲线：接球人朝球走，预判点就往后退，两者差恒为
"接球人已走距离的一半"。以 10 ft/s 走 0.4s ≈ 4 ft，一半 ≈ 2 ft——
加上停止裕量与离散 tick，正是实测的 3.4 ft。

而**接球判定只在飞行终点那一刻做**（Pass 分支在 tau≥1 时裁决）——
球中途从接球人身边飞过不构成接球。于是"朝来球方向移动"这一**完全正确的
篮球行为**，反而必然导致终点的 miss。

**真实篮球：接球发生在第一次触球，不在球的预定落点。**

### 18.3 顺带发现的两处独立缺陷

1. **P-1 违反（全知泄漏）**：`ball_velocity_estimate` 直接读球态的
   `to_pos`/`duration`——传球人的冻结意图。其文档注释声称"用上一 tick 与
   本 tick 的球位置差"——**注释与实现不一致**（契约-代码漂移）。
   该泄漏让估计更准（非失败原因），但违反有限信息原则，应修。
2. **噪声公式双重应用**：`receive_estimate_noise()` 已含 `(1−sense)`，
   调用处再乘一次 → 实际噪声 = `2.5×(1−sense)²`，低观察力球员的噪声
   被意外压缩（方向良性，但与设计意图不符）。

### 18.4 修复设计（按第一性原理排序）

```text
Fix-1（机制二的根）：接球裁决改为"首次触球"语义
  Pass 飞行期间逐 tick 检查 |ball − 接球人| ≤ catch_radius && will_receive
  → 立即接住（与到达时同一转移）。接球人朝球走 = 更早触球 = 接住。
  这同时让"估计点后退"无害——接球人向球移动时必然先与球相遇。
  真实性不受损：拦截仍由 intercept 回放先行裁决。

Fix-2（机制一）：Layer B 的 lane_risk 双重计费
  球干净到达（未被拦截/点掉）时，接球概率不应再受走廊惩罚。
  先修 Fix-1 后重测——若 B 否决仍显著，再动 lane_risk_weight（走 A/B）。

Fix-3（P-1 纯度）：ball_velocity_estimate 改为真实的观测差分
  （与文档注释一致），消除对 to_pos/duration 的全知读取。

Fix-4（噪声公式）：去掉双重 (1−sense)，恢复设计意图 2.5×(1−sense)。
  注意这会让低 sense 球员噪声变大——须与 Fix-1 联合验证。
```

**Fix-1 是决定性的**：它把"接球"从"点到达问题"改为"区域相遇问题"，
这才是接球的物理本质，也使 P-1 的"可能接不到"保留在**真正困难的传球**
（快、远、人被卡位）上，而不是惩罚正确行为。

### 18.5 Fix-1 + Fix-2 已实施（验收）

| 指标 | 基线 | Fix-1 | Fix-1+2 | 真实 |
| --- | ---: | ---: | ---: | ---: |
| 传球失败率（每传球） | 28.3% | 28.3% | **16.9%** | ~5% |
| 其中掉球（层 A/B） | 17.0% | — | **5.0%** | ~2% |
| 其中拦截 | 11.3% | — | 11.9% | ~2-3% |
| 失误/回合 | 0.353 | 0.273(seed1) | **0.287** | 0.1447 |

- Fix-1：接球触发条件 `tau >= 1.0` → `tau >= 1.0 || receiver_touch`（裁决逻辑完全复用）
- Fix-2：`resolve_pass_arrival` 删除 lane_risk 项 + 删除 `PassPolicy.lane_risk_weight`
  （净删一个参数；决策层的走廊惩罚保留在 constraint 通道，无双计）
- golden hash v56 = `0xcaa7befc5149f0e3`（漂移原因文档化）
- 测试：golden_hash 4/4、attribution_integrity 2/2（补 `BALL_POKED_LOOSE` 窗口事实）、
  pass_information 2/2（**P-1 两项验证保持通过**）、physics_invariants、roster_order_neutrality 全绿
- 5 守卫全绿；棘轮无需调整（参数净减少）

### 18.6 剩余缺口（按主导性排序）

1. **拦截 11.9%**（真实 ~2-3%）：`intercept_steal_slope 0.12 / tip 0.20 / ceiling 0.25/0.40`
   ——纯规则通道参数，走 A/B 联合标定（gap.md §15.7）。
2. **丢球类 8%（真实 53.6%）**：Round-14 机制已接线，`poke_attempt_rate_per_sec` 待标定。
3. **违例 24%（真实 12.1%）**：8 秒/24 秒违例仍偏多。
4. n=1.54 vs 3.0：传球数偏低——Fix-1/2 之后需重测（失败率下降应自然延长传球链）。

---

## 19. Round-16：全问题清单 → 第一性原理归因 → 依次落地

### 19.1 问题清单（r15 基线，6 场）

| # | 问题 | 现状 | 真实 | 一阶归因 |
| --- | --- | ---: | ---: | --- |
| P1 | 传球选择病态 | 中位 28.3ft, 48%>30ft | ~16ft | 效用距离失明（5 参数声明未接线） |
| P2 | 拦截率 | 10.4% | 2-3% | P1 下游 + 斜率标定 |
| P3 | 24s 违例 | 12.8/场 | ~0.5/场 | **未归因**（见 17.2） |
| P4 | n 传球/回合 | 1.65 | 3.0 | pass_base==dwell_base |
| P5 | 失误构成（丢球 7%） | 7% | 53.6% | poke 未标定 |
| P6 | 失误总量 | 0.274 | 0.1447 | P2+P3+P5 |
| P7 | FGA/得分 | 0.71 | 0.90 | P3/P6 下游 |
| P8 | 回合/场 | 230 | 200 | P6 下游 |

### 19.2 P3 的根因（本轮最深发现，三层嵌套）

**症状**：24s 违例 12.8/场（真实 ~0.5）。违例回合中位仅 2 次传球。

**逐层剥离**（球态转移探针）：

```text
层1: 违例回合时长 p50=24.0s ✓ 时钟本身正常
层2: 但 p10=11.2s → 24s 违例不可能在 11.2s 发生
层3: t=110.92 DRIVE_SCORE(进球) → 球态 Drive→Held，无 SHOT_RELEASE
     → 进球被静默丢弃，回合继续，时钟烧尽
```

**根因（两处叠加）**：

1. **事实账目违规**：驱动裁决预掷 `finish_made=true` 并已发出 DRIVE_SCORE 事件，
   但空间门（距筐>16ft）把该进球静默丢弃——实测 **80% 的 DRIVE_SCORE（场均
   52.7 个）从未变成出手**，事件流与记分簿自相矛盾。
2. **时间吞噬器**：停滞后 `transition_phase(Initiation)` 强制重新等待
   `tactical_initiation_seconds=6.5s`。突破受阻是**进攻的延续**而非新回合；
   每次停滞白燃 6.5s，2-3 个循环即 24s 到期。

### 19.3 修复（按依赖顺序落地）

| Fix | 内容 | 位置 |
| --- | --- | --- |
| 3 | 传球效用距离衰减接线（3 参数）；删 2 个与 constraint 通道重复的 lane 死参数 | pipeline.rs / rules.rs |
| 4a | 驱动停滞不再回 Initiation，直接 ActionExecution | match_engine.rs |
| 4b | DriveOutcome 按真实分支申报（停滞 → successful=false） | match_engine.rs |
| 5 | 发球阶段 Dwell 按 `inbound_elapsed/5s` 线性加压（发球保持距离优选） | pipeline.rs |
| 6 | 标定：pass_base 1.15 / intercept 斜率减半 / poke rate 2.2（A/B 证据 §17.5-17.7） | rules.rs / resolve.rs |

**Fix-5 的往返教训**：距离衰减接入后发球员宁愿 Dwell 也不发长球 → 5 秒违例
0→22 次/场。第一版用「豁免发球」修复，但丢失接球人优选且长发球失败回升。
最终方案：**保留衰减（优选），把义务压力加在替代动作上**（Dwell 随 5 秒
时钟线性贬值）——这才符合「可以选择发得好，但不能选择不发」的真实语义。

### 19.4 最终验收（3 seed full，golden v57 = 0xfdb47231f042e55d）

| 指标 | r15 起点 | **最终** | 真实 | 倍差 |
| --- | ---: | ---: | ---: | ---: |
| 失误/回合 | 0.353 | **0.247** | 0.1447 | 1.71× |
| 传球距离 p50 | 28.3ft | **~20ft** | ~16ft | 1.25× |
| 24s 违例/场 | 12.8 | **2.3** | ~0.5 | 4.6× |
| 5s/8s 违例/场 | 0/3.0 | **1.0/3.0** | 0.1/0.2 | — |
| 丢球占失误 | 0.4% | **36%** | 53.6% | — |
| 违例占失误 | 20% | **13%** | 12.1% | ✓ |
| 得分回合率 | — | **43%** | ~45% | ✓ |
| 回合/场 | 230 | 231 | ~200 | 1.16× |

测试：golden_hash 4/4、attribution 2/2、pass_information 2/2（P-1 验证保持）、
physics_invariants 2/2、roster_neutrality 2/2；4 seeds 0 不变量违规；5 守卫全绿。

### 19.5-19.7 标定证据（--rules override A/B，3 seeds/arm）

pass_base：0.82 → n=1.45/失败15.0%/e=0.180；1.15 → n=2.46/10.7%/0.231。
intercept 斜率减半：失败 10.7%→7.5%，e 0.231→0.178。
poke rate：0.55→丢球8%；1.5→24%(0.052/回合)；2.2→36%(0.090)；3.0→40%(0.108)。
真实丢球率 0.0776 → 取 2.2。

### 19.8 剩余缺口（移交下轮）

1. 传球类失误率 0.124 vs 0.0486（2.6×）——拦截仍偏高，需再标定或
   走廊约束加重（选择层避开拥堵走廊）。
2. n=2.47 vs 3.0——pass_base 与拦截标定联合后接近收敛，剩 0.82×。
3. 8 秒后场违例 3.0/场 vs 0.2——独立机制，未动。
4. P-1 纯度两处（ball_velocity_estimate 全知泄漏、(1-sense)² 双重应用）。
5. 跳球 index（#16）。

---

## 20. Round-17：全 seed Hard 归零（三项独立缺陷，两个活锁级）

用户要求重审 todo 进度并继续根因分析。复核发现 round-16 后 9 seeds 中
3 个仍有 Hard（seed 2/6/7 各一），逐一归因：

### 20.1 缺陷 A（seed 6）：地板球无人追 → 247 秒全队站桩

**表象**：回合时长 258.4s（上限 40s），`POSSESSION_DURATION_BOUNDS` Hard。

**逐层剥离**（周期性状态探针）：

```text
冻结期间: ball=Discriminant(7) ← 查枚举表 = LooseBall（不是 RimRebound！）
          phase=FlightAndRebound, sc=0.0（球在飞行中 → 连 24s 违例都不触发）
球位 (85.6, 29.0) vel=0.00；最近球员 59.1 ft
```

**根因**：追逐条件硬编码 `cur_dist <= 25.0`。罚球不中的地板球停在空档区，
59 ft 内无人满足 25 ft → 无人追 → 冻结到节末。

**第一性原理**：活球是全场**唯一完全可观测**的对象（不属于任何人的私有
信息）。地板上躺着一颗活球时，「去抢球」压倒一切战术站位。

**修复**：`LooseBall` 无条件追逐（RimRebound 的落点预判追逐保留 25 ft）。

### 20.2 缺陷 B（seed 2）：防守篮板被记成失误

**表象**：`TURNOVER_ACTOR_CONSISTENCY` Hard——turnover_player_id=null。

**事件链**：FT 不中 → 无人抢到 → 地板球 → 防守方收下 → 球权易主 →
松球路径默认 `TurnoverLooseBall` 终结 → 要求一个不存在的「失误球员」。

**第一性原理**：投/罚不中弹出的球，防守方收下是**防守篮板**，不是失误。
既虚增失误数，又制造归因矛盾。

**修复**：`RimRebound → LooseBall` 转换时登记 `DefensiveRebound` 源；
防守方收下按篮板归因（rebounder=收球人）；进攻方收下则继续回合（ORB）。

### 20.3 缺陷 C（seed 7）：合法长回合被误判

**表象**：42.4s > 40s 上限，超 2.4s。

**解剖**：2.6s 发球准备（死球）+ 24s 进攻 + 出手飞行 + ORB + 14s + 终结飞行。
5 传 2 突破 ORB 后得分的**完全合法**回合。

**根因**：容差 `duration_tolerance_seconds=2.0` 只覆盖飞行，不覆盖回合
开始的死球准备时间。合法上界 ≈ 2.6+24+2+14+1 ≈ 43.6s > 40s。

**修复**：fixture `nba.v2` 容差 2.0 → 6.0（数据契约字段，附证据调整）。

### 20.4 验收

| 项 | 结果 |
| --- | --- |
| 9 seeds Hard（42,1,2,3,6,7,100,999,31337） | **全部 0** |
| golden hash | v58 = `0x1ce391e109621bbf`（漂移已文档化） |
| n（传球/回合） | 1.45 → **2.27**（真实 3.0） |
| 传球失败率 | 15.0% → **10.4%**（真实 ~5%） |
| 失误/回合 | 0.248（真实 0.145） |
| 测试 | golden 4/4、attribution 2/2、pass_information 2/2、physics/roster 4/4、evaluator 30/30 |
| 5 守卫 | 全绿 |
| 探针残留 | 0 |

### 20.5 方法论收获

1. **Discriminant 打印必须对照枚举表**——本轮曾误判 Discriminant(7) 为
   RimRebound，浪费一次探针周期。
2. **fixture 是 include_str! 编译进二进制的**——改 JSON 必须重建 bin
   （陈旧二进制陷阱第 4 次）。
3. 三个 Hard 是三个**互相独立**的缺陷，恰好各占一个 seed——单 seed 通过
   不等于收敛，必须全 seed 矩阵复核。

### 20.6 Round-17 续：评判器与引擎的账目对齐（两项）

RHYTHM_DURATION 曾是最大 soft 项（7.7 次/场）。两类残余：

**(1) ORB 窗口漏记**：罚球不中→地板球→进攻方收下，语义上是**前场篮板**
（回合继续 + 14s 窗口），但 round-17.2 的重标路径不发 `REBOUND` 事实 →
评判器的时长带自变量缺一个窗口 → 合法的 ~37s 回合被判超带。

修复：篮板源地板球被进攻方收下时补发 `ReboundContest{is_offensive:true}`。

**(2) 死球失误的 0 秒时长**：发球 5 秒违例发生在**游戏时钟停止**期间，
回合 duration 合法为 0，但 turnover 带 min=0.5 误判。

修复：fixture turnover 带 min 0.5 → 0.0（死球失误是合法 0 时长）。

RHYTHM_DURATION：23 → 8 → **3**（3 场）。

### 20.7 Round-17 总结

| 指标 | 会话起点 | **现在** | 真实 |
| --- | ---: | ---: | ---: |
| Hard（11 seeds） | 多 seed 多条 | **全部 0** | 0 |
| 失误/回合 | 0.353 | **0.248** | 0.1447 |
| 传球失败率 | 28.3% | **10.4%** | ~5% |
| n 传球/回合 | 1.54 | **2.27** | 3.0 |
| 失误构成（传/丢/违例） | 79/0.4/20 | **52/32/16** | 33.6/53.6/12.1 |
| 得分回合率 | — | **40.5%** | ~45% |

测试：golden 4/4（v58 行为未再漂移）、attribution/pass_information/
physics/roster/evaluator 全绿；5 守卫全绿；探针残留 0。
