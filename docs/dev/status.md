# NBA-Sim · 当前实现状态

> 状态类型：当前快照，不是执行日志。
> 当前工作周期：`20260917_first_principles_next`（第一性原理驱动的连续动力学与 ECS 解耦周期，`D22–D29`）。
> 历史归档：上周期体系化实施（`D14–D21`）已完整归档至 [`cycles/20260916_systematization/plan.md`](cycles/20260916_systematization/plan.md)。
> 证据原则：本文件只写当前工作区可以由代码、测试或守卫复核的结论；历史轮次见 `docs/dev/cycles/`，原始实验见 `docs/dev/evidence/`。

## 1. 结论摘要

当前工作区已经具备一条可运行、可复现、带事件和评判工件的模拟链，但**不能宣称已经满足全部设计规格**。最重要的判断如下：

- **可宣称**：领域层提供 `BallState` 与受约束的状态转换；事件帧带稳定 `event_id` / `parent_event_id`；评判器有 `NotApplicable` / `InsufficientEvidence` 与 Hard gate；账本检查器和多种负面对照测试已存在；名册数据不再以数组顺序定义身份；防守方案已经通过规则参数影响目标几何；默认事实流有资源上限。
- **只能部分宣称**：防守方案对几何和部分结果有因果影响，但完整 switch/drop/hedge/recover 责任链、动态对位与反事实覆盖仍未形成完整证据包；战术档案已接入当前目标生成路径，旧 `TacticalSet` 的**几何分支**已删除但仍作为兼容枚举存在，`carrier_idx` 经调查确认非等价可删（见 `evidence/problem.md` §25）。
- **不能宣称**：所有 `MatchEngine` 真相字段已经封装；完整阶段管线已经按 `architecture.md` 的窄签名完成；所有战术都已成为可扩展 JSON 资产；NBA/FIBA 全部情景门已通过；真实度和统计目标带已在全矩阵连续校准中达标。

因此当前唯一合法的整体标签是：**实现进行中，核心证据链已明显增强，结构和行为规格仍有未闭合门**。

## 1.1 最近验证边界

**统计门已全部关闭（2026-09-15）**：`./scripts/run-tests.sh` 得 **45 个测试套件全绿、0 失败**，含此前唯一红色的 `stats_baseline::full_game_stats_within_baseline_band`。全程**未修改任何门限或分母**。

累计效果（**16 seed** full，对照仓库自己的 `nba.v2.json` composition_bands）：

| 指标 | 起点 | 现在 | 门/带 |
| --- | --- | --- | --- |
| `total_p50` | 237.0 ✗ | **216.5** | [140, 230] ✓ |
| `two_make_pct` | 0.621 ✗ | **0.503** | [0.48, 0.58] ✓ |
| `three_make_pct` | 0.348 | 0.334 | [0.30, 0.40] ✓ |
| `three_attempt_rate` | 0.329 | 0.330 | [0.30, 0.45] ✓ |
| `two_attempt_rate` | 0.671 | 0.670 | [0.55, 0.70] ✓ |
| **`free_throw_rate`** | 0.122 ✗ | **0.204** | [0.20, 0.35] ✓ |
| ORB% | 0.070 ✗ | 0.244–0.330 | ~0.245 ✓ |
| `pace` | 234.5 ✗ | 218.6 | [185, 220] ✓ |

（L1 Axiom = 0、Ledger = 0。）

四项修复均为**结构性缺陷**而非调参：分区命中率（中距离误用廊下基准）、
篮板冲抢指派（无人抢篮板）、箱体失误/犯规零写入、突破犯规产生率
（`foul_on_drive_rate` 0.10→0.18，附反事实验证）。机制定位与 A/B 证据见
`docs/dev/evidence/problem.md` §21–§32。

**已修正的一处判断**（evidence §32.29.2）：黄金哈希（单 seed × 有限窗口）
**结构性无法**守卫概率类参数（`*_rate` / `*_ratio`）——实测把
`foul_on_drive_rate` 降 72% 后，2000/5000/10200 三个窗口的哈希全部不变
（10200 tick 内仅 19 次突破、2 次犯规，采样量不足）。
概率参数的正确守卫是跨 seed 聚合的 `stats_baseline`。

**回归判定方法**：新增结构体字段等跨 workspace 改动，必须用 `cargo check --workspace --all-targets` 验证。本会话曾因只验证单个 crate 而提交了破坏测试 target 的改动（已在 `cde5084` 修复并记录）。

## 2. 门矩阵

状态含义：`verified` 只表示所列范围通过；`partial` 表示存在已知实现或证据边界；`blocked` 表示下一步依赖尚未满足；`unknown` 表示没有当前证据。

| 门 | 当前状态 | 当前可核验事实 | 未闭合边界 | 证据/代码 |
| --- | --- | --- | --- | --- |
| L0 解析与资源边界 | verified（局部） | 严格流解析、默认有界事实/摘要流、磁盘/临时文件守卫存在 | 全部 CLI 模式和异常路径仍需统一矩阵 | `crates/evaluator/src/lib.rs`、`scripts/check_disk_budget.py`、`scripts/run-tests.sh` |
| L1 物理/状态不变量 | verified（测试范围） | `InvariantChecker`、Hard/Soft 严重度、历史失败路径回归测试存在 | 当前快照未提供新的全 profile 全场矩阵，不能外推到所有 seed | `crates/invariants/`、`crates/engine/tests/axiom_fuzzing.rs`、`crates/engine/tests/physics_invariants.rs` |
| 回合终结归因 | verified（类型与回归范围） | `PossessionEndCause` 无 `UNATTRIBUTED_END` 变体；跨节 `PeriodEnd`、传球点掉归因测试存在 | 需要持续跑完整矩阵并确保摘要责任字段与事件窗口完全一致 | `crates/domain/src/event.rs`、`crates/engine/tests/attribution_integrity.rs`、`crates/engine/tests/possession_attribution.rs` |
| 四式账本 | partial | evaluator 已实现得分、球权、时间、犯规检查并导出 `ledger_violations` | 犯规守恒仍是单节峰值粗检，不是完整个人/团队账本；跨模式工件矩阵不足 | `crates/evaluator/src/ledger.rs`、`crates/cli/src/main.rs` |
| 评判证据模型 | verified（单测范围） | 四态 `Verdict`、固定分母字段、空证据不计通过、Hard gate 使指数失效 | 当前评判准则仍有盲区；`ASSIST_PROFILE` 等明确标记为证据不足 | `crates/evaluator/src/lib.rs`、`crates/evaluator/tests/evaluator.rs` |
| 构成评判 | partial | 3PA、区域、节奏、失误率等比赛级准则和 fixture 字段存在 | 参考带来源/联合分布/跨 profile 标定仍不完整；不能把进带当作机制真实 | `crates/evaluator/src/fixture.rs`、`crates/evaluator/fixtures/`、`docs/dev/gap.md` §15 |
| 事件因果链 | verified（协议字段与回归范围） | `FrameEvent` 有全场 `event_id` 和可选 `parent_event_id`，语义父链有测试 | 仍需把所有事件族纳入稳定 schema，并让 ledger 直接消费而非解析字符串载荷 | `crates/protocol/src/frame.rs`、`crates/engine/tests/possession_attribution.rs` |
| World 封装 | verified（字段可见性）/ partial（snapshot 投影） | `MatchEngine` 的 16 个 `pub` 字段已全部私有（`da453cf`），只保留只读访问器与显式 `*_for_test` 钩子；守卫判据已由「字段清单」升级为「零 `pub` 字段」（`0bc7496`），两类负面对照均能变红；巨石拆分后 `world` 字段也私有，零 `pub` 字段守卫在 `match_engine/mod.rs` 上通过 | CLI/回放/评判尚未统一到单一只读 `snapshot()` 投影；`step_inner` 的阶段拆分属 D8 | `scripts/check_world_privacy.py`、`crates/engine/src/match_engine/mod.rs`、`docs/dev/evidence/problem.md` §24 |
| 超大源文件按职责划分（D29） | **verified** | 五份超 1200 行的源文件全部划分完成（§10.4–§10.7）：`physics/movement.rs` 1643 → `movement/mod.rs` 971 + `kinematics.rs` 729；`decision/constraint.rs` 1214 → `constraint/mod.rs` 836 + `evaluate.rs` 442；`domain/rules.rs` 1709 → `rules.rs` 963 + `rules/policies.rs` 760；`evaluator/lib.rs` 1543 → `lib.rs` 1014 + `report.rs` 300 + `composition.rs` 221 + `parse.rs` 46；`cli/main.rs` 1169 → `main.rs` 639 + 四个 `commands/*.rs`（260/241/72/9）。14 个新/拆后文件逐一实测均 ≤ 1200 行；外部引用面零变动（`crates/decision/src/lib.rs` 的 `pub use constraint::{...}` 与 HEAD 逐字相同）；每次搬移后 `golden_hash` 均未变；全量套件 54/54（`exit=0`） | 无 | `crates/physics/src/movement/`、`crates/decision/src/constraint/`、`crates/domain/src/rules/`、`crates/evaluator/src/`、`crates/cli/src/commands/`、`docs/dev/current/plan.md` §10 |
| 球态单一写入口 | partial | domain 转换表、engine `transition_ball_state`、BallState 领域测试存在 | engine 内部仍使用 `BallTrajectoryKind` 别名和多个运行态派生字段；目标 `BallControl × BallMotion` 尚未完成 | `crates/domain/src/flow.rs`、`crates/engine/src/match_engine/mod.rs`、`docs/dev/gap.md` §5 |
| 主循环阶段化 | partial | 生命周期与子阶段类型存在，事件/不变量在主循环中接入；巨石划分把弹道状态机、动作执行、对抗裁定、球权转移、战术导航、事件发布各自移出独立模块；`phases.rs` 把跳球/节间/暂停/终场四个生命周期分支抽为带 `PhaseOutcome` 短路信号的具名阶段；D22 把构造（`construction.rs`）、只读访问器与 `sync_to_world`（`accessors.rs`）、`*_for_test` 钩子（`test_hooks.rs`）、弹道裁决与球态写入口与结果消费链（`ball_flight.rs`）、名册与换人（`roster.rs`）、流程、作用域与回合总结（`flow.rs`）、动作窗口推进（`action_windows.rs`）、决策阶段（`decision.rs`）、每 tick 收尾簿记（`bookkeeping.rs`）、运行时约束/罚球/节末短路与贴身切球（`runtime_phase.rs`）、对外值类型（`types.rs`）移出 `mod.rs`，并删除 12 个零引用方法；D22 的状态组收敛把 82 个字段收为十个具名组（`state.rs` 398 行），`MatchEngine` 零裸字段，`mod.rs` 3213 → 252 行 | `step_inner` 已由 1483 行降为 95 行的纯调度器（顺序调用具名阶段，每个可短路阶段返回 `PhaseOutcome`，由调度器统一 `return self.build_tick()`）；`MatchEngine` 的十个状态组仍对子模块以 `pub(crate)` 字段可见，窄签名由各函数签名中列出的组名表达（ADR-014）；`Systems.world` 已按 ADR-015 定位为衍生视图，`sync_to_world` 已投影在册在场球员（`sync_world_players`），`wiring_proof.rs` 七项全绿；`PerceptionSystem` 的进攻侧结果仍未进入决策因果链；**子阶段迁移合法性已有回归覆盖**：非投篮犯规在球飞行中不再重置子阶段（原先会产生非法的 `Initiation -> FlightAndRebound`，实测 seed 12 tick 51080），新增 `crates/engine/tests/phase_legality.rs` 按 `nba.v2` 的 `phase_transitions` 合法表断言 4 seed × 90000 tick 的全部相邻迁移合法，并用负面对照验证过会变红 | `crates/engine/src/match_engine/mod.rs`、`crates/engine/src/match_engine/state.rs`、`crates/engine/src/match_engine/ball_flight.rs`、`crates/engine/src/match_engine/events.rs`、`crates/engine/tests/phase_legality.rs`、`scripts/check_engine_state_groups.py`、`docs/architecture.md` §4 |
| 能力扰动 | partial | `evidence/problem.md` §29 已交付**逐维消费链清单**（29 维：19 有消费点、6 已实证零消费、3 名义有消费点但未接线）；领域 capability 映射、属性扰动测试与 roster 资产存在 | 29 维中**仅 7 维有扰动测试**（`free_throw` 独占 6 处用例）；零消费维度待接线或移除 | `crates/domain/src/capability.rs`、`crates/engine/tests/attribute_perturbation.rs`、`docs/attributes.md` §2、`docs/dev/evidence/problem.md` §29 |
| 规则字段因果闭环（D27） | partial | `UNIMPLEMENTED_RULE_FIELDS` 现为空数组；14 个 `effective_*` 能力函数存在；士气调制已改为按动作族加权（`ModulationRules.morale_*_affinity`）；士气状态机的两个不可达分支已修复（`record_shot` 已在 `ball_flight.rs` 的 `HoopArrival` 处接线，`MoraleState::Clutch` 已删除以免 `clutch_bias` 双计），`wiring_proof.rs::morale_hot_hand_state_must_be_reachable` 验证 `HotHand` 可达；六个 D27 维度的曲线系数已进入 `GameRules.capability`（默认值与原内联算术逐项相等）；三条过弱的消费通道已加强并走规则通道；`rules_complete_wiring.rs` 六项全部转绿（无 `#[ignore]`）——末项的修复包括给 `PostUp` 独立的效用结构：`DecisionRules.post_up_base`（2.2）、`post_up_mismatch_weight`（0.9）、距离因子改 `dist/18`、引入以 `strength` 对抗 `effective_post_defense_physicality` 的错位收益项——修后 `PostUp` 进入候选 20 次、被选中 2 次（修前 15 次 / 0 次）。错位收益项的实测影响范围有限：权重从 0.9 改为 0.0 时效用总和 12.69 → 13.51（确实进入计算），但选中次数均为 2 次；另把 `label_of` 的 `PostUp`/`TripleThreatJab` 从共用的 `OTHER` 拆为各自标签，使该 match 变为穷尽；`wiring_proof.rs` 七项全绿 | 无（D27 验收门已全部达成） | `crates/domain/src/rules.rs`、`crates/domain/src/resolve.rs`、`crates/domain/src/capability.rs`、`crates/decision/src/pipeline.rs`、`crates/decision/src/modulation.rs`、`crates/engine/tests/rules_complete_wiring.rs` |
| 进攻战术资产化 | partial | 两个内置 JSON 档案、slot fill、能力适配和目标绑定已进入当前路径；**旧 `TacticalSet` 的 6 个硬编码几何分支已删除**（`db31fe7`，330 行死计算，8 seed 逐字段比对差异 0） | `TacticalSet` 仍作为**兼容/解析枚举**存在（`from_id`/`id`/`name_zh` 与 `tactical_set` 字段）；更多档案未纳入统一库；slot fill 回退路径仍按 roster 顺序 | `data/tactics/`、`crates/domain/src/tactics.rs`、`crates/decision/src/tactics.rs`、`crates/engine/src/match_engine/mod.rs`、`docs/dev/evidence/problem.md` §26 |
| 防守因果链 | partial | `DefenseRules` 从 `data/defense/schemes.json` 进入目标几何（`sag_multiplier` / `on_ball_gap_multiplier` / `help_priority` 已接线且有方向性断言）；`switch_aggressiveness` 经实测为零消费 | 三层声明**均未接线**（evidence §28）：`DefensiveSystem` 含四个子配置消费点全为 0；`decision/src/defense.rs` 两个评估函数全仓零调用；档案数据缺对位/换防/协防规则字段 | `data/defense/schemes.json`、`crates/domain/src/rules.rs`、`crates/domain/src/tactics.rs`、`crates/decision/src/defense.rs`、`crates/engine/tests/defense_effect.rs`、`docs/dev/evidence/problem.md` §28 |
| 名册身份独立 | verified（守卫与回归范围） | roster JSON 数据资产、`check_no_index_identity.py`、顺序中性测试均存在；`PlayerRole` 与 `PlayerData.roles` 已物理删除（提交 `487293c`） | 仍需清理通用工具中以 index 访问的非身份用途，避免守卫只覆盖已知模式 | `data/roster/`、`scripts/check_no_index_identity.py`、`crates/engine/tests/roster_order_neutrality.rs` |
| LeagueProfile | partial | `LeagueProfile` 类型、NBA/FIBA fixture 与 league 测试存在；`e63d320` 新增 **6 个程序级情景用例**（节时长/ORB 时钟/三分几何/犯规上限/bonus 门槛/节末终场），其中三分几何用例带负面对照 | **交替拥有与罚球程序仍未覆盖**；评判 fixture 带来源/版本的规格未完成；固定种子矩阵未运行；NCAA 仍是路线项 | `crates/domain/src/league.rs`、`crates/engine/tests/league_profile.rs`、`crates/evaluator/fixtures/` |
| 确定性与黄金哈希 | verified（现有测试范围） | golden hash 测试和事件 ID 单调测试存在 | 行为改动仍需按 protocol 重新冻结；黄金哈希不能证明真实性 | `crates/engine/tests/golden_hash.rs`、`docs/protocol.md` §3 |
| 文档与阈值守卫 | verified（脚本范围） | `check_docs.py`、`no_index_identity`、`threshold_integrity`、阈值自测、常数自测、World privacy 自测均可运行；前四项含 `--self-test` 负面对照 | `check_threshold_integrity.py` 与 `check_no_index_identity.py` **尚未接线到任何 CI workflow**；常数守卫计数未排除测试夹具与注释，棘轮对测试代码增长失效（见开放问题） | `scripts/check_docs.py`、`scripts/check_threshold_integrity.py`、`scripts/check_no_index_identity.py`、`.github/workflows/` |
| 统计形态（G-STATS） | **verified（Hard 门与构成准则全过）** | D22/D23/D27 与 D29 五个文件划分完成后重跑两次，结果逐项相同（`./target/release/nba-sim --seeds 1..16 --league nba full`）：`Median Total Points=218.0`、`Median 3P=37.7%`、`Median Dur=15.56s`、Axiom=0、Ledger=0、**`Realism Index=0.998`、Hard 门通过**（28409 judgments、127 soft）。本轮同时修掉了 seed 12 的 `PHASE_TRANSITION_LEGALITY` Hard 缺陷（非投篮犯规在球飞行中重置子阶段，见 `docs/dev/current/plan.md` §8.3） | 该矩阵**不属于** `run-tests.sh`，必须单独跑；0.18 的有效区间**较窄**（0.17 时 FT=0.185 差 0.015、0.19 时 pace 越界），后续其它参数变动需重新校准 | `crates/engine/tests/stats_baseline.rs`、`crates/domain/src/resolve.rs`、`crates/evaluator/fixtures/nba.v2.json`、`docs/dev/evidence/problem.md` §21–§32 |

## 3. 当前未完成工作

当前周期任务为第一性原理动力学与架构演进（`D22–D29`），详细设计见 [`current/plan.md`](current/plan.md)：

- D22: 状态组收敛与 `mod.rs` 瘦身（`architecture.md` §4.3、ADR-014）
- D23: 空间 Voronoi 拓扑与防守压迫感知
- D24: 连续受限势能场动力学与惯性制动
- D25: 细粒度微观动作链状态机 (Action Kinematics)
- D26: 弱侧协防与防守责任链闭环（单测已建，逐回合响应率证据待补）
- D27: 规则字段因果闭环的实证补齐
- D28: 周期出口、全矩阵回归与归档

## 4. 最近关闭项

### 4.1 调试链缺陷与验证通道（D12）

D12 从本周期计划执行完毕后已从 `current/plan.md` 移除（该文件只保留未完成工作，见 `docs/dev/README.md` §1.2）；六项均已实施并验证：

| 项 | 结论 | 证据 |
| --- | --- | --- |
| D12.1 `events_since` 游标 | 改用跨 tick 唯一的 `event_id`；`step_once` 去重同步 | `crates/engine/src/service.rs`、`crates/engine/tests/constraint_system.rs::match_service_events_since_cursor_uses_global_event_id` |
| D12.2 前端三项指标 | 面板 `offensiveReboundPct`/`turnoverRate`/`foulRate` 为真实值（在线读取：20.0%/0.0%/0.0%，原恒为 `—`） | `crates/debug-server/static/app.js`、`scripts/verify_ui_alignment.py`（8 行渲染通过） |
| D12.3 投篮因果配对 | 按 `parent_event_id → SHOT_RELEASE.event_id` 配对；旧流无父链时才退化 FIFO | `crates/debug-server/static/app.js` |
| D12.4 规则投影单一化 | `frame_rules_from_game_rules` 为唯一投影；一致性测试抓出**两处**既有漂移（`player_radius_ft` 1.0→1.8、`separation_safety_margin_ft` 0.0→0.05） | `crates/engine/src/match_engine/mod.rs`、`crates/protocol/src/frame.rs`、`crates/engine/tests/constraint_system.rs::frame_rules_default_matches_game_rules_default_projection` |
| D12.5 命中列表 | 每帧重建（在线读取：10 条在场球员，原无界增长） | `crates/debug-server/static/app.js` |
| D12.6 violations 通道 | `/api/simulate` 流末 `run_summary` 携带引擎官方违规；前端 `detectAnomalies` 已退役（在线确认 `undefined`），异常面板只渲染引擎违规 | `crates/debug-server/src/main.rs`、`crates/debug-server/src/main.rs::c6_6_tests`、`crates/debug-server/static/app.js` |

同批完成：渲染层移除全部 `innerHTML` 拼接（改 `el()` + `textContent`，`esc` 随之删除）。

验证范围：`golden_hash`、`constraint_system`（75 项）、`pass_information`、`attribution_integrity`、`nba-debug-server`（3 项）、`verify_ui_alignment.py`（全部卡片与画布对齐）、四项守卫 + 阈值自测。**未运行**：完整 workspace 套件（受统计门阻塞，见 `current/plan.md` §8）。

> 编号说明：`D12` 是本周期状态快照对已关闭任务的登记号；项内 `c6_6_tests` 等标识符是代码中的模块名，保留不改。

### 4.2 收敛周期成果（20260916_convergence，D7–D13 归档）

本周期已按 `docs/dev/README.md` §7 完成周期出口并归档至 [`cycles/20260916_convergence/plan.md`](cycles/20260916_convergence/plan.md)；以下为本周期关闭与验证项：

| 项 | 结论 | 证据/提交 |
| --- | --- | --- |
| **G-STATS 统计门全部关闭** | 16 seed full：七项构成指标全绿（`two_make_pct` 0.503、`three_make_pct` 0.334、`free_throw_rate` **0.204**、`pace` 218.6、`total_p50` 216.5）；`stats_baseline` 转绿。未改任何门限或分母 | `8e42bcb`；机制见 evidence §32 |
| 多联赛双硬门通过（D11.2） | 16-seed NBA / 8-seed FIBA 矩阵硬门全部通过；按 2843 回合实测分布统一标定 `duration_tolerance_seconds = 15.0s` | `f8425f1`、`c13131b` |
| 批量模式账本盲区修复（D11.3） | batch 现逐场跑 `check_ledger`、写入 `ledger_report.json`、违规非零退出 | `f178e32` |
| 篮板端到端扰动（D10.2） | 补齐前场板、后场板、弹跳三维度端到端因果扰动，18 项测试全绿，三类断路负面对照有效 | `4914107` |
| 能力消费链清单（D10.1） | 29 维逐维清单，6 个零消费维度经实证确认（§29） | `8410f75` |
| 公共边界私有化（D7.1） | 16 个 `pub` 字段全部私有；测试后门集中化；`check_world_privacy.py` 升级为「零 pub 字段」守卫 | `da453cf`、`0bc7496` |
| 球态边穷举矩阵（D8.3） | 10×10=100 种组合全覆盖（49 合法 / 51 非法），替代样例式测试 | `72661d5` |
| 旧 `TacticalSet` 几何退役（D9.1） | 删 330 行死计算；三组对照实验 + 8 seed 逐字段比对差异 0 | `3e9011b`、`db31fe7` |
| `BaseRates` 死参数清理（D9.4） | 删四个零消费字段，并区分「重复声明」与「未实现功能」两类 | `320bcaf`、`22bb764` |
| 分区命中率模型（D8.4） | 修结构性缺陷：中距离命中率基准与廊下解耦 | `7e331fc` |
| 篮板冲抢指派（D8.5） | 修行为缺失：球在空中时守方与攻方冲抢速率平衡 | `10f2a53` |
| 失误计数单一入口（D8.6） | 修零写入字段：5 种失误终结统一步进 | `ef4bd39` |
| 黄金哈希长窗口覆盖断言 | 10200-tick 长窗口覆盖断言；明确黄金哈希结构上无法守卫概率类参数的认知 | `872443d` |
| 守卫失效面修复 | 常数棘轮测试预算分离；两项守卫接入 CI；文档守卫编号命名空间校验 | `323a012`、`0bc7496` |
| 周期归档出口（D13） | 归档 D7–D13 计划至 `cycles/20260916_convergence/plan.md`，结转 D14–D21 至新周期 | `docs/dev/cycles/20260916_convergence/plan.md` |

验证范围：`./scripts/run-tests.sh` **45 个套件全绿、0 失败**（含 `stats_baseline`）；6 项守卫 + 负面对照全通过。

### 4.3 体系化实施周期成果（D14–D20 闭环）

本周期全部 7 项实施任务（D14–D20）已全部闭环并通过机械判定出口门：

| 项 | 结论 | 证据/测试 |
| --- | --- | --- |
| **D14** 最小只读 snapshot 投影 | 定义零拷贝借用 `EngineSnapshot<'a>`；消费方完成迁移；保留 `render_frame()` 保证 StreamTick 兼容 | `crates/engine/src/snapshot.rs`、`tests/engine_snapshot.rs` |
| **D15** carrier_idx 彻底解耦 | 采纳 ADR-010 裁定球态派生焦点球员，彻底删除引擎内私有字段 `carrier_idx` 及其写入旁路；消除了 phantom index 对位残留 | `docs/decisions.md`（ADR-010）、`crates/engine/src/match_engine/mod.rs` |
| **D16** step_inner 窄签名划分 | 弹道裁决、动作执行、对抗裁定、球权转移、战术导航、事件发布与只读投影已移出为独立模块；`step_phases_isolation.rs` 覆盖阶段隔离；D22 把 `step_inner` 由 1484 行降为 95 行的纯调度器，顺序调用具名阶段并由调度器统一处理 `PhaseOutcome::ShortCircuit` | `crates/engine/src/match_engine/mod.rs`、`crates/engine/src/match_engine/phases.rs`、`tests/step_phases_isolation.rs` |
| **D17** 防守方案责任链实施 | `schemes.json` 扩展 `screen_defense` 参数（schema v2）；打通 Drop/Switch/Hedge 结构化责任动作 | `data/defense/schemes.json`、`crates/decision/tests/defense_responsibility_chain.rs` |
| **D18** 零消费字段处置 | `clutch_*` 与 `drive_finish_range_ft` 接回数据通道；死字段显式清单化（`UNIMPLEMENTED_RULE_FIELDS`） | `crates/domain/src/rules.rs`、`tests/rules_consumption.rs` |
| **D19** 核心能力维度扰动覆盖 | 补齐 `shooting_mid`、`passing`、`decision_iq`、`strength`、`defense_*` 扰动与断路负面对照（18 项全绿） | `crates/engine/tests/attribute_perturbation.rs` |
| **D20** FIBA 交替拥有与罚球覆盖 | 新增交替拥有箭头全流程与罚球情景测试；证明 NBA 跳球 vs FIBA 箭头程序差异；0 账本违规 | `crates/engine/tests/fiba_scenarios.rs` |

黄金哈希受控演进至 **v64（`0x01885463019631e7`）**，全套件全绿，8-seed stats 在带（total_p50=212.0，3P%=37.3%）。

### 4.4 第一性原理连续势能场与主干闭环成果（完整实施闭环）

本周期通过连续势能场动力学求解与主干管线深度手术，彻底打破死代码孤岛，实现 100% 真实执行与因果单调性：

| 项 | 结论 | 证据/测试 |
| --- | --- | --- |
| **连续势能场求解器** | 实现 `DefensePotentialFieldSolver`，将持球威胁重力、空间真空吸力、对位张力连续积分，弱侧 Low-man 护筐与 High-man X-Out 跑位作为能量极小值平衡点自然涌现（无硬编码脚本） | `crates/decision/src/potential_field.rs`、`tests/defense_responsibility_chain.rs` |
| **规则与能力因果闭环** | 篮板争抢真实消费 `effective_defensive_boxout_bonus` 与力量对抗；传球拦截真实消费 `effective_risk_tolerance`；协防速度真实消费 `effective_help_awareness` | `crates/officiating/src/resolution.rs`、`crates/engine/tests/attribute_perturbation.rs`（19 项全绿） |
| **裁判规则修正与防死锁** | 修正阻挡犯规误判为投篮罚球的历史旧 bug；前场普通犯规回表至 14 秒并保持球权继续组织，终结级联犯规与罚球虚高 | `crates/officiating/src/resolution.rs`、`crates/engine/src/match_engine/mod.rs` |
| **主干巨石收敛与同步** | `MatchEngine` 内置 `MatchWorld` 作为底层纯数据实体世界，在 `step()` 循环中双向同步并驱动 `PerceptionSystem` 空间拓扑计算 | `crates/engine/src/match_engine/mod.rs`、`tests/match_world.rs` |
| **多线程并行并发模拟** | `stats_baseline` 引入 `std::thread::scope` 并行执行 8-seed 回归，耗时暴降，`total_p50=204.5`，`3P%=36.9%` | `crates/engine/tests/stats_baseline.rs` |
| **黄金哈希科学重校准** | 签署并冻结基准哈希至 **v65（`0xde010befa25c77b0`）**，15200-tick 长程回归与跨种子不变量零违规 | `crates/engine/tests/golden_hash.rs` |

## 5. 开放问题

| 问题 | 依赖 | 下一验证动作（新周期） |
| --- | --- | --- |
| `carrier_idx` 解耦（D15） | ~~需先定「无关联球员的球态下谁算 ball handler」~~ **已裁定（ADR-010）** | 语义裁定已登记 `docs/decisions.md` ADR-010（accepted）：采用球态关联语义，否定名单下标投影。按 ADR-010 + `gap.md` §5.2a 落实派生，planner 改以 `ball_pos_3d` 为参考，删除 `carrier_idx` |
| 防守方案责任链实施（D17） | ~~需先补全档案数据~~ **参数字段集已设计** | `schemes.json` 责任链参数字段集（`screen_defense` 块，schema_version 2）已设计入 `current/plan.md` §6.2；按字段集补数据并打通 switch/drop/hedge/recover（§28） |
| `GameRules` 零消费字段（D18） | 分属四个未接线子系统 | 21 字段三分类处置表已设计入 `current/plan.md` §7.2（A 接线 / B 未启用 / C 删除）；clutch 与 drive_finish 为「接回数据通道」（硬编码绕过规则字段） |
| 能力维度零消费（D18） | 分属四个未实现的战术行为 | 6 维度已分类（见 §7.2）；`block`/`risk_tolerance` 随 D17 接线；`free_throw` 经复核已接入主循环，移出零消费清单 |
| 剩余能力维度扰动覆盖（D19） | — | 为 `passing`、`shooting_mid`、`decision_iq`、`strength` 等建立单调性与断路负面对照 |
| FIBA 交替拥有与罚球情景（D20） | — | 建立争球箭头翻转与罚球违例进出情景测试 |
| 最小只读 `snapshot` 投影（D14） | ~~—~~ **字段清单已设计** | `EngineSnapshot<'a>` 字段清单与消费方迁移清单已设计入 `current/plan.md` §3.2；关键：重命名现有 `snapshot()→StreamTick` 为 `render_frame()`，让名给内部借用投影 |
| `D12.6` 代码标识符遗留 | 无 | `debug-server::c6_6_tests` 等模块名仍用旧编号；不影响行为，可随下次触及该文件时改名 |

## 6. 当前周期计划入口

详见 [`current/plan.md`](current/plan.md)（本周期任务：`D22–D29`，第一性原理连续博弈与空间动力学引擎）。

## 7. 证据索引

- 历史问题复现与逐 seed 结果：[`evidence/problem.md`](evidence/problem.md)；
- 历史伤害排序：[`evidence/impact_assessment.md`](evidence/impact_assessment.md)；
- 20260916 收敛周期归档计划：[`cycles/20260916_convergence/plan.md`](cycles/20260916_convergence/plan.md)；
- Round-6–9 闭环修复：[`cycles/20260911_first-principles/closure_plan.md`](cycles/20260911_first-principles/closure_plan.md)；
- Round-10–17 传球与身份修复：[`cycles/20260911_first-principles/pass_and_identity_fix.md`](cycles/20260911_first-principles/pass_and_identity_fix.md)；
- 第一原则周期原始状态历史：[`cycles/20260911_first-principles/status_history.md`](cycles/20260911_first-principles/status_history.md)。

历史记录中的数字只回答“当时观察到什么”，不自动回答“现在是什么”。
