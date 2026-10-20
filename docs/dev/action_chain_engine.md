# 动作链内核

> 定位：`crates/sim` 的设计与当前状态。旧引擎（`crates/engine`）与本内核并存，迁移完成后旧引擎删除。
> 参数来源与判定结果见 `docs/dev/evidence/action_chain_validation/` 与 `data/calibration/`。验收记录由 `nba-sim validate-core --out <目录>` 生成，同目录的 `run.json` 记下产出它们的提交、种子与校准表来源。

## 1. 要解决的问题

旧引擎把比赛结果压成一个标量命中概率，再用弹道把概率画成抛物线。出手位置来自几何候选点与十项加法效用，命中率来自另一套加法模型；两个近似相乘，聚合统计因此长期对不上，每次调参都会破坏另一个指标。

本内核把动作放到因果链的中间：

```text
状态 → 决策（选动作） → 执行 → 结算（动作的后果） → 新状态 → 决策 → …
```

结果不是预先抽出来的，它是动作结算的产物。动作不是凭空出现的，它是策略从状态里选出来的。状态不是写死的，它是上一个动作结算的产物。

## 2. 三层权责

| 层 | 决定什么 | 输入 | 输出 |
| --- | --- | --- | --- |
| 决策层 | 谁在什么状态下选哪个动作 | 状态 + 能力 + 倾向 + 战术 | 动作意图 |
| 结算层 | 动作在这个状态下产生什么后果 | 动作 + 双方能力 + 几何 + 时机 | 状态变化 + 事实 |
| 表现层 | 这些事实长什么样 | 事实 + 时间戳 | 轨迹与弧线 |

表现层没有裁定权。动画是动作链的投影，行为是动作链本身。

## 3. 动作空间

动作是带前置条件、时长与结算签名的意图。结算产出的是状态变化，不是结果。

| 动作 | 结算产出 |
| --- | --- |
| 掩护 | 接触、防守人路线（挤过/绕过/换防）、分离距离 |
| 突破 | 是否过掉首要防守人、到达位置、丢球 |
| 传球 | 拦截、接球质量、控制权 |
| 空切 | 摆脱盯防、接球位置 |
| 背身 | 取得深位、防守夹击 |
| 出手 | 距离、干扰、出手类型、命中/不中/被封盖/被犯规 |
| 篮板 | 前场或后场归属、落点 |

出手类型由动作语境决定，位置必须与语境匹配，不匹配时不出手。接球投只发生在三分线外，突破终结只发生在篮下与近筐，背身终结只发生在近筐与中距离。这是“出手位置由动作链产生”的实现方式。

防守动作在生产路径上有三个入口，都读防守人自己的能力与倾向：

- **封盖**：出手前先选定最近的封盖人（按 `block` 与 `vertical` 加权），按 `block`、`vertical`、干扰与 `block_aggressiveness` 抽是否封到。封到则这次出手必不中，并给封盖人记一次；若这次起跳撞到投篮人（由 `physicality` 决定），改判投篮犯规。校准表给的是包含封盖与犯规在内的边际命中率，所以“未被封盖且未被犯规”的出手要把这两部分先扣掉再条件化；被封盖的出手必不中，被犯规的出手只以 `and_one_rate` 投进（约 12%），其余给罚球。
- **抢断**：候选人是场上所有防守人，权重同时含位置与能力：`1/(距离+1) × (0.5+steal) × (0.7+0.6·gamble_steal)`。抽中的那个人再按同一组能力决定拦截概率，传球人的 `passing` 与 `decision_iq` 压低它。成功则记抢断并终结回合；失败不影响传球。另有一条不记抢断的传球出界路径，由 `passing` 决定，用来维持失误总量。
- **协防**：干扰强度按防守人的 `help_aggressiveness` 加权，倾向越高，同样的位置给出更强的干扰。
- **非投篮犯规**：突破失位后的手部犯规（`physicality` 与 `help_aggressiveness` 决定概率）在 bonus 状态下给两罚；掩护时掩护人没站住则吹移动掩护，记 `TurnoverKind::OffensiveFoul` 并转换球权。两者都计入团队犯规数，bonus 阈值沿用 `GameRules` 里的 `bonus_fouls_per_period`。

这三项都是单次尝试，尚未建模“尝试→失位→被过”的连续段，缺口的记在 §9。

## 4. 参数表

`data/calibration/` 下每张表带 `provenance`（数据源、赛季、样本量、脚本、方法）。引擎只读，表缺失或样本为零直接失败。

| 表 | 内容 | 来源 |
| --- | --- | --- |
| `shot_make` | 按出手类型与距离分箱的命中率、分区技巧离散度、干扰灵敏度 | 投篮档案逐球拟合 |
| `shot_profile` | 出手类型份额、区域份额、出手来源份额、条件分布 | 投篮档案逐球统计 |
| `possession` | 回合结局速率、时长分位、联盟基准 | 回合数据 |
| `rebound` | 分区前场篮板率、联盟基准 | 回合数据与事件文本 |
| `freethrow` | 联盟罚球命中率、技巧离散度 | 事件文本 |
| `formations` | 半场落位坐标 | 阵型几何 |
| `policy` | 策略权重、结算速率、封盖参数 | 矩匹配 |

`scripts/calibrate/fit_tables.py` 从原始数据生成前五张表；`scripts/calibrate/fit_policy.py` 用矩匹配调整策略权重。`policy.json` 分三块：`weights` 是无量纲响应系数，`geometry` 是英尺与秒类的几何量（如推进起步衰减距离），`bands` 是判定带宽。三者不混在同一张表里，避免把英尺当权重读。

## 5. 判定

判定分四层，前两层在结算时逐动作记录，后两层对批次做比较。

| 层 | 内容 | 失败后果 |
| --- | --- | --- |
| 几何合法性 | 出手点、突破位移与时长、传球线路 | 阻断 |
| 语境合法性 | 出手类型必须有对应动作，否则记录违规 | 阻断 |
| 分布比较 | 区域份额、来源份额、回合结局速率、罚球率、得分、篮板、节奏 | 报告失败项 |
| 反事实 | 扰动能力后回合响应方向 | 测试失败 |

判定单元是动作与回合；聚合统计只作为投影呈现，不进入前两层。

`nba-sim sim-core --gate` 输出四层结果。当前批次几何与语境违规为零，二十一项分布比较全部在带内。

## 6. 协议流

`World::stream_tick` 把世界投影成 `StreamTick`：逐帧球员位置与朝向、球的状态、比分、犯规、罚球余额、规则快照与逐帧事实。`nba-sim sim-core --stream <path>` 写出 NDJSON，路径以 `.gz` 结尾时压缩。事实载荷使用球员 id，索引只在进程内作为句柄。

浏览器适配层 `crates/bball-wasm` 使用内嵌校准表（`Params::embedded`），因此 WASM 目标不需要文件系统；host 与 wasm32 两个目标均可构建。

## 7. 验证入口

`crates/validation/core_validation` 为新内核提供验证：四层判定加事实流独立重建。重建只读事实，把得分、出手、篮板、失误重新累加后与箱体对账，两侧分叉即判定失败。`nba-sim validate-core --seed N --games M` 以退出码给出结论，`--out <目录>` 指定工件目录，缺省为仓库根下的运行目录。

### 机制实验

`crates/validation/mechanism_core` 替代旧引擎的 MECH 层：每个属性跑两组，主队属性取低值（0.10）与高值（0.95），客队作为同种子对照组。裁决以种子为抽样单元做配对 bootstrap：逐种子计算基准与扰动的差值，取 5%–95% 区间；区间完全落在零的预期一侧记 `pass`，跨越零记 `insufficient`，方向与预期相反记 `defect`。

结果分两类证据。`wiring` 的度量直接取自消费者公式的输入：`speed` 与 `acceleration` 的度量是 `mean_first_action_seconds`，由 `advance_seconds = distance / speed + (speed / accel) · exp(-distance / 30) + 2.3` 算出来，它只证明接线接通。`outcome` 的度量是比赛产物：三区命中率、罚球命中率、篮板率、失误率，它们证明技巧改变了结果。两类在报告里分列。

入项：投篮三区、罚球、速度、加速度、体能、力量、前场篮板、后场篮板、决策智商、封盖、抢断，共十三项，每属性 40 个种子。中投的度量是中投命中率（中投命中除以中投出手）：技巧直接决定的就是命中率，出手份额由策略层的长两分惩罚与距离项决定，属于另一个判据。封盖与抢断的度量分别是封盖除以对手出手、抢断除以对手回合。`stamina` 的度量目前不可用：轮换把体能低的球员换下，两个扰动臂的在场体能分布趋同，已登记为 `mechanism_change`。

排除声明只写在 `data/calibration/known_insufficient.json`，代码里没有硬编码的排除项。`resolution` 分两种：`more_games` 必须写 `required_games` 与推导依据，归入 `insufficient`；`mechanism_change` 表示效应在结构上接近零，归入 `not_applicable`。`certification.json` 把四类分开写成 `known_insufficient`、`new_insufficient`、`not_applicable` 与 `marginal`。

通过项记录区间宽度与「靠零边界 / 点估计」比值；区间宽度超过点估计本身标为边缘通过。`marginal` 栏只记录不阻断，摘要因此能看出哪几项离零很近。当前批次没有边缘通过项。

重实验只在 `validate-core --mechanism` 离线运行，结果写入工件目录；接口只读这个工件，不在请求里同步重跑。旧实现把上千次短跑放进一次请求，三百分秒不返回。

运行目录一律从仓库根拼接（`params::workspace_root`），不从进程当前目录解析；从任何工作目录启动都能读到同一份工件。

### 工件

`validate-core` 写出六件：`core_validation.json`、`mechanism_core.json`、`certification.json`、`batch_statistical_results.json`、`branch_coverage.json`、`run.json`。`certification.json` 是三态总结（`pass`、`defects_found`、`insufficient_evidence`），同时供给页面认证总览；`batch_statistical_results.json` 是逐准则的观测值、基准、带宽区间与相对误差；`branch_coverage.json` 记录实际判定过的准则数（分布准则加机制属性）。`run.json` 记录产出它们的提交、工作区是否干净、种子与场次、机制实验配置，以及七张表各自的来源。工件目录不计入脏工作区判定，否则每次重跑都会把自己标记成脏的。

## 8. 旧引擎的删除

旧引擎的四个包（`engine`、`decision`、`officiating`、`evaluator`）与只服务它们的 `physics`、`semantics`、`invariants`、`test-support` 已从工作区删除，命令行只剩 `sim-core` 与 `validate-core`，`crates/validation` 只剩 `core_validation` 与 `mechanism_core`。`parquet`、`rusqlite`、`arrow` 三个依赖随之退出。

调试服务器只保留新内核：`/api/simulate` 只接受 `seed` 与 `scope`，`/api/session/*` 指向 `SessionService`，`/api/validation/*` 读 `validate-core` 的工件。`/api/studio` 与战术、名单面板已删除：新内核没有战术与 Play，这些面板没有可配置的对象。

旧引擎的设计记录保留在 `docs/decisions.md` 与 `docs/dev/cycles/`，作为历史，不再维护。

## 9. 已知缺口

- 卡位还没有独立动作：篮板归属只按能力与位置加权，没有先占位的接触结算。
- 十二个倾向字段中有八个进入生产路径；其余四个（空切、冲板、风险容忍、转换冲刺）在 `scripts/check_r6_tendency_wiring.py` 里登记为待接入。
- 无球跑位只覆盖空切与掩护，弱侧轮转与错位攻击尚未进入动作空间。
- 封盖与抢断是单次尝试，没有“尝试→失位→被过”的连续段；协防只通过干扰强度加权，没有独立的轮转动作。
- 非投篮犯规已接上两条路径：突破失位后的手部犯规（进 bonus 给两罚）与移动掩护（进攻犯规、球权转换）。还有两类没接：停球后的抱住犯规与抢篮板时的推人。
- 体能的机制指标还不可用：轮换把体能降到替换阈值的首发换下，两个扰动臂的在场体能分布趋同，全队命中率衡量的是轮换节奏。登记在 `data/calibration/known_insufficient.json`，resolution 为 `mechanism_change`；改指标（首发在场时段的命中率或按轮换分段）后重新跑。
- 走步与二次运球已接上：持球状态机（运球中 / 已收球）记在回合状态里，站定的调整运球以收球结束，收球后再运球判 `DoubleDribble`，收球后突破超过 `gather_travel_allowance_ft` 判 `Travel`；策略层在已收球时不再把运球列为候选。`OutOfBounds` 还没有产生点。
- 走步的容限量仍是 20 英尺，比两步加起跳（11–14 英尺）大：收到 12 英尺实测两轮都不成立——允许收球后突破时失误率升到 0.234；禁止收球后突破时回合数 208.2、and-1 2.06 同时出带。根子在收球点由一次 1.4 秒的调整运球决定，而不是由真实的上篮收球决定。把容限改对之前要先有收球模型（突破末段的收球点），否则每次上篮都会吹走步。
- 因此走步在正常回合里几乎不触发，也没有走步率判据：需要先有收球模型，再按 `PossessionRecord.ended_by` 的构成加一条判据。
- 犯规总数偏低：投篮犯规 19 左右、非投篮犯规 9 左右，合计约 29，联盟参考是 40。缺的是无球端的松球犯规与推人；把 `team_fouls_per_game` 加进判据会直接红（29 对 40），所以这一条等松球犯规建模后再登记。
- `validation.md` §9.2 的场景目录尚未移植，`crates/sim/tests` 只有一个集成测试文件。
- 内建名册的主客队属性相同，球队差异还没有进入引擎。
- 检查点级统计差值对照（原 `data/reference/stat_deltas.v1.json`）没有替代品；分布层由二十一项准则承接。
- `crates/debug-server/static/app.js` 里战术与名单面板的渲染函数已不可达（面板与 `/api/studio` 已删除），它们的删除待完成。`data/tactics/plays/*.json` 仍被 `crates/domain/tests/playbook_data.rs` 当作 PlaySpec 解析夹具使用，引擎不再读它们。
