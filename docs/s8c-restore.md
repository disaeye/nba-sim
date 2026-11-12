# 参数分组与出手模型

## 策略参数

`data/calibration/policy.json` 的 `weights` 保存无量纲响应系数，`geometry`
保存英尺与秒类几何量，`bands` 保存判定带宽，`process` 保存过程判据的设计
边界，`shot_distance` 保存出手类型 × 是否三分 × 距离的实测命中率曲线。
最终参数向量由 `scripts/final_validation.sh` 的固定 16 场门禁与认证工件
核对，逐条观测值见 `docs/dev/evidence/gap_closure_cycle/gate.txt`。

出手模型由同一套 `ShotModel` 给出四区基准命中率、技能幅度、罚球基准与
有效筐口；瞄准误差由基准命中率反解。影响行为的系数都从参数表读取。

## 几何数据

- `data/calibration/shot_distance.json`：出手类型 × 是否三分 × 距离的实测
  命中率曲线，来自冻结的 `shot_make` 表。这是「距离衰减」组的实测数据：
  投篮模型用它把目标命中率换成瞄准误差，因此模型预测的命中率等于实测值。
- `shot_distance` 作为运行时表加载（`RUNTIME_TABLES`），生产代码不读
  验证表 `shot_make`。

## 参数分组

`scripts/calibrate/fit_policy.py` 把可拟合的参数分成七组，每组有独立的
键集合、独立的取值范围和独立的拟合入口（`--group <名字>`）：

| 组 | 键 | 量纲 |
| --- | --- | --- |
| `aim_error` | `shot_aim_sigma_scale`、`shot_aim_movement_sensitivity` | 无量纲 |
| `distance_decay` | `shot_distance_decay_three_scale_ft`、`shot_distance_decay_three_power`、`shot_aim_distance_scale` | 英尺 |
| `contest` | `shot_quality_contest_sensitivity` | 无量纲 |
| `shot_phase` | `shot_phase_sigma_*`、`shot_gather_sigma_scale` | 无量纲 |
| `decision_temperature` | `temperature_min`、`temperature_max`、`temperature_fatigue_discount` | 无量纲 |
| `pace` | `seconds_per_possession`、`dribble_setup`、`shoot_early_slack` | 秒 |
| `behavior` | `shoot_catch_bonus`、`shoot_off_dribble_penalty`、`drive_base`、`post_base`、`cut_base`、`pass_base`、`pass_open`、`epv_bonus_scale` | 无量纲 |

三条约束由 `crates/sim/tests/calibrator_groups.rs` 守住：

1. 一个参数键只属于一个组：跨组串写在结构上不可能。
2. 每个键有自己的范围：组内也不会出现「同一个数值同时表示两种量纲」。
3. 时间阈值（秒）、速度尺度（英尺每秒）、距离尺度（英尺）分属不同组：
   校准器不可能把同一个数值同时写进这三种量纲。

校准器不再有全局的 `min(max(value, 0.02), 6.0)` 夹取。每个键的范围由
它自己的物理量纲决定。

## 物理概率下限

- 超远距离的三分命中目标按距离连续衰减到零：`three_distance_decay` 用
  `exp(-((d - arc)/L)^q)`，没有任何下界；`MIN_LOG_ARGUMENT` 只出现在
  对数与除法里（`shot_quality.rs`）。
- 投篮模型里所有概率的夹取都是 `[0, 1]`，没有 `0.02` 这样的物理下限。

## 末段三分

`endgame_three_pressure` 按比分、剩余时间与暂停情况给出处境加成：处于比分劣势、
剩余时间少于 `endgame_three_required_seconds`、分差在
`endgame_hold_margin_max` 以内、且本方没有暂停可用时成立。常规出手没有
距离上限，末段超远抛投是处境驱动的选择。

## 出手与空间模型

发球队员从当前位置走到本方底线发球点，发球时间按走完的距离延长。持球
候选站位来自阵型给出的弧顶持球位，站位外扩限制在三分线内侧。绕行保留
朝目标方向的分量，回避方向一经选定不再改变。持球人变化或对位防守人离
持球人过远时，防守职责层重算对位。

高吊球的飞行时间由切入人跑到接球点的时间决定，接球位置就是他真能到达的
位置。瞄准误差只作为接球位置的变化量，抛体由出手计划给出，模型与执行层共用同一
条轨迹。干扰修正按每类出手基准干扰计算，只对强于基准的干扰加罚。被犯规
的出手计入 `FGA`，统计口径与出手事实一致。

## 参数分组的对应关系

`scripts/calibrate/fit_policy.py` 的七组对应上面的机制：

- `aim_error`：瞄准误差总尺度与移动带来的额外误差。
- `distance_decay`：三分线外的衰减长度与幂次、两分区域的距离尺度。
- `contest`：干扰相对各出手类型基准干扰的放大系数。
- `shot_phase`：各持球阶段对瞄准误差的倍率。
- `decision_temperature`：逆温度区间与体能折扣。
- `pace`：回合时长与出手宽限（秒）。
- `behavior`：各候选动作的倾向权重。

运行时参数包括 `shot_distance.json`（实测距离曲线）、
`shot_quality_base_free_throw` / `shot_quality_skill_free_throw`（罚球基准）、
`shot_contest_reference_rim` / `_near` / `_perimeter`（各出手类型的基准干扰）、
`shot_phase_sigma_*`（各阶段倍率）、`hold_continuation_scale`（持球时钟期权）、
`break_speed_edge_ftps` / `break_max_runners`（快攻跑位的速度优势门槛与人数）、
`cut_open_defender_ft`（空位要求的最小防守距离）。

## 出手类型与进攻分类的对应

`ShotKind::creation()` 决定一次出手在进攻分类里的归属，档案的
`kind_by_creation` 是口径依据：

| 分类 | 出手类型 |
| --- | --- |
| set | CatchShoot、Dunk、Layup |
| pull_up | PullUp、StepBack、Floater |
| post | Turnaround、Hook |
| drive | DrivingLayup、DrivingDunk、ReverseLayup |
| cut | CuttingLayup、CuttingDunk |
| transition | RunningLayup、RunningDunk、RunningJump、AlleyOop |
| putback | Putback |

规则覆盖的判据跟着分类走，但每条判据要看实际的前提：只有真正的
CatchShoot 才需要「前面有一次传球」，只有 DrivingLayup 一类才需要
「前面有一次突破」。收球步之后的篮下完成不属于任何一种前提，因此不被
判为违例。

## 校准取值与复现路径

最终策略参数保存在 `data/calibration/policy.json`。`scripts/calibrate/
fit_policy.py` 的七组分工与取值范围由 `crates/sim/tests/calibrator_groups.rs`
守住；影响行为的系数从参数表读取。复现时运行 `scripts/final_validation.sh`，
读取 `docs/dev/evidence/gap_closure_cycle/gate.txt` 的逐条判定、
`docs/dev/evidence/action_chain_validation/` 的 NBA 认证、
`docs/dev/evidence/fiba_validation/` 的 FIBA 认证与
`.work/evidence/final-validation.summary` 的逐项退出码。

### 出手类型与进攻分类的对应（`ShotKind::creation()`）

`CatchShoot` / `Dunk` / `Layup` 归入 set，`PullUp` / `StepBack` 归入
pull_up，`DrivingLayup` / `DrivingDunk` / `ReverseLayup` / `Floater` 归入
drive，`Hook` / `Turnaround` 归入 post，`CuttingLayup` / `CuttingDunk` 归入
cut，`RunningLayup` / `RunningDunk` / `RunningJump` / `AlleyOop` 归入
transition，`Putback` 归入 putback。

