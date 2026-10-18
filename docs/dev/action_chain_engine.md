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

`nba-sim sim-core --gate` 输出四层结果。当前批次几何与语境违规为零，十七项分布比较全部在带内。

## 6. 协议流

`World::stream_tick` 把世界投影成 `StreamTick`：逐帧球员位置与朝向、球的状态、比分、犯规、罚球余额、规则快照与逐帧事实。`nba-sim sim-core --stream <path>` 写出 NDJSON，路径以 `.gz` 结尾时压缩。事实载荷使用球员 id，索引只在进程内作为句柄。

浏览器适配层 `crates/bball-wasm` 使用内嵌校准表（`Params::embedded`），因此 WASM 目标不需要文件系统；host 与 wasm32 两个目标均可构建。

## 7. 验证入口

`crates/validation/core_validation` 为新内核提供验证：四层判定加事实流独立重建。重建只读事实，把得分、出手、篮板、失误重新累加后与箱体对账，两侧分叉即判定失败。`nba-sim validate-core --seed N --games M` 以退出码给出结论，`--out <目录>` 指定工件目录，缺省为仓库根下的运行目录。

### 机制实验

`crates/validation/mechanism_core` 替代旧引擎的 MECH 层：每个属性跑两组，主队属性取低值（0.10）与高值（0.95），客队作为同种子对照组。裁决以种子为抽样单元做配对 bootstrap：逐种子计算基准与扰动的差值，取 5%–95% 区间；区间完全落在零的预期一侧记 `pass`，跨越零记 `insufficient`，方向与预期相反记 `defect`。

结果分两类证据。`wiring` 的度量直接取自消费者公式的输入：`speed` 与 `acceleration` 的度量是 `mean_first_action_seconds`，由 `advance_seconds = distance / speed + (speed / accel) · exp(-distance / 30) + 2.3` 算出来，它只证明接线接通。`outcome` 的度量是比赛产物：三区命中率、罚球命中率、篮板率、失误率，它们证明技巧改变了结果。两类在报告里分列。

入项：投篮三区、罚球、速度、加速度、体能、力量、前场篮板、后场篮板、决策智商，共十一项。中投的度量是中投命中率（中投命中除以中投出手）：技巧直接决定的就是命中率，出手份额由策略层的长两分惩罚与距离项决定，属于另一个判据。当前批次的中投命中率从 0.2451 到 0.3824，区间 `[+21.6%, +99.4%]`。

排除声明只写在 `data/calibration/known_insufficient.json`，代码里没有硬编码的排除项。`resolution` 分两种：`more_games` 必须写 `required_games` 与推导依据，归入 `insufficient`；`mechanism_change` 表示效应在结构上接近零，归入 `not_applicable`。`certification.json` 把四类分开写成 `known_insufficient`、`new_insufficient`、`not_applicable` 与 `marginal`。

通过项记录区间宽度与「靠零边界 / 点估计」比值；区间宽度超过点估计本身标为边缘通过。`marginal` 栏只记录不阻断，摘要因此能看出哪几项离零很近。当前两项：中投命中率下界比 0.39，体能下界比 0.24。

重实验只在 `validate-core --mechanism` 离线运行，结果写入工件目录；接口只读这个工件，不在请求里同步重跑。旧实现把上千次短跑放进一次请求，三百分秒不返回。

运行目录一律从仓库根拼接（`params::workspace_root`），不从进程当前目录解析；从任何工作目录启动都能读到同一份工件。

### 工件

`validate-core` 写出六件：`core_validation.json`、`mechanism_core.json`、`certification.json`、`batch_statistical_results.json`、`branch_coverage.json`、`run.json`。`certification.json` 是三态总结（`pass`、`defects_found`、`insufficient_evidence`），同时供给页面认证总览；`batch_statistical_results.json` 是逐准则的观测值、基准、带宽区间与相对误差；`branch_coverage.json` 记录实际判定过的准则数（分布准则加机制属性）。`run.json` 记录产出它们的提交、工作区是否干净、种子与场次、机制实验配置，以及七张表各自的来源。工件目录不计入脏工作区判定，否则每次重跑都会把自己标记成脏的。

## 8. 旧引擎的删除

旧引擎的四个包（`engine`、`decision`、`officiating`、`evaluator`）与只服务它们的 `physics`、`semantics`、`invariants`、`test-support` 已从工作区删除，命令行只剩 `sim-core` 与 `validate-core`，`crates/validation` 只剩 `core_validation` 与 `mechanism_core`。`parquet`、`rusqlite`、`arrow` 三个依赖随之退出。

调试服务器只保留新内核：`/api/simulate` 只接受 `seed` 与 `scope`，`/api/session/*` 指向 `SessionService`，`/api/validation/*` 读 `validate-core` 的工件。`/api/studio` 与战术、名单面板已删除：新内核没有战术与 Play，这些面板没有可配置的对象。

旧引擎的设计记录保留在 `docs/decisions.md` 与 `docs/dev/cycles/`，作为历史，不再维护。

## 9. 已知缺口

- 防守只建模贴防与协防收缩，封盖、抢断、协防、卡位都只在概率里出现，倾向字段 `gamble_steal`、`block_aggressiveness`、`help_aggressiveness`、`physicality` 还没有独立结算可接。
- 十二个倾向字段中只有四个进入生产路径（出手、突破、掩护、传球）；其余八个在 `scripts/check_r6_tendency_wiring.py` 里登记为待接入。
- 无球跑位只覆盖空切与掩护，弱侧轮转与错位攻击尚未进入动作空间。
- 回合结算的篮板归属只按能力与位置加权，没有卡位动作。
- `validation.md` §9.2 的场景目录尚未移植，`crates/sim/tests` 只有一个集成测试文件。
- 内建名册的主客队属性相同，球队差异还没有进入引擎。
- 检查点级统计差值对照（原 `data/reference/stat_deltas.v1.json`）没有替代品；分布层由十七项准则承接。
