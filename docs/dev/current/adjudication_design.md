# 裁决提交入口

> 依据 `full_simulation_plan.md` §8.1：停表原因、违例种类、犯规分类、罚则计划、恢复计划、发球地点、恢复上下文与时钟调整必须是类型化数据，并且由单一提交入口写入比分、球权与时钟，避免各个结算分支各自改写状态。

## 当前形态

`World::adjudicate` 是停表结算的提交入口。原始事实先由 `World::step` 检出为 `RawIncident`，裁决层按联赛档案生成 `AdjudicationBatch` 与 `PenaltyQueue`，提交入口在一个批次内写入比分、球权、犯规账本、程序、时钟、罚则、动作运行时与事件投影。

| 数据结构 | 语义 | 当前实现 |
| --- | --- | --- |
| `RawIncident` | 判定读到的原始事实与当事人 | `crates/sim/src/incident.rs` |
| `AdjudicationBatch` | 一次提交中的完整裁决计划 | `crates/sim/src/adjudicate_incidents.rs` |
| `PenaltyQueue` | 按顺序执行的罚则 | `crates/sim/src/incident.rs` |
| `Adjudication` | 停表原因、得分、罚则、恢复方式与时钟调整 | `crates/sim/src/officiating/mod.rs` |
| `StreamTick` | 权威状态、程序资格与版本投影 | `crates/sim/src/stream.rs` |

## 类型化数据

| 类型 | 语义 | 当前实现 |
| --- | --- | --- |
| `StoppageCause` | 为什么停表：进球、违例、犯规、暂停、节末、设备原因 | `crates/sim/src/officiating/mod.rs::StoppageCause` |
| `ViolationKind` | 违例细分：走步、二次运球、后场八秒、回场、出界、紧逼五秒、进攻三秒、发球五秒 | `crates/sim/src/officiating/mod.rs::ViolationKind` |
| `FoulKind` | 犯规分类：投篮、非投篮、进攻、技术、恶意、无球、松球 | `crates/domain/src/event.rs::FoulKind` |
| `RestartPlan` | 恢复方式：发球、跳球、罚球、节次开局、比赛结束 | `crates/sim/src/officiating/mod.rs::RestartPlan` |
| `InboundLocation` | 发球地点：底线、边线断点、中场、前场延伸线 | `crates/sim/src/officiating/mod.rs::InboundLocation` |
| `ResumeContext` | 恢复上下文：恢复球权方、发球地点与恢复时刻 | `crates/sim/src/officiating/mod.rs::ResumeContext` |
| `ClockAdjustment` | 时钟调整：比赛钟与进攻钟设定、计时器起停与检查点恢复 | `crates/sim/src/officiating/mod.rs::ClockAdjustment` |

## 单一提交入口

`World::adjudicate(adjudication: Adjudication)` 承担全部写入，`Adjudication` 由停表原因构造：

```rust
pub struct Adjudication {
    pub cause: StoppageCause,
    pub possession_end: Option<(PossessionEndCause, &'static str)>,
    pub points: u32,
    pub clock: ClockAdjustment,
    pub restart: RestartPlan,
}
```

提交顺序固定为：结束回合与比分账本、时钟调整、恢复阶段与发球地点。`RawIncident` 只描述判定读到的事实，裁决层负责把事实转换成 `Adjudication`，结算分支调用 `World::adjudicate` 提交。

## 覆盖范围

| 范围 | 内容 |
| --- | --- |
| 违例 | 出界、后场八秒、回场、紧逼五秒、24 秒、二次运球、走步 |
| 球权变更 | 防守篮板、松球易主、抢断传球、丢球 |
| 得分与罚球 | 投篮命中、罚球结算、保留球权 |
| 犯规 | 进攻、非投篮、投篮、技术犯规的回合结束与罚球程序 |
| 节次与程序 | 节末、终场、换人窗口、暂停恢复 |

`RestartPlan::Inbound::ready_after` 传停表时长；暂停恢复由 `Phase::Timeout` 携带 `ResumePhase`。场景回归覆盖 `rules_flow`、`rule_violations`、`rotation`、`event_ledger` 与 `action_scenarios`。

## 与验证的关系

`crates/validation/src/core_validation.rs` 的回合审计要求「每个开始的回合恰好有一个类型化终态」。`World::adjudicate` 是产生终态的提交入口，`Fact::PossessionEnded` 的 `end_reason` 由 `Adjudication` 给出。
