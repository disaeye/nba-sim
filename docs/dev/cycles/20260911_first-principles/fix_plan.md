# NBA-Sim · GAP 修复方案（gap.md 实施契约）

> **状态：已废弃的历史计划**
> 本文档属于已结束周期；F 编号仅保留作历史遗留项 ID。当前状态见 `docs/dev/status.md`，当前计划见 `docs/dev/current/plan.md`；执行顺序与验收门以当前计划为准。
>
> 文档类型：实施计划与验收契约
> 上游契约：`docs/dev/gap.md`
> 证据来源：`docs/dev/status.md`、`docs/dev/evidence/problem.md`、本轮代码审计与实测复现
> 纪律：本文档写"如何修、按什么顺序修、用什么证据验收"；现状与结果写入 `status.md` / `problem.md`

---

## 0. 本轮审计已确认的实测事实

以下不是推断，而是本轮在工作区直接复现的结果：

| 编号 | 事实 | 证据 |
| --- | --- | --- |
| E1 | `BALL_WITH_HOLDER` Hard 违反可复现，根因是**罚球期间权威球态仍是 `Held{carrier_id}`**，而球被放到篮筐/罚球点 | seed=1 full tick=58954：holder=A_3，球(5.2,25.0)，A_3(27.1,40.0)，dist=26.51ft，`game_flow=FreeThrow`, `sub_phase=DeadBallReset`, event=`FREE_THROW` |
| E2 | `backcourt_elapsed` 在跨半场后**永不重置**，只在 `transition_phase(Initiation)` 时清零 | 8 秒违例 `EIGHT_SECOND_BACKCOURT` 是 seed 0 单节最高频终端（26 次 `TURNOVER_VIOLATION`）的主要来源 |
| E3 | 评测器 `made_arrivals` 字段**从未被赋值**，恒为 0 | `SCORE_SOURCE_CAUSALITY` Hard defect 在 seed 0/1/7/42 分别 18/15/25/16 条，全部为误报 |
| E4 | 真实度严重失真：三分命中率 53–67%（目标 30–40%），每场回合数 236–301（真实约 200），3P 出手占比极高 | `stats_baseline` 实测：seed 42 `3P=62/92`；AGG `3P%_median=61.4`，`avg_poss=273.9` |
| E5 | `BALL_POSITION_DISCONTINUITY` 在 full scope 复现 | seed 555 tick=24324：球单 tick 位移 57.7ft（（0.37,0.92）→（0.94,0.50）） |
| E6 | 1q scope 成功率低于问题文档记录 | seed 0/1/7/42 `UNATTRIBUTED_END = 0`（该路径在本轮 1q 未复现）；但 full scope 仍有物理 Hard |
| E7 | 常数守卫 `WHITELIST_FILES` 是**整文件白名单**，31 个文件全豁免，阈值 `CURRENT_THRESHOLD=0` 形同虚设 | `scripts/check_inline_constants.py` |
| E8 | 事件无 `event_id`/`parent_event_id`；无 `EventEnvelope`（gap.md §7.1） | `crates/domain/src/event.rs` 仅 `GameEvent` 标签枚举；`FrameEvent` 有 `sequence` 无 ID |
| E9 | 无 `PossessionLedgerEntry`/`ActionLedger`/账本平衡检查（gap.md §7.3、§7.5） | `grep -rn Ledger crates` 仅命中日志字符串 |
| E10 | `World` 不是私有结构：`MatchEngine` 全部字段 `pub`，含 `pub possession`、`pub carrier_idx`、`pub ball_state`、`pub home_score` | `crates/engine/src/match_engine.rs:41+` |

---

## 1. 修复优先级（按 gap.md §1.2 排序）

```text
P0-A 状态与因果正确性：E1 罚球球态、E2 回场时钟、E5 球瞬移
P0-B 评测正确性：E3 made_arrivals 误报（否则所有 L2 证据不可信）
P1   真实度校准：E4 三分倾向/回合数（依赖 P0-B 的真实证据）
P2   结构契约：E8 事件 ID、E9 账本、E10 World 封装
P3   守卫与工具：E7 常数守卫重写、CI 负面对照
```

**关键纪律**：P0-B 必须先于 P1 完成，否则用错误的评判器校准会放大错误。

---

## 2. 分阶段实施方案

### 阶段 F1 · 球权权威状态与时钟（P0-A）

**F1.1 罚球程序显式球态（修 E1）**

现状：罚球期间 `ball_state` 保持 `Held{A_3}`，但 `ball_pos_3d` 被设为 `hoop_pos`/`rim_height`，导致 `BALL_WITH_HOLDER`。

修复：

- 罚球开始时经唯一写入口 `transition_ball_state` 转入 `Dead{pos: ft_spot, z: free_throw_z, last_touch_team}`；
- 每次 `FreeThrowAttempt` 前球置于罚球点（`Court::free_throw_pos`），而非篮筐；
- 命中后保持 `Dead` 直到发球程序；不中转入 `RimRebound`；
- 禁止在罚球路径直接写 `ball_pos_3d` 而不改权威态。

新增测试（先红后绿）：

- `free_throw_never_reports_holder`：罚球全程 `frame.ball.holder_id == None`；
- `free_throw_ball_within_leash_or_dead`：不变量检查 0 Hard；
- `free_throw_chain_events`：`FREE_THROW`→（命中）`Dead`→inbound /（不中）`RimRebound`。

**F1.2 回场时钟真实重置（修 E2）**

现状：`backcourt_elapsed` 仅在进入 `Initiation` 时清零；球过半场后继续累加，8 秒后无条件判违例。

修复：

- 在语义层（`step_inner` 的语义/裁决阶段）检测"进攻方持球越过中线"事实，重置 `backcourt_elapsed`；
- 前场判定使用 `is_in_frontcourt()` 同一几何函数，禁止第二套中线常数；
- 8 秒违例只在"仍处后场且连续时间 ≥ 阈值"时成立；
- 死球、换边、发球开始必须重置。

新增测试：

- `backcourt_clock_resets_on_halfcourt_cross`；
- `eight_second_violation_requires_continuous_backcourt`；
- `eight_second_negative_control`：人为断路（不重置）时测试必须失败。

**F1.3 球瞬移（修 E5）**

现状：`BALL_POSITION_DISCONTINUITY` 在 full scope 复现（57.7ft/tick）。

修复：

- 定位该 tick 的生命周期转换路径（预期在 `Dead`↔`InboundTransfer`/得分后重定位）；
- 所有离散 placement 必须走显式 `PlacementStarted/PlacementApplied` 事实（gap.md §4.3），并让检查器在 placement 阶段豁免；
- 禁止直接写 `ball_pos_3d` 造成跨帧跳变。

新增测试：

- `no_ball_teleport_outside_placement_phase`；
- `placement_facts_present_for_discrete_moves`。

### 阶段 F2 · 评测器正确性（P0-B）

**F2.1 修复 `made_arrivals`（修 E3）**

现状：`PossessionWindow.made_arrivals` 从未写入，`SCORE_SOURCE_CAUSALITY` 恒误报。

修复：

- `"SCORE"` 事件（`HoopArrival{is_made:true}`）递增 `made_arrivals`；
- `"SHOT_MISS"` 单独计数；
- 补充负面对照测试：构造"有 SCORE 无来源"的合成流，准则必须报 defect；构造"有来源"的流必须 pass。

**F2.2 严格解析与证据模型**

- `parse_stream` 改为默认严格（坏行 = 错误），软解析仅在显式 `--lenient` 下可用；
- `Judgment` 增加 `NotApplicable` 与 `InsufficientEvidence`（gap.md §15.1），当前只有 Pass/Defect；
- 空流/证据不足不得得满分（已部分实现，需覆盖全部准则）；
- 每条准则输出 `opportunities/passes/defects/insufficient` 固定分母。

**F2.3 真实度指数不抵消硬门**

- `AttributionReport` 增加 `hard_gate_failed: bool`；
- 任何 Hard defect 存在时 `realism_index` 标记为无效，而非仅数值下降；
- 报告同时列出各准则缺陷率与证据覆盖率。

### 阶段 F3 · 真实度校准（P1）

**F3.1 出手选择分布（修 E4 三分/回合数）**

现状：3P 占比与命中率严重偏高，回合数偏高。

修复路径（全部走 JSON override，先 A/B 再改默认）：

- 降低 `three_point_utility_multiplier` 或引入按距离的出手效用曲线；
- 检查 `shot_make_3pt` 基线 + `spacing_bonus` + `shot_pct_ceiling=0.85` 是否叠加过头（`spacing_bonus` 目前直接加在命中率上）；
- 每次改动附 ≥8 seed 的 `baseline_stats.json` vs `candidate_stats.json` 与归因 diff。

**F3.2 回合数与节奏**

- 校准 `dwell_base`、`action_duration_seconds`、`estimated_possessions_per_period` 的一致性；
- 让 `full` scope 以**真实比赛时钟/终场**结束（`ScopeBoundary::Game` 已是此语义），而非按回合预算推进。

### 阶段 F4 · 结构契约（P2）

**F4.1 事件 ID 与因果链（修 E8）**

- `GameEvent` 发布时分配 `event_id`；`FrameEvent` 增加 `event_id`/`parent_event_id`；
- 动作生命周期事件接入 parent（gap.md §7.2）；
- `PASS_TIPPED`/`PASS_DROPPED`/`STEAL` 保持独立 kind，不使用 `TURNOVER_STEAL` 伪装。

**F4.2 回合账本（修 E9）**

- 新增 `PossessionLedgerEntry` 与 `PossessionEndCause` 显式枚举；
- 消除 `UNATTRIBUTED_END`：若无法归因，评测器输出 `IncompleteEvidence` 且完整性门失败；
- 实现 gap.md §7.5 四个平衡式，输出 `ledger_violations.ndjson`。

**F4.3 World 封装（修 E10）**

- `MatchEngine` 真相字段改 `pub(crate)`；
- 外部只经 `snapshot()/step()/apply_command()`；
- `has_ball` 仅由 `holder()` 派生；
- 移除 `carrier_idx` 的写入路径（保留派生读取直至 F5 完成）。

### 阶段 F5 · 决策与战术（P2，依赖 F4）

- `carrier_idx` 索引绑定迁移为能力适配 slot fill（gap.md §10.3）；
- 防守动作 `switch/drop/hedge/over/under/recover` 补真实执行器（gap.md §10.4）;
- 决策 trace 100% 覆盖，执行点重校验。

### 阶段 F6 · 守卫与 CI（P3）

**F6.1 常数守卫重写（修 E7）**

- 移除整文件白名单；改为按行/符号的显式豁免（含 reason + reviewer）；
- 核心文件（`match_engine.rs`、`tactics.rs`、`pipeline.rs`）禁止整文件豁免；
- 扫描浮点常数、整数行为阈值、字符串战术 ID 分支、球员 ID 分支；
- CI 注入行为常数的负面对照必须使守卫变红。

**F6.2 资源与证据治理**

- 遵循 problem.md §14.4：默认 `cargo check -p <crate>`；长测试用临时 `CARGO_TARGET_DIR`；
- 事件流写 `/dev/shm` 且单 seed 结束即删；
- 每次验证前后记录 `df -h` 与 `du -sh target`。

---

## 3. 验收门（gap.md §18.6 目标）

| 门 | 内容 | 当前 | 目标 |
| --- | --- | --- | --- |
| G-BALL | `BALL_*` Hard = 0（全 seed、full scope） | 失败（E1/E5） | 0 |
| G-CLOCK | 8 秒/回场违例仅真实发生 | 失败（E2） | 情景测试通过 |
| G-EVAL | `SCORE_SOURCE_CAUSALITY` 无误报 | 失败（E3） | 0 误报 |
| G-COVER | `POSSESSION_COVERAGE` = 100%，`UNATTRIBUTED_END` = 0 | 部分 | 100% |
| G-REPLAY | 同输入同 seed digest 一致 | 通过 | 保持 |
| G-STATS | 3P% ∈ [30,40]%，回合数 ∈ 真实带 | 失败（E4） | 入门 |
| G-GUARD | 守卫负面对照变红 | 失败（E7） | 通过 |

---

## 4. 执行顺序与依赖

```text
F1.1 罚球球态 ─┐
F1.2 回场时钟 ─┼→ F2.1 made_arrivals → F2.2/2.3 证据模型 → F3 校准
F1.3 球瞬移   ─┘
                                ↓
                     F4 结构契约 → F5 决策战术
                                ↓
                     F6 守卫与 CI（可与 F4 并行）
```

每个子项纪律：**先写红测试 → 实现 → 定向测试绿 → 跑 ≥8 seed 取证 → 更新 status/problem**。

---

## 5. 本轮将执行的范围

本轮在环境约束下（磁盘 8.6GB 可用，禁止无界全量测试）执行：

1. F2.1 `made_arrivals` 修复 + 测试（最高性价比，立即恢复 L2 证据可信度）；
2. F1.1 罚球球态修复 + 测试；
3. F1.2 回场时钟重置 + 测试；
4. F6.1 常数守卫去整文件白名单（按行豁免）；
5. 定向验证：相关 crate 测试 + ≥8 seed 1q 取证。

F1.3/F2.2/F2.3/F3/F4/F5 作为后续任务保留在 todo 中，附本轮已定位根因。
