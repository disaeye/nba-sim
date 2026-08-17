# NBA Sim — 战术系统架构设计

> **状态(0.9.0)**:本文档是 0.7/0.8 时期的设计稿。战术选择已从"加权随机抽签"演进为 **fit→权重**(`systemFitness + matchupFit` 实时对位评分,`weighted` 抽签保留多样性),`selectTactic` 函数已删除,`actionForHandler` 已由 EV 决策(`decideHandlerAction`)取代。§4/§9 保留为历史记录,当前实现以 `src/tactics/team-plan.ts` 与 `src/decision/expected-value.ts` 为准。

## 1. 问题诊断

### 当前架构的致命缺陷

引擎有 **4 层决策管线**，但第 1 层（战术选择）过于稀薄：

```
buildTeamPlan → planSpatialTargets → intentsFromSpatialPlan → applyLiveDecision
     ↑                ↑                      ↑                      ↑
  只有2种战术     没有 per-tactic 目标    intent = assignment.action   sticky drive 永久覆盖 pass
```

**`buildTeamPlan` 只输出两种半场战术（PNR / ISO），两者终结路径都是"持球人突破到篮下"。** 没有任何战术产生三分出手、中距离出手、或背打出手的自然路径。

`plays.json` 的 5 个 play 是**死代码** —— `types.ts` 注释："There is no per-step play walker"。`selectPlay` 选了 playId，`buildTeamPlan` 从不读它。

**出手位置不应该在 `actionForHandler` 里用 zone 阈值调，而应该是战术体系的自然产物。**

---

## 2. 设计原则

1. **出手位置 = f(战术 kind, 角色, 阶段)**。每种战术定义自己的空间目标和终结动作，出手分布自然涌现。
2. **战术选择 = f(阵容画像, 比赛情境, 随机权重)**。不是确定性 if-else，而是基于阵容能力和情境的加权随机。
3. **战术在 possession 内是 sticky 的**。一次 possession 选定一种战术，除非 possession 结束或球权翻转。
4. **每种战术是一个自包含的 spec**：定义 5 个进攻角色的 assignment、空间目标、终结动作、防守回应。

---

## 3. 战术目录（8 种）

### 3.1 TRANSITION_PUSH（转换进攻）

| 属性 | 值 |
|---|---|
| 触发 | possessionPhase = ADVANCE |
| handler | advance → 到前场后 attack_rim 或 pull_up_3 |
| screener | rim_crash（跟进扣篮位） |
| spacer_strong | strong_wing（快攻三分） |
| spacer_weak | weak_wing |
| secondary | trail（拖车跟进） |
| 终结 | handler 篮下出手(60%) / handler 三分(25%) / 传给跟进 screener(15%) |
| 防守 | 退守 + 护筐 |

### 3.2 PNR_ROLL（挡拆顺下）

| 属性 | 值 |
|---|---|
| 触发 | EXECUTE, 阵容有 screener, handler 需要掩护 |
| handler | 借掩护 → drive 到篮下 / pull-up 中距离 / 传给 roll man |
| screener | 设掩护 → 顺下到篮下（rim_crash） |
| spacer_strong | strong_corner（底角三分埋伏） |
| spacer_weak | weak_corner |
| secondary | weak_wing（弱侧三分） |
| 终结 | handler 篮下(35%) / screener 顺下接球(25%) / handler 中距离(20%) / 底角三分 kick-out(20%) |
| 防守 | DROP（大个子沉退护筐）/ SWITCH（换防） |

### 3.3 PNR_POP（挡拆外弹）

| 属性 | 值 |
|---|---|
| 触发 | EXECUTE, 阵容有 screener, 随机权重 ~30%（当 screener 可外弹时） |
| handler | 借掩护 → drive 吸引防守 → 传给外弹 screener |
| screener | 设掩护 → **外弹到三分线**（strong_wing 或 trail 三分位） |
| spacer_strong | strong_corner |
| spacer_weak | weak_corner |
| secondary | weak_wing |
| 终结 | screener 接球三分(35%) / handler 篮下(30%) / 底角三分 kick-out(20%) / handler 中距离(15%) |
| 防守 | SWITCH（换防，大个子追外弹）/ DROP |

### 3.4 DRIVE_KICK（突破分球）

| 属性 | 值 |
|---|---|
| 触发 | EXECUTE, 阵容有 ≥2 spacer, handler 是 slasher |
| handler | 直接 drive → 吸引协防 → **分球给底角三分手** |
| screener | 弱侧掩护或 space（拉开） |
| spacer_strong | strong_corner（接 kick-out 三分） |
| spacer_weak | weak_corner（接 kick-out 三分） |
| secondary | weak_wing（接 kick-out 三分） |
| 终结 | 底角三分(40%) / handler 篮下(30%) / 侧翼三分(20%) / 中距离(10%) |
| 防守 | 协防人 stay-home（不帮） / rim protector 护筐 |

### 3.5 POST_UP（背打）

| 属性 | 值 |
|---|---|
| 触发 | EXECUTE, 阵容有 screener（大个子）, 随机权重 ~15% |
| handler | 传球给低位 → 拉开空间 → 等回传 kick-out |
| screener | **低位要位（paint_post）→ 接球 → 背打 → 出手或分球** |
| spacer_strong | strong_corner（接 kick-out 三分） |
| spacer_weak | weak_corner |
| secondary | weak_wing |
| 终结 | 背打篮下/油漆区(50%) / kick-out 三分(25%) / 中距离 fadeaway(25%) |
| 防守 | 单防 / 偶尔包夹 |

### 3.6 OFF_BALL_SCREEN（无球掩护）

| 属性 | 值 |
|---|---|
| 触发 | EXECUTE, 阵容有 ≥2 spacer, 随机权重 ~15% |
| handler | 持球等无球掩护完成 → **传给跑出的 shooter** |
| screener | **给 spacer 设无球掩护**（pin-down / flare） |
| spacer_strong | **借掩护跑出空位**（curl 到 wing 或 fade 到 corner）→ 接球三分 |
| spacer_weak | weak_corner（拉开） |
| secondary | weak_wing |
| 终结 | shooter 接球三分(50%) / 中距离 curl(30%) / 篮下 backdoor(20%) |
| 防守 | 追防 / 换防 / fight through |

### 3.7 ISO（单打）

| 属性 | 值 |
|---|---|
| 触发 | EXECUTE, handler 是 elite creator, 随机权重 ~15% |
| handler | **直接单打** → drive / step-back 3 / pull-up 中距离 |
| screener | space（弱侧拉开） |
| spacer_strong | strong_corner |
| spacer_weak | weak_corner |
| secondary | trail（弧顶三分） |
| 终结 | handler 篮下(35%) / handler 中距离(35%) / handler 三分(30%) |
| 防守 | 单防 / 不协防 |

### 3.8 HANDOFF（手递手）

| 属性 | 值 |
|---|---|
| 触发 | EXECUTE, 阵容有 ≥2 creator, 随机权重 ~10% |
| handler | 运球到 wing → **与 secondary 手递手** → 拉开 |
| secondary | **接手递手** → drive 或 pull-up |
| screener | 弱侧掩护 / space |
| spacer_strong | strong_corner |
| spacer_weak | weak_corner |
| 终结 | receiver 篮下(40%) / 中距离(30%) / 三分(30%) |
| 防守 | 追防 / 换防 |

---

## 4. 战术选择逻辑

### 选择时机

战术在 **SETUP → EXECUTE 转换时**选定一次，之后 sticky 到 possession 结束。

### 选择函数

```typescript
function selectTactic(
  sense: LiveCourtSense,
  lineup: LineupPackage,
  previous: TeamPlan | null,
  rng: Rng,
): TacticKind
```

### 选择规则

```
1. ADVANCE 或不在前场 → TRANSITION_PUSH
2. 上一拍已有战术（同 possession 内）→ 保持不变（sticky）
3. 首次进入 EXECUTE → 加权随机：
   weights = {
     PNR_ROLL:      lineup.screener.length > 0 ? 25 : 0,
     PNR_POP:       lineup.screener.length > 0 ? 15 : 0,
     DRIVE_KICK:    lineup.spacer.length >= 2 ? 15 : 0,
     POST_UP:       lineup.screener.length > 0 ? 15 : 0,
     OFF_BALL_SCREEN: lineup.spacer.length >= 2 ? 15 : 0,
     ISO:           10,
     HANDOFF:       lineup.creator.length >= 2 ? 5 : 0,
   }
   → rng.next() 加权选择
```

### 情境修正

```
- shotClock <= 8（晚钟）→ ISO 权重 ×2（关键时刻单打）
- shotClock <= 4（绝杀）→ ISO 权重 ×3
- 不修正已有 sticky 战术
```

---

## 5. 阶段模型

### 复用现有 TeamPlanStage，per-tactic 语义映射

| Stage | PNR_ROLL | PNR_POP | DRIVE_KICK | POST_UP | OFF_BALL_SCREEN | ISO | HANDOFF |
|---|---|---|---|---|---|---|---|
| SCREEN_APPROACH | 持球人带球到掩护点 | 同 | 持球人观察 | 大个要位 | spacer 跑位 | 持球人观察 | 持球人运向 wing |
| SCREEN_USE | 借掩护 | 借掩护 | 直接突破 | 低位接球 | spacer 借掩护跑出 | 开始单打 | 手递手交换 |
| ADVANTAGE | roll man 顺下 / handler 突破 | screener 外弹 | 协防到位 → kick | 背打完成 | shooter 空位 | 创造空间 | receiver 决策 |
| TERMINAL | 出手/传球 | 出手/传球 | 出手/传球 | 出手/传球 | 出手/传球 | 出手 | 出手 |

---

## 6. 空间目标 per-tactic

每个战术 kind × 每个 role 定义目标点。核心改动在 `desiredOffenseTarget`。

### handler 目标

| kind | action=advance | action=drive | action=shoot | action=pass | action=relocate |
|---|---|---|---|---|---|
| PNR_ROLL | 向 rim 方向 8ft | rim+4ft | 当前位置 | 当前位置 | attack pocket |
| PNR_POP | 向 rim 方向 8ft | rim+4ft | 当前位置 | 当前位置 | attack pocket |
| DRIVE_KICK | 向 rim 方向 8ft | rim+4ft | 当前位置 | 当前位置 | attack pocket |
| POST_UP | 向 wing 移动 | rim+4ft | 当前位置 | **post player 位置** | wing 等待 |
| OFF_BALL_SCREEN | wing 持球 | rim+4ft | 当前位置 | **shooter 空位** | wing 持球 |
| ISO | 向 rim 方向 8ft | rim+4ft | 当前位置 | 当前位置 | isolation pocket |
| HANDOFF | **向 wing 移动到 handoff 点** | rim+4ft | 当前位置 | secondary 位置 | wing |

### screener 目标

| kind | action=screen | action=cut(roll) | action=space | action=pop | action=post |
|---|---|---|---|---|---|
| PNR_ROLL | defender-handler 线上 | rim+3ft | strong_wing | — | — |
| PNR_POP | defender-handler 线上 | — | — | **rim+22ft wing（三分线外弹）** | — |
| POST_UP | — | — | strong_wing | — | **paint_post（低位要位）** |
| OFF_BALL_SCREEN | **spacer 路线上（无球掩护）** | — | — | — | — |

### spacer 目标

| kind | spacer_strong | spacer_weak | secondary |
|---|---|---|---|
| PNR_ROLL | strong_corner | weak_corner | weak_wing |
| PNR_POP | strong_corner | weak_corner | weak_wing |
| DRIVE_KICK | strong_corner | weak_corner | weak_wing |
| POST_UP | strong_corner | weak_corner | weak_wing |
| OFF_BALL_SCREEN | **curl 到 wing（借掩护跑出）** | weak_corner | weak_wing |
| ISO | strong_corner | weak_corner | trail（弧顶） |
| HANDOFF | strong_corner | weak_corner | weak_wing |

---

## 7. 终结动作 → 出手位置映射

### actionForHandler per-tactic

```
PNR_ROLL:
  ADVANTAGE + open wing → shoot(3)
  ADVANTAGE + paint crowded → pass(roll man)
  ADVANTAGE + open mid → shoot(2)
  else → drive

PNR_POP:
  ADVANTAGE + screener 外弹到三分位 → pass(screener)
  ADVANTAGE + handler open → shoot
  else → drive

DRIVE_KICK:
  ADVANTAGE + paintDefenders>=2 → pass(corner spacer)
  else → drive

POST_UP:
  SCREEN_USE → pass(screener at post)
  ADVANTAGE → space / wait for kick-back
  (screener becomes the scorer)

OFF_BALL_SCREEN:
  SCREEN_USE → pass(spacer running off screen)
  ADVANTAGE → pass(shooter open)

ISO:
  ADVANTAGE + open → shoot
  else → drive

HANDOFF:
  SCREEN_USE → pass(secondary at handoff point)
  ADVANTAGE → space / shoot if open
```

### spacer 终结（当接球后成为 handler）

当 pass 完成后，receiver 成为新 handler，actionForHandler 按其所在 zone 判断：
- 在 corner/wing 且 open → **shoot(3)**（catch-and-shoot）
- 在 paint 且 open → shoot(2)
- 被压迫 → pass / drive

---

## 8. 预期出手分布

### 目标分布（modern NBA）

| 区域 | 占比 | 来源战术 |
|---|---|---|
| 篮下/油漆区 | 30-35% | PNR_ROLL drive, POST_UP, DRIVE_KICK drive, transition layup |
| 中距离 | 12-18% | ISO pull-up, PNR_ROLL pull-up, POST_UP fadeaway |
| 三分（弧顶+侧翼） | 25-30% | PNR_POP pop, OFF_BALL_SCREEN catch-shoot, ISO step-back |
| 底角三分 | 10-15% | DRIVE_KICK kick-out, PNR_ROLL kick-out, POST_UP kick-out |

### 与当前对比

| 区域 | 当前 | 目标 |
|---|---|---|
| 篮下/油漆区 | ~62% | 30-35% |
| 中距离 | ~5% | 12-18% |
| 三分 | ~6% | 35-45% |

---

## 9. 实现改动清单

### 9.1 `src/tactics/team-plan.ts`（核心重构）

- **扩展 `TeamPlanKind`**：加入 `PNR_ROLL | PNR_POP | DRIVE_KICK | POST_UP | OFF_BALL_SCREEN | HANDOFF`
- **新增 `selectTactic` 函数**：加权随机选择，考虑阵容画像 + 情境
- **重写 `buildTeamPlan` 的 kind 选择**：用 `selectTactic` 替代当前确定性 if-else
- **重写 `actionForHandler`**：per-tactic 分支，每种战术有自己的终结逻辑
- **扩展 `buildTeamPlan` 的 assignments**：per-tactic 角色分配（screener 做 pop/post/off-ball-screen 而非永远 roll）

### 9.2 `src/spatial/team-planner.ts`（空间目标重构）

- **重写 `desiredOffenseTarget`**：per-tactic + per-role 目标点
  - screener: pop 外弹到三分线 / post 要位 / off-ball-screen 设无球掩护
  - spacer: OFF_BALL_SCREEN 时 curl 跑出 / 其他战术保持 corner spacing
  - handler: POST_UP 时移到 wing 传球位 / HANDOFF 时运到 handoff 点

### 9.3 `src/decision/step.ts`（传 RNG）

- `DecisionStepInput` 加 `rng?: Rng`
- `runDecisionStep` 把 rng 传给 `buildTeamPlan`

### 9.4 `src/sim-tick.ts`（传 RNG 到 decision）

- `applyLiveDecision` 把 `ctx.rng` 传给 `runDecisionStep`

### 9.5 `src/spectator/narrate.ts`（叙事扩展）

- POST_UP 事件 → "#X 低位要位 → #Y 喂球 → #X 背打出手"
- OFF_BALL_SCREEN → "#X 借无球掩护跑出空位 → 接球三分"
- PNR_POP → "#X 挡拆外弹到三分线 → 接球出手"
- DRIVE_KICK → "#X 突破吸引防守 → 分球 #Y 底角三分"

### 9.6 `config/plays.json`（同步更新，可选）

- 加入新 play 条目（pnr_pop, drive_kick, post_up, off_ball_screen, handoff）
- 这些仍然是 advisory，但至少 playId 与实际战术一致

### 9.7 测试 + Golden

- `alignment-flow.test.ts`：调整断言窗口
- `reality-gap.test.ts`：确保 0 error violations
- golden fixtures：重新生成
- pace bands：确保 pace/turnover/possession-length 在 band 内

---

## 10. 数据流（重构后）

```
Possession 开始
  → selectPlay(mode, rng) → playId（advisory）
  → bindRoles(package, play) → RoleBinding（谁打什么角色）

每个 tick（shouldDecide=true 时）:
  → perceiveLiveCourt(state) → LiveCourtSense（场上态势）
  → buildTeamPlan(sense, lineup, pkg, binding, phase, previous, screenExec, rng)
      → selectTactic(sense, lineup, previous, rng) → TacticKind  [首次 EXECUTE 时选定]
      → stage 判定（基于 phase + screenReady + kind）
      → actionForHandler(sense, stage, kind, phase) → handler 动作
      → assignments（5 人角色动作，per-tactic）
  → planSpatialTargets(sense, plan)
      → desiredOffenseTarget(assignment, sense, plan.kind) [per-tactic 目标点]
      → defenseTarget（防守人位置）
      → solveTeamSeparation（物理分离）
  → intentsFromSpatialPlan(spatial, plan, sense) → Intent[]
  → applyLiveDecision(intents)
      → sticky drive override（允许 pass/shoot 打断）
      → factsFromBallIntent → runFacts → Events
```
