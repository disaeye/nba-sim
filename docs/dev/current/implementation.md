# 当前实施进度

> 唯一权威实施范围、依赖和验收条件见 [`full_simulation_plan.md`](full_simulation_plan.md)。本文件只记录进度、已完成验证和下一项工作，不另立方案。

## 当前目标

按 `full_simulation_plan.md` 的顺序完成 NBA 与 FIBA 完整比赛模拟内核。当前继续推进比赛程序、并行动作、规则裁决、独立验证与完整资源验收；已通过的项目只说明各自测试范围。

## 阶段进度

| 阶段 | 状态 | 当前记录 |
| --- | --- | --- |
| 计划文档整理 | 已完成 | `full_simulation_plan.md` 是唯一权威方案；`plan.md` 是入口，本文件仅记录进度。 |
| 规则与能力目录 | 进行中 | 建立 NBA 2025-26、FIBA 2026 v1.1 来源记录、程序与动作目录及结构测试；规则参数差异和官方 PDF 核验边界记录于 `docs/dev/evidence/rule_catalog_sources.md`。完整条款映射和动作路径审查仍未完成。 |
| 权威状态与计时 | 已完成基础阶段 | 独立 `Clock` 与 `Phase::clock_advance` 已接入；PassFlight 两钟递减、FreeThrowFlight 停钟并有直接测试。世界生命周期和状态写入权仍分散，不能视为满足唯一权威写入路径。 |
| 比赛程序与规则裁决 | 进行中 | 开场跳球、NBA 常规节交替发球与加时跳球、FIBA 后续节次交替拥有发球具备直接模拟测试；完整接触规则、非法触球/争球重跳、出界位置、裁决计划与其余程序仍待实现。 |
| 并行动作系统 | 部分实现 | 全局单动作 `World::active` 已移除：每名球员持有自己的 `ActionRuntime`，多人配合由 `World::interactions` 按交互编号结算，`DefensiveActionRuntime` 另有 `CloseoutContest`、`StealAttempt` 与 `BlockJump` 三种防守动作（起跳封盖在执行窗口内抬高摸高，起跳的人不参与卡位）。持球人、无球进攻球员与防守人各自持有调度：无球球员用自己的候选集与逆温度选择跑位或空切（`policy.rs::off_ball_tick`），防守人自己决定是否伸手切球（`world.rs::advance_steal_attempts`）。掩护仍由持球人发起；迟疑步与轴心脚步未建模。`action_chain` 5 项、`assignment` 2 项、`continuity` 2 项、`fake_protection` 7 项、`defensive_technique` 6 项、`dribble_technique` 4 项、`shot_model_invariants` 4 项和 `allocation_budget` 1 项定向测试通过。 |
| 决策、战术与能力 | 部分接入 | 假动作、护球、运球技术细分、无球跑位与空切已接入候选、执行与后果消费者（干扰、抄截半径、切球成功率、球的暴露程度）；动作目录中其余技术细分（轴心脚、包夹护球、无球掩护、卡位、终结步法）尚无可选候选。 |
| 事件、回放与完整验证 | 进行中 | 已增加事件派生球员与球队统计账本、稳定事件序号、带历史缺口检测的游标接口、SessionService 内存检查点恢复及 NBA/FIBA 已选规则场景。完整比赛入口测试比较 CLI 与 WASM 的整场摘要和全部投影事件（seed 211）。`rule_matrix` 扩到 10 项：开球程序与交替拥有、球队犯规罚球阈值（阈值前/正好/之后、双方主体、双联赛）、末段窗口边界、无球犯规罚则、犯规计数器分类、配置变更生效、前场篮板时钟重置的触筐条件、加时与暂停配额、缩短档案不改变判罚阈值。引擎是否按档案执行仍由 `rules_flow` 等定向测试负责。检查点磁盘持久化与完整动作族覆盖仍未完成。指纹工件已在确认行为变化后重新登记；`five_dimension` 3/3 通过（战术先验份额按持球决策为分母，候选集扩张后带宽下界记录为 0.04）。 |
| 校准与文档验收 | 进行中 | 当前结构和行为改动后的完整校准尚未运行；2026-10-21 旧批次数值仅保留为历史参考。已按完整模拟方案复核当前缺项与验证证据，并更新状态文档的适用日期。校准门禁和最终文档交叉核验仍待完成。 |

## 本轮新增验证证据

本轮已运行并通过以下检查：

- `cargo check --workspace`：通过；最大常驻内存约 277 MiB。
- `cargo test --workspace --no-run`：全部测试目标编译通过；最大常驻内存约 370 MiB，单 Cargo 作业，命令设置 2 GiB 虚拟内存上限。
- `cargo test -p nba-sim-core --test allocation_budget -- --test-threads=1`：零分配稳态测试通过；最大常驻内存约 190 MiB。
- `cargo test -p nba-sim-core --test frame_stream --test replay --test allocation_budget -- --test-threads=1`：5 项通过，包含帧流因果 ID、确定性回放和零分配稳态；最大常驻内存约 181 MiB。
- `cargo test -p nba-sim-core --test event_sequence --test interface_determinism -- --test-threads=1`：4 项通过，覆盖受限历史事件顺序、检查点续跑、重复运行和完整比赛报告比较；最大常驻内存约 178 MiB。
- `cargo test -p nba-validation --test event_ledger --test rule_matrix -- --test-threads=1`：9 项通过，覆盖 NBA/FIBA 规则值、节次犯规门限、攻守篮板时钟策略、事件统计重建及 NBA/FIBA 生产比赛对账；最大常驻内存约 31 MiB。
- `cargo test -p nba-wasm --lib -- --test-threads=1`：2 项通过，包含 WASM 入口与 `SessionService` 逐帧、增量事件 JSON 一致；最大常驻内存约 371 MiB。
- `cargo test -p nba-sim-cli --test interface_parity -- --test-threads=1`：1 项通过，seed 211 的完整比赛摘要与全部投影事件一致；事件 JSON 浮点值按原始 `f32` 精度范围比对。
- 本轮独立防守动作回归：`ulimit -v 2097152; NBA_TEST_MEMORY_MAX=1G CARGO_BUILD_JOBS=1 cargo test -p nba-sim-core --test action_chain --test assignment --test continuity --test shot_model_invariants --test allocation_budget -- --test-threads=1`，5 个测试目标共 14 项通过：`action_chain` 5/5、`assignment` 2/2、`continuity` 2/2、`shot_model_invariants` 4/4、`allocation_budget` 1/1。覆盖防守动作与投篮并行、实际移动和干扰几何、传球/突破动作路径、阵容职责、连续性、投篮模型与稳态零分配。
- `ulimit -v 2097152; NBA_TEST_MEMORY_MAX=1G CARGO_BUILD_JOBS=1 cargo test -p nba-sim-core --test engine --test rules_flow -- --test-threads=1`：27 项通过，包含 17 项完整引擎测试与 10 项规则流程测试；平均回合数检查在这次运行通过。
- `cargo test -p nba-sim-core --test policy_utility a_closer_defender_lowers_the_shot_epv -- --exact --test-threads=1`：1 项通过，验证更近防守位置降低投篮预期得分。
- `cargo test -p nba-domain --test rule_catalog -- --test-threads=1`：目录场景编号、证据测试映射和缺少证据记录检查通过（2 项）。
- `cargo check --workspace --all-targets`：全部目标通过；新增防守动作事实投影、工作区测试目标与罚球阶段初始化均可编译。
- `ulimit -v 2097152; NBA_TEST_MEMORY_MAX=1G CARGO_BUILD_JOBS=1 cargo test --workspace --no-run -- --test-threads=1`：全部工作区测试目标成功编译；最大内存受 2 GiB 虚拟内存限制，单 Cargo 作业。
- `python3 scripts/calibrate/freeze.py --check`：冻结的 `shot_make`、`rebound`、`freethrow` 三张表哈希未改变。
- `python3 scripts/check_docs.py`：文档守卫通过，扫描 39 个文档、201 处引用。
- `git diff --check`：通过。

相关验证命令设置 2 GiB 虚拟内存上限、单 Cargo 作业和单测试线程。最近一次完整工作区运行早于 CloseoutContest 修改，仅 `baseline_fingerprint_matches_recorded_artifact` 失败；目前没有在 CloseoutContest 修改后重跑完整工作区行为测试。先前单独执行的 `engine::game_shape_is_within_the_professional_range` 曾得到 233.8 个回合的失败值，但后续 `engine` 全套测试 17/17 通过，故该异常未能复现。`five_dimension` 调整后的定向测试 3/3 通过。完整校准未运行：规则、程序和动作实现尚未满足完整方案的校准前置验收条件；现有 `fit_policy.py` 会直接修改 `data/calibration/policy.json`，暂不运行。

## 既有验证证据

先前相关验证通过：

- `cargo check --workspace`：工作区编译检查通过。
- `cargo test --workspace --no-run`：全部测试目标编译通过；最大常驻内存约 488 MiB，构建任务为单作业。
- `cargo test -p nba-sim-core --test jump_ball --test runtime_tables --test rules_flow -- --test-threads=1`：19 项通过，覆盖 NBA 开场与加时跳球、NBA 第二和第三节交替发球、FIBA 第二和第三节交替拥有发球箭头推进、PassFlight 两钟、罚球停钟、末罚篮板、暂停恢复及 100 场阵容连续性。最大常驻内存约 362 MiB，单作业构建。
- `cargo test -p nba-sim-core --test ball_physics -- --test-threads=1`：5 项通过，球速上限回归通过；扫掠手部接触拦截回归断言防守人身份和接触事件。
- `cargo test -p nba-sim-core --test finishing -- --test-threads=1`：6 项通过，包含常规第一、第二节换向后的双队三分线分类。
- `cargo test -p nba-sim-core --test finishing shooter_zone_matches_offensive_half_after_period_direction_switch -- --exact --test-threads=1`：通过；`shooter_zone` 在常规第一、第二节按实际进攻方向识别双方出手分区。
- `cargo test -p nba-domain --test rule_catalog --test game_flow_transitions --test rules_geometry -- --test-threads=1`：18 项通过，包含 NBA/FIBA 节次跳球策略映射及动作场景测试证据完整性校验。

以上仅为相关回归，不代表完整工作区行为测试通过。

规则与动作目录新增后，以下领域测试通过：

- `cargo test -p nba-domain --test rule_catalog -- --test-threads=1`：2 项通过；程序目录逐条关联已通过场景与测试函数身份，动作目录逐条映射已通过场景、缺少场景与测试函数身份。
- `cargo test -p nba-domain --test game_flow_transitions --test rules_geometry -- --test-threads=1`：16 项通过。
- 新测试单项最大常驻内存约 43 MiB，未使用交换空间。

目录测试验证 JSON 结构、NBA/FIBA 版本来源身份、程序与动作场景编号唯一性，并要求动作目录中的每个场景都有通过测试映射或明确的缺少证据记录；它不代表目录未覆盖规则实现已通过。NBA 官方规则 PDF 的直接全文请求未成功，尚未逐条核对完整 PDF；FIBA 规则已读取官方 2026 v1.1 PDF 与 2026 变更资料。详情见 `docs/dev/evidence/rule_catalog_sources.md`。

## 校准记录适用范围

完整校准验收尚未运行，因为规则程序和动作实现仍有未闭合项，现有证据不能证明方案 §12 G 的机制与规则前置验收已满足。本节数字来自 2026-10-21 历史批次。`docs/dev/status.md` 的总体校准结果早于本轮内核修改。`shooting_fouls_per_game` 16.531、目标下限 19.597，以及 40 种子机制实验中五项指标的 `insufficient` 结论，仅适用于当时的代码和参数，不能代表当前工作区。现阶段不运行会改写 `data/calibration/policy.json` 的拟合程序；完成规则和动作验收后，再按完整方案执行校准、机制实验、独立验证与资源测量。

## 已确认的待修复项

1. `JumpBall` 当前由参赛球员身高、起跳能力和确定性抽样决定拍球归属，随后由几何条件让接应人收球；跳球员实际伸手触球、对手点拍与触球合法性尚未接入身体接触证据。
2. 跳球球体目前以开环抛物轨迹运行，尚未处理双方提前触球、同队连续触球、无人控制及规则要求的重复跳球。
3. NBA 加时使用跳球、NBA 后续常规节与 FIBA 后续节次使用交替拥有发球的入口已按档案分流；FIBA 开场跳球后箭头、第二与第三节发球后箭头及 NBA 第二与第三节开局分配已有直接回归。重跳场景的箭头变化仍待验证。
4. `begin_inbound` 仍统一设置在底线中点，发球授权、计时、地点类别、发球违例和接球后的控制建立还不完整。
5. 生命周期、罚则、犯规和违例处理尚未统一成方案中的裁决计划及提交入口。
6. 投篮分区生产入口现按节次进攻方向计算；轮转方向回归已经覆盖半场换向。全局单动作已移除，持球人、无球进攻球员与防守人各自持有调度与动作运行时；尚未完成的是战术层面的技术细分（迟疑步、轴心脚步、包夹护球、无球掩护、卡位）与掩护发起方由持球人改为掩护人。

## 下一项工作

继续按 `full_simulation_plan.md` 的阶段依赖实现动作运行时、动作后果链及 NBA/FIBA 未完成程序；更新规则场景矩阵与行为字段测试。完成相关行为验收后，再运行独立的全量校准与资源验收，并根据审查通过的预期行为更新基线指纹。NBA 来源登记为 2025-26 官方规则 PDF；FIBA 使用 2026 v1.1 官方规则 PDF，生效日期为 2026-10-01。
