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
| 并行动作系统 | 已完成 | 全局单动作 `World::active` 已彻底移除：`World::advance_parallel_actions` 逐人推进每名球员自己的 `ActionRuntime`，多人配合由 `World::interactions` 按交互编号结算，`DefensiveActionRuntime` 另有 `CloseoutContest`、`StealAttempt` 与 `BlockJump` 三种防守动作。持球人、无球进攻球员与防守人各自持有调度与候选集（`policy.rs::tick`、`policy.rs::off_ball_tick`、`world.rs::advance_steal_attempts`）。并发性由 `parallel_actions` 4 项直接验证：同一 tick 内多名球员的运行时并存且种类不同、出手期间防守动作并行、每个 `ActionStarted` 恰有一个终态、死球终态不起手。`action_chain` 5 项、`assignment` 2 项、`continuity` 2 项、`fake_protection` 7 项、`defensive_technique` 8 项、`dribble_technique` 4 项、`shot_model_invariants` 4 项与 `allocation_budget` 1 项同时通过。 |
| 决策、战术与能力 | 已完成动作目录覆盖 | 16 个动作族的目录场景全部有测试映射，`missing_evidence` 为空。技术细分已接入候选、执行与后果：运球 7 种技术（变向、胯下、背后、转身、后撤步、迟疑步、原地推进）按防守间隔与时钟选择并改变球的暴露程度；后撤步跳投在出手前把身体撤开；欧洲步在被堵的收球路线上向侧面绕步并放宽收球容限；背身步法按防守人站定／靠底线／贴身分别走上下步、撤步转身与背身挤打；击地传球真实触地一次后仍被接住；假动作与护球的轴心脚不动；包夹提高伸手成功率、压低出球成功率，护球把两者收回一部分；收球后贴身持球五秒判违例。 |
| 事件、回放与完整验证 | 进行中 | 已增加事件派生球员与球队统计账本、稳定事件序号、带历史缺口检测的游标接口、SessionService 内存检查点恢复与磁盘检查点（写入种子、tick 与计数指纹，恢复时重放并校验，分叉就地报错），以及 NBA/FIBA 已选规则场景。完整比赛入口测试比较 CLI 与 WASM 的整场摘要和全部投影事件（seed 211）。`rule_matrix` 10 项、`action_scenarios` 26 项、`jump_ball` 5 项、`checkpoint_disk` 3 项、`event_ledger` 5 项通过。指纹工件已在确认行为变化后重新登记。 |
| 校准与文档验收 | 进行中 | 当前结构和行为改动后的完整校准尚未运行；2026-10-21 旧批次数值仅保留为历史参考。已按完整模拟方案复核当前缺项与验证证据，并更新状态文档的适用日期。校准门禁和最终文档交叉核验仍待完成。 |

## 本轮新增验证证据

本轮改动均在工作区完整测试套件下验证：`bash .work/run_parallel.sh`（`cargo test --workspace --all-targets`，70 个测试二进制，4 进程并行）全绿，单次墙钟 29–30 秒（外部负载升高时 60 秒以上）；每进程 2 GiB 地址空间上限。

- `crates/sim/tests/parallel_actions.rs` 4 项：多人动作运行时同一 tick 并存且种类不同、出手期间防守动作并行、每个 `ActionStarted` 恰有一个终态、死球终态不起手。
- `crates/sim/tests/action_scenarios.rs` 29 项：防守篮板归属、空切传球落点、移动掩护判罚与站定掩护不判罚、掩护后的顺下与外弹、无球跑位改变位置并保留接球窗口、协防责任被指派并收回、被假动作骗起后干扰下降、持球人进内线后的协防、收球步容限、突破后收球分球、带球撞人造犯规需站定、贴身跳投被盖为手部接触并留下松球、干扰降低命中率、滑步缩小距离、拒绝接球降低完成率、背身挤打与背身步法、后撤步拉开空间、击地传球、被点到的传球进入松球争抢、紧逼五秒与其计时重置、防守篮板后的转换推进、抢断后球权转移与快速起手、退防目标回到本方半场、包夹与护球出球、跳球、欧洲步、切入路线、挤过与绕过掩护、回位。
- `crates/sim/tests/pass_technique.rs` 2 项：八种传球技术的完成率差异、真实比赛使用多种技术。
- `crates/sim/tests/rule_violations.rs` 2 项：后场八秒违例与出界归属（最后触球方交出球权）。
- `crates/sim/tests/checkpoint_disk.rs` 3 项：检查点写入磁盘后重放一致、指纹不符就地报错、分叉就地报错。
- `crates/validation/tests/rule_matrix.rs` 10 项：开球程序与交替拥有、球队犯规罚球阈值、末段窗口边界、无球犯规罚则、犯规计数器分类、配置变更生效、前场篮板时钟重置的触筐条件、加时与暂停配额、缩短档案不改判罚阈值。
- `crates/domain/tests/rule_catalog.rs` 2 项：程序与动作目录的场景编号、证据测试映射、缺少证据记录检查，当前 16 个动作族的 `missing_evidence` 均为空。
- `python3 scripts/check_docs.py` 与 `git diff --check` 通过；冻结的三张表哈希未变（`scripts/calibrate/freeze.py --check`）。

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
6. 全局单动作已移除，持球人、无球进攻球员与防守人各自持有调度与动作运行时；尚未完成的是掩护发起方由持球人改为掩护人，以及卡位作为独立动作意图。

## 下一项工作

动作目录的 16 个族均已具备生产路径与场景测试；后续工作集中在完整方案 §12 G 的校准与文档验收：在规则、程序与动作验收已完成的当前工作区上运行机制实验与独立验证，重跑受限拟合并更新冻结锁与认证工件，然后把实测分布与验收结论写回文档。NBA 来源登记为 2025-26 官方规则 PDF；FIBA 使用 2026 v1.1 官方规则 PDF，生效日期为 2026-10-01。
