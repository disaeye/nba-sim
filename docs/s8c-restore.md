# S8c 恢复记录与参数分组

## 恢复的策略参数

失败拟合（此前 `fit_policy.py` 的旧版单组坐标下降）把
`data/calibration/policy.json` 的权重写成了 best loss 0.2649 的那组取值，
其留出集表现是 43.1 分、三分命中率 0.003。

恢复来源：`.work/s8c-backup/policy.json`（该目录在本次修复开始前保存的
失败拟合之前的快照）。恢复方式是用文件编辑工具把
`data/calibration/policy.json` 的权重写回快照里的取值，随后用
`diff` 逐字节确认两份文件一致。

保留的几何修复（未随失败拟合一起回退）：

- `shot_quality_base_rim` / `_near` / `_mid` / `_three`：四区基准命中率。
- `shot_quality_skill_rim` / `_near` / `_mid` / `_three`：技能幅度。
- `shot_quality_base_free_throw` / `shot_quality_skill_free_throw`：罚球基准。
- `free_throw_aim_sigma`：删除。罚球改用同一个 `ShotModel` 的有效筐口与
  释放几何，瞄准误差由罚球基准命中率反解，不再有独立的固定值。

新增的几何数据（不属于失败拟合的权重）：

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

`endgame_three_pressure` 按比分、剩余时间与暂停情况给出处境加成：落后、
剩余时间少于 `endgame_three_required_seconds`、分差在
`endgame_hold_margin_max` 以内、且本方没有暂停可用时成立。常规出手没有
距离上限，末段超远抛投是处境驱动的选择。

## 本轮修复的机制缺陷

按时间顺序，每个缺陷都改变了比赛的样子：

1. **发球瞬移**：`begin_inbound` 把发球队员直接放到中线发球点，位置瞬移
   二十到三十英尺。现在发球点改到本方底线，发球队员自己走过去，发球时间
   按走完的距离延长。
2. **持球目标指向篮筐**：`Dribble` 候选的落点是篮筐，于是每次「拿着球」
   都变成一次无意识的冲筐，进攻全程贴在篮下，出手区域几乎全是篮下。
   现在落点是弧顶持球位。
3. **弧顶持球位在三分线外**：`top_of_key` 写死在 (64, 25)，离筐 25 英尺，
   于是持球人的每次急停跳投都是三分球。现在取自阵型的持球点，落在弧内。
4. **站位外扩没有上限**：`spot_target` 的外扩可以把球员推到三分线外，
   现在上限是三分线内侧 0.5 英尺。
5. **绕行只沿切向**：`steer_target` 返回纯切向的点，突破与空切都会绕着
   防守人停在半路。现在允许前进的球员保留朝目标方向的分量，回避方向一经
   选定不再改变。
6. **对位不随持球人变化**：传球之后防守人还守着上一个对位，接球人面前
   常常没有防守人，投篮命中率因此高得不像真实比赛。现在对位在持球人变化
   或对位防守人离持球人过远时重算，并且职责层立刻换人。
7. **空切的高吊球不等切入人**：高吊球按平直传球的速度飞到切入人当前
   位置，切入人还没到球就落了。现在高吊球的飞行时间由切入人跑到接球点的
   时间决定，落点就是他真能到达的位置。
8. **投篮瞄准误差重新瞄准**：瞄准误差原本改变球的落点，飞行时长也跟着
   变，于是模型预测的入射角与实际飞行的入射角不是一回事。现在瞄准误差
   只作为落点偏移，抛体由出手计划给出，模型与执行层共用同一条轨迹。
9. **干扰被重复计入**：实测曲线本身含有联盟平均干扰，模型又从零开始加
   罚。现在干扰修正按每类出手的基准干扰计算，只对强于基准的干扰加罚。
10. **被犯规的出手不计入 FGA**：NBA 统计里被犯规的出手仍是一次出手，
    引擎把它排除在 FGA 之外，出手数与命中率同时失真。

## 参数分组的落点

`scripts/calibrate/fit_policy.py` 的七组对应上面的机制：

- `aim_error`：瞄准误差总尺度与移动带来的额外误差。
- `distance_decay`：三分线外的衰减长度与幂次、两分区域的距离尺度。
- `contest`：干扰相对各出手类型基准干扰的放大系数。
- `shot_phase`：各持球阶段对瞄准误差的倍率。
- `decision_temperature`：逆温度区间与体能折扣。
- `pace`：回合时长与出手宽限（秒）。
- `behavior`：各候选动作的偏好权重。

模型新增的运行时参数：`shot_distance.json`（实测距离曲线）、
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

## 本轮的校准取值与复现路径

发布验收所用的策略参数写回 `data/calibration/policy.json` 的 `weights`，
取值来源与复现路径如下：

- 起点是本文件第一节记录的恢复快照（`.work/s8c-backup/policy.json`，即失败
  拟合之前的几何成立状态）。
- 之后的每一处改动都只动 `weights` 里的一个键，值与改动理由记录在
  `docs/s8_release_acceptance.md` 的验收表里；校准器
  `scripts/calibrate/fit_policy.py` 的七组分工与取值范围不变。
- 复现方式：从恢复快照出发，按 `docs/s8_release_acceptance.md` 第一节的
  指标表逐项核对 `data/calibration/policy.json` 的当前 `weights`，再跑
  `validate-core --seed 1 --games 32` 与 `--seed 33 --games 32` 复核。

### 出场类型与进攻分类的对应（`ShotKind::creation()`）

`CatchShoot` / `Dunk` / `Layup` 归入 set，`PullUp` / `StepBack` 归入
pull_up，`DrivingLayup` / `DrivingDunk` / `ReverseLayup` / `Floater` 归入
drive，`Hook` / `Turnaround` 归入 post，`CuttingLayup` / `CuttingDunk` 归入
cut，`RunningLayup` / `RunningDunk` / `RunningJump` / `AlleyOop` 归入
transition，`Putback` 归入 putback。

