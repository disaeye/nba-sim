# 当前实施进度

> 唯一权威实施范围、依赖和验收条件见 [`full_simulation_plan.md`](full_simulation_plan.md)。本文件记录当前工作区的实施状态与验证证据，判定数字以 `docs/dev/evidence/` 下的工件为准。阶段与证据的逐项对应见 [`../evidence/stage_gate_map.md`](../evidence/stage_gate_map.md)。

## 当前目标

NBA 与 FIBA 完整比赛内核对应 A–H 工作包；比赛流程真实性对应 I–L 工作包。

## 阶段进度

| 阶段 | 状态 | 当前记录 |
| --- | --- | --- |
| A. 规则与覆盖目录 | 已完成 | 22 项规则程序按版本、条款、场景矩阵、测试身份与生产消费者审计；16 个动作族与 111 个动作条目按 `ActionSpecTable` 逐项接入。`data/rules/rule_catalog.json` 的 `known_current_gaps` 为空，动作目录每条证据指向真实测试。 |
| B. 权威状态与联赛入口 | 已完成 | 时钟以整数微秒为唯一单位，`ClockDirective` 有 `Start`、`Stop`、`Set`、`Restore` 四种形态，`ClockController` 是唯一改钟入口。七种球态由迁移表约束，`ControlState` 是控制权唯一权威，事件信封带程序身份、控制序列身份与来源事件。11 个策略族、完整 `MatchSetup`、磁盘检查点与回放清单从 CLI、WASM、debug-server 和会话检查点恢复同一场比赛。 |
| C. 规则程序与特殊事件 | 已完成 | 22 项规则程序都有生产消费路径与场景矩阵。原始事实由 `World::step` 主循环检出，裁决层按联赛档案生成 `AdjudicationBatch` 与 `PenaltyQueue`，由统一提交入口原子写入。特殊犯规、技术犯规、伤病、球队技术犯规与转换阻断由完整比赛自发产生；抽样式接触事实只在活球阶段成立。 |
| D. 复核与节末结算 | 已完成 | 11 个 `ReviewScope` 与联赛档案双向映射，复核按来源事件身份抽样观察误差。教练挑战支持实质改判，节末结算按出手瞬间比赛钟裁定进球；`PeriodSettlementCompleted` 发布后比分、球权、动作与计时器冻结。 |
| E. 并行动作系统 | 已完成 | `World::advance_parallel_actions` 逐人推进每名球员自己的 `ActionRuntime`，多人配合由 `World::interactions` 按交互编号结算；防守动作有 `CloseoutContest`、`StealAttempt`、`BlockJump` 三种运行时。能力、倾向、战术与实时几何共同驱动候选、执行与后果。 |
| F. 事件、回放与协议投影 | 已完成 | 事件派生球员与球队账本，独立验证器只读事件流重建时钟与控制序列。`StreamTick` 带比赛阶段、程序、球态、计时器运行标志、三个版本号、活动动作与程序资格。`FrameRules`、`FrameEvent` 与 `RenderAction` 使用同一动作身份。 |
| G. 校准与验收 | 已完成 | 校准表、规则场景与独立账本可分别验证。 |
| H. 动作动画与资源观测 | 已完成 | 两类球场渲染器读取同一 `ActionSpecTable` 与动作配方，渲染映射、未知值负面对照和回放定位都有自动验证。七类资源场景的峰值常驻内存、运行时间、事件数、动作数、加时数与每 tick 分配写入 `docs/dev/evidence/performance.json`。 |
| I. 移除批量门禁并建立片段验证 | 已完成 | 默认短片段验证、快照直接恢复、预算与缺帧定位全部具备；CLI、CI、API、界面和工件守卫无固定批量门禁。 |
| J. 贯通半场挡拆、感知、协防与选择 | 已完成 | 半场片段独立重建掩护几何与窗口；感知、候选、重校验、执行和终态分别记录。成对实验覆盖同路线不同几何、看见/没看见、选择后成功/失败与换防/沉退/挤过。 |
| K. 验证整场连续行为 | 已完成 | 跨回合证据区分重复掩护得分与高难度命中；合法窗口改变防守方案并发布依据。转换、前场篮板续攻、程序恢复、末段和 NBA/FIBA 一致性由现有生产场景与账本覆盖。 |
| L. 复现实战过程研究并公布证据覆盖 | 已完成 | `scripts/research/process_study.py` 以独立逐事件归档与显式模拟样本做过程比较；三项时间指标给出样本量、聚类区间与差值，区域构成、传球、掩护、协防与底角窗口分项列为证据不足；研究不进入发布门禁。 |

## 验证证据

各阶段对应的测试身份、输入范围与代码路径集中在 `docs/dev/evidence/stage_gate_map.md`；本页不重复维护测试清单。行为与参数冻结后由 `scripts/final_validation.sh` 运行一次完整工作区测试，逐项退出码写入 `.work/evidence/final-validation.summary`，全量测试日志写入 `.pi/tasks/final-workspace-tests.log`。每个测试进程受 `.cargo/config.toml`、`scripts/mem-capped-runner.sh` 与 `scripts/run-tests.sh` 的 2 GiB 地址空间上限约束。

`scripts/final_validation.sh` 运行静态检查、资源检查、渲染回归和工作区测试。`scripts/check_final_artifacts.py` 核对规则目录与资源工件。

真实比赛过程研究由 `scripts/research/process_study.py` 显式运行，结果写入 `docs/dev/evidence/process_research.json`；它不参与 `final_validation.sh` 或发布门禁。

## 校准记录

`scripts/calibrate/fit_policy.py` 可以按职责估计 `policy.json` 的系数；`scripts/calibrate/fit_tables.py` 根据逐球和逐回合资料生成参考表。

`nba-sim validate-core --mechanism ATTRIBUTE` 对指定属性运行配对实验，并写出该属性的结果。

## 比赛流程验证

`validate-core` 默认检查开场控制和首个回合，可用 `--scenario` 选定片段。完整比赛需显式给出 `--games`。场景按步数限制起点搜索和推进，失败记录保留复现输入。调试服务器基于当前回放的已生成事实核对账本；查询验证结果时不重新模拟比赛。

种子 1 的两个场景分别在 tick 33 和 tick 177 结束。使用开发构建运行默认场景组，NBA 耗时 0.02 秒、峰值常驻内存 11,036 KiB；FIBA 耗时 0.01 秒、峰值常驻内存 12,632 KiB。`crates/validation/tests/fragment.rs` 验证快照直接恢复与连续推进产生相同帧和事件，并验证步数超限及缺帧定位。比赛流程真实性的完整交付条件见 `docs/dev/current/full_simulation_plan.md` §19。

## 验收证据入口

A-H 工作包按 `docs/dev/current/full_simulation_plan.md` 的最终验收条件完成；I、J 工作包的逐项证据见 [`../evidence/process_stages.md`](../evidence/process_stages.md)。当前状态由 `docs/dev/status.md`、阶段证据 `docs/dev/evidence/stage_gate_map.md` 与最终证据工件给出。
