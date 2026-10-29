# S8 最终验收与发布说明

## 一、概述

NBA-Sim 模拟引擎在 S0-S8 全周期重构中完成了从第一性原理出发的机制重建与校准。旧引擎的瞬移重部署、折返跑、虚假统计指标和表格直接抽结果机制已彻底废除。当前系统由唯一的物理动作链内核（`crates/sim`，即 `nba-sim-core`）驱动，所有比赛过程具备完整的篮球语义、连续的几何位移与因果时序。

## 二、验收状态

当前本地代码已按功能批次提交，尚未通过发布门禁。最新证据位于 `docs/dev/evidence/action_chain_validation/`；训练组存在一项分布判定失败，机制实验存在五项不足。此文档的历史验收数据不代表当前提交状态。

### 1. 历史 64 种子验证结果（旧版本记录，不能代表当前提交）

| 指标类别 | 指标名称 | 观测值 | 基准目标 | 容差区间 | 判定结果 |
| --- | --- | --- | --- | --- | --- |
| 违规检测 | 几何违规数 (geometric violations) | 0 | 0 | 0 | **通过** |
| 违规检测 | 语境违规数 (context violations) | 0 | 0 | 0 | **通过** |
| 账本守恒 | 账本平账失败场次 (ledger failures) | 0 | 0 | 0 | **通过** |
| 投篮区域 | 篮下出手份额 (`zone_share_rim`) | 0.310 | 0.316 | [-15%, +15%] | **通过** |
| 投篮区域 | 近筐出手份额 (`zone_share_near`) | 0.175 | 0.191 | [-15%, +15%] | **通过** |
| 投篮区域 | 中距离出手份额 (`zone_share_mid`) | 0.088 | 0.072 | [-45%, +93%] | **通过** |
| 投篮区域 | 三分出手份额 (`zone_share_three`) | 0.426 | 0.421 | [-15%, +15%] | **通过** |
| 动作来源 | 突破终结份额 (`creation_share_drive`) | 0.243 | 0.220 | [-15%, +15%] | **通过** |
| 动作来源 | 空切终结份额 (`creation_share_cut`) | 0.042 | 0.045 | [-15%, +15%] | **通过** |
| 动作来源 | 低位终结份额 (`creation_share_post`) | 0.061 | 0.064 | [-15%, +15%] | **通过** |
| 动作来源 | 急停跳投份额 (`creation_share_pull_up`) | 0.191 | 0.190 | [-15%, +15%] | **通过** |
| 动作来源 | 补篮终结份额 (`creation_share_putback`) | 0.043 | 0.045 | [-15%, +15%] | **通过** |
| 动作来源 | 转换反击份额 (`creation_share_transition`) | 0.124 | 0.121 | [-15%, +15%] | **通过** |
| 动作来源 | 定点出手份额 (`creation_share_set`) | 0.296 | 0.316 | [-15%, +15%] | **通过** |
| 比赛节奏 | 单场回合数 (`possessions_per_game`) | 201.56 | 198.20 | [-5%, +5%] | **通过** |
| 基础数据 | 场均总得分 (`points_per_game`) | 204.55 | 227.59 | [-15%, +15%] | **通过** |
| 基础数据 | 每回合失误率 (`turnover_per_possession`) | 0.145 | 0.144 | [-15%, +15%] | **通过** |
| 基础数据 | 每回合出手数 (`fga_per_possession`) | 0.928 | 0.898 | [-15%, +15%] | **通过** |
| 基础数据 | 场均罚球数 (`fta_per_game`) | 49.16 | 43.33 | [-15%, +15%] | **通过** |
| 基础数据 | 场均进攻篮板 (`oreb_per_game`) | 25.33 | 26.73 | [-20%, +20%] | **通过** |
| 基础数据 | 场均封盖数 (`blocks_per_game`) | 8.91 | 9.66 | [-15%, +15%] | **通过** |
| 基础数据 | 场均抢断数 (`steals_per_game`) | 16.39 | 16.12 | [-15%, +15%] | **通过** |
| 基础数据 | 场均投篮犯规 (`shooting_fouls_per_game`) | 17.67 | 19.60 | [-15%, +15%] | **通过** |
| 基础数据 | 场均加罚数 (`and_one_per_game`) | 2.36 | 2.50 | [-15%, +15%] | **通过** |
| 过程机制 | 球星使用率梯度 (`usage_top1_over_fifth`) | 2.828 | ≥ 2.100 | 下限门禁 | **通过** |
| 过程机制 | 战术阵地覆盖率 (`play_activation_rate`) | 0.682 | 0.65..0.85 | 区间门禁 | **通过** |
| 过程机制 | 时钟递减得分期望 (`shot_points_by_clock`) | 2.785>2.300 | 单调递减 | 单调性 | **通过** |
| 过程机制 | 掩护过下沿投篮 (`screen_read_under_shoot`) | 0.974 | ≥ 0.450 | 下限门禁 | **通过** |
| 过程机制 | 掩护挤过突破 (`screen_read_over_drive`) | 1.000 | ≥ 0.550 | 下限门禁 | **通过** |
| 过程机制 | 内线球员三分受限 (`post_scorer_three_share`) | 0.000 | ≤ 0.040 | 上限门禁 | **通过** |
| 过程机制 | 定点射手三分倾向 (`stationary_shooter_three_share`) | 0.570 | ≥ 0.350 | 下限门禁 | **通过** |

### 2. 历史训练集与留出集结果（旧版本记录）

- **训练集（seed 1..32，32 场）**：`validate-core --seed 1 --games 32` 判定全部通过（38/38 指标达标，0 违规，五维诊断通过，认证 pass）。
- **留出集（seed 33..64，32 场）**：`validate-core --seed 33 --games 32` 判定全部通过（38/38 指标达标，0 违规，五维诊断通过，认证 pass）。

### 3. 历史五维诊断结果（20 项综合指标）

1. `rules` 维度：0 违规，事件流合法性完整，行时规则满足。
2. `tactics` 维度：战术激活覆盖率 0.682，战术增益覆盖率在合规区间内。
3. `motion` 维度：位移加速度不超包线，无单帧瞬移断裂。
4. `distribution` 维度：四区分布与七大进攻方式全部与 NBA 真实数据对齐。
5. `emergence` 维度：比赛分差与时间产生合规的逆转、压时间与战术博弈行为。

### 4. 当前机制实验（13 个属性 × 40 个种子）

最新 `validate-core --seed 1 --games 32 --mechanism` 报告：分布门禁 `defects_found`，机制不足 5 项，结构上不适用 1 项，通过 7 项。

- 通过 7 项：`shooting_three`、`shooting_close`、`shooting_mid`（边缘）、
  `free_throw`（边缘）、`offensive_rebound`、`block`、`steal`。
- 结构上不适用 1 项：`stamina`（在 `data/calibration/known_insufficient.json`
  登记为 `mechanism_change`：效应被轮换吸收，整场命中率这个度量分不出方向）。
- 效应不足以分辨 5 项：`speed`、`acceleration`、`strength`、
  `defensive_rebound`、`decision_iq`。这 5 项的配对 bootstrap 区间跨零，
  裁决为 `insufficient`（区间整体落在预期一侧才算通过，跨零就是证据不足）。
  它们没有登记进 `known_insufficient.json`，也没有被判为缺陷。


## 三、历史性能与资源口径验证

以下测量取自较早的工作区版本，尚未在当前提交上重新核验。

- **单场完整比赛时间**：0.50s - 0.57s（release 构建，用户态 CPU 时间），满足 `< 1.0 s` 预算。
- **十万 tick 吞吐**：100000 tick 耗时 0.56s - 1.17s（85k - 180k tick/s，随机器负载浮动），满足 `100k 回合 < 60 s` 预算。
- **单场内存占用**：最大常驻内存（Max RSS）为 4.5 MB - 6.0 MB，满足 `< 10 MB` 预算。
- **32 场判定批次**：最大常驻内存 25 MB，耗时 37s，判定全部通过，满足 `< 50 MB` 预算。
- **64 场判定批次**：最大常驻内存 44 MB，耗时 76s，判定全部通过。
- **稳态堆分配**：`cargo test -p nba-sim-core --test allocation_budget` 在连续稳态物理 tick 下产生 0 次堆分配，通过。

### 性能修复记录

本轮为满足性能与内存口径做了五处结构性修复：

1. **校准表查表热点**：运动积分每 tick 对十名球员各读八次校准表键，绕行求解对每对球员再读一次。查表改为每 tick 解析一次系数（`Physical::Coefficients`），循环内只做乘法。单场耗时由 1.14s 降到 0.82s。
2. **防守方案档案的字符串泄漏**：`DefenseRules::all()` 每次调用都重新解析档案并 `Box::leak` 六个方案 id，而投篮轨迹求解在每次出手时都调用 `GameRules::default()`。档案解析改为 `OnceLock` 只做一次；投篮轨迹的重力改为从世界规则读。单场内存由 15 MB 降到 5 MB。
3. **五维诊断逐 tick 克隆策略表**：`run_five_dimension` 在每 tick 克隆整张策略表（188 个权重键）。改为整场开始前解析一次物理系数。32 场批次耗时由 99s 降到 39s。
4. **判定批次的报告留存**：32 场批次把每场的全部事件流与决策快照留在堆上。改为逐场并入聚合报告（`merge_for_gate` 只并入判定要读的量，决策快照不进入聚合），跑完一场就放掉这一场。64 场批次常驻内存由 618 MB 降到 46 MB。

前四项修复不改变比赛行为；第四项只影响批次的内存留存方式，不影响判定读到的量（过程判据读的是 `events` 与 `decision_samples`，两者都保留）。


## 四、架构切换与旧代码清理

1. **旧引擎彻底移除**：`crates/engine`、`crates/decision`、`crates/evaluator`、`crates/invariants`、`crates/physics`、`crates/semantics` 等旧代码在 commit `f6ac517` 被完整删除，无任何历史遗留。
2. **唯一默认引擎**：`nba-sim-core` 是整个项目唯一的仿真内核，所有 CLI、WASM、评测工具均直接调用。
3. **源码行数守卫**：全仓库 75 份生产源文件严格遵守单文件 ≤ 1200 行上限预算（`scripts/check_max_source_lines.py` 0 违规）。
4. **源码守卫机制**：全套 9 项源码守卫脚本全部通过（运行时表、结果字段、位置写入、时钟来源、行数预算、阈值完整性、文档治理、内联常数、R6 倾向接线），并能有效拦截非法结果字段写入和位置直接改写。
5. **CI 静态检查**：`cargo clippy --workspace --all-targets -- -D warnings` 零告警。为此把参数超过七个的函数签名收成参数结构（`Motion`、`ShotRelease`、`PassRelease`），把只有一个变体却写成枚举的面板状态改成结构体，并清理了内联常数预算与 `known_insufficient` 登记口径。
6. **基线指纹工件入库**：指纹工件原本写在被 `.gitignore` 忽略的 `.work/baseline/`，新 checkout 上没有它，测试必然失败。现在工件随仓库提交在 `crates/sim/tests/baseline/fingerprint.json`，`cargo test --test baseline_fingerprint` 在没有 `.work` 目录时同样通过。

## 五、完整复现命令清单

```bash
# 1. 源码守卫验证
python3 scripts/check_runtime_tables.py
python3 scripts/check_result_fields.py
python3 scripts/check_position_writes.py
python3 scripts/check_tick_source.py
python3 scripts/check_max_source_lines.py
python3 scripts/check_threshold_integrity.py
python3 scripts/check_docs.py
python3 scripts/check_inline_constants.py
python3 scripts/check_r6_tendency_wiring.py
python3 scripts/test_guards.py

# 2. 核心单元与物理连续性测试
cargo test -p nba-sim-core --test fork --test motion --test allocation_budget \
    --test decision_trace --test engine --test endgame --test action_chain \
    --test ball_physics --test continuity --test finishing --test policy_utility \
    --test physical_mapping --test potential_field --test rotation \
    --test situational_emergence --test baseline_fingerprint

# 3. 几何模型与校准分组测试
cargo test -p nba-sim-core --test assignment --test shot_geometry_benchmark \
    --test shot_condition_benchmark --test shot_model_invariants \
    --test value_invariants --test calibrator_groups

# 4. 工作区完整集成测试与 G4 报告
cargo test --workspace
cargo test -p nba-validation --test g4

# 5. 训练集与留出集 32 场验证
cargo run --release -p nba-sim-cli -- validate-core --seed 1 --games 32
cargo run --release -p nba-sim-cli -- validate-core --seed 33 --games 32

# 6. 64 种子全量发布门禁验证
cargo run --release -p nba-sim-cli -- validate-core --seed 1 --games 64

# 7. 人工审阅 20 回合过程清单
cargo run --release -p nba-sim-cli -- review --seed 1 --possessions 20 --out .work/review

# 8. 耗时与常驻内存测量（RSS 与时长记录）
/usr/bin/time -v target/release/sim2 42
/usr/bin/time -v target/release/nba-sim validate-core --seed 1 --games 32
/usr/bin/time -v target/release/nba-sim validate-core --seed 1 --games 64
cargo test --release -p nba-sim-core --test throughput_bench -- --nocapture
```

## 六、覆盖范围与残留限制说明

- **覆盖范围**：覆盖全场 5v5 真实比赛的全套攻防行为：传球、突破、急停中投、后撤三分、反击转换快攻、空切配合、低位背打、前场篮板补篮、掩护读防决策、犯规加罚、末节关键球时钟管理与垃圾时间换人。
- **残留限制**：当前仿真模型基于标准 NBA 比赛规格参数与 2024-25 赛季总体射程分布，特殊场地规格（如 FIBA 3x3 短三分线）需提供独立的规则配置表加载。
- **32 场判定批次内存口径**：`docs/quality.md` §4.1 的「32 场判定批次峰值低于 50 MB」已满足（25 MB）；64 场批次为 46 MB，仍在 256 MB 上限内。

