# 模拟事件流校验问题记录

> 本文档按 seed=0..99 严格串行生成：每次只保留当前一场事件流，逐帧检查后再按回合检查；当前场的事件流、评判工件和临时数据在进入下一 seed 前删除。以下记录只写入已实际观测到的问题，静态设计差距另列于末尾。

## 1. 运行范围与执行方式

- 执行 `seed=0..99`，每个 seed 只生成一场 `1q` 流；流被逐行读取并完成帧级、事件级、回合级检查后立即删除，再开始下一个 seed。
- 每场规模：21,411–24,075 tick；100 场均完成，未出现死循环或超时。
- 100 场中 98 场 CLI 正常退出，2 场因引擎内 Hard 不变量失败退出码为 1：seed 49、seed 54。
- 结果证据：98 场成功完成评判工件，共产生 `45,788` 条逐回合/逐阶段/比赛级评判记录；2 场在 Hard 不变量失败后按 CLI 约定提前退出，因此没有评判工件。汇总只保留在本次分析内存中，未保留事件流。

## 2. 已实际观测到的问题（按严重度）

### P0 · seed=49 出现 Hard `BALL_SPEED`

- **复现**：seed 49，tick 2760。
- **事实**：`CONTROL_TRANSFER` 球速 `95.70 ft/s`，帧内允许上限（含 5% 过渡容差）`89.25 ft/s`；CLI 以退出码 1 结束。
- **影响**：控制交接轨迹违反物理速度约束；与 `docs/quality.md §1.1` 的“物理不可能必须捕获”一致，但引擎基线未做到零 Hard 违反。
- **定位**：`crates/engine/src/match_engine.rs` 的篮板/松球后 `ControlTransfer` 建立路径，以及 `crates/physics/src/ballistics.rs` 的交接采样共同决定该轨迹；需在规则通道内保证交接位移、持续时间和球速上限相容。
- **设计符合性**：检测器行为符合设计；生成轨迹不符合 `docs/charter.md §4 C3` 物理约束和 `docs/design.md §3.1 M1` 标准种子零 Hard 违反验收。

### P0 · seed=54 出现 Hard `BALL_WITH_HOLDER`

- **复现**：seed 54，tick 14050。
- **事实**：球状态为 `DRIVE`，`ball.holderId=A_1`，球与持球人距离 `24.02 ft`，超过帧内 `holder_leash_ft=3.0 ft`；CLI 以退出码 1 结束。
- **影响**：突破期间输出了球权持有人，但球的采样位置没有保持在持球人 leash 内，形成球人与球分离；违反 `docs/quality.md §1.1` 的球权/物理底线。
- **定位**：`crates/physics/src/ballistics.rs` 的 `BallState::Drive` 采样将球放在运动方向偏移处，而 `crates/engine/src/match_engine.rs` 的突破目标/球员运动状态可能在该帧已经远离原突破者；需让 Drive 球位置始终相对 driver 合法，或在状态转换前清晰结束持球语义，不能输出自相矛盾的 holder。
- **设计符合性**：不变量捕获符合设计；轨迹输出不符合 `docs/architecture.md §3.1–3.2` 的持球派生一致性与 `docs/design.md §3.1 M2` 的 `BALL_*` 零违反验收。

### P1 · 回合级失误归因评判口径缺口（98/100 场，2,215 条 `TURNOVER_ATTRIBUTION` defect）

- **事实**：评判器 `TURNOVER_ATTRIBUTION` 在 98 场中产生 2,215 条 Hard defect；它的实现只把 `STEAL`、`PASS_DROPPED`、`VIOLATION` 视为失误原因，不把事件模型已有的 `PASS_TIPPED` 计入归因。
- **复核证据**：seed 0 的单场复核中，多个 `TURNOVER_STEAL` 回合的事件窗口明确包含 `PASS_TIPPED`，但仍被该准则判为“without attributable cause”；因此这批数字同时包含评判器漏识别，不能全部归因成引擎没有产生原因事件。
- **影响**：L2 评判结果与实际事件流不一致，且 `PASS_TIPPED → LooseBall →` 对方拿球的终端总结被引擎标成 `TURNOVER_STEAL`，使“抢断”和“传球被点掉后形成失误”在回合摘要中混淆。
- **定位**：`crates/evaluator/src/lib.rs` 的 `TURNOVER_ATTRIBUTION` 判定需覆盖 `PASS_TIPPED`（以及其他已定义的失误原因事件）；`crates/engine/src/match_engine.rs` 的 `start_loose_ball_transition` 需区分直接抢断与松球易主的终端语义，保证 `PossessionSummary.terminal_event`、`turnover_player_id` 和事件链一致。
- **设计符合性**：当前不满足 `docs/quality.md §2.1` “每个失误有归因（抢断/传球失误/违例）”及 `docs/charter.md §6` 的观测闭环；其中一部分是评判器实现缺口，不能简单作为决策失误率问题处理。

### P1 · 回合时长大量异常短（98/100 场，至少 2,292 条 `RHYTHM_DURATION` defect）

- **事实**：`RHYTHM_DURATION` 在 98 场中产生 2,292 条 Soft defect；典型回合时长被序列化为 `0.1s`，而 NBA fixture 的 turnover 时长下限为 `0.5s`。
- **细分**：`TURNOVER_STEAL` 共 2,720 个，其中 2,010 个时长为 `0.1s`；`TURNOVER_VIOLATION` 共 1,751 个，其中 168 个为 `0.1s`。
- **影响**：大量“持球后立即被抢断”的结果虽可能在篮球中发生，但当前模型把其压缩到固定的 `0.1s`，不符合设计要求的结果条件节奏分布；同时削弱回合因果解释和比赛节奏真实性。
- **定位**：`crates/engine/src/match_engine.rs` 的 `emit_possession_summary` 使用 `(start_clock-end_clock).abs().max(0.1)`，而抢断/回合边界在同一 tick 内完成；应保留真实 elapsed tick 时间并让事件顺序/回合切分反映实际飞行或控制时间，不能用下限填充掩盖零时长。
- **设计符合性**：不变量可能允许该值，但不满足 `docs/quality.md §2.1` 回合时长与 `§2.1(b)` 节奏评判要求；属于“可能但不像”的 L2 缺陷。

### P1 · 失误率远超参考分布（98/100 场 `TURNOVER_RATE` defect）

- **事实**：98 场达到回合评判门槛并触发 `TURNOVER_RATE` defect；参考带为 `[0.05, 0.35]`，实际各场通常约 `0.41–0.61`（seed 0 为 `0.51`）。
- **影响**：失误过程产量明显偏高，说明决策/传球/抢断链路的概率校准不在 `nba.v1` 参考分布内。
- **定位**：责任归因按评判器为 `decision`；需从 `DecisionRules`、传球/抢断候选效用与规则化概率链检查，而不是只修改汇总阈值。
- **设计符合性**：违反 `docs/quality.md §2.5` 当前 sanity net 与 `§2.1(b)` 真实度节奏/动作构成要求，也说明 `docs/charter.md §6` 的跑批→评判→归因闭环确实暴露了未校准行为。

### P1 · 传球接球人与走廊不一致（56/100 场，78 条 `PASS_CORRIDOR_REACHABLE` defect）

- **事实**：评判器观察到接球人在到达时距离传球线段超过允许走廊；典型：seed 0 possession 65，A_2 距目标/走廊约 `5.0 ft`。fixture 走廊半径为 `4.0 ft`，评判器额外容差后仍判缺陷。
- **影响**：传球目标点、接球人实际跑位和到达时刻之间不一致；违反 `docs/quality.md §2.1(a)` “成功传球接球人必须在传球走廊可达范围”。
- **定位**：`crates/engine/src/match_engine.rs` 的 `execute_pass` 保存 `to_pos`，`crates/physics/src/ballistics.rs` 的 Pass 采样使用接球人当前位置插值，`crates/evaluator/src/lib.rs` 却用事件目标点与接球帧位置复核；目标预测、接球移动和评判口径需要统一。
- **设计符合性**：不满足 `docs/architecture.md §2` 弹道/语义链和 `docs/quality.md §2.1` 走廊因果准则。

### P1 · 少数抢断者不在传球走廊（2 场，2 条 `STEAL_CORRIDOR` defect）

- **事实**：seed 35 possession 26，A_3 距 H_3→H_1 传球走廊 `48.4 ft`；seed 67 possession 53，H_4 距 A_4→A_1 走廊 `38.8 ft`，均被评判器判 Hard defect。
- **影响**：事件标记为抢断，但抢断者空间位置与传球路径不相容，属于明显叙事断裂。
- **定位**：引擎的拦截触发条件与事件中记录的 `position`/传球线段不一致，需检查 Pass 拦截采样、位置记录和事件窗口配对。
- **设计符合性**：违反 `docs/quality.md §2.1(a)` 的 STEAL 走廊约束；评判器已正确暴露问题。

### P2 · 少数重压出手（2 场，2 条 `SHOT_QUALITY_CONTEST` defect）

- **事实**：seed 10 possession 54 的 H_2 contest `0.77`，seed 17 possession 67 的 A_1 contest `0.81`，超过 fixture `0.75` 重压阈值。
- **影响**：高压出手不是物理不可能，但作为当前设计的出手质量清单应记录为低质量选择；需结合球员能力、比赛时钟与动作候选判断是否为合理的节末/无替代选择。
- **设计符合性**：评判器行为符合 `docs/quality.md §2.1(b)`；当前回合选择仍有 Soft 真实度缺陷，不能以“合法”替代“合理”。

### P2 · 单回合传球数超出参考带（1 场，1 条 `ACTION_COMPOSITION_PASSES` defect）

- **事实**：seed 84 possession 22 为得分回合，传球数 `11`，超过 NBA fixture 得分回合 `[0,9]`。
- **影响**：动作构成偏离参考分布，属于 Soft 真实度缺陷。
- **设计符合性**：符合评判器应报缺陷的要求，但当前决策输出不满足 `docs/quality.md §2.1(b)` 动作构成目标。

## 3. 逐帧物理/状态分析结论

- 100 场逐帧检查未发现：球员越界、球员速度超限、球高度越界、比分倒退、比分单 tick 跳变超 3 分、同节比赛时钟回流、多人同时持球、summary 重复索引或事件序列倒退。
- 观测到的全局最小在场球员间距为 `2.999916 ft`，而 L1 当前判定阈值为 `min_player_separation_ft × 0.5 = 1.8 ft`；因此未触发 L1，但相对于 `GameRules.min_player_separation_ft=3.6 ft` 的完整几何分离目标存在约 `0.600084 ft` 的边界压缩。该差异来自不变量当前“半阈值 Soft”口径，需在设计上明确这是允许的安全缓冲还是未满足完整几何约束，不能把“未触发”误写成“完全符合”。
- 球的逐帧最大观测速度在不同状态下明显高于 `ball_max_speed_ftps`，但官方 L1 只在非持球球以及 `CONTROL_TRANSFER` 特定路径检查；因此最大值本身不能直接作为新的 Hard 违反。已报告的 seed 49 是官方检查实际捕获的过渡球速违规，seed 54 是官方捕获的 holder leash 违规。
- 100 场 `1q` 的回合数为每场 59–110，均值约 83.74；`PossessionSummary` 没有重复索引，终端事件包括 `UNATTRIBUTED_END` 共 128 条。后者不是本次评判器 Hard 直接报错，但其名称表示兜底结束路径，仍与“每个回合有完整语义因果总结”的目标存在审计风险，应在修复因果窗口时一并解释/消除。

## 4. 与设计文档的实现符合性审计

### 已观察到符合的部分

- 事件流确实以 fixed tick 输出，并且单场可以由 seed 重放。
- `FrameRules` 随帧输出，L1 检查使用帧内规则；标准 5v5、球员边界、速度、比分/时钟等基础检查在本轮 100 场中未发现对应问题。
- 事件日志携带序号、时间和 payload；回合 summary、阶段事件、传球/出手/篮板/违例等事件可被逐帧解析。
- CLI 对 Hard L1 违反返回非零退出码；seed 49/54 的行为证明检测链实际生效，而非仅写 stderr。

### 尚不能宣称“完全实现”或本轮运行中已被反证的部分

- **M1/L1 零 Hard 基线未满足**：100 场 1q 中 seed 49、54 各有一条 Hard 违反。
- **M8 逐回合因果闭环未满足**：`TURNOVER_ATTRIBUTION` 2,215 条 Hard defect，且大量回合走 `UNATTRIBUTED_END` 兜底总结。
- **M8 真实度目标未完全满足**：虽然未失败的场次 realism index 约为 `0.8979–0.9553`（均值约 `0.9343`），但 98 场存在失误率、节奏或走廊等缺陷；不能以指数均值代替逐条裁决。尤其 charter 成功判据要求 `≥0.90` 且 top 缺陷连续两轮收缩，本轮只有一轮采样，无法证明“连续两轮收缩”。
- **C1/TA1 的声明式战术完整实现无法仅由事件流证明**：代码中仍存在 `TacticalSet` 枚举与 `carrier_idx` 参数化索引调用（`crates/decision/src/tactics.rs`、`crates/engine/src/match_engine.rs`），而设计文档要求新增战术应通过 JSON 档案、slot fill 按能力适配。静态结构与文档声称的完全数据资产化之间存在需要继续修复/核验的差距。
- **M10/C3 的“切换档案而非联赛特化代码”需静态+FIBA 运行双重证明**：本轮只执行 NBA seed 0..99 `1q`，不能把 NBA 结果外推为 FIBA 完整实现。
- **性能与 zero-allocation 等质量条款未在本轮单场事件流审计中验证**，不能将历史 status 声明当成本轮证据。

## 5. 本轮结论

本轮 100 场不是“全部通过”：基础帧级大部分约束稳定，但存在 2 条可复现 Hard 物理/球权错误，以及大规模 L2 因果、节奏、失误率和走廊缺陷。因此当前实现不能依据本轮证据宣称已经完全实现 docs 设计文档；问题已集中记录，所有生成的事件流和临时工件均已在 seed 切换前清理。

## 6. 修复设计与后续验证契约（本轮续审）

以下不是将未执行的修复冒充为结果，而是依据已定位的代码路径固定实施边界；修复完成后必须重新跑 seed 矩阵并以新证据更新本节。

### 6.1 球轨迹与罚球

- `Drive` 必须保持 `BallState::carried_by()` 的持球语义：球的 XY 位置由 driver 当前坐标加受规则限制的持球偏移派生，偏移的最大范数不得超过 `invariant_holder_leash_ft`；若 driver 不存在或动作已结束，必须先经 `transition_ball_state` 转为 `Held`/`Shot`/`Dead`，不得继续以 `DRIVE + holder` 输出远离球员的轨迹。
- `ControlTransfer` 的目标点和持续时间必须在创建时依据球员当前坐标与 `ball_max_speed_ftps` 计算，并对端点/协议舍入保留 `invariant_speed_tolerance_ftps` 安全余量；交接状态不能以过渡期持球标志掩盖不可行的位移。
- 罚球属于停表的显式事件链：`FreeThrowAttempt` 必须先记录，命中后只增加 1 分并进入下一罚或发球，未命中后进入 `RimRebound`；不得通过直接改球状态绕过唯一写入口。罚球命中率继续从 `free_throw` 能力与 `GameRules.resolve` 派生，不能恢复全局常数。

### 6.2 回合因果与传球

- `PASS_TIPPED` 是失误归因中的合法原因；评判器必须把它与 `PASS_DROPPED`、`STEAL`、`VIOLATION` 分开记录，并且 `PassTipped → LooseBall → LooseBallSecured` 不得被摘要伪装为直接 `TURNOVER_STEAL`。
- `PossessionSummary` 的 `terminal_event`、`turnover_player_id`、`rebounder_id` 必须与同一回合事件窗口一致；直接抢断使用 `STEAL + defender`，点掉后由对手收球使用 `PASS_TIPPED + LOOSE_BALL_SECURED + 收球人`。
- `duration_seconds` 必须来自单调的 `current_time` 差值。允许真实的亚秒回合，但禁止用 `max(0.1)` 把零时长伪装成经过时间；若同 tick 完成边界，最小值应由一个实际固定 tick 或显式事件时刻提供。
- 传球 release、目标点、receiver movement 和 receive tick 必须共享同一坐标/时间语义。首选在 release 时冻结接球目标并让接球人沿可达轨迹收敛；禁止采样时又改用 receiver 当前坐标、评判时再用旧 `to_pos`，造成三套事实。

### 6.3 决策校准与战术迁移

- 先修因果和物理 Hard 缺陷，再从 `TURNOVER_RATE`、`RHYTHM_DURATION`、`PASS_CORRIDOR_REACHABLE` 的归因账本选参数；任何概率/效用调整走 `GameRules`/`DecisionRules`/`ResolveConfig` JSON override，附修改前后至少 8 个 seed 的统计与归因 diff。
- `TacticalSet`/`carrier_idx` 当前仍是静态风险点。迁移目标是 `OffensiveSystem` JSON 的 slot/requirements/sequence 加纯函数 slot fill；新增档案不得改引擎枚举或按球员 ID 分支。迁移期间必须保留事件 schema 和能力响应测试，避免用黄金哈希掩盖行为变化。

## 7. 回归门与串行重放协议

- 每个修复先跑针对性测试：BallState 非法边、ControlTransfer 速度、Drive leash、罚球事件链、`PASS_TIPPED` 归因和 summary 字段一致性；失败测试必须先红后绿。
- 再以 release CLI 严格串行生成 `seed=0..99` 的 `1q` 流；每场逐行运行同一 `InvariantChecker` 与 evaluator，记录 Hard/Soft、summary 覆盖、终端事件、回合时长和走廊缺陷，随后删除该场所有流与评判工件，再进入下一 seed。
- 物理门：全矩阵 `BALL_*` Hard = 0，双方各 5 人、单一持球、球高/球速/球员速度和比分时钟不变量均为 0；L1 与 L2 结果必须分栏，不能把“未触发”写成“完全符合”。
- 因果门：`POSSESSION_COVERAGE` 无缺口；每个失误有可识别原因；`UNATTRIBUTED_END` 为 0，除非另有事件链证据并在报告中解释；每个得分、篮板和罚球事件能回接到对应来源。
- 真实度门：至少 8 场才可做 sanity net；同时逐回合缺陷率必须按准则报告。真实度指数不得替代 Hard 零违反、回合零遗漏或连续两轮 top-defect 收缩证据。

## 8. 文档与证据交付

- 修复代码、针对性回归、运行矩阵和归因报告属于同一证据链；更新 `docs/status.md` 前必须有可复现命令和实际输出，禁止沿用历史“全里程碑完成”声明覆盖新失败结果。
- 本文档只保留实际观测结果；设计目标、验收门和未执行项必须明确标注，避免把计划写成通过结论。
- 任何新默认参数按 `docs/design.md §2` 记录基线、JSON override、前后统计、归因变化及黄金哈希处理；如果行为有意变化，必须重新冻结哈希并说明原因。

## 9. 修复前续审记录（已由 §10 更新）

本节保留修复前的实施契约和状态快照，仅用于解释问题发现顺序；“没有修改引擎行为、没有重跑 100 个 seed”的结论属于 §10 修复之前，不代表当前工作区状态。当前实际修复与验证结果以 §10 为准。

## 10. 续审已实际完成的修复与验证（当前工作区）

本节只记录本轮已经执行并观察到的结果；它不把仍未通过的 L2 评判门写成通过结论。

### 10.1 已落地修复

- `ControlTransfer` 到达后不再把过渡球伪装成持球：协议渲染的 `holderId` 仅在 `Held`、`Drive`、`InboundReady` 输出；交接飞行由固定端点和速度预算驱动。
- 松球收球路径不再先切成 `Held` 再继续输出 `ControlTransfer`，而是先完成回合归属更新，再以固定目标位置建立交接轨迹；作用域结束时直接冻结为死球。
- 成功传球接球时删除接球人瞬移到球位置的回退，接球人继续由物理运动到达冻结的 release 目标，避免 `PLAYER_SPEED` 瞬移。
- 罚球结算期间清除旧持球标志并把球置于 `Dead`/罚球点语义，避免命中事件帧将球错误渲染在旧持球人身上。
- 战术运行路径增加 `TacticalSetSpec::builtin` JSON 档案加载和 profile planner；引擎每 tick 消费档案槽位元数据，不再从具体战术枚举分支生成当前运行目标。旧枚举接口仍保留为兼容测试/外部调用面，不能据此宣称所有战术已完全资产化。
- 通过规则通道调整决策与拦截默认参数：`dwell_base` 从 `0.72` 调至 `0.82`；拦截 steal/tip slope 与上下限收窄。该调整意图是降低此前明显偏高的失误产量。

### 10.2 针对性验证证据

- `cargo test -p nba-engine --test axiom_fuzzing`：8 个长时种子测试通过。
- `cargo test -p nba-engine --test constraint_system test_missed_final_free_throw_starts_rebound_from_last_ball_position`：1 个通过；强制罚球路径现在与正常罚球一样清除旧持球并先写入 `Dead` 球状态，最后一次罚球不中后 `RimRebound.from_pos/from_z` 与 `ball_pos_3d` 一致。
- `cargo test -p nba-engine --test constraint_system`：58 个通过；`cargo test -p nba-engine --test golden_hash`：4 个通过。由于本轮修复改变了确定性轨迹，黄金锚点按 `docs/design.md §2.3` 重新冻结为 `0x59b96ffa5ab114bb`，并在测试历史中记录原因。
- `cargo test -p nba-domain --test ball_ownership`：2 个通过；`cargo test -p nba-physics --test backend_contract`：17 个通过；`cargo test -p nba-evaluator --test evaluator`：13 个通过；`cargo test -p nba-invariants --test checker`：3 个通过；决策战术单测：1 个通过。
- release CLI 重放 `seed=0 1q`：24,543 ticks，77 summaries，0 Axiom Violations；逐帧复核的最大持球球距为 `1.4844 ft`，低于 `holder_leash_ft=3.0 ft`；`CONTROL_TRANSFER` 帧不再输出 `holderId`。
- release CLI 重放历史失败种子 `seed=49 1q` 与 `seed=54 1q`：两场均退出码 0，分别生成 23,728 与 23,192 ticks，未生成 violations 工件；这只证明历史复现路径在 `1q` 范围内修复，不替代全矩阵证据。
- 随后以 release CLI 串行运行 `seed=0..99 1q`，每场输出及评判工件在下一 seed 前删除：100/100 退出码 0，100/100 完成；tick 范围 `22,073–25,510`，总计 `2,375,022`；回合范围 `65–92`，均值 `79.55`；逐场 `0` Hard Axiom Violations。
- 该矩阵共产生 `46,720` 条评判记录、`1,337` 条 evaluator defect；realism index 范围 `0.955–0.991`，均值 `0.97851`。这些 L2 defects 仍需按准则处理，不能用 realism index 抵消。

### 10.3 当前未通过的验收门

- `TURNOVER_ATTRIBUTION`、`RHYTHM_DURATION`、`PASS_CORRIDOR_REACHABLE`、`TURNOVER_RATE` 等 L2 缺陷在矩阵中仍存在；全矩阵零 Hard Axiom 只证明 L1 物理/状态检测门，不等于因果与真实度门全部通过。
- `UNATTRIBUTED_END` 仍可观测：seed=0 `1q` 中为 1 条；违例回合的 `turnover_player_id` 仍有缺失，需继续补事件窗口归因。
- `docs/tactics.md` 要求的完整能力约束 slot fill、matchup 和 rotation 管线未由本轮事件流证明；profile 接线只覆盖当前运行目标生成的可达路径。
- 本轮只验证 NBA `1q`；FIBA 档案、整场生命周期、性能与 zero-allocation 条款不由本矩阵推出。

### 10.4 清理状态

- 每场 `matrix_seed_*.ndjson` 及其评判/违规工件均已在进入下一 seed 前删除；`/dev/shm/matrix.results` 与 `/dev/shm/matrix.log` 仅为本轮汇总证据，不属于仓库交付物。
- 仓库中的 `docs/problem.md` 保留实际问题和本节证据；未新增批量事件流或临时统计文件。

## 11. 续审后的回归发现（当前工作区）

### 11.1 全场统计测试与 `full` scope 语义不一致

- 重新执行 `cargo test -p nba-engine --test stats_baseline full_game_stats_within_baseline_band -- --nocapture`，测试失败；实际输出为 `total_p50=279.5`、`avg_poss=331.5`、`avg_dur=11.67s`、`3P%_median=68.8`，失败断言为 `avg possessions 331.5 outside stage gate G4 [172,240]`。
- 失败不是暂时性超时：8 个种子均完成全场循环，但 `full` scope 当前仍按 `estimated_possessions_per_period=35 × 4` 推进，实际完成回合约为 `315–388`。因此统计测试把“每节估计回合预算”当成了“整场真实回合数”，而门控范围 `[172,240]` 与当前执行语义不一致。
- 当前工作区相较此前修复还将 `current_possession_start_time` 用于摘要时长、将传球/交接目标冻结，并修正了 Drive leash；这些修改不能解释该统计测试失败，因为失败的回合数已经由 `full` 生命周期/回合边界计数暴露。
- 这条失败必须在验收前解决：要么让 `full` scope 以真实比赛时钟/终场状态结束并让全场回合数落入测试门，要么按校准协议重新定义并记录统计门；不能仅把断言注释掉或将失败标成通过。

### 11.2 当前可复核的证据边界

- 已观察到的历史控制运行同样打印 `avg_poss=331.5` 并在相同断言处失败；因此不能把该失败归咎于单个新物理修复，也不能声称当前基线测试通过。
- 本轮回归没有重新声称 `cargo test --workspace` 全绿；受失败统计门阻断，完整验证应在修复 `full` scope/统计契约后重新执行。

### 11.3 清理

- 本次回归仅使用 `/dev/shm` 临时 worktree/输出；验证结束后已移除临时 worktree。仓库未新增事件流或评判工件。

## 12. 当前工作区后续复核（2026-09-07）

### 12.1 已验证的修复路径

- `cargo test -p nba-engine --test constraint_system`：60 个测试通过；其中包含 full scope 生命周期、冻结 release target 的 `PASS_RECEIVED` 坐标、传球到达复放 release outcome 三条针对性回归。
- 交叉 crate 回归通过：`nba-evaluator` evaluator 8 个、`nba-physics` backend contract 13 个、`nba-domain` ball ownership 2 个、`nba-invariants` checker 17 个；engine lifecycle regression 与 possession narrative 各 1 个通过。
- release CLI `seed=0 1q`：24,676 ticks、79 summaries、0 Axiom Violations；`seed=42 1q`：23,651 ticks、59 summaries、0 Axiom Violations。当前过程仍明确区分 L1 不变量结果与 L2 评判结果。
- 规则通道中的当前行为调整实际已生效：`DecisionRules.dwell_base=0.82`、`TacticalRules.action_duration_seconds=9.5`、战术起始/决策间隔为 `6.5s/2.4s`，以及拦截斜率/上下限收窄；这些参数均位于 `GameRules`/`ResolveConfig`，没有新增运行时内联概率常数。

### 12.2 尚未通过的验证

- `cargo test -p nba-engine --test stats_baseline full_game_stats_within_baseline_band -- --nocapture` 仍失败：8 seed 聚合为 `total_p50=223.5`、`avg_poss=273.8`、`avg_dur=13.77s`、`3P%_median=73.3`，失败为回合数 G4 `[172,240]` 上界；这不是通过修改断言解决的问题，说明决策/终端边界仍需按归因账本继续校准。
- 当前 1q smoke 的得分、回合数与零 L1 违反不能外推为全场 sanity net 或设计文档完全实现；全场统计门、三分命中率目标带、逐回合 L2 缺陷收缩、FIBA 全场及性能条款仍需独立证据。
- 由于本轮只做了 focused regressions 与少量 CLI smoke，没有重新声称 seed=0..99 全矩阵通过；此前历史矩阵证据与本节当前工作区证据分开保存，不能混写。

### 12.3 临时数据清理

- 已删除本轮 `/dev/shm` 临时事件流、`/tmp/nba_batch_*.ndjson` 及历史临时验证目录；仓库未新增事件流工件。

### 12.4 续修归因事件

- 违例终端现在在同一回合窗口写入 `RuleViolation` 因果事实，评判器同时识别事件日志与帧事件中的 `VIOLATION`/`RULE_VIOLATION`/`ENFORCEMENT_APPLIED`；相关 `nba-evaluator` 8 个测试仍全绿，engine targeted scope/pass tests 仍全绿。
- 该修复只补齐可观察归因，不宣称失误率、三分命中率或全场 sanity 门已经通过；仍须以完整矩阵重新量化缺陷收缩。

## 13. 当前代码版本的 seed=0..99 串行复核（2026-09-07）

### 13.1 执行口径

- 使用刚编译的 `target/release/nba-sim`，严格按 seed 递增顺序运行每个 `1q`；每个 seed 只保留当前 `/dev/shm/nba_current_matrix.ndjson` 及其评判工件，解析后立即删除再进入下一个 seed。
- 100/100 进程返回码为 0；累计 `2,350,976` ticks、`6,779` 个 `PossessionSummary`；逐场 ticks 范围 `21,447–24,943`，逐场 summary 范围 `53–83`。
- L1 Axiom Violations：100 场均为 0；本轮没有发现历史 seed=49/54 的 Hard 失败。该结论只适用于当前 `1q` 矩阵，不外推 full/FIBA。

### 13.2 逐帧与逐回合汇总

- 终端摘要：`SCORE=2,337`、`DEFENSIVE_REBOUND=948`、`TURNOVER_VIOLATION=1,670`、`TURNOVER_PASS_TIPPED=940`、`TURNOVER_STEAL=793`、`UNATTRIBUTED_END=91`。`UNATTRIBUTED_END` 出现在 62/100 场，说明回合零遗漏仍未满足审计门。
- 事件总数：`PASS=8,995`、`PASS_RECEIVED=6,624`、`PASS_TIPPED=1,052`、`PASS_DROPPED=676`；传球成功/失败与点掉链路已被记录，但仍有 summary 归因窗口需要继续收敛。
- 逐场平均失误摘要率约 `0.4993`，范围 `0.3276–0.6290`；99/100 场触发 `TURNOVER_RATE`，因此决策/裁决概率仍明显偏离 fixture 参考带 `[0.05,0.35]`。
- 各场摘要平均时长（按场平均后再跨 seed 平均）约 `13.79s`；`RHYTHM_DURATION` 共 `559` 条 Soft defect，出现在 99/100 场。矩阵 runner 未保留所有流，因此不把未保存的全矩阵 min/max 冒充为测量结果；seed=0 探针实际观察到 `0.03998–33.18176s`，其中存在亚 tick 级短回合。

### 13.3 当前 L2 评判缺陷账本

- 100 场共 `39,172` 条 judgments、`724` 条 defects；realism index 范围 `0.9867–0.9994`，均值 `0.99410`。指数只能描述评判结果聚合，不能抵消 Hard/因果/统计门。
- 缺陷按准则：`RHYTHM_DURATION=559`、`TURNOVER_RATE=99`、`TURNOVER_ATTRIBUTION=52`、`STEAL_CORRIDOR=6`、`PASS_CORRIDOR_REACHABLE=5`、`POSSESSION_DURATION_BOUNDS=2`、`SHOT_QUALITY_CONTEST=1`。
- 与此前基线相比，`PASS_TIPPED` 已被评判器识别为合法失误原因，但当前矩阵仍有 52 条 `TURNOVER_ATTRIBUTION`；这表明剩余缺陷来自事件窗口/终端语义未完全对齐，不能将全部归因于 evaluator 漏识别。

### 13.4 设计符合性结论

- 当前代码版本已达到：1q L1 物理/状态零 Hard、历史两条物理回归路径不再复现、传球 release target 冻结与到达复放回归通过、违例终端补写可观察因果事实。
- 当前代码版本仍未达到：回合零遗漏（`UNATTRIBUTED_END=91`）、失误率参考带、节奏缺陷零化、走廊/抢断走廊全通过、全场 sanity gate 与 3P% 终态目标带。因此不能声称“完全实现 docs 设计文档”。

### 13.5 清理与证据边界

- 串行 runner 的临时流已按 seed 清理；保留的 `/dev/shm/nba_current_matrix_results.json` 是本轮聚合证据，不是运行时输入。单 seed 探针流已在本节写入证据后删除；仓库未新增事件流工件。

## 14. 验证流程运行事故（2026-09-10）

### 14.1 事故事实

- 在未设置构建目录上限、未在长测试批次之间清理的情况下，连续执行了多次 `cargo test`，触发了 workspace 全量编译与大量测试构建缓存累积。
- `/home/ubuntu/workspace/code/nba-sim/target` 最大约 **9.1 GiB**；根分区 `/dev/vda2` 达到 **100%**，剩余空间约 **491 MiB**。
- 仓库内同时存在约 `172 MiB` 的 `game.ticks.ndjson`、约 `128 MiB` 的 `output/game.ticks.ndjson`，但本次磁盘耗尽的主因是 Rust `target/` 构建产物，而不是单个事件流文件。
- 事故期间曾启动一次定向测试构建；发现磁盘压力后立即停止继续验证，未继续运行全量测试。

### 14.2 恢复动作

- 执行 `cargo clean`，删除约 **9.1 GiB** 构建产物。
- 分区从 100% 恢复到约 84–85%，可用空间恢复到约 **9 GiB**。
- 清理后只做了必要的 `cargo check` / 定向检查；不再声称全量回归在本次事故后重新通过。

### 14.3 根因与流程缺陷

- 将 `cargo test` 当作默认验证命令，未区分 `cargo check`、单 crate 测试、单测试目标和 workspace 全量测试的资源成本。
- 没有在长测试前检查 `df -h`，也没有在构建前后记录 `du -sh target`。
- 没有使用独立、可清理的 `CARGO_TARGET_DIR`，没有对生成的大型 NDJSON 输出设置生命周期和大小边界。
- 这是验证流程和资源治理缺陷，不应归咎于模拟器业务逻辑；但它确实造成了环境级可用性风险，必须纳入项目问题账本。

### 14.4 后续验证约束

- 默认只运行 `cargo check -p <相关 crate>`；测试必须说明目标、范围和预计资源占用。
- 禁止未经确认直接运行 `cargo test`（workspace 全量）或长时间 full/matrix 模拟。
- 每次构建/测试前后检查磁盘空间；当可用空间低于预设安全线时立即停止，不进入下一轮编译。
- 长测试使用临时 `CARGO_TARGET_DIR`，结束后删除；事件流写入 `/dev/shm` 或临时目录，并在单个 seed/场次结束后删除。
- 验证报告必须记录实际执行的命令、是否中途终止、构建目录大小变化和清理动作；未执行的测试不得标记为通过。
