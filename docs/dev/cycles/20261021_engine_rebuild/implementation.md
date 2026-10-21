# 当前队列的实施入口

> 状态：当前活跃计划的一部分，只描述 [`plan.md`](plan.md) 中未完成项的类型、事实与测试入口。
> 当前门状态见 [`status.md`](../status.md)；内核设计与缺口清单见 [`action_chain_engine.md`](../action_chain_engine.md)。

## 过程级判据进 gate

对应 [`plan.md`](plan.md) 的队列项二。

类型：`gate::LayerResult` 的判据扩展与 `GameReport` 的观测字段。

事实：`GameReport` 已含 `shot_clock_at_release`、`contest_samples`、`shot_zone_counts`、`creation_counts`、`possessions[].ended_by`、`box_score`。缺失的是传球链、受助攻归属与逐 tick 位置轨迹的聚合。

测试入口：`crates/sim/tests/engine.rs` 的判定批次用例；`validate-core` 写出的 `batch_statistical_results.json` 与 `branch_coverage.json`。

## 决策层消费掩护路线与出手时钟门槛

对应 [`plan.md`](plan.md) 的队列项三。

类型：`policy::choose_action` 的候选收敛与门槛、`policy.json` 的新增系数。

事实：`DefenderView.route` 由 `World::decision_state` 回写；`shoot_score` 已算出 `expected_points`；`world.clock.shot_clock` 与 `params.shot_make` 提供门槛所需输入。路线以候选集收敛生效（`Under` 保留 `Shoot`/`Pass`/`Dribble`，`Over` 保留 `Drive`/`Finish`/`Pass`），仅在掩护后首个决策步作用；门槛比较的是按分区最高可得分的归一化分值，绝对分值会把抛投一刀切掉。`contest_for` 对钻过掩护的分离距离与转换期防守未落位分别折减干扰。

测试入口：`crates/sim/tests/screen_route.rs`；`crates/sim/tests/engine.rs`；`scripts/check_inline_constants.py`。

## 角色经倾向通道分化

对应 [`plan.md`](plan.md) 的队列项四。

类型：`PlayerTendencies` 的维度扩展、名册数据、`policy::choose_action` 的出手区域偏好、`policy::spot_target` 的站位伸缩。

事实：`PlayerTendencies` 现有十三个字段，九个进入生产路径；`offensive_role` 在 `crates/sim` 零读取（ADR-020）。投射倾向通过 `policy.json` 的 `shoot_three_preference` 与 `spacing_*` 两组系数生效，代码里没有角色分支。

测试入口：`scripts/check_r6_tendency_wiring.py`、`gate::evaluate` 的 `post_scorer_three_share` 与 `stationary_shooter_three_share` 两条判据。

## 协防引力场求解器

对应 [`plan.md`](plan.md) 的队列项五。

类型：`PotentialFieldRules` 的求解器、`PossessionState` 的对位数组、`DefenderView.route` 的写回。

事实：`PotentialFieldRules` 的 46 个参数已有 schema 与缺省值；`resolve/screen.rs` 已算出 `Over`、`Under`、`Switch` 并写入 `StateDelta.defender_route`；`DefenderView` 已有 `route: Option<DefenderRoute>` 字段。

测试入口：`crates/sim/tests/engine.rs` 的场景用例；`docs/tactics.md` §2.5 的双阈值滞回要求对应的稳定态断言。

## Play 选板接线

对应 [`plan.md`](plan.md) 的队列项六。

类型：`Params` 的档案加载、`PossessionState` 的激活与冷却状态、`policy::choose_action` 的候选剪枝与偏好加项、`Fact` 的激活事实。

事实：`PlaySpec` 的 schema、校验与错误类型在 `crates/domain/src/play.rs`；九份档案与七份体系档案由 `crates/sim/src/tactics.rs` 加载并校验槽位引用。`Fact` 新增 `PlayActivated` 与 `PlayWindowEnded`；`play_call_rate` 决定哪些回合叫战术，`play_called` 保证每回合只叫一次。

测试入口：`crates/sim/tests/tactics.rs`（槽位校验、抑制声明、硬抑制移除候选族、窗口内决策不选被抑制族）；`crates/domain/tests/play_spec.rs` 与 `playbook_data.rs` 保留 schema 与档案解析覆盖。

## 传球走廊风险与强制截断

对应 [`plan.md`](plan.md) 的队列项七。

类型：`state::pass_lane_clearance` 的纵向投影、`resolve::shot` 的收球分支、`policy::choose_action` 的候选截断、`policy.json` 的新增系数。

事实：`state::pass_lane_clearance` 只算两端之间的防守人，传球风险与接球质量都读它；`resolve/shot.rs` 在时钟充裕且出手瞬间干扰已到位时返回 `Completion::Interrupted`，把球收回来继续进攻；`truncation_ratio` 在抽样前剔除劣势候选。

测试入口：`crates/sim/tests/engine.rs`；`scripts/check_inline_constants.py`。

## 轮换、犯规危机与残局

对应 [`plan.md`](plan.md) 的队列项八。

类型：`rotation` 模块的体能恢复与换人判据、`endgame` 模块的胜率与末段约束、`resolve/shot` 的收球分支、防守端故意犯规入口。

事实：`rotation::update_stamina` 双向计算体能；`rotation::substitute_if_needed` 按四条判据换人，替补候选排除本身也需要休息的人；`rotation::record_foul` 登记犯满；`endgame::shot_constraint` 与 `intentional_foul` 读 `policy.json` 的残局系数，故意犯规经 `resolve::foul::apply_non_shooting_foul` 结算。

测试入口：`crates/sim/tests/rotation.rs`（四分钟体能回升、六犯不再上场）、`crates/sim/tests/endgame.rs`（三分必投、压时间、故意犯规、砍人与最后两分钟无球限制）。
