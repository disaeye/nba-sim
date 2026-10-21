# 当前执行步骤

对应 [`plan.md`](plan.md) 的剩余工作。上一轮引擎改造的步骤记录已归档在 [`cycles/20261021_engine_rebuild/`](../cycles/20261021_engine_rebuild/implementation.md)。

## 聚合判定校准

类型：校准，只动 `data/calibration/policy.json` 与名册数据，不改机制。

事实：`gate::evaluate` 的二十一条聚合判据里，近筐份额、空切份额、回合数、使用率层级四项越界；十八条过程判据除使用率层级外全部通过。

步骤：

- 近筐份额：先定下突破落点的抛投生成规则，再看 `post_base` 与 `drive_finish_*` 是否需要回调；
- 空切份额：把空切从基础分抬升改为目标生成入口，`cut_base` 回到中性值；
- 回合数：从动作时长与决策间隔入手，`dribble_base` 与 `pass_base` 已验证改不动它；
- 使用率层级：按原型改写 `data/roster/` 的 `shoot_frequency`，保持 ADR-020 的「代码不按角色分支」。

测试入口：`nba-sim validate-core --seed 1 --games 32`；`scripts/check_inline_constants.py`。
