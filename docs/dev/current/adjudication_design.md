# 裁决提交入口的设计与迁移清单

> 依据 `full_simulation_plan.md` §8.1：停表原因、违例种类、犯规分类、罚则计划、恢复计划、发球地点、恢复上下文与时钟调整必须是类型化数据，并且由单一提交入口写入比分、球权与时钟，避免各个结算分支各自改写状态。

## 现状

停表结算分散在 8 个模块、21 个调用点，每处自行组合「记失误或犯规」「结束回合」「改时钟」「设定阶段」：

| 模块 | 结算内容 | 需要的信息 |
| --- | --- | --- |
| `ball.rs::out_of_bounds` | 最后触球方交出球权、按出界点发球 | 最后触球、出界点 |
| `resolve/mod.rs`（运球/二次运球） | 走步与二次运球 | 违例种类、恢复球权方 |
| `resolve/drive.rs`（三处） | 走步、带球撞人、丢球 | 违例或进攻犯规、后续发球方 |
| `resolve/shot.rs::finish` | 命中、投失、被盖、投篮犯规 | 得分、篮板归属、罚球计划 |
| `resolve/foul.rs`（两处） | 非投篮犯规、进攻犯规、技术犯规 | 罚则计划、保留球权方、恢复位置 |
| `resolve/freethrow.rs::finish` | 末罚结果、保留球权 | 罚球计划终态、恢复计划 |
| `resolve/rebound.rs`（三处） | 防守篮板、松球易主、二次进攻 | 球权变更、进攻时钟重置 |
| `world.rs::end_possession` 的调用方与节末收尾 | 节末、终场、24 秒、紧逼五秒、后场八秒、回场 | 停表原因、时钟调整 |

`World::end_possession(end_reason: PossessionEndCause, termination_detail, points)` 已经承担「关回合、发事件、写记录」这三件事，但比分、时钟与阶段仍由调用方各自写入；`Phase::DeadBall` 的 17 个写入点各自决定恢复方与恢复时间。

## 目标类型

| 类型 | 语义 | 现有对应 |
| --- | --- | --- |
| `StoppageCause` | 为什么停表：进球、违例、犯规、暂停、节末、设备原因 | 分散在 `PossessionEndCause` 与字符串 detail |
| `ViolationKind` | 违例细分：走步、二次运球、后场八秒、回场、出界、紧逼五秒、进攻三秒、发球五秒 | `TurnoverKind` 的违例子集 |
| `FoulClassification` | 犯规分类：投篮、非投篮、进攻、技术、恶意、无球、松球 | `FoulKind` 与 `event::FoulClassification` 的领域枚举 |
| `PenaltyPlan` | 罚球计划：执行方、次数、是否末罚开篮板、是否保留球权、技术犯规插入与恢复 | `FreeThrowMeta` 与 `FreeThrowFollowUp` |
| `RestartPlan` | 恢复方式：发球、跳球、罚球、节次开局、比赛结束 | `Phase` 的目标阶段 |
| `InboundLocation` | 发球地点：底线、边线断点、中场、前场延伸线 | `inbound_spot` 与 `begin_inbound` 的默认底线 |
| `ResumeContext` | 恢复上下文：中断前的程序、控制方、剩余时钟、未执行罚则 | 尚未建模 |
| `ClockAdjustment` | 时钟调整：比赛钟归零、进攻时钟重置值、是否停表 | 分散在各分支的 `set_shot_clock` 调用 |

## 单一提交入口

`World::adjudicate(adjudication: Adjudication)` 承担全部写入，`Adjudication` 由停表原因构造：

```rust
pub struct Adjudication {
    pub cause: StoppageCause,
    pub points: Option<(TeamSide, u32)>,
    pub foul: Option<FoulBooking>,
    pub violation: Option<ViolationKind>,
    pub penalty: Option<PenaltyPlan>,
    pub restart: RestartPlan,
    pub clock: ClockAdjustment,
    pub resume: Option<ResumeContext>,
}
```

提交顺序固定为：比分与犯规账本、回合结束、时钟调整、恢复阶段与发球地点。任何结算分支只负责构造 `Adjudication` 并调用 `adjudicate`，不再直接写 `score`、`clock`、`Phase::DeadBall`。

## 迁移顺序

1. 违例类：出界、后场八秒、回场、紧逼五秒、24 秒。这五处已有类型化 `TurnoverKind` 与 `PossessionEndCause`，可直接构造 `Adjudication`。
2. 球权变更类：防守篮板、松球易主、二次进攻。需要 `RestartPlan::Live` 与进攻时钟调整。
3. 犯规类：非投篮、投篮、进攻、技术。需要 `PenaltyPlan` 与保留球权语义，是迁移量最大的一组。
4. 节末与终场：需要 `restart: RestartPlan::PeriodBreak | GameOver` 与时钟归零。
5. 暂停与换人恢复：需要 `ResumeContext` 保存被中断的程序。

每一步迁移都以现有测试为回归基线（`rules_flow`、`rule_violations`、`rotation`、`event_ledger`），迁移完成后旧分支删除，不保留两份写入路径。

## 与验证的关系

`crates/validation/src/core_validation.rs` 的回合审计已经要求「每个开始的回合恰好有一个类型化终态」。收拢提交入口后，`adjudicate` 成为唯一能产生终态的路径，审计条件与代码结构一致，`Fact::PossessionEnded` 的 `end_reason` 也不再依赖各分支自行选择。
