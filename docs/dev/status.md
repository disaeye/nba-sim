# NBA-Sim · 当前实现状态

> 状态类型：当前快照，不是执行日志。
> 当前工作周期：`20260911_first-principles` 的后续收敛周期。
> 证据原则：本文件只写当前工作区可以由代码、测试或守卫复核的结论；历史轮次见 `docs/dev/cycles/`，原始实验见 `docs/dev/evidence/`。

## 1. 结论摘要

当前工作区已经具备一条可运行、可复现、带事件和评判工件的模拟链，但**不能宣称已经满足全部设计契约**。最重要的判断如下：

- **可宣称**：领域层提供 `BallState` 与受约束的状态转换；事件帧带稳定 `event_id` / `parent_event_id`；评判器有 `NotApplicable` / `InsufficientEvidence` 与 Hard gate；账本检查器和多种负面对照测试已存在；名册数据不再以数组顺序定义身份；防守方案已经通过规则参数影响目标几何；默认事实流有资源上限。
- **只能部分宣称**：防守方案对几何和部分结果有因果影响，但完整 switch/drop/hedge/recover 责任链、动态对位与反事实覆盖仍未形成完整证据包；战术档案已接入当前目标生成路径，旧 `TacticalSet` 的**几何分支**已删除但仍作为兼容枚举存在，`carrier_idx` 经调查确认非等价可删（见 `evidence/problem.md` §25）。
- **不能宣称**：所有 `MatchEngine` 真相字段已经封装；完整阶段管线已经按 `architecture.md` 的窄签名落地；所有战术都已成为可扩展 JSON 资产；NBA/FIBA 全部情景门已通过；真实度和统计目标带已在全矩阵连续校准中达标。

因此当前唯一合法的整体标签是：**实现进行中，核心证据链已明显增强，结构和行为契约仍有未闭合门**。

## 1.1 最近验证边界

**统计门已关闭（2026-09-15）**：`./scripts/run-tests.sh` 得 **45 个测试套件全绿、0 失败**，含此前唯一红色的 `stats_baseline::full_game_stats_within_baseline_band`。全程**未修改任何门限或分母**。

累计效果（8 seed full，对照仓库自己的 `nba.v2.json` composition_bands）：

| 指标 | 起点 | 现在 | 门/带 |
| --- | --- | --- | --- |
| `total_p50` | 237.0 ✗ | **213.5** | [140, 230] ✓ |
| `two_make_pct` | 0.621 ✗ | **0.536** | [0.48, 0.58] ✓ |
| `three_make_pct` | 0.348 | 0.337 | [0.30, 0.40] ✓ |
| `three_attempt_rate` | 0.329 | 0.334 | [0.30, 0.45] ✓ |
| ORB% | 0.070 ✗ | **0.244–0.330** | ~0.245 ✓ |
| `pace` | 234.5 ✗ | 218.4 | [185, 220] ✓ |
| `free_throw_rate` | 0.122 ✗ | 0.118 | [0.20, 0.35] **✗ 未闭合** |

三项修复均为**结构性缺陷**而非调参：分区命中率（中距离误用篮下基准）、篮板冲抢指派（无人抢篮板）、箱体失误/犯规零写入。机制定位与 A/B 证据见 `docs/dev/evidence/problem.md` §21–§23。

**唯一剩余越界项**：`free_throw_rate`。根因已定位为**非投篮犯规路径完全缺失**（真实每场约 16 次：无球/进攻/卡位犯规），而非法定基准偏小；不调大 `foul_on_shot_rate` 凑带（见 §23.11.1）。

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
| World 封装 | verified（字段可见性）/ partial（snapshot 投影） | `MatchEngine` 的 16 个 `pub` 字段已全部私有（`da453cf`），只保留只读访问器与显式 `*_for_test` 钩子；守卫判据已由「字段清单」升级为「零 `pub` 字段」（`0bc7496`），两类负面对照均能变红 | CLI/回放/评判尚未统一到单一只读 `snapshot()` 投影；`step_inner` 的阶段拆分属 D8 | `scripts/check_world_privacy.py`、`crates/engine/src/match_engine.rs`、`docs/dev/evidence/problem.md` §24 |
| 球态单一写入口 | partial | domain 转换表、engine `transition_ball_state`、BallState 领域测试存在 | engine 内部仍使用 `BallTrajectoryKind` 别名和多个运行态派生字段；目标 `BallControl × BallMotion` 尚未完成 | `crates/domain/src/flow.rs`、`crates/engine/src/match_engine.rs`、`docs/dev/gap.md` §5 |
| 主循环阶段化 | partial | 生命周期与子阶段类型存在，事件/不变量在主循环中接入 | `step_inner` 仍是大型编排函数，尚未兑现窄签名静态 Phase 序列 | `crates/engine/src/match_engine.rs`、`docs/architecture.md` §4 |
| 能力扰动 | partial | `evidence/problem.md` §29 已交付**逐维消费链清单**（29 维：19 有消费点、6 已实证零消费、3 名义有消费点但未接线）；领域 capability 映射、属性扰动测试与 roster 资产存在 | 29 维中**仅 7 维有扰动测试**（`free_throw` 独占 6 处用例）；零消费维度待接线或移除 | `crates/domain/src/capability.rs`、`crates/engine/tests/attribute_perturbation.rs`、`docs/attributes.md` §2、`docs/dev/evidence/problem.md` §29 |
| 进攻战术资产化 | partial | 两个内置 JSON 档案、slot fill、能力适配和目标绑定已进入当前路径；**旧 `TacticalSet` 的 6 个硬编码几何分支已删除**（`db31fe7`，330 行死计算，8 seed 逐字段比对差异 0） | `TacticalSet` 仍作为**兼容/解析枚举**存在（`from_id`/`id`/`name_zh` 与 `tactical_set` 字段）；更多档案未纳入统一库；slot fill 回退路径仍按 roster 顺序 | `data/tactics/`、`crates/domain/src/tactics.rs`、`crates/decision/src/tactics.rs`、`crates/engine/src/match_engine.rs`、`docs/dev/evidence/problem.md` §26 |
| 防守因果链 | partial | `DefenseRules` 从 `data/defense/schemes.json` 进入目标几何（`sag_multiplier` / `on_ball_gap_multiplier` / `help_priority` 已接线且有方向性断言）；`switch_aggressiveness` 经实测为零消费 | 三层声明**均未接线**（evidence §28）：`DefensiveSystem` 含四个子配置消费点全为 0；`decision/src/defense.rs` 两个评估函数全仓零调用；档案数据缺对位/换防/协防规则字段 | `data/defense/schemes.json`、`crates/domain/src/rules.rs`、`crates/domain/src/tactics.rs`、`crates/decision/src/defense.rs`、`crates/engine/tests/defense_effect.rs`、`docs/dev/evidence/problem.md` §28 |
| 名册身份独立 | verified（守卫与回归范围） | roster JSON 数据资产、`check_no_index_identity.py`、顺序中性测试均存在；`PlayerRole` 与 `PlayerData.roles` 已物理删除（提交 `487293c`） | 仍需清理通用工具中以 index 访问的非身份用途，避免守卫只覆盖已知模式 | `data/roster/`、`scripts/check_no_index_identity.py`、`crates/engine/tests/roster_order_neutrality.rs` |
| LeagueProfile | partial | `LeagueProfile` 类型、NBA/FIBA fixture 与 league 测试存在；`e63d320` 新增 **6 个程序级情景用例**（节时长/ORB 时钟/三分几何/犯规上限/bonus 门槛/节末终场），其中三分几何用例带负面对照 | **交替拥有与罚球程序仍未覆盖**；评判 fixture 带来源/版本的契约未完成；固定种子矩阵未运行；NCAA 仍是路线项 | `crates/domain/src/league.rs`、`crates/engine/tests/league_profile.rs`、`crates/evaluator/fixtures/` |
| 确定性与黄金哈希 | verified（现有测试范围） | golden hash 测试和事件 ID 单调测试存在 | 行为改动仍需按 protocol 重新冻结；黄金哈希不能证明真实性 | `crates/engine/tests/golden_hash.rs`、`docs/protocol.md` §3 |
| 文档与阈值守卫 | verified（脚本范围） | `check_docs.py`、`no_index_identity`、`threshold_integrity`、阈值自测、常数自测、World privacy 自测均可运行；前四项含 `--self-test` 负面对照 | `check_threshold_integrity.py` 与 `check_no_index_identity.py` **尚未接线到任何 CI workflow**；常数守卫计数未排除测试夹具与注释，棘轮对测试代码增长失效（见开放问题） | `scripts/check_docs.py`、`scripts/check_threshold_integrity.py`、`scripts/check_no_index_identity.py`、`.github/workflows/` |
| 统计形态（G-STATS） | **verified（`stats_baseline` 通过）/ partial（`free_throw_rate` 仍越界）** | `stats_baseline` 全绿；8 seed full 的 `total_p50` 213.5 / `two_make_pct` 0.536 / `three_make_pct` 0.337 / `three_attempt_rate` 0.334 / `pace` 218.4 均在各自带内；ORB% 0.244–0.330（真实 ~0.245） | 唯一未闭合：`free_throw_rate` 0.118 越出 [0.20, 0.35]。根因**已收窄到量级**：实测接触 `impact_speed` 中位数 6.18 ft/s vs 犯规候选门槛 12.76 ft/s——真实犯规多数不以高速碰撞为前提（§23.12）。不得调大 `foul_on_shot_rate` 凑带 | `crates/engine/tests/stats_baseline.rs`、`crates/evaluator/fixtures/nba.v2.json`、`docs/dev/evidence/problem.md` §21–§23 |

## 3. 当前未完成工作

当前只保留仍然需要动作的事项；已完成的 Round 记录不在这里复制。

### 3.1 收敛公共边界（字段可见性已完成，快照投影待做）

已完成（`da453cf` / `0bc7496`）：

- 16 个 `pub` 字段全部私有，只保留只读访问器（`tick_index`/`rules`/
  `physics`/`home_roster_order`/`away_roster_order`）；
- 测试后门集中为显式命名的 `physics_mut_for_test` / `rules_mut_for_test` /
  `modulation_for_test`；
- `check_world_privacy.py` 判据由「字段清单」升级为「零 `pub` 字段」，
  两类负面对照（真相字段 / 配置字段）均能变红。

仍需动作：

- 让 CLI、回放、评判与测试统一经**最小只读 `snapshot()`** 观察，
  不再各自选访问器；
- 事件流与黄金哈希在此过程中的变更需可解释（本步尚未触及）。

出口：外部调用只能通过 `step`、`snapshot`、只读访问器和显式命令观察/推进比赛；守卫与编译测试同时通过。

### 3.2 完成球态与阶段边界

- 将 engine 对 `BallTrajectoryKind` 的兼容别名收敛为领域 `BallState` 的明确消费面；
- 分离 `BallControl` 与 `BallMotion` 的内部职责，避免 `ball_pos_3d`、`carrier_idx` 和球态载荷重复承担真相；
- 把 `step_inner` 拆为可单测的窄签名阶段，并证明 `InvariantPhase` 无旁路。

出口：所有状态边有穷举测试；阶段写权限可由类型或编译边界表达；相关黄金哈希按协议重验。

### 3.3 完成战术/防守行为链

- 删除旧 `TacticalSet` 作为主路径的几何分支，统一从版本化档案生成机会；
- 让防守 scheme 进入责任分配、协防/换防/恢复动作和裁决，而不仅是目标点倍率；
- 为每个档案字段登记消费链和扰动链，移除无效字段与 roster-order fallback。

出口：新增档案不改 Rust；能力、进攻档案和防守档案都能在控制场景与比赛场景改变可解释的过程/结果；反事实负面对照有效。

### 3.4 补齐能力与阵容覆盖（部分完成）

已完成（`6726506`、`3631ef6`、`528d8c6`；清单见 evidence §29）：

- 逐维列出 29 个维度的消费点（19 有真实消费点、6 已实证零消费、
  3 名义有消费点但未接线）；
- 补齐 `effective_catch_radius` / `receive_estimate_noise` /
  `poke_check_success` 三条零覆盖链路（含分离性断言：接球半径不得消费
  `off_ball_sense`）；
- 补齐篮板轴三条端到端扰动（`offensive_rebound` / `defensive_rebound` /
  `off_ball_sense`，用固定种子 4000 次采样估概率后比较方向）；
- 把单调性判定抽成可复用 harness（含幅度门），并用**同一函数**验证三类
  断路负面对照 + 三条真实链路。扰动测试共 18 项，全部通过。

仍需动作：

- 剩余维度（`passing` / `shooting_mid` / `shooting_three` / `decision_iq` /
  `strength` / `vertical` 等）仍无独立响应证据；
- 6 个零消费维度待接线或标为未批准提案（§29）；
- 明确轮换、疲劳、教练策略是否属于当前周期。

出口：每个保留维度至少有一条正向和一条断路负面对照；死维度被接线或从 schema 移除。

### 3.5 形成真实的多联赛证据包（部分完成）

已完成（`e63d320`、`c13131b`、`f178e32`）：

- 6 个程序级情景用例（节时长 / ORB 时钟 / 三分几何 / 犯规上限 /
  bonus 门槛 / 节末终场），三分几何用例带负面对照；
- **16 seed full 固定矩阵已运行**（seeds 0..15，NBA，fixture nba.v2，
  121.5s），四类结果**分期记录**而非只留综合指数（evidence §30）：
  L1 violations = 0；评判 32894 条裁决、coverage 0.995、140 defect
  全为 Soft、hard_gate_failed=false；构成准则除 `free_throw_rate`
  0.119 外全部入带；`total_p50` 219.0 在门内；
- 16 seed 与 8 seed 结论一致，无新暴露的越界项；
- 矩阵运行**暴露并修复**了批量模式不做账本对平的缺陷（`f178e32`：
  batch 现逐场跑 `check_ledger`、落盘 `ledger_report.json`、
  违规则非零退出）。

仍需动作：

- 交替拥有与罚球程序的专题情景仍未覆盖；
- **FIBA 侧 8 seed full 矩阵已跑并通过**（`5504281` 定位、`f8425f1` 修复）：
  初次运行报 **2 Hard**（`POSSESSION_DURATION_BOUNDS`），根因定位为
  **容差标定不足**而非引擎缺陷——`duration_tolerance_seconds` 是 `1c86c49`
  新加字段，值 `fiba.v1/nba.v1=2.0`、`nba.v2=6.0`，互不一致且未经测量。
  按 **2843 个回合**的停表开销实测分布（FIBA p95=6.32/p99=8.59/max=12.89；
  NBA p95=6.45/p99=8.47/max=12.89）统一标定为 **15.0s**，两 league 的
  HARD gate 现均通过（FIBA Hard 2→0、NBA 判定不变）；
  门的区分力经负面对照验证（tol=−40 时报 439 Hard）；
- 评判 fixture 带来源/版本的契约未完成（`protocol.md` §1）。

出口：每个 profile 的情景门、回放一致性和评判工件完整；未覆盖的 NCAA 继续标记为路线项。

## 4. 最近关闭项

### 4.1 调试链缺陷与验证通道（D12）

D12 从本周期计划执行完毕后已从 `current/plan.md` 移除（该文件只保留未完成工作，见 `docs/dev/README.md` §1.2）；六项均已落地并验证：

| 项 | 结论 | 证据 |
| --- | --- | --- |
| D12.1 `events_since` 游标 | 改用跨 tick 唯一的 `event_id`；`step_once` 去重同步 | `crates/engine/src/service.rs`、`crates/engine/tests/constraint_system.rs::match_service_events_since_cursor_uses_global_event_id` |
| D12.2 前端三项指标 | 面板 `offensiveReboundPct`/`turnoverRate`/`foulRate` 为真实值（在线读取：20.0%/0.0%/0.0%，原恒为 `—`） | `crates/debug-server/static/app.js`、`scripts/verify_ui_alignment.py`（8 行渲染通过） |
| D12.3 投篮因果配对 | 按 `parent_event_id → SHOT_RELEASE.event_id` 配对；旧流无父链时才退化 FIFO | `crates/debug-server/static/app.js` |
| D12.4 规则投影单一化 | `frame_rules_from_game_rules` 为唯一投影；一致性测试抓出**两处**既有漂移（`player_radius_ft` 1.0→1.8、`separation_safety_margin_ft` 0.0→0.05） | `crates/engine/src/match_engine.rs`、`crates/protocol/src/frame.rs`、`crates/engine/tests/constraint_system.rs::frame_rules_default_matches_game_rules_default_projection` |
| D12.5 命中列表 | 每帧重建（在线读取：10 条在场球员，原无界增长） | `crates/debug-server/static/app.js` |
| D12.6 violations 通道 | `/api/simulate` 流末 `run_summary` 携带引擎官方违规；前端 `detectAnomalies` 已退役（在线确认 `undefined`），异常面板只渲染引擎违规 | `crates/debug-server/src/main.rs`、`crates/debug-server/src/main.rs::c6_6_tests`、`crates/debug-server/static/app.js` |

同批完成：渲染层移除全部 `innerHTML` 拼接（改 `el()` + `textContent`，`esc` 随之删除）。

验证范围：`golden_hash`、`constraint_system`（75 项）、`pass_information`、`attribution_integrity`、`nba-debug-server`（3 项）、`verify_ui_alignment.py`（全部卡片与画布对齐）、四项守卫 + 阈值自测。**未运行**：完整 workspace 套件（受统计门阻塞，见 `current/plan.md` §8）。

> 编号说明：`D12` 是本周期状态快照对已关闭任务的登记号；项内 `c6_6_tests` 等标识符是代码中的模块名，保留不改。

### 4.2 本周期后续收敛（统计门关闭与结构欠账）

在本周期内完成、已验证、且不再作为待办的项：

| 项 | 结论 | 证据/提交 |
| --- | --- | --- |
| **G-STATS 统计门关闭** | 8 seed full：`total_p50` 237.0→**213.5**、`two_make_pct` 0.621→**0.536**、ORB% 0.070→**0.244–0.330**、`pace` 234.5→**218.4**；`stats_baseline` 转绿。**未改任何门限或分母** | `7e331fc`、`10f2a53`、`ef4bd39`；机制见 evidence §21–§23 |
| 分区命中率模型（D8.4） | 修结构性缺陷：旧三分支只产生两种基准，使中距离按廊下基准（0.565）结算 | `7e331fc` |
| 篮板冲抢指派（D8.5） | 修行为缺失：球在空中时守方靠近速率是攻方约 42 倍而两者近零 | `10f2a53` |
| 失误计数单一入口（D8.6） | 修零写入字段：5 种失误终结只有 2 个自增点（实测 52 vs 12） | `ef4bd39` |
| 抛投犯规路径（D8.7，部分） | 新增 `foul_on_shot_rate` 与 `box_score.fouls` 计数；`free_throw_rate` 仅升至 0.118（未闭合） | `9bef004`、`b47e88f` |
| `BaseRates` 死参数清理（D9.4） | 删四个零消费字段，并区分「重复声明」与「未实现功能」两类 | `320bcaf`、`22bb764` |
| 旧 `TacticalSet` 几何退役（D9.1） | 删 330 行死计算；三组对照实验 + 8 seed 逐字段比对差异 0 | `3e9011b`、`db31fe7` |
| 球态边穷举矩阵（D8.3） | 10×10=100 种组合全覆盖（49 合法 / 51 非法），替代样例式测试 | `72661d5` |
| NBA/FIBA 程序级 fixture（D11.1，部分） | 6 个程序级情景用例 + 负面对照 | `e63d320` |
| 能力消费链清单（D10.1） | 29 维逐维清单，6 个零消费维度经实证确认 | `8410f75` |
| 黄金哈希窗口缺口 | 窗口内无 2 分出手而不报错，现改为断言覆盖 | `041cdfe` |
| 守卫失效面修复 | 常数棘轮不再把测试夹具计入生产预算；两项守卫接入 CI；文档守卫新增编号命名空间校验 | `323a012`、`0bc7496` |

验证范围：`./scripts/run-tests.sh` **45 个套件全绿、0 失败**（含 `stats_baseline`）；6 项守卫 + 负面对照全通过。

## 5. 开放问题

| 问题 | 依赖 | 下一验证动作 |
| --- | --- | --- |
| `free_throw_rate` 越界（唯一未闭合统计项） | 需重建犯规判定口径 | 实测接触 `impact_speed` 中位数 6.18 ft/s vs 犯规门槛 12.76 ft/s；真实犯规多数不以高速碰撞为前提（evidence §23.12）。应在 semantics 分类 + officiating 概率两层重建，附 8 seed 矩阵与反事实；**禁止**调大 `foul_on_shot_rate` 凑数 |
| `carrier_idx` 删除阻塞 | 需先定「无关联球员的球态下谁算 ball handler」 | evidence §25 实证非等价重构（8 seed 213.5→212.0）；旧/新语义含义不同，属 gap.md §5.2 状态边责任问题 |
| `GameRules` 零消费字段 | 分属四个未接线子系统 | 12 个已实证零消费（§27）；`def_*` 三字段随 D9.2 接线，`transition_*`/`clutch_*` 属 plan §10 范围外，需先决定是否本周期补齐 |
| 能力维度零消费 | 分属四个未实现的战术行为 | 6 个已实证零消费（§29）：`agility`/`shooting_close`/`cut_frequency`/`screen_frequency`/`offensive_rebound_frequency`/`transition_sprint`；接线或标为未批准提案 |
| 防守责任链三层未接线 | 需先补全档案数据 | `data/defense/schemes.json` 只有 4 个几何倍率，缺对位/换防/协防规则字段（§28） |
| `D12.6` 代码标识符遗留 | 无 | `debug-server::c6_6_tests` 等模块名仍用旧编号；不影响行为，可随下次触及该文件时改名 |

## 6. 当前周期计划入口

详见 [`current/plan.md`](current/plan.md)。计划只描述上述未完成工作的依赖和验收，不复制已经完成的 Round-6–17 执行记录。

## 7. 证据索引

- 历史问题复现与逐 seed 结果：[`evidence/problem.md`](evidence/problem.md)；
- 历史伤害排序：[`evidence/impact_assessment.md`](evidence/impact_assessment.md)；
- Round-6–9 闭环修复：[`cycles/20260911_first-principles/closure_plan.md`](cycles/20260911_first-principles/closure_plan.md)；
- Round-10–17 传球与身份修复：[`cycles/20260911_first-principles/pass_and_identity_fix.md`](cycles/20260911_first-principles/pass_and_identity_fix.md)；
- 本周期原始状态历史：[`cycles/20260911_first-principles/status_history.md`](cycles/20260911_first-principles/status_history.md)。

历史记录中的数字只回答“当时观察到什么”，不自动回答“现在是什么”。
