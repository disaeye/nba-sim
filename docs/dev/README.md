# NBA-Sim 开发文档

> `docs/dev/` 记录当前实现、执行进度、可复核证据与历史周期。所有当前结论都应能对应代码、测试或可重现的验证。

## 0. 当前入口

- **当前状态**：[`status.md`](status.md)（当前代码、验证范围与证据入口）
- **当前实施入口**：[`current/plan.md`](current/plan.md)
- **唯一权威实施方案**：[`current/full_simulation_plan.md`](current/full_simulation_plan.md)
- **当前执行进度**：[`current/implementation.md`](current/implementation.md)
- **观测视图**：[`visualization.md`](visualization.md)
- **跨周期差距**：[`gap.md`](gap.md)
- **里程碑路线**：[`roadmap.md`](roadmap.md)
- **原始问题与影响证据**：[`evidence/problem.md`](evidence/problem.md)、[`evidence/impact_assessment.md`](evidence/impact_assessment.md)
- **规则目录核验记录**：[`evidence/rule_catalog_sources.md`](evidence/rule_catalog_sources.md)
- **历史周期**：[`cycles/`](cycles/)

当前执行依据的阅读顺序：`status.md` → `current/plan.md` → `current/full_simulation_plan.md` → `current/implementation.md`。历史周期只用于查阅归档内容。

## 1. 文档职责

| 文件 | 唯一职责 |
| --- | --- |
| `status.md` | 当前代码状态、验证范围与证据入口 |
| `current/plan.md` | 当前方案入口，不复制完整设计与任务正文 |
| `current/full_simulation_plan.md` | 唯一权威实施方案：目标范围、结构、依赖、实施次序与最终验收要求 |
| `current/implementation.md` | 阶段状态、验证证据与当前残差 |
| `gap.md` | 跨周期差距与依赖关系 |
| `roadmap.md` | 项目里程碑及其顺序 |
| `evidence/` | 可重现的命令、输入、原始输出与分析 |
| `cycles/` | 已结束周期的完整历史记录 |

任何完整方案只在 `full_simulation_plan.md` 保存一份。`plan.md` 只提供入口；`implementation.md` 只记录进度，不另设目标或另写一套方案。

## 2. 当前状态与证据

`status.md` 记录当前工作区能够核验的状态，每条状态指向代码、测试、守卫或最终验收工件。`docs/dev/evidence/` 保存门禁、认证、机制实验、性能与阶段证据；`.work/evidence/final-validation.summary` 保存验收链的逐项退出码。

`evidence/` 允许保存命令、输入、原始输出与分析。历史周期归档放在 `cycles/`，当前结论以 `status.md` 与最终工件为准。

## 3. 当前实施进度

`implementation.md` 按 `full_simulation_plan.md` 的依赖阶段记录阶段状态、代码与数据路径、测试身份、验证证据和当前残差。用户确认目标范围或核心结构变化时，更新 `full_simulation_plan.md`，并检查目录入口、验收任务和实现进度。进度页不复制完整方案。

## 4. 编号范围

跨文件使用的编号应在此处登记后再使用；编号不得与其他规格、周期或测试清单混淆。

| 编号范围 | 所属文档 | 用途 |
| --- | --- | --- |
| G0–G9 | `gap.md` | 跨周期差距标识 |
| M1–M10 | `roadmap.md`、`protocol.md` | 里程碑标识 |
| D0–D6 | `cycles/20260911_first-principles/plan.md` | 已归档周期任务 |
| D7–D13 | `cycles/20260916_convergence/plan.md` | 已归档周期任务 |
| D14 及后续 | 当前周期方案或执行记录 | 当前周期任务；须先登记到唯一实施方案 |
| R1–R12 | 历史周期记录 | 已完成轮次与证据索引 |
| F1–F7 | 历史周期记录 | 遗留事项的历史标识 |

历史编号只用于追溯。新增任务编号前先确认既有编号范围的用途。

## 5. 更新与归档

每次实现变更至少更新一处相关记录：

- 当前已验证状态变化时更新 `status.md`；
- 阶段、证据或当前残差变化时更新 `current/implementation.md`；
- 用户确认方案范围或结构变化时更新 `current/full_simulation_plan.md`；
- 新增原始验证时保存至 `evidence/`。

一个周期满足以下条件后才能归档：

- 当前阶段已验证完成；
- `status.md` 已记录当前结果与证据入口；
- 原始证据与执行记录已归入 `evidence/` 或对应周期目录；
- 当前入口及实施进度已更新。

归档目录使用 `YYYYMMDD_theme`。归档文件注明历史周期属性和当前状态入口。

## 6. 设计、决策与实现记录

- 稳定的篮球规则与系统设计进入 `docs/` 对应规格；
- 跨模块取舍进入 `docs/decisions.md`；
- 本目标的完整实施方案与验收要求进入 `current/full_simulation_plan.md`；
- 当前代码路径与测试事实进入 `status.md` 或 `current/implementation.md`；
- 命令输出、输入数据与失败原文进入 `evidence/` 或周期归档。

方案获用户确认后，稳定设计结论应进入对应规格。当前方案保留执行依赖和验收范围；状态页与进度页只记录代码、测试和证据，不复制设计全文。
