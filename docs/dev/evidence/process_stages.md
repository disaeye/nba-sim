# I–L 比赛流程真实性证据

> 本文件记录 `full_simulation_plan.md` §16–§19 中 I–L 工作包的证据。每项只记录已执行的输入、中间事实和退出码；统计分布不作为真实性判定。

## I. 移除批量统计门禁并建立片段验证

- 状态：已完成。
- 入口：`validate-core` 默认运行有界短片段，完整比赛需显式给出 `--games`；CI、调试服务器和工件守卫没有固定批量门禁入口。
- 快照：`crates/validation/src/fragment.rs` 直接保存和恢复生产世界；`crates/validation/tests/fragment.rs` 4 项验证连续运行与快照恢复一致、缺帧定位和预算耗尽。
- 证据：NBA 与 FIBA 默认场景均低于 `0.03s`，峰值常驻内存约 `13 MiB`。

## J. 贯通半场挡拆、感知、协防与选择

- 状态：已完成。
- 机会重建：`crates/validation/src/halfcourt.rs` 只从公开帧重建掩护起点、计划落点、到达时间差、结算距离和接触/接近判据；`crates/validation/src/opportunities.rs` 独立重建出手窗口的建立与关闭。
- 感知、选择、执行分开：
  - 感知：决策记录带观察摘要、读到的球员、未读球员和记忆时效；`ScreenDecisionEvidence` 记录是否读到对位防守人和掩护搭档。
  - 选择：候选数量、所选候选、概率、估值分项和重校验结果来自同一决策记录。
  - 执行：动作身份、接触、位移、路线、使用状态与终态事件由独立帧几何复核。
- 生产片段：`crates/validation/tests/halfcourt_fragment.rs` 2 项验证可恢复片段、成功使用和选择后执行失败都能给出自己的观察、候选、几何与终态；失败片段必须指出接触或接近哪一项不成立。
- 同几何不同防守方式：`crates/sim/tests/action_scenarios.rs::defensive_scheme_changes_the_screen_route_on_identical_geometry` 用完全相同的站位、对位和种子，只改变守方方案档案，分别得到换防、沉退和挤过三种路线。
- 同路线不同几何与感知：`crates/validation/tests/paired_screen.rs` 的基准、推远防守人和隐藏防守人三臂来自同一快照。可见几何改变时真实到达时刻和持球人估值同步改变；隐藏防守人时真实几何改变而持球人的估值停在旧位置。
- 防守责任：`crates/sim/src/defense.rs` 的协防、回位和换防目标有生产消费路径，掩护方案由守方球队档案决定。
- 已验证命令：
  - `cargo test -p nba-validation --test halfcourt_fragment`：2 passed。
  - `cargo test -p nba-validation --test paired_screen`：1 passed。
  - `cargo test -p nba-validation`：全部通过，峰值常驻内存约 `394 MiB`。
  - `cargo test --workspace`：全部通过，峰值常驻内存约 `574 MiB`。
- 证据边界：选择频率没有统计显著性声明；本阶段只证明中间事实随几何、感知和防守方案发生可解释变化。

## K. 验证整场连续行为

- 状态：已完成。
- 跨回合调整：`crates/sim/src/coach.rs` 在回合边界从公开出手事实和掩护使用事实积累带时效的证据，区分反复借同一机制得分与偶然高难度命中；证据足够时在合法决策窗口改变守方方案，并发布 `CoachAdjustment` 事实，记录证据计数与目标方案。
- 成对证据：`crates/sim/tests/coach_adjustment.rs` 2 项。相同掩护机会与得分次数下，轻松得分改成换防方案，高难度命中保持原方案；没有掩护或不满足证据条件时不产生调整。
- 整场连续行为：转换与抢断推进、防守篮板推进、前场篮板二次进攻、停表恢复、罚球、暂停、节末和加时由既有生产场景与规则矩阵覆盖；`crates/validation` 独立账本、协议帧、事件序列和 NBA/FIBA 档案回归保持通过。
- 已验证命令：`cargo test -p nba-sim-core` 全部通过；`cargo test -p nba-validation` 全部通过；`cargo test --workspace` 全部通过，峰值常驻内存约 `574 MiB`。

## L. 形成真实比赛研究证据

- 状态：已完成。
- 研究入口：`scripts/research/process_study.py` 读取独立保留的逐事件归档与显式运行的模拟样本，按同一时间口径计算指标；研究结果不进入 `final_validation.sh`、CI 或发布门禁。
- 参考样本：2024-25 NBA 常规赛逐事件归档，1,230 场；归档 SHA-256 与来源登记在 `docs/dev/evidence/process_research.json`。
- 已有测量：防守篮板后首次出手、前场篮板后首次出手、抢断后首次出手的时间与本队出手占比；每项给出样本量、按比赛聚类的 95% 区间与模拟差值。
- 证据不足：前场篮板后出手区域构成因参考 shotdetail 与逐事件归档赛季不相交，保持 `InsufficientEvidence`；逐回合传球、掩护使用、协防顺序与底角窗口缺少追踪或人工标注，同样保持 `InsufficientEvidence`。
- 结果与复现：`docs/dev/evidence/process_research.md` 与 `docs/dev/evidence/process_research.json`。
