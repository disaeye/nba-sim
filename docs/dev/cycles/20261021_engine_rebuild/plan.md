# 当前执行队列

> 状态：当前活跃计划，只列未完成工作。已完成工作随周期移入 `docs/dev/cycles/`，在 `status.md` 保留结论与证据入口。
> 当前门状态见 [`status.md`](../status.md)；内核设计与缺口清单见 [`action_chain_engine.md`](../action_chain_engine.md)；验收语义见 `docs/protocol.md`。
> 每项的类型、事实与测试入口见 [`implementation.md`](implementation.md)。

## 1. 队列与依赖

```text
Q2 过程级判据进 gate
  → Q2b 决策层消费掩护路线与出手时钟门槛
      → Q2c 角色经倾向通道分化
          → Q3 协防引力场求解器 → Q4 Play 选板接线
          → Q5 传球走廊风险与强制截断
          → Q6 轮换、犯规危机与残局
```

`Q2` 是其余各项的验收前提：没有过程级判据，行为改动的效果无法与换一种凑数方式区分。`Q2b` 与 `Q2c` 只依赖已就位的观测层，排在战术层之前。

## 2. Q2 · 过程级判据进 gate

类型：观测层扩展，不改变模拟行为。

现状：`gate::evaluate` 的判据全部是批次聚合量，没有一条检查出手时钟分布、防守距离、使用率或残局行为。

验收条件，新增判据全部进入 `gate::evaluate` 并出现在判定工件里：

| 判据 | 目标 |
| --- | --- |
| 出手时钟分档份额 | 四个分档贴近真实赛季分布 |
| 早攻强投率 | `shot_clock > 15` 且 `contest > 0.6` 的跳投占比 < 1.5% |
| 防守距离分档 | 紧逼、空位、大空位三档份额 |
| 使用率比值 | 队内第一与第五的 `fga + 0.44·fta` 比值 ≥ 2.1 |
| 受助攻率极差 | 队内极差 ≥ 0.35 |
| 传球链构成 | 零传球回合 12%~18%，均值 2.8~3.5 次 |
| 乒乓传球率 | 小于 1.5% |
| 掩护路线响应 | `route == Under` 时干拔三分占比 ≥ 45%；`route == Over` 时突破占比 ≥ 55% |
| 出场时间上限 | 不超过节数 × 12 + 加时 × 5 |

## 3. Q2b · 决策层消费掩护路线与出手时钟门槛

类型：机制实现，改变行为。

现状：`DefenderView.route` 已回写进决策层，`policy::choose_action` 在掩护后首个决策步把候选收敛到路线应答集；`Shoot` 需通过按分区最高可得分的归一化时钟门槛，时钟充裕时受干扰的出手直接排除。实测早攻强投率 1.22%、`Under` 后干拔三分 46.0%、`Over` 后篮下攻击 70.0%，出手期望得分三档 1.384 / 1.117 / 0.996。

验收条件：

- `Under` 后干拔三分占比 ≥ 45%；
- `Over` 后突破占比 ≥ 55%，篮下攻击计入 `Drive` 与 `Finish` 两族；
- 早攻强投率 ≤ 1.5%；
- 出手期望得分在 24-18 秒、17-7 秒、6 秒以下三档严格递减；
- 路线项与门槛系数全部位于 `policy.json`，`scripts/check_inline_constants.py` 通过。

## 4. Q2c · 角色经倾向通道分化

类型：数据通道扩展与机制实现，改变行为。

现状：`PlayerTendencies` 新增 `three_point_preference`，名册按原型赋值（背身中锋 0.05 至定点射手 0.95），`policy::choose_action` 按它调整出手效用，`policy::spot_target` 按它伸缩站位到篮筐的距离。实测背身中锋三分份额 0.0%、定点射手 67.5%，`crates/sim` 不读 `offensive_role`。

验收条件：

- `PlayerTendencies` 承载投射倾向，名册按原型赋值；
- 背身型中锋的三分出手占其总出手 < 4%，定点射手的三分出手占其总出手 > 35%；
- `crates/sim` 不读 `offensive_role`；
- `scripts/check_r6_tendency_wiring.py` 登记新维度并接入生产路径。

## 5. Q3 · 协防引力场求解器

类型：机制实现，改变行为。

现状：`defense::solve_defense_targets` 已替换按到球距离排序的旧实现。势能由对位羁绊、朝篮筐下沉、弱侧协防引力三项叠加，参数全部读 `world.rules.tactics.defense.potential_field`；`PossessionState.matchup` 记录对位表，只在掩护换防时交换。实测五个种子下强侧底角防守人位移 0.00 ft、弱侧低位人目标点距篮 0.84–3.75 ft、突破分球接球人获空间 15.26–18.40 ft。

验收条件：

- 持球人突破到禁区时，强侧底角防守人位移 < 2.5 ft；
- 同一条件下弱侧低位防守人距篮 < 6 ft；
- 突破分球的接球人最近防守距离，比普通外线传导高 ≥ 3.2 ft；
- 无掩护、无过人的回合里对位数组不变；
- 求解器零堆分配；
- 掩护路线写回 `DefenderView.route`，且与 `Fact::Screen` 记录的路线一致。

## 6. Q4 · Play 选板接线

类型：机制实现，改变行为。

现状：`params.tactics` 加载七份体系档案与九份剧本（含浏览器目标的内嵌副本），启动期校验每份剧本的 `then.slot` 在被引用体系里存在。`tactics::update_play_window` 在活球回合里求值触发谓词，每回合最多叫一次战术；`policy::choose_action` 在窗口内应用 `carrier_preferences` 与 `inhibitions`。实测激活率 0.756、窗口配对差 0。

验收条件：

- 半场阵地回合的 `PlayActivated` 发出率落在 65%~85%；
- 窗口结束发 `PlayWindowEnded`；
- 声明 `inhibitions` 的档案激活期内，被移除的候选族选中次数为 0；
- 每份 `PlaySpec` 的 `then.slot` 在对应 `TacticalSetSpec` 里存在，启动期校验失败即报错；
- 至少两份档案声明 `inhibitions`，且顶层偏好覆盖 `Shoot` 与 `Drive`。

## 7. Q5 · 传球走廊风险与强制截断

类型：机制实现，改变行为。

现状：`state::pass_lane_clearance` 只统计纵向投影在两端之间的防守人，传球风险与接球质量都读它；`resolve/shot.rs` 在时钟充裕且出手瞬间干扰已到位时把球收回来（`Completion::Interrupted`），不再强行出手；`policy::choose_action` 在抽样前按 `truncation_ratio` 剔除劣势候选。实测乒乓传球率 0.0004、早攻强投率 0.0000。

验收条件：

- 传球走廊风险计入纵向投影区间，站在传球人身后的防守人不算截断者；
- 乒乓传球占比 < 1.5%；
- 劣势动作强制截断生效，截断阈值与温度全部位于 `policy.json`，`scripts/check_inline_constants.py` 通过。

## 8. Q6 · 轮换、犯规危机与残局

类型：机制实现，改变行为。

现状：`rotation::update_stamina` 双向计算体能（在场消耗、场下与停表恢复），`rotation::substitute_if_needed` 按四条判据换人（犯满离场、体能阈值、首发犯规危机、垃圾时间），`rotation::record_foul` 登记犯满；`endgame::win_probability` 给出解析胜率，`endgame::shot_constraint` 给出末节出手约束，`endgame::intentional_foul` 判定故意犯规。实测单场换人 21.9 次、最长出场 22.1 分钟/场。

验收条件：

- 单场换人不少于 8 次，无球员出场时间超过节数 × 12 + 加时 × 5；
- 坐满 4 分钟的球员体力上升；
- 六犯球员后续不再上场；
- Q4 剩 7 秒落后 3 分时三分出手占比 ≥ 92%；
- Q4 剩 26 秒领先 4 分时首次出手在进攻时钟 5 秒以内的比例 ≥ 90%，防守方 3.5 秒内故意犯规比例 ≥ 92%；
- 罚球命中率 36% 的中锋在 Q4 剩 3:30 被故意犯规次数 > 0，剩 1:30 无球故意犯规次数 = 0。
