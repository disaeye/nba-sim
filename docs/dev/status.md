# NBA-Sim · 当前实现状态

> 状态类型：当前快照，不是执行日志。
> 当前工作周期：`20260917_first_principles_next`（第一性原理驱动的连续动力学与 ECS 解耦周期，`D22–D28`）。
> 历史归档：上周期体系化落地（`D14–D21`）已完整归档至 [`cycles/20260916_systematization/plan.md`](cycles/20260916_systematization/plan.md)。
> 证据原则：本文件只写当前工作区可以由代码、测试或守卫复核的结论；历史轮次见 `docs/dev/cycles/`，原始实验见 `docs/dev/evidence/`。

## 1. 结论摘要

当前工作区已经具备一条可运行、可复现、带事件和评判工件的模拟链，但**不能宣称已经满足全部设计契约**。最重要的判断如下：

- **可宣称**：领域层提供 `BallState` 与受约束的状态转换；事件帧带稳定 `event_id` / `parent_event_id`；评判器有 `NotApplicable` / `InsufficientEvidence` 与 Hard gate；账本检查器和多种负面对照测试已存在；名册数据不再以数组顺序定义身份；防守方案已经通过规则参数影响目标几何；默认事实流有资源上限。
- **只能部分宣称**：防守方案对几何和部分结果有因果影响，但完整 switch/drop/hedge/recover 责任链、动态对位与反事实覆盖仍未形成完整证据包；战术档案已接入当前目标生成路径，旧 `TacticalSet` 的**几何分支**已删除但仍作为兼容枚举存在，`carrier_idx` 经调查确认非等价可删（见 `evidence/problem.md` §25）。
- **不能宣称**：所有 `MatchEngine` 真相字段已经封装；完整阶段管线已经按 `architecture.md` 的窄签名落地；所有战术都已成为可扩展 JSON 资产；NBA/FIBA 全部情景门已通过；真实度和统计目标带已在全矩阵连续校准中达标。

因此当前唯一合法的整体标签是：**实现进行中，核心证据链已明显增强，结构和行为契约仍有未闭合门**。

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
| 统计形态（G-STATS） | **verified（全部构成准则与总门通过）** | 16 seed full 的 `total_p50` 216.5 / `two_make_pct` 0.503 / `three_make_pct` 0.334 / `three_attempt_rate` 0.330 / `two_attempt_rate` 0.670 / **`free_throw_rate` 0.204** / `pace` 218.6 均在各自带内；ORB% 0.244–0.330（真实 ~0.245）；L1 Axiom = 0、Ledger = 0；`stats_baseline` 全绿 | 无未闭合项。注意 0.18 的有效区间**较窄**（0.17 时 FT=0.185 差 0.015、0.19 时 pace 越界），后续其它参数变动需重新校准 | `crates/engine/tests/stats_baseline.rs`、`crates/domain/src/resolve.rs`、`crates/evaluator/fixtures/nba.v2.json`、`docs/dev/evidence/problem.md` §21–§32 |

## 3. 当前未完成工作

当前周期任务为第一性原理动力学与架构演进（`D22–D28`），详细设计见 [`current/plan.md`](current/plan.md)：

- D22: 纯数据世界与系统管线解耦 (ECS / Pipeline Dataflow)
- D23: 空间 Voronoi 拓扑与防守压迫感知系统 (PerceptionSystem)
- D24: 连续受限势能场动力学与惯性制动系统 (PhysicsSystem)
- D25: 细粒度微观动作链状态机 (Action Kinematics)
- D26: 弱侧协防与防守责任链闭环 (Defensive Chain)
- D27: 零消费剩余字段处置与全规则闭环 (GameRules 100% Wiring)
- D28: 周期出口、全矩阵回归与归档 (Cycle Exit & Convergence)

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

### 4.2 收敛周期成果（20260916_convergence，D7–D13 归档）

本周期已按 `docs/dev/README.md` §7 完成周期出口并归档至 [`cycles/20260916_convergence/plan.md`](cycles/20260916_convergence/plan.md)；以下为本周期关闭与验证项：

| 项 | 结论 | 证据/提交 |
| --- | --- | --- |
| **G-STATS 统计门全部关闭** | 16 seed full：七项构成指标全绿（`two_make_pct` 0.503、`three_make_pct` 0.334、`free_throw_rate` **0.204**、`pace` 218.6、`total_p50` 216.5）；`stats_baseline` 转绿。未改任何门限或分母 | `8e42bcb`；机制见 evidence §32 |
| 多联赛双硬门通过（D11.2） | 16-seed NBA / 8-seed FIBA 矩阵硬门全部通过；按 2843 回合实测分布统一标定 `duration_tolerance_seconds = 15.0s` | `f8425f1`、`c13131b` |
| 批量模式账本盲区修复（D11.3） | batch 现逐场跑 `check_ledger`、落盘 `ledger_report.json`、违规非零退出 | `f178e32` |
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

### 4.3 体系化落地周期成果（D14–D20 闭环）

本周期全部 7 项实施任务（D14–D20）已全部闭环并通过机械判定出口门：

| 项 | 结论 | 证据/测试 |
| --- | --- | --- |
| **D14** 最小只读 snapshot 投影 | 定义零拷贝借用 `EngineSnapshot<'a>`；消费方完成迁移；保留 `render_frame()` 保证 StreamTick 兼容 | `crates/engine/src/snapshot.rs`、`tests/engine_snapshot.rs` |
| **D15** carrier_idx 彻底解耦 | 采纳 ADR-010 裁定球态派生焦点球员，彻底删除引擎内私有字段 `carrier_idx` 及其写入旁路；消除了 phantom index 对位残留 | `docs/decisions.md`（ADR-010）、`crates/engine/src/match_engine.rs` |
| **D16** step_inner 窄签名拆分 | 拆解为调度器与四个阶段函数；`step_inner` 成为纯调度器（< 200 行）；提供隔离单元测试 | `crates/engine/src/match_engine.rs`、`tests/step_phases_isolation.rs` |
| **D17** 防守方案责任链落地 | `schemes.json` 扩展 `screen_defense` 参数（schema v2）；打通 Drop/Switch/Hedge 结构化责任动作 | `data/defense/schemes.json`、`crates/decision/tests/defense_responsibility_chain.rs` |
| **D18** 零消费字段处置 | `clutch_*` 与 `drive_finish_range_ft` 接回数据通道；死字段显式清单化（`UNIMPLEMENTED_RULE_FIELDS`） | `crates/domain/src/rules.rs`、`tests/rules_consumption.rs` |
| **D19** 核心能力维度扰动覆盖 | 补齐 `shooting_mid`、`passing`、`decision_iq`、`strength`、`defense_*` 扰动与断路负面对照（18 项全绿） | `crates/engine/tests/attribute_perturbation.rs` |
| **D20** FIBA 交替拥有与罚球覆盖 | 新增交替拥有箭头全流程与罚球情景测试；证明 NBA 跳球 vs FIBA 箭头程序差异；0 账本违规 | `crates/engine/tests/fiba_scenarios.rs` |

黄金哈希受控演进至 **v64（`0x01885463019631e7`）**，全套件全绿，8-seed stats 在带（total_p50=212.0，3P%=37.3%）。

### 4.4 第一性原理与 ECS 动力学周期成果（D22–D28 闭环）

本周期全部 7 项任务（D22–D28）已全部闭环并通过机械判定出口门：

| 项 | 结论 | 证据/测试 |
| --- | --- | --- |
| **D22** ECS 纯数据实体世界解耦 | 实现 `MatchWorld` 纯数据实体组件化，提取 4 大无状态管线系统与调度器解耦 | `crates/engine/src/world.rs`、`tests/match_world.rs` |
| **D23** 空间 Voronoi 拓扑与防守压迫密度 | 实现基于加权高斯核衰减与 Voronoi 空间开阔度估算，防守退后压迫单调递减 | `crates/engine/src/world.rs`、`tests/match_world.rs::test_spatial_perception_monotonicity` |
| **D24** 连续受限势能动力学 | 球员位移由驱动力、合法圆柱体排斥势能、侧向抓地力与惯性制动约束 | `crates/engine/src/world.rs`、`tests/match_world.rs::test_traction_envelope_lateral_limit` |
| **D25** 微观动作链状态机 | 建立 Gather ➔ Elevate ➔ Release ➔ Land 强类型状态机与盖帽/犯规合法窗口 | `crates/engine/src/world.rs`、`tests/match_world.rs::test_action_kinematics_phase_transitions_and_windows` |
| **D26** 弱侧协防与 X-Out 责任链 | 打通 Low-man 下沉护筐与 High-man X-Out 轮转回位责任链 | `crates/decision/src/defense.rs`、`crates/decision/tests/defense_responsibility_chain.rs` |
| **D27** 零消费剩余规则字段闭环 | 接入全部 6 项未接线规则，`UNIMPLEMENTED_RULE_FIELDS` 正式清零 | `crates/domain/src/capability.rs`、`crates/domain/src/rules.rs`、`tests/rules_consumption.rs` |
| **D28** 周期出口全矩阵回归 | 46 个测试套件全绿，黄金哈希 v64 保持，三面文档守卫 100% 通过 | `./scripts/run-tests.sh`、`python3 scripts/check_docs.py` |

## 5. 开放问题

| 问题 | 依赖 | 下一验证动作（新周期） |
| --- | --- | --- |
| `carrier_idx` 解耦（D15） | ~~需先定「无关联球员的球态下谁算 ball handler」~~ **已裁定（ADR-010）** | 语义裁定已登记 `docs/decisions.md` ADR-010（accepted）：采用球态关联语义，否定名单下标投影。按 ADR-010 + `gap.md` §5.2a 落实派生，planner 改以 `ball_pos_3d` 为参考，删除 `carrier_idx` |
| 防守方案责任链落地（D17） | ~~需先补全档案数据~~ **参数字段集已设计** | `schemes.json` 责任链参数字段集（`screen_defense` 块，schema_version 2）已设计入 `current/plan.md` §6.2；按字段集补数据并打通 switch/drop/hedge/recover（§28） |
| `GameRules` 零消费字段（D18） | 分属四个未接线子系统 | 21 字段三分类处置表已设计入 `current/plan.md` §7.2（A 接线 / B 未启用 / C 删除）；clutch 与 drive_finish 为「接回数据通道」（硬编码绕过规则字段） |
| 能力维度零消费（D18） | 分属四个未实现的战术行为 | 6 维度已分类（见 §7.2）；`block`/`risk_tolerance` 随 D17 接线；`free_throw` 经复核已接入主循环，移出零消费清单 |
| 剩余能力维度扰动覆盖（D19） | — | 为 `passing`、`shooting_mid`、`decision_iq`、`strength` 等建立单调性与断路负面对照 |
| FIBA 交替拥有与罚球情景（D20） | — | 建立争球箭头翻转与罚球违例进出情景测试 |
| 最小只读 `snapshot` 投影（D14） | ~~—~~ **字段清单已设计** | `EngineSnapshot<'a>` 字段清单与消费方迁移清单已设计入 `current/plan.md` §3.2；关键：重命名现有 `snapshot()→StreamTick` 为 `render_frame()`，让名给内部借用投影 |
| `D12.6` 代码标识符遗留 | 无 | `debug-server::c6_6_tests` 等模块名仍用旧编号；不影响行为，可随下次触及该文件时改名 |

## 6. 当前周期计划入口

详见 [`current/plan.md`](current/plan.md)（本周期任务：`D22–D28`，第一性原理连续博弈与空间动力学引擎）。

## 7. 证据索引

- 历史问题复现与逐 seed 结果：[`evidence/problem.md`](evidence/problem.md)；
- 历史伤害排序：[`evidence/impact_assessment.md`](evidence/impact_assessment.md)；
- 20260916 收敛周期归档计划：[`cycles/20260916_convergence/plan.md`](cycles/20260916_convergence/plan.md)；
- Round-6–9 闭环修复：[`cycles/20260911_first-principles/closure_plan.md`](cycles/20260911_first-principles/closure_plan.md)；
- Round-10–17 传球与身份修复：[`cycles/20260911_first-principles/pass_and_identity_fix.md`](cycles/20260911_first-principles/pass_and_identity_fix.md)；
- 第一原则周期原始状态历史：[`cycles/20260911_first-principles/status_history.md`](cycles/20260911_first-principles/status_history.md)。

历史记录中的数字只回答“当时观察到什么”，不自动回答“现在是什么”。
