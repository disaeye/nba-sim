# 已归档周期计划 · 体系化落地（2026-09-16）

> 文档类型：已归档周期计划（历史只读）。当前活动任务见 `docs/dev/current/plan.md`。
> 归档状态：已完成（D14–D21 全部闭环并通过 45 套件全绿守卫）。
> 结论摘要与当前入口见：`docs/dev/status.md`。
> 编号范围：`D14–D21`。

## 0. 周期目标

上一周期（`20260916_convergence`）已完成公共边界私有化、球态转移穷举测试、篮板端到端扰动、NBA/FIBA 情景矩阵和统计门全部关闭（七项构成指标全绿）。

本周期聚焦上周期结转的结构性债务与未接线子系统，完成体系化落地：

1. **统一快照**：落地最小只读 `snapshot` 投影，消除外部直接依赖内部 getter 的分散耦合；
2. **球态纯化**：解耦 `carrier_idx`，消除双重 holder 依赖；
3. **阶段拆分**：将 2082 行的 `step_inner` 拆分为窄签名阶段函数；
4. **防守责任链**：补齐 `schemes.json` 数据资产，打通 switch/drop/hedge/recover 责任链；
5. **零消费处置**：接线或清理 `GameRules` 21 个零消费字段与 6 个零消费能力维度；
6. **全维度覆盖**：为剩余能力维度建立端到端因果扰动与负面对照测试；
7. **多联赛余量**：覆盖 FIBA 特有的交替拥有与罚球进出情景。

## 1. 执行纪律

- 一个任务只有在代码、针对性测试和守卫都通过后才能关闭；
- 运行结果必须标明 seed、scope、league profile 和输出模式；
- 行为参数改动先用规则覆盖做 A/B，再决定是否改默认值；
- 结构重构与行为校准分开提交和验收；
- 统计基准保持守卫：任何改动必须保证 `./scripts/run-tests.sh` 45 套件全绿，且 16-seed 构成指标不破带；
- 当前状态只更新 `docs/dev/status.md`，原始输出进入 `docs/dev/evidence/`。

## 2. 任务依赖

```text
D14 最小只读 snapshot 投影 ───────────► D21 周期出口与归档
  │                                           ▲
  ├─► D15 carrier_idx 解耦与球态归一           │
  │     └─► D16 step_inner 窄签名拆分         │
  │                                           │
  ├─► D17 防守方案责任链落地                   │
  │     └─► D18 GameRules/能力零消费处置      │
  │                                           │
  ├─► D19 剩余能力维度因果扰动覆盖             │
  └─► D20 FIBA 交替拥有与罚球情景覆盖 ─────────┘
```

## 3. D14 · 落地 `MatchEngine` 最小只读 `snapshot` 投影

### 3.1 背景与现状

上周期（D7.1）已将 16 个真相字段全部私有化，但外部调用者（CLI、评判器、测试、回放）目前通过散落的只读 getter 获取状态。按 `docs/architecture.md`，应当提供轻量级不可变只读投影。

**现状盘点**（getter 与消费方）：`MatchEngine` 现有 27 个只读 `&self` getter（`match_engine.rs:5939-6052`），按时钟族 / 比分区 / 球态球权族 / 流转犯规罚球族 / 球员名单物理族 / 回合校验族分散。关键事实：

- 现有 `MatchEngine::snapshot()`（`match_engine.rs:6272`）**只是 `build_tick()` 的包装**，产出 `StreamTick`（`protocol/src/frame.rs:277`）——那是面向前端序列化展示的**渲染帧**（含归一化坐标、字符串枚举、`RenderPlayer` 等），每 tick 重度克隆（球员 String clone、sort、`format!`），不是类型安全的内部只读投影；
- `carrier_id()` 是**私有**方法，外部读持球人只能 pattern match `ball_state()` 或走 `snapshot().frame.ball.holder_id`；
- 消费方分布：CLI 只读比分/时钟/回合/终局；evaluator 主要消费 `step()` 返回的 `StreamTick`；debug-server 读 `is_finished()` / `last_tick_violations()` 并经 `MatchService::snapshot()`；engine tests 是最大消费方，跨全部六个字段族。

### 3.2 设计：`EngineSnapshot` 只读投影

新建 `crates/engine/src/snapshot.rs`，定义与渲染帧（`StreamTick`）分离的**内部只读借用投影**：

```rust
/// 引擎内部状态的最小只读投影。生命周期绑定引擎，零堆分配。
/// 与 StreamTick 的关系：StreamTick 是面向传输的渲染帧（拥有数据、可序列化）；
/// EngineSnapshot 是面向内部观察的借用视图（引用 + 标量拷贝）。
pub struct EngineSnapshot<'a> {
    // 时钟族
    pub current_time: f32, pub game_clock: f32, pub shot_clock: f32, pub period: u32,
    // 比分区与统计
    pub score: ScoreView,                 // home/away + team_fouls_home/away
    pub box_score: &'a MatchBoxScore,
    // 球态与球权（含派生 handler，取代私有 carrier_id 的外部读取需求）
    pub ball: BallView<'a>,               // &BallTrajectoryKind + ball_pos_3d + handler: Option<&str> + inbound_baseline
    // 流转与死球
    pub flow: FlowView,                   // game_flow + sub_phase + free_throws_remaining + free_throw_shooter
    // 球员与名单（借用，不克隆）
    pub players: PlayersView<'a>,         // &PhysicsEngine + home/away_roster_order
    // 回合与校验
    pub possession_id: u64, pub completed_possessions: u64,
    pub violations: &'a [String], pub pending_events: &'a [GameEvent],
    pub is_finished: bool,
}
impl<'a> MatchEngine { pub fn snapshot(&'a self) -> EngineSnapshot<'a>; }
```

设计要点：

- 字段为**标量拷贝 + 只读引用**，无 `String`/`Vec` 克隆，开销远低于 `build_tick()`；
- 与现有 `snapshot() -> StreamTick` **命名冲突**：重命名现有渲染帧方法为 `render_frame()`（或 `tick_frame()`），把 `snapshot()` 名字让给内部投影——这是本任务的语义核心；
- `ball.handler` 由 `BallState::associated_player()`（ADR-010）派生，为 D15 移除 `carrier_id()` 私有回退做准备；
- `MatchService::snapshot()` 随之改为返回 `EngineSnapshot`，debug-server 渲染层再自行映射到 `StreamTick`。

### 3.3 工作项

- 新建 `crates/engine/src/snapshot.rs` 定义 `EngineSnapshot<'a>` 及各 View 子结构；
- 重命名现有 `snapshot() -> StreamTick` 为 `render_frame()`，新增 `snapshot() -> EngineSnapshot`；
- 迁移消费点（按族）：CLI（比分/时钟/回合/终局）、evaluator（终局控制）、debug-server（终局/违规/服务快照）、engine tests（六族逐一替换 getter 调用为 `snapshot()` 字段访问）；
- 保留 `physics()` / `rules()` 等少数深层借用 getter（不强行纳入投影，避免一次改动过大）；
- 验证黄金哈希与事件流不受影响（纯读取侧重构）。

### 3.4 出口门

- CLI、evaluator、tests 统一从 `engine.snapshot()` 获取只读视图，六族字段不再各自直接调 getter；
- `EngineSnapshot` 无堆分配（可断言：构造不触发球员/事件克隆）；基准 tick 耗时保持 < 50µs；
- 全测试套件通过，黄金哈希不变。

## 4. D15 · `carrier_idx` 解耦与球态归一

### 4.1 背景与现状

上周期调查（`evidence/problem.md` §25）确认直接删除 `carrier_idx` 会使 8-seed `total_p50` 产生非预期漂移（213.5→212.0）。原因在于无持球人球态（`LooseBall`、`RimRebound`、`Dead`、`ControlTransfer`）下，旧逻辑回退到名单同序球员（`roster[possession][carrier_idx]`），新逻辑为球态关联人。

**前置裁定已决**：无持球人球态下的归属语义已由 `docs/decisions.md` **ADR-010（accepted）** 裁定——采用球态关联语义（`BallState::associated_player()` / `possessing_team()`），否定名单下标投影；`carrier_idx` 删除是该裁定落地后的机械结果。详见 `docs/dev/gap.md` §5.2a。

### 4.2 工作项

- 依据 ADR-010 与 `docs/dev/gap.md` §5.2a 的裁定表，逐球态落实 handler / possession 派生；
- 战术 planner 几何参考点改造：无 handler 球态（`ControlTransfer` / `LooseBall` / `RimRebound` 的阵地与转换布置）以 `ball_pos_3d` 为参考，不再消费 `carrier_idx` 实参；
- 将 `carrier_idx` 的隐式名单索引彻底替换为明确的球态枚举派生；
- 对齐行为后切断旁路字段写入，从引擎结构体删除 `carrier_idx`；
- 验证 16-seed 统计指标与 L1 账本保持零违规（迁移期行为变化属有意语义修正，附 8-seed 矩阵 + 反事实证据，非行为中性重构）。

### 4.3 出口门

- 不存在由两个可写字段共同决定 holder/possession 的路径；
- `carrier_idx` 字段从引擎结构体中移除；
- 全套件与构成指标测试通过。

## 5. D16 · `step_inner` 窄签名阶段拆分

### 5.1 背景与现状

`crates/engine/src/match_engine.rs` 中的 `step_inner` 当前长达 2082 行，单体函数聚合了决策、运动推进、冲突与犯规判定、统计写入与事件发射，阶段写权限与执行顺序缺乏编译期约束。

### 5.2 工作项

- 按 `docs/architecture.md` 拆分四个窄签名阶段：
  1. `phase_decision(&self, ...) -> DecisionBundle`
  2. `phase_motion(&mut self, &DecisionBundle, ...)`
  3. `phase_resolution(&mut self, ...)`
  4. `phase_accounting_and_events(&mut self, ...)`
- 阶段之间通过参数显式传递不可变输入；
- 每一阶段提供独立的隔离单元测试。

### 5.3 出口门

- `step_inner` 成为纯调度器，代码行数降至 < 200 行；
- 各阶段函数有明确的窄输入与输出；
- 45 测试套件全绿，黄金哈希确定性保持或有意受控迁移。

## 6. D17 · 防守方案责任链落地

### 6.1 背景与现状

`evidence/problem.md` §28 实证：`DefensiveSystem` 的四个子配置（`SchemeAssignments` 等）零消费，`decision/src/defense.rs` 的评估函数零调用。`data/defense/schemes.json` 目前仅有 4 个几何倍率，缺乏 switch/drop/hedge/recover 的行为规则字段。

**现状盘点**（接线缺口）：

- **唯一生效路径**：`decision/src/tactics.rs:456-494` 的几何公式（领防间隔 `defensive_gap_ft * on_ball_gap_multiplier`、协防深度 `help_sag_ratio * sag_multiplier`、协防方向 `help_priority → hoop_weight/tilt`），输入仅 `DefenseRules` 的 4 个几何倍率 + help_blend 4 参；
- **候选层已具雏形**：`DefensiveCandidateAction` 枚举（`decision/src/defense.rs:26-78`）已有全部 7 变体——`StayOnAssignment / GambleInterception / RotateRimHelp / StayOnShooter / DropCoverage / HedgeAndRecover / SwitchAssignment`，但两个评估函数零调用；
- **档案数据缺字段**：`schemes.json` 每方案只有 `sag_multiplier / on_ball_gap_multiplier / help_priority / switch_aggressiveness`（后者实证零消费），无责任链行为参数；`schema_version` 已声明但解析层（`rules.rs:773-822` `DefenseRules::all()`）未消费，可直接用于本次升级。

### 6.2 设计：责任链参数字段集

在 `schemes.json` 每方案新增 `screen_defense` 行为块，并升级 `schema_version: 1 → 2`。字段按 `docs/tactics.md` §2.2.2 与 `gap.md` §10.4 五要素（触发/主体/执行/降级/响应）设计：

```json
{
  "schema_version": 2,
  "schemes": [{
    "id": "def_drop_coverage",
    "sag_multiplier": 1.38, "on_ball_gap_multiplier": 1.3, "help_priority": 0.72,
    "screen_defense": {
      "strategy": "drop",               // drop | switch | hedge | blitz | show
      "drop_depth_ft": 6.0,             // 沉退目标深度（执行几何）
      "contain_base": 0.75,             // 遏制强度基线 → 接 def_drop_contain_base
      "switch_threshold": 0.8,          // 触发换防的掩护质量/错位阈值
      "mismatch_tolerance": 0.6,        // 换防后容忍的错位度，超出则触发 recover/scram
      "hedge_aggressiveness": 0.0,      // 延误强度（hedge/blitz 用）
      "recover_speed_ratio": 0.0,       // 恢复回追速率（hedge/show 用）
      "help_rotation_trigger": 0.7      // 触发轮转协防的突破渗透阈值
    }
  }]
}
```

接线映射：

- `contain_base` 接线现有 `DecisionRules.def_drop_contain_base / def_hedge_contain_base`（§27 实证零消费），消除死字段；
- `switch_threshold / mismatch_tolerance` 驱动 `SwitchAssignment` 候选的触发与 `evaluate_*` 评估；
- `DefensiveSystem` 档案层（`tactics.rs:59-95`）的 `ScreenDefenseConfig` 等 String 字段需数值化对齐，或明确降级为展示标签、以 `schemes.json` 数值块为权威（二选一，避免双源）。

### 6.3 工作项

- 扩展 `schemes.json`（schema_version 2），为 6 方案补 `screen_defense` 行为块；
- 升级 `DefenseRules` 与解析（`rules.rs:773-822`）消费新字段，接线 `def_*_contain_base`；
- 接通 `decision/src/defense.rs` 的 `SwitchAssignment / DropCoverage / HedgeAndRecover` 候选生成与 `evaluate_*` 评估；
- 为 Drop、Switch、Hedge 分别建立控制场景用例 + 反事实场景测试（改方案必须改变责任与对位几何）；
- 明确 `DefensiveSystem` 档案层与 `schemes.json` 的单一权威关系。

### 6.4 出口门

- 至少三种防守方案（Drop、Switch、Hedge）在控制测试中表现出符合战术定义的结构化责任差异；
- `DefensiveSystem` 字段不再全零消费；
- 统计指标保持在带。

## 7. D18 · `GameRules` 与能力维度的零消费处置

### 7.1 背景与现状

`evidence/problem.md` §27 逐一实证了 21 个 `GameRules` 零消费字段；§29 实证了 6 个零消费能力维度（`agility`、`shooting_close`、`cut_frequency`、`screen_frequency`、`offensive_rebound_frequency`、`transition_sprint`）。

**代码复核关键发现**（决定分类）：

- **clutch 族**：`decision/src/modulation.rs:114` 硬编码 `clock<=120.0 && margin<=5 && period>=4`，**机制已接线但绕过规则字段**（违反 charter C1）——处置是"接回数据通道"而非"新接线"；
- **攻框终结族**：`drive_finish_range_ft` 被 `match_engine.rs:2828` 硬编码 `16.0_f32` 绕过，同样"接回数据通道"；`drive_dunk_*` / `drive_floater_*` 判定机制未实现；
- **block / risk_tolerance**：消费点在零调用的孤岛 `decision/src/defense.rs` → 实际零消费；
- **free_throw**：`free_throw_probability()` 已经主循环 `resolve_free_throw`（`match_engine.rs:4326`）正式接入 → **非零消费，移出本任务**。

### 7.2 三分类处置表

| 项 | 子系统 | 分类 | 动作 |
| --- | --- | --- | --- |
| `def_switch_base` / `def_drop_contain_base` / `def_hedge_contain_base` | 防守责任链 | **A · 本周期接线** | 由 D17 接线（`contain_base` 进 `schemes.json` 责任链参数） |
| `clutch_period` / `clutch_time_remaining` / `clutch_score_margin` | clutch 情境 | **A · 本周期接线** | `modulation.rs:114` 硬编码改读规则字段，消除 C1 违反 |
| `drive_finish_range_ft` | 攻框终结 | **A · 本周期接线** | `match_engine.rs:2828` 硬编码 `16.0` 改读规则字段 |
| `drive_dunk_*`（3）/ `drive_floater_min_dist_ft` | 攻框终结（扣篮/抛投判定） | **B · 显式未启用** | 判定机制未实现，标记 `#[doc(unused_subsystem)]` + 登记未启用清单 |
| `transition_speed_ratio` / `transition_defense_threshold_ratio` / `transition_sprint_ratio` | 转换进攻 | **B · 显式未启用** | 快攻机制未接线（`plan.md` §11 暂不纳入），标记未启用 |
| `screen_hold_separation_ft` / `screen_roll_separation_ft` | 掩护执行 | **B · 显式未启用** | 掩护后分离几何未接执行层，标记未启用 |
| `ball_bounce_amplitude_ft` / `max_player_turn_rate_rad_per_sec` / `pivot_foot_tolerance_ft` / `flight_intercept_radius_ft` / `intercept_lane_radius_ft` | 物理/几何 | **C · 评估删除** | 无对应未实现子系统承接（走步/拦截判定路径已用他参），逐个确认无设计意图后删除 |
| `agility` / `shooting_close` | 能力维度 | **B · 显式未启用** | 变向/近距分区未实现，标记未启用维度 |
| `cut_frequency` / `screen_frequency` / `offensive_rebound_frequency` / `transition_sprint` | 倾向维度 | **B · 显式未启用** | 切入/掩护/冲抢/快攻倾向无消费，标记未启用 |
| `block` / `risk_tolerance` | 防守评估 | **A · 随 D17 接线** | D17 接通 defense.rs 后自然获得消费点 |

**分类定义**：A = 本周期接线并测响应；B = 移入显式未启用清单（schema 保留 + 注解）；C = 确认废弃后删除并同步 schema/default。

### 7.3 工作项

- 按上表逐项处置：A 类接回数据通道并补响应测试（clutch / drive_finish / def_* 随 D17）；B 类在 schema 加未启用注解并登记 `docs/dev/gap.md` 未启用清单；C 类逐个实证无设计意图后删除；
- 建立"未启用清单"机制：在 `GameRules` / `PlayerAttributes` / `PlayerTendencies` 对 B 类字段加统一注解，守卫可机械校验"保留字段要么有消费要么有未启用注解"；
- `free_throw` 移出零消费清单（已实证接入主循环）；
- 全测试套件通过。

### 7.4 出口门

- 活跃 `GameRules` 与 `PlayerAttributes`/`PlayerTendencies` 中每个保留字段都有代码消费或有明确的未启用状态注解；
- 无静默死字段（守卫可机械判定）；
- 全测试套件通过。

## 8. D19 · 剩余能力维度因果扰动覆盖

### 8.1 背景与现状

上周期（D10.2）已为篮板三维度（`rebounding_offensive`、`rebounding_defensive`、`vertical`）补齐端到端因果扰动。全套 29 维度中，仍有 `passing`、`shooting_mid`、`decision_iq`、`strength` 等核心维度缺少单调性与断路测试。

### 8.2 工作项

- 为 `shooting_mid`、`passing`、`decision_iq`、`strength`、`perimeter_defense`、`interior_defense` 建立扰动测试套件；
- 每项测试包含：
  - 单调性检验（能力提升则对应产出单调提升/失误单调下降）；
  - 故意断路负面对照（断路必红、恢复必绿）；
- 统一集成至 `crates/engine/tests/attribute_perturbation.rs`。

### 8.3 出口门

- 核心能力维度扰动测试覆盖率达到 ≥ 80%；
- 每项测试均有负面对照证据；
- 45 套件全绿。

## 9. D20 · FIBA 交替拥有与罚球情景覆盖

### 9.1 背景与现状

上周期（D11.1）已建立 6 个 FIBA 程序级情景用例。FIBA 规则特有的交替拥有箭头（争球程序）以及罚球违例进出情景尚未形成独立验证用例。

### 9.2 工作项

- 在 `crates/engine/tests/fiba_scenarios.rs` 中新增交替拥有情景测试；
- 覆盖争球触发、球权箭头翻转、节初发球使用箭头的全流程；
- 覆盖罚球进出与加罚违例程序差异。

### 9.3 出口门

- 交替拥有与罚球程序情景测试通过；
- NBA 与 FIBA 在争球场景下的分歧可证明（NBA 跳球 vs FIBA 箭头）；
- 保持 0 账本违规。

## 10. D21 · 周期出口与归档

### 10.1 工作项

- 在 `docs/dev/status.md` 记录本周期收敛成果、关闭项与转交项；
- 核对 `docs/dev/README.md` §7 归档条件；
- 归档本文件至 `docs/dev/cycles/YYYYMMDD_systematization/`。

## 11. 暂不纳入本周期

- UI 视觉与路线动画；
- 经营层成长与交易系统；
- NCAA 规则闭环；
- WASM 存废判定。
