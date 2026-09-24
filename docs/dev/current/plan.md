# 当前执行队列

> 文档类型：当前未完成工作的执行顺序。
> 已结束周期的设计和实测保留在 `docs/dev/cycles/` 与 `docs/dev/evidence/`。
> 当前可复核状态见 `docs/dev/status.md`。关闭条件见 `docs/dev/gap.md`。

## 1. 队列

```text
R1 结果权交回过程
  → R2 篮下出手分布
  → R3 无球与对抗动作生命周期
  → R4 挡拆防守责任图
  → R5 规则程序与犯规账本
  → R6 倾向规格与个体评判
  → R7 有来源的参考分布和真实度目标线
```

后一项可以使用前一项已经发布的事实，不提前用统计带声明关闭。

## 2. R1 · 结果权交回过程

规格要求见 `docs/basketball.md` §3。当前 `BallState::Pass.receive_success` 与 `Drive` 的终结字段仍在球态创建时写入；投篮飞行已带概率和干扰，工作区还有未完成改动。本项按规格把每类结果改到对应事实出现的时刻：

1. 出手开始只保留出手人、出手位置、动作窗口和当时可见的防守事实；
2. 球离手后由弹道、封盖窗口和触筐事实产生命中、不中、封盖和犯规；
3. 传球在到达时产生接住、掉球、点掉或抢断；
4. 突破在接触和终结窗口结束时产生成功、失败、犯规和得分；
5. 固定种子下，决定结果的事实和结果事件保持可重放。

关闭证据：

- 同一出手参数在封盖成功和封盖失败下得到不同球态；
- 结果事件的 `parent_event_id` 指向决定它的飞行、接触或封盖事件；
- 现有得分账本继续对平；
- 黄金哈希按行为变更重新冻结，并写明变化来自结果决定点。

代码入口：

- `crates/engine/src/match_engine/execution.rs`
- `crates/engine/src/match_engine/state.rs`
- `crates/engine/src/match_engine/ball_flight/`
- `crates/engine/src/match_engine/block.rs`
- `crates/domain/src/flow.rs`
- `crates/engine/tests/shot_release_timing.rs`

## 3. R2 · 篮下出手分布

R1 完成后，用空切接球攻框、前场板补篮、转换终结和禁区落位四条动作链产生篮下出手。效用斜率不单独作为关闭手段。

关闭证据：16-seed 全场的篮下出手占比进入 `nba.v2.json` 的 `rim_share_of_fga` 带，同时三分命中率、罚球率、账本和 Hard 门保持通过。

## 4. R3 · 动作生命周期

已有 `OffBallActionKind`、`PostMoveKind`、`DribbleMoveKind`、`JumperKind` 和 `RimFinishKind`。本项只把已经具备物理窗口的动作接成开始、完成、取消和失败；没有窗口的细分保持为类型，不进入候选。

优先顺序：掩护建立、顺下、外弹、背切、空切、卡位、补篮。

## 5. R4 · 防守责任图

以一次挡拆为最小闭环：掩护接触、持球人利用、防守选择挤过、绕过、换防、延误或沉退、弱侧轮转、失败后的空位。势能场继续提供空间量，责任转移发布为事件。

关闭证据：同一进攻输入下两套防守档案的行为差异可沿事件定位，中性档案回到行为基线。

## 6. R5 · 规则程序

按 `docs/basketball.md` §5 和 §6 补齐走步、三秒、干扰、个人犯规、球队罚则和逐次罚球。`check_foul_conservation` 改为重建个人累计、球队累计、离场、罚则和罚球结果。NBA 与 FIBA 的差异只来自 `LeagueProfile`。

## 7. R6 · 个体身份

`attributes.md` 的 12 个倾向与 `PlayerTendencies` 的 8 个字段对齐。每个新增倾向先有消费点，再进入档案。随后补球员使用率、助攻父链、对位结果和末节体能评判。`ASSIST_PROFILE` 在助攻可从事件重建前保持 `InsufficientEvidence`。

## 8. R7 · 评判基准

先补联合分布和情境准则，再替换为带来源、版本和统计口径的参考数据。真实度目标线在 ADR-009 的条件满足后另行登记。
