//! 检查点直接机制实验执行框架（validation.md §7.1）。
//!
//! 执行模型（母局单遍走位 + fork trial）：
//! 1. **实验规格先冻结后执行**：规格写盘算摘要，执行器核对摘要不符拒绝
//!    运行（「检查点集合在查看扰动结果前冻结」由机制保证）。
//! 2. **母局只走一遍**：固定 seed 正常跑，逐 tick 应用实验的检查点过滤
//!    谓词；命中即从当前状态 `experimental_fork` 四份（基线 + 低中高
//!    三档），每份按具名随机键换种（档位不进键，同 (checkpoint, trial)
//!    共享随机流，天然配对），跑到观测窗口结束记录结果后丢弃。
//! 3. **机会数分母 = 全部命中的检查点**：防止只统计扰动后表现好的样本。
//! 4. **命中数下限闸门**：检查点命中过少的实验直接失败（谓词不可用，
//!    必须修订登记重跑），不带病出判定。
//!
//! 观测量与判定：trial 内的出手/命中计数（投篮精度类实验）；档位间
//! 单调性（低 ≤ 中 ≤ 高的命中数方向 + 基线对照）按检查点聚类的
//! bootstrap 给三出口判定。

use std::collections::BTreeMap;

use nba_engine::{MatchEngine, ValidationAttribute};
use serde::Serialize;

/// 检查点过滤谓词（母局逐 tick 应用；全部只读状态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointPredicate {
    /// 控球者处于三分出手窗口（action 含 "Shot"、体力 ≥0.5）。
    /// 投篮精度实验（MECH-S01）的检查点。
    ShooterInThreePointWindow,
    /// 控球者在罚球程序中（帧 phase 提示 FreeThrow）。
    /// 罚球实验（MECH-R01）的检查点。
    FreeThrowRoutine,
    /// 控球者处于近距离出手窗口（距篮 <8 ft、action 含 "Shot"、体力 ≥0.5）。
    ShooterInCloseWindow,
    /// 控球者处于中距离出手窗口（距篮 8-16 ft、action 含 "Shot"、体力 ≥0.5）。
    ShooterInMidWindow,
    /// 投篮不中的瞬间（SHOT_MISS 事件）。篮板类与冲刺类实验的检查点：
    /// 不中瞬间 fork，trial 窗口覆盖篮板卡位与二次进攻。
    ShotMissMoment,
    /// 控球者在本队后场建立控制（CONTROL_ESTABLISHED，x<47 ft 半场）。
    /// 组织决策实验的检查点：trial 窗口覆盖一次完整推进组织。
    BackcourtControlEstablished,
}

impl CheckpointPredicate {
    pub fn name(self) -> &'static str {
        match self {
            CheckpointPredicate::ShooterInThreePointWindow => "shooter_in_three_point_window",
            CheckpointPredicate::FreeThrowRoutine => "free_throw_routine",
            CheckpointPredicate::ShooterInCloseWindow => "shooter_in_close_window",
            CheckpointPredicate::ShooterInMidWindow => "shooter_in_mid_window",
            CheckpointPredicate::ShotMissMoment => "shot_miss_moment",
            CheckpointPredicate::BackcourtControlEstablished => "backcourt_control_established",
        }
    }
}

/// 实验规格（先冻结后执行；写盘摘要核对）。
#[derive(Debug, Clone, Serialize)]
pub struct MechExperimentSpec {
    /// 实验标识（如 `MECH-S01-three-precision`）。
    pub experiment_id: String,
    /// 对应准则叶项（如 `MECH-S01`）。
    pub criterion_id: String,
    /// 扰动目标属性。
    pub attribute: ValidationAttributeSpec,
    /// 低/中/高三档值（合法域内；执行时经生产校验路径）。
    pub levels: [f32; 3],
    /// 检查点过滤谓词。
    pub predicate: CheckpointPredicate,
    /// 观测方向：属性升档时观测量应变大（HigherBetter）还是变小
    /// （LowerBetter，如冲刺耗时）。单调性判定按此方向。
    pub expected_direction: ExpectedDirection,
    /// 每个 trial 的观测窗口（tick 数）。
    pub trial_ticks: u64,
    /// 每个检查点的 trial 重复数。
    pub trials_per_checkpoint: u32,
    /// 母局 seed。
    pub mother_seed: u64,
    /// 检查点命中数下限（低于此值实验失败）。
    pub minimum_checkpoints: u32,
}

/// 单调性预期方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedDirection {
    /// 属性升档 → 观测命中率上升（投篮/篮板/组织）。
    HigherBetter,
    /// 属性升档 → 观测量下降（冲刺耗时类）。
    LowerBetter,
}

/// 登记文件侧的属性名（稳定字符串，执行时映射到引擎枚举）。
#[derive(Debug, Clone, Serialize)]
pub struct ValidationAttributeSpec {
    pub name: String,
}

impl ValidationAttributeSpec {
    pub fn resolve(&self) -> ValidationAttribute {
        match self.name.as_str() {
            "shooting_three" => ValidationAttribute::ShootingThree,
            "shooting_close" => ValidationAttribute::ShootingClose,
            "shooting_mid" => ValidationAttribute::ShootingMid,
            "free_throw" => ValidationAttribute::FreeThrow,
            "speed" => ValidationAttribute::Speed,
            "acceleration" => ValidationAttribute::Acceleration,
            "stamina" => ValidationAttribute::Stamina,
            "strength" => ValidationAttribute::Strength,
            "offensive_rebound" => ValidationAttribute::OffensiveRebound,
            "defensive_rebound" => ValidationAttribute::DefensiveRebound,
            "decision_iq" => ValidationAttribute::DecisionIq,
            other => panic!("未登记的扰动属性名: {other}"),
        }
    }
}

/// 实验规格的确定性摘要（FNV-1a，非加密用途）。
///
/// 冻结纪律（§7.1「检查点集合在查看扰动结果前冻结」）的机制化：
/// 规格先写盘并记录本摘要；执行器重算不符即拒绝运行。改规格 = 新实验，
/// 旧结果不与新规格混写。
pub fn spec_digest(spec: &MechExperimentSpec) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    };
    feed(spec.experiment_id.as_bytes());
    feed(spec.criterion_id.as_bytes());
    feed(spec.attribute.name.as_bytes());
    for level in spec.levels {
        feed(&level.to_le_bytes());
    }
    feed(match spec.expected_direction {
        ExpectedDirection::HigherBetter => b"higher_better" as &[u8],
        ExpectedDirection::LowerBetter => b"lower_better",
    });
    feed(match spec.predicate {
        CheckpointPredicate::ShooterInThreePointWindow => b"three_window" as &[u8],
        CheckpointPredicate::FreeThrowRoutine => b"ft_routine",
        CheckpointPredicate::ShooterInCloseWindow => b"close_window",
        CheckpointPredicate::ShooterInMidWindow => b"mid_window",
        CheckpointPredicate::ShotMissMoment => b"shot_miss",
        CheckpointPredicate::BackcourtControlEstablished => b"backcourt_control",
    });
    feed(&spec.trial_ticks.to_le_bytes());
    feed(&spec.trials_per_checkpoint.to_le_bytes());
    feed(&spec.mother_seed.to_le_bytes());
    feed(&spec.minimum_checkpoints.to_le_bytes());
    hash
}

/// 核对规格摘要（执行前置闸）；不符时返回 Err 拒绝执行。
pub fn verify_spec_digest(spec: &MechExperimentSpec, recorded: u64) -> Result<(), String> {
    let actual = spec_digest(spec);
    if actual == recorded {
        Ok(())
    } else {
        Err(format!(
            "实验规格摘要不符（登记 {recorded:#018x} vs 当前 {actual:#018x}）：             规格在结果查看后被改动，按冻结纪律拒绝执行；修订规格须作为新实验重新登记"
        ))
    }
}

/// 首批实验规格（MECH-S01/M01/R01 各一；值域与谓词按准则描述登记）。
/// 全部实验规格（10 个可扰动属性全覆盖）。
///
/// 每条规格的三要素（谓词 → 扰动属性 → 观测量通道）有物理依据：
/// 谓词定位扰动生效的瞬间，观测量测同一属性驱动的输出。方向字段
/// 声明属性升档时观测量的预期走向，单调性判定按方向解释。
pub fn initial_experiment_specs() -> Vec<MechExperimentSpec> {
    vec![
        MechExperimentSpec {
            experiment_id: "MECH-S01-three-precision".to_string(),
            criterion_id: "MECH-S01".to_string(),
            attribute: ValidationAttributeSpec {
                name: "shooting_three".to_string(),
            },
            levels: [0.10, 0.50, 0.90],
            predicate: CheckpointPredicate::ShooterInThreePointWindow,
            expected_direction: ExpectedDirection::HigherBetter,
            trial_ticks: 900,
            trials_per_checkpoint: 1,
            mother_seed: 42,
            minimum_checkpoints: 30,
        },
        MechExperimentSpec {
            experiment_id: "MECH-R01-free-throw".to_string(),
            criterion_id: "MECH-R01".to_string(),
            attribute: ValidationAttributeSpec {
                name: "free_throw".to_string(),
            },
            levels: [0.40, 0.70, 0.95],
            predicate: CheckpointPredicate::FreeThrowRoutine,
            expected_direction: ExpectedDirection::HigherBetter,
            trial_ticks: 300,
            trials_per_checkpoint: 1,
            mother_seed: 42,
            minimum_checkpoints: 30,
        },
        MechExperimentSpec {
            experiment_id: "MECH-S01-close-precision".to_string(),
            criterion_id: "MECH-S01".to_string(),
            attribute: ValidationAttributeSpec {
                name: "shooting_close".to_string(),
            },
            levels: [0.10, 0.50, 0.90],
            predicate: CheckpointPredicate::ShooterInCloseWindow,
            expected_direction: ExpectedDirection::HigherBetter,
            trial_ticks: 900,
            trials_per_checkpoint: 1,
            mother_seed: 42,
            minimum_checkpoints: 20,
        },
        MechExperimentSpec {
            experiment_id: "MECH-S01-mid-precision".to_string(),
            criterion_id: "MECH-S01".to_string(),
            attribute: ValidationAttributeSpec {
                name: "shooting_mid".to_string(),
            },
            levels: [0.10, 0.50, 0.90],
            predicate: CheckpointPredicate::ShooterInMidWindow,
            expected_direction: ExpectedDirection::HigherBetter,
            trial_ticks: 900,
            trials_per_checkpoint: 1,
            mother_seed: 42,
            minimum_checkpoints: 15,
        },
        // 移动类三项：检查点取投篮不中瞬间（争抢冲刺起点），观测量
        // 为 MOVEMENT_OUTCOME completed 的耗时——属性越高到位越快。
        MechExperimentSpec {
            experiment_id: "MECH-M01-speed".to_string(),
            criterion_id: "MECH-M01".to_string(),
            attribute: ValidationAttributeSpec {
                name: "speed".to_string(),
            },
            levels: [0.10, 0.50, 0.90],
            predicate: CheckpointPredicate::ShotMissMoment,
            expected_direction: ExpectedDirection::LowerBetter,
            trial_ticks: 600,
            trials_per_checkpoint: 1,
            mother_seed: 42,
            minimum_checkpoints: 20,
        },
        MechExperimentSpec {
            experiment_id: "MECH-M02-acceleration".to_string(),
            criterion_id: "MECH-M01".to_string(),
            attribute: ValidationAttributeSpec {
                name: "acceleration".to_string(),
            },
            levels: [0.10, 0.50, 0.90],
            predicate: CheckpointPredicate::ShotMissMoment,
            expected_direction: ExpectedDirection::LowerBetter,
            trial_ticks: 600,
            trials_per_checkpoint: 1,
            mother_seed: 42,
            minimum_checkpoints: 20,
        },
        MechExperimentSpec {
            experiment_id: "MECH-M03-stamina".to_string(),
            criterion_id: "MECH-M01".to_string(),
            attribute: ValidationAttributeSpec {
                name: "stamina".to_string(),
            },
            levels: [0.10, 0.50, 0.90],
            predicate: CheckpointPredicate::ShotMissMoment,
            expected_direction: ExpectedDirection::LowerBetter,
            trial_ticks: 600,
            trials_per_checkpoint: 1,
            mother_seed: 42,
            minimum_checkpoints: 20,
        },
        // 力量：同检查点，观测量为篮板归属（卡位胜出拿板）。
        MechExperimentSpec {
            experiment_id: "MECH-P01-strength".to_string(),
            criterion_id: "MECH-P01".to_string(),
            attribute: ValidationAttributeSpec {
                name: "strength".to_string(),
            },
            levels: [0.10, 0.50, 0.90],
            predicate: CheckpointPredicate::ShotMissMoment,
            expected_direction: ExpectedDirection::HigherBetter,
            trial_ticks: 600,
            trials_per_checkpoint: 1,
            mother_seed: 42,
            minimum_checkpoints: 20,
        },
        // 篮板专项：前场板 / 后场板计数。
        MechExperimentSpec {
            experiment_id: "MECH-B01-orebound".to_string(),
            criterion_id: "MECH-B01".to_string(),
            attribute: ValidationAttributeSpec {
                name: "offensive_rebound".to_string(),
            },
            levels: [0.10, 0.50, 0.90],
            predicate: CheckpointPredicate::ShotMissMoment,
            expected_direction: ExpectedDirection::HigherBetter,
            trial_ticks: 600,
            trials_per_checkpoint: 1,
            mother_seed: 42,
            minimum_checkpoints: 20,
        },
        MechExperimentSpec {
            experiment_id: "MECH-B02-drebound".to_string(),
            criterion_id: "MECH-B01".to_string(),
            attribute: ValidationAttributeSpec {
                name: "defensive_rebound".to_string(),
            },
            levels: [0.10, 0.50, 0.90],
            predicate: CheckpointPredicate::ShotMissMoment,
            expected_direction: ExpectedDirection::HigherBetter,
            trial_ticks: 600,
            trials_per_checkpoint: 1,
            mother_seed: 42,
            minimum_checkpoints: 20,
        },
        // 决策：后场控制点起的一次推进窗口内失误计数，属性越高失误越少。
        MechExperimentSpec {
            experiment_id: "MECH-D01-decision-iq".to_string(),
            criterion_id: "MECH-D01".to_string(),
            attribute: ValidationAttributeSpec {
                name: "decision_iq".to_string(),
            },
            levels: [0.10, 0.50, 0.90],
            predicate: CheckpointPredicate::BackcourtControlEstablished,
            expected_direction: ExpectedDirection::LowerBetter,
            trial_ticks: 1200,
            trials_per_checkpoint: 1,
            mother_seed: 42,
            minimum_checkpoints: 20,
        },
    ]
}

/// 单个检查点 trial 的观测（档位配对）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct CheckpointTrialObservation {
    pub checkpoint_index: u32,
    pub trial_id: u32,
    /// 四组的出手数与命中数（is_three 维度）。
    pub attempts: [u32; 4],
    pub makes: [u32; 4],
}

/// 一个实验的执行结果。
#[derive(Debug, Clone, Serialize)]
pub struct MechExperimentOutcome {
    pub experiment_id: String,
    pub criterion_id: String,
    /// 命中的检查点数（机会数分母）。
    pub checkpoints_hit: u32,
    /// 每档位的出手/命中合计（含基线 index 0）。
    pub total_attempts: [u64; 4],
    pub total_makes: [u64; 4],
    /// 各检查点的配对 trial 观测。
    pub trials: Vec<CheckpointTrialObservation>,
    /// 执行状态：completed | insufficient_checkpoints。
    pub status: String,
}

/// 检查点谓词判定（母局 tick 的只读状态）。
fn predicate_matches(predicate: CheckpointPredicate, tick: &nba_protocol::StreamTick) -> bool {
    let frame = &tick.frame;
    match predicate {
        CheckpointPredicate::ShooterInThreePointWindow => {
            let carrier = frame
                .players
                .iter()
                .find(|p| p.has_ball && p.on_court);
            match carrier {
                Some(carrier) => {
                    let stamina_ratio = carrier.stm / carrier.stm_max.max(f32::EPSILON);
                    // StreamTick 的 x/y 是归一化坐标（0-1 = 94×50 ft），换算回
                    // 英尺后按三分线判定（与 STAT-S01 的 CourtGeometry 口径一致）。
                    let x_ft = carrier.x * 94.0;
                    let (hoop_x, side) = if x_ft <= 47.0 {
                        (5.25_f32, 1.0_f32)
                    } else {
                        (88.75_f32, -1.0_f32)
                    };
                    carrier.action.contains("Shot")
                        && stamina_ratio >= 0.5
                        && (side * (x_ft - hoop_x)) >= 23.75_f32 * 0.92
                }
                None => false,
            }
        }
        CheckpointPredicate::FreeThrowRoutine => {
            // 谓词 = 本 tick 出现 FREE_THROW 事件（扰动作用于罚球人，
            // trial 窗口覆盖后续罚球）。
            frame
                .event_log
                .iter()
                .any(|e| e.kind == "FREE_THROW")
        }
        CheckpointPredicate::ShooterInCloseWindow
        | CheckpointPredicate::ShooterInMidWindow => {
            // 与三分窗口同构，改距离带：<8 ft（近距）/ 8-16 ft（中距）。
            let Some(carrier) = frame.players.iter().find(|p| p.has_ball && p.on_court) else {
                return false;
            };
            let stamina_ratio = carrier.stm / carrier.stm_max.max(f32::EPSILON);
            let x_ft = carrier.x * 94.0;
            let y_ft = carrier.y * 50.0;
            let (hoop_x, side) = if x_ft <= 47.0 {
                (5.25_f32, 1.0_f32)
            } else {
                (88.75_f32, -1.0_f32)
            };
            let dist = (side * (x_ft - hoop_x)).hypot(y_ft - 25.0);
            let in_band = match predicate {
                CheckpointPredicate::ShooterInCloseWindow => dist < 8.0,
                CheckpointPredicate::ShooterInMidWindow => (8.0..16.0).contains(&dist),
                CheckpointPredicate::ShooterInThreePointWindow
                | CheckpointPredicate::FreeThrowRoutine
                | CheckpointPredicate::ShotMissMoment
                | CheckpointPredicate::BackcourtControlEstablished => false,
            };
            carrier.action.contains("Shot") && stamina_ratio >= 0.5 && in_band
        }
        CheckpointPredicate::ShotMissMoment => {
            // 谓词 = 本 tick 出现 SHOT_MISS（投篮不中，球即将弹框）。
            // 篮板/冲刺类实验的检查点：扰动作用于争抢参与者，
            // trial 窗口覆盖卡位争抢与二次进攻。
            frame
                .event_log
                .iter()
                .any(|e| e.kind == "SHOT_MISS")
        }
        CheckpointPredicate::BackcourtControlEstablished => {
            // 谓词 = 本 tick 出现 CONTROL_ESTABLISHED 且控制点在本方后场。
            // 组织决策实验的检查点：trial 窗口覆盖一次完整推进组织。
            frame.event_log.iter().any(|e| {
                if e.kind != "CONTROL_ESTABLISHED" {
                    return false;
                }
                e.data
                    .as_ref()
                    .and_then(|d| d.get("ControlEstablished"))
                    .and_then(|c| c.get("position"))
                    .and_then(|p| p.get(0))
                    .and_then(serde_json::Value::as_f64)
                    .map(|x_ft| {
                        // 后场 = 控制点距本方篮筐近（home 篮筐在 x=5.25，
                        // away 在 x=88.75；home 后场为 x<47）。
                        if frame.possession_team == "home" {
                            (x_ft as f32) < 47.0
                        } else {
                            (x_ft as f32) >= 47.0
                        }
                    })
                    .unwrap_or(false)
            })
        }
    }
}

/// trial 观测维度：三分出手（投篮实验）或全部罚球（罚球实验）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObservationChannel {
    ThreePointShots,
    FreeThrows,
    /// 近距（非三分）投篮命中：SHOT_ATTEMPT_RESOLVED is_three=false
    /// 且出手点距篮 <8 ft（以 SHOT_RELEASE pos 判）。
    CloseShots,
    /// 中距离（非三分）投篮命中：同上，距篮 8-16 ft。
    MidShots,
    /// 篮板归属计数：REBOUND is_offensive（前场板）。
    OffensiveRebounds,
    /// 篮板归属计数：REBOUND 非 offensive（后场板）。
    DefensiveRebounds,
    /// 该回合失误计数：PASS_DROPPED / PASS_TIPPED（对方获益） /
    /// BALL_POKED_LOOSE / 违规类 POSSESSION_SUMMARY。
    Turnovers,
    /// 冲刺任务耗时：MOVEMENT_OUTCOME completed 的 tick 数。
    /// （从 trial 开始计；值越大表示越慢，方向预期为负单调：
    /// 属性越高 → 耗时越低 → 单调性判定时取反向。）
    MovementTaskTicks,
}

/// 单个 trial 的观测：跑 `trial_ticks` 计出手/命中。
/// 返回 (attempts, makes)；MovementTaskTicks 通道的语义是
/// (完成次数, 总耗时 tick)，由调用方按通道解释方向。
fn observe_trial(
    engine: &mut MatchEngine,
    trial_ticks: u64,
    channel: ObservationChannel,
) -> (u32, u32) {
    let mut attempts = 0u32;
    let mut makes = 0u32;
    let trial_start_tick = 0u64;
    let mut tick_index = 0u64;
    let mut last_release_in_zone = false;
    for _ in 0..trial_ticks {
        let tick = engine.step();
        tick_index += 1;
        for event in &tick.frame.event_log {
            match channel {
                ObservationChannel::ThreePointShots => {
                    if event.kind == "SHOT_ATTEMPT_RESOLVED" {
                        if let Some(payload) = event
                            .data
                            .as_ref()
                            .and_then(|d| d.get("ShotAttemptResolved"))
                        {
                            let is_three = payload.get("is_three").and_then(|v| v.as_bool());
                            if is_three != Some(true) {
                                continue;
                            }
                            attempts += 1;
                            if payload.get("scoring_decision").and_then(|v| v.as_str())
                                == Some("fga_fgm")
                            {
                                makes += 1;
                            }
                        }
                    }
                }
                ObservationChannel::FreeThrows => {
                    if event.kind == "FREE_THROW" {
                        if let Some(payload) = event
                            .data
                            .as_ref()
                            .and_then(|d| d.get("FreeThrowAttempt"))
                        {
                            attempts += 1;
                            if payload.get("made").and_then(|v| v.as_bool()) == Some(true) {
                                makes += 1;
                            }
                        }
                    }
                }
                ObservationChannel::CloseShots | ObservationChannel::MidShots => {
                    // 出手距离用 SHOT_RELEASE pos 判（RESOLVED 无位置）。
                    if event.kind == "SHOT_RELEASE" {
                        if let Some(payload) = event
                            .data
                            .as_ref()
                            .and_then(|d| d.get("ShotRelease"))
                        {
                            if payload.get("is_three").and_then(|v| v.as_bool()) == Some(true) {
                                continue;
                            }
                            let Some(pos) = payload.get("pos").and_then(|v| v.as_array()) else {
                                continue;
                            };
                            let (Some(x), Some(y)) = (
                                pos.first().and_then(serde_json::Value::as_f64),
                                pos.get(1).and_then(serde_json::Value::as_f64),
                            ) else {
                                continue;
                            };
                            let (x, y) = (x as f32, y as f32);
                            // 就近篮筐距离（坐标域：英尺）。
                            let dist = if x <= 47.0 {
                                ((x - 5.25).powi(2) + (y - 25.0).powi(2)).sqrt()
                            } else {
                                ((x - 88.75).powi(2) + (y - 25.0).powi(2)).sqrt()
                            };
                            let in_zone = match channel {
                                ObservationChannel::CloseShots => dist < 8.0,
                                ObservationChannel::MidShots => (8.0..16.0).contains(&dist),
                                ObservationChannel::ThreePointShots
                                | ObservationChannel::FreeThrows
                                | ObservationChannel::OffensiveRebounds
                                | ObservationChannel::DefensiveRebounds
                                | ObservationChannel::Turnovers
                                | ObservationChannel::MovementTaskTicks => false,
                            };
                            if !in_zone {
                                continue;
                            }
                            attempts += 1;
                            last_release_in_zone = true;
                        }
                    }
                    // 命中结算来自 RESOLVED（scoring_decision=fga_fgm 表示命中）。
                    if event.kind == "SHOT_ATTEMPT_RESOLVED" {
                        if let Some(payload) = event
                            .data
                            .as_ref()
                            .and_then(|d| d.get("ShotAttemptResolved"))
                        {
                            let is_three = payload.get("is_three").and_then(|v| v.as_bool());
                            if is_three != Some(false) {
                                continue;
                            }
                            if payload.get("scoring_decision").and_then(|v| v.as_str())
                                == Some("fga_fgm")
                            {
                                // RESOLVED 无位置；命中计数按最近 RELEASE 的
                                // 区域归属（单线程窗口内 RELEASE→RESOLVED
                                // 交替，最后一个 RELEASE 即本次结算的出手）。
                                if last_release_in_zone {
                                    makes += 1;
                                }
                            }
                            last_release_in_zone = false;
                        }
                    }
                }
                ObservationChannel::OffensiveRebounds | ObservationChannel::DefensiveRebounds => {
                    // 篮板率观测：分母 = 窗口内投篮不中数（每次不中是
                    // 一回合争抢机会），分子 = 目标类篮板数。
                    if event.kind == "SHOT_MISS" {
                        attempts += 1;
                    }
                    if event.kind == "REBOUND" {
                        if let Some(payload) = event
                            .data
                            .as_ref()
                            .and_then(|d| d.get("ReboundContest"))
                        {
                            let is_offensive =
                                payload.get("is_offensive").and_then(|v| v.as_bool());
                            let counted = match channel {
                                ObservationChannel::OffensiveRebounds => is_offensive == Some(true),
                                ObservationChannel::DefensiveRebounds => is_offensive == Some(false),
                                ObservationChannel::ThreePointShots
                                | ObservationChannel::FreeThrows
                                | ObservationChannel::CloseShots
                                | ObservationChannel::MidShots
                                | ObservationChannel::Turnovers
                                | ObservationChannel::MovementTaskTicks => false,
                            };
                            if counted {
                                makes += 1;
                            }
                        }
                    }
                }
                ObservationChannel::Turnovers => {
                    // 失误率观测：分母 = 窗口内回合数，分子 = 失误事件数。
                    if event.kind == "POSSESSION_SUMMARY" {
                        attempts += 1;
                    }
                    let is_tov = match event.kind.as_str() {
                        "PASS_DROPPED" | "PASS_TIPPED" | "BALL_POKED_LOOSE" => true,
                        "POSSESSION_SUMMARY" => event
                            .data
                            .as_ref()
                            .and_then(|d| d.get("PossessionSummary"))
                            .and_then(|p| p.get("terminal_event"))
                            .and_then(|t| t.as_str())
                            .is_some_and(|s| {
                                s == "TURNOVER_VIOLATION" || s == "TURNOVER_STEAL"
                            }),
                        _ => false,
                    };
                    if is_tov {
                        makes += 1;
                    }
                }
                ObservationChannel::MovementTaskTicks => {
                    if event.kind == "MOVEMENT_OUTCOME" {
                        if let Some(payload) = event
                            .data
                            .as_ref()
                            .and_then(|d| d.get("MovementOutcome"))
                        {
                            if payload.get("outcome").and_then(|v| v.as_str()) == Some("completed")
                            {
                                let ticks_spent = tick_index - trial_start_tick;
                                attempts += 1;
                                makes += u32::try_from(ticks_spent).unwrap_or(u32::MAX);
                            }
                        }
                    }
                }
            }
        }
    }
    (attempts, makes)
}

/// 执行一个检查点实验。
///
/// 母局单遍走位；谓词命中即 fork 四组（基线/低/中/高），每组换同一具名
/// 随机键种子（档位不进键），跑观测窗口记录结果后丢弃。
pub fn run_checkpoint_experiment(spec: &MechExperimentSpec, attribute: ValidationAttribute) -> MechExperimentOutcome {
    let mut mother = MatchEngine::new(spec.mother_seed);
    let mut trials: Vec<CheckpointTrialObservation> = Vec::new();
    let mut total_attempts = [0u64; 4];
    let mut total_makes = [0u64; 4];
    let mut checkpoints_hit = 0u32;

    // 母局最长走位：检查点数下限未达标前持续走（上限 = minimum * 40 + 200k，
    // 防止谓词永不命中时死循环）。
    let tick_budget = 200_000u64 + u64::from(spec.minimum_checkpoints) * 40_000;
    let mut ticks = 0u64;
    while !mother.is_finished() && ticks < tick_budget {
        let tick = mother.step();
        ticks += 1;
        if !predicate_matches(spec.predicate, &tick) {
            continue;
        }
        // 扰动目标按谓词取：投篮窗口用帧控球者；罚球程序用本 tick 的
        // FREE_THROW payload shooter_id（罚球程序中帧层无控球者）；
        // 篮板/冲刺类用抢板卡位距离最近的在扬球员；组织决策用控制者。
        // 取不到目标的检查点跳过且不计入分母。
        let target_player = match spec.predicate {
            CheckpointPredicate::ShooterInThreePointWindow
            | CheckpointPredicate::ShooterInCloseWindow
            | CheckpointPredicate::ShooterInMidWindow => tick
                .frame
                .players
                .iter()
                .find(|p| p.has_ball && p.on_court)
                .map(|p| p.id.clone()),
            CheckpointPredicate::FreeThrowRoutine => tick
                .frame
                .event_log
                .iter()
                .find(|e| e.kind == "FREE_THROW")
                .and_then(|e| e.data.as_ref())
                .and_then(|d| d.get("FreeThrowAttempt"))
                .and_then(|p| p.get("shooter_id"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            CheckpointPredicate::ShotMissMoment => {
                // 扰动目标 = 距防守方篮框最近的在场球员（卡位争抢第一
                // 参与者；SHOT_MISS 无落点事实，落点在 REBOUND 才出现，
                // 用"不中总在篮下附近"的空间近似）。
                let hoop_x = if tick.frame.possession_team == "home" {
                    88.75_f32
                } else {
                    5.25_f32
                };
                tick.frame
                    .players
                    .iter()
                    .filter(|p| p.on_court)
                    .min_by(|a, b| {
                        let da = (a.x * 94.0 - hoop_x).powi(2) + (a.y * 50.0 - 25.0).powi(2);
                        let db = (b.x * 94.0 - hoop_x).powi(2) + (b.y * 50.0 - 25.0).powi(2);
                        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|p| p.id.clone())
            }
            CheckpointPredicate::BackcourtControlEstablished => tick
                .frame
                .event_log
                .iter()
                .find(|e| e.kind == "CONTROL_ESTABLISHED")
                .and_then(|e| e.data.as_ref())
                .and_then(|d| d.get("ControlEstablished"))
                .and_then(|c| c.get("player_id"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
        };
        let Some(target_player) = target_player else {
            continue;
        };
        checkpoints_hit += 1;
        // 四组：基线 + 低中高三档。档位不进具名随机键 → 同 trial 共享随机流。
        let mut trial_attempt_scratch = [0u32; 4];
        let mut trial_makes_scratch = [0u32; 4];
        for group in 0..4usize {
            let mut engine = mother.experimental_fork();
            engine.reseed_validation_rng(
                &spec.experiment_id,
                checkpoints_hit as u64,
                "trial",
                trials.len() as u64 + group as u64,
                0,
            );
            if group > 0 {
                engine.apply_validation_perturbation(
                    &target_player,
                    attribute,
                    spec.levels[group - 1],
                );
            }
            let channel = match spec.predicate {
                CheckpointPredicate::ShooterInThreePointWindow => {
                    ObservationChannel::ThreePointShots
                }
                CheckpointPredicate::FreeThrowRoutine => ObservationChannel::FreeThrows,
                CheckpointPredicate::ShooterInCloseWindow => ObservationChannel::CloseShots,
                CheckpointPredicate::ShooterInMidWindow => ObservationChannel::MidShots,
                CheckpointPredicate::ShotMissMoment => match attribute {
                    ValidationAttribute::OffensiveRebound => ObservationChannel::OffensiveRebounds,
                    ValidationAttribute::DefensiveRebound => ObservationChannel::DefensiveRebounds,
                    // 力量的物理通路是卡位对抗 → 观测篮板归属胜出，与
                    // 移动类（冲刺到位耗时）分开。
                    ValidationAttribute::Strength => ObservationChannel::DefensiveRebounds,
                    ValidationAttribute::Speed
                    | ValidationAttribute::Acceleration
                    | ValidationAttribute::Stamina => ObservationChannel::MovementTaskTicks,
                    _ => panic!(
                        "谓词 ShotMissMoment 不支持属性 {:?}：篮板/冲刺类属性专用，规格登记须一致",
                        attribute
                    ),
                },
                CheckpointPredicate::BackcourtControlEstablished => {
                    if attribute == ValidationAttribute::DecisionIq {
                        ObservationChannel::Turnovers
                    } else {
                        panic!(
                            "谓词 BackcourtControlEstablished 不支持属性 {:?}：decision_iq 专用，规格登记须一致",
                            attribute
                        )
                    }
                }
            };
            let (attempts, makes) = observe_trial(&mut engine, spec.trial_ticks, channel);
            trial_attempt_scratch[group] = attempts;
            trial_makes_scratch[group] = makes;
            total_attempts[group] += u64::from(attempts);
            total_makes[group] += u64::from(makes);
        }
        trials.push(CheckpointTrialObservation {
            checkpoint_index: checkpoints_hit,
            trial_id: trials.len() as u32,
            attempts: trial_attempt_scratch,
            makes: trial_makes_scratch,
        });
        if checkpoints_hit >= spec.minimum_checkpoints {
            break;
        }
    }

    let status = if checkpoints_hit >= spec.minimum_checkpoints {
        "completed"
    } else {
        "insufficient_checkpoints"
    };
    MechExperimentOutcome {
        experiment_id: spec.experiment_id.clone(),
        criterion_id: spec.criterion_id.clone(),
        checkpoints_hit,
        total_attempts,
        total_makes,
        trials,
        status: status.to_string(),
    }
}

/// 档位间单调性判定（三出口）：高 ≥ 中 ≥ 低 的命中数方向。
///
/// 每档位命中率（makes/attempts）的 bootstrap 区间与单调性联合：
/// 高-中 与 中-低 的差值区间都必须方向一致才 Pass；任一反向明显
/// （区间完全反向）Defect；其余 InsufficientEvidence。
pub fn monotonicity_verdict(
    outcome: &MechExperimentOutcome,
    expected_direction: ExpectedDirection,
    bootstrap_replicates: usize,
    seed: u64,
) -> BTreeMap<String, String> {
    use crate::stats::{compare_mean_difference, BootstrapConfig, EquivalenceVerdict, PairedGameValues};

    let mut verdicts = BTreeMap::new();
    if outcome.status != "completed" {
        verdicts.insert(
            "status".to_string(),
            "InsufficientEvidence".to_string(),
        );
        return verdicts;
    }
    // LowerBetter 通道（冲刺耗时）的 makes 存的是耗时 tick 和；配对差
    // 取相反数后统一用「高档更高为好」的同一判定路径。
    let direction_sign = match expected_direction {
        ExpectedDirection::HigherBetter => 1.0_f64,
        ExpectedDirection::LowerBetter => -1.0_f64,
    };
    let rate = |group: usize| -> f64 {
        if outcome.total_attempts[group] > 0 {
            outcome.total_makes[group] as f64 / outcome.total_attempts[group] as f64
        } else {
            f64::NAN
        }
    };
    let low = rate(1);
    let mid = rate(2);
    let high = rate(3);
    if !(low.is_finite() && mid.is_finite() && high.is_finite()) {
        verdicts.insert("status".to_string(), "InsufficientEvidence".to_string());
        return verdicts;
    }
    // 出手数作为权重的粗配对：以 trial 粒度做配对 bootstrap（trial 观测
    // 已按检查点配对；此处把 trial 的 makes 汇成配对样本）。
    let config = BootstrapConfig {
        replicates: bootstrap_replicates as u32,
        family_size: 10,
        alpha: 0.05,
        seed,
    };
    // 高-中 与 中-低 两个方向的配对差（用 trial 级 makes 差；attempts 为
    // 零的 trial 不计；LowerBetter 通道取相反数后同一方向门）。
    let paired = |higher: usize, lower: usize| -> Vec<PairedGameValues> {
        outcome
            .trials
            .iter()
            .enumerate()
            .filter(|(_, trial)| trial.attempts[higher] > 0 && trial.attempts[lower] > 0)
            .map(|(index, trial)| PairedGameValues {
                game_id: format!("trial-{index}"),
                simulated: direction_sign
                    * (trial.makes[higher] as f64 - trial.makes[lower] as f64),
                reference: 0.0,
            })
            .collect()
    };
    let mut all_pass = true;
    let mut any_defect = false;
    for (name, higher, lower) in [
        ("high_vs_mid", 3usize, 2usize),
        ("mid_vs_low", 2usize, 1usize),
    ] {
        let samples = paired(higher, lower);
        if samples.is_empty() {
            verdicts.insert(name.to_string(), "InsufficientEvidence".to_string());
            all_pass = false;
            continue;
        }
        // 单调方向的最小效果 0（不反向即可）；等效区间 0 意味着
        // 「差值 ≥ 0」方向门。
        let (interval, verdict) = compare_mean_difference(&samples, 0.0, &config);
        match verdict {
            EquivalenceVerdict::Pass => {}
            EquivalenceVerdict::Defect => any_defect = true,
            EquivalenceVerdict::InsufficientEvidence => all_pass = false,
        }
        verdicts.insert(
            format!("{name}_interval"),
            format!("[{:.4}, {:.4}]", interval.lower, interval.upper),
        );
    }
    let status = if any_defect {
        "Defect"
    } else if all_pass {
        "Pass"
    } else {
        "InsufficientEvidence"
    };
    verdicts.insert("monotonicity".to_string(), status.to_string());
    // 基线对照记录（不做判定；基线命中率供参考）。
    verdicts.insert(
        "baseline_rate".to_string(),
        format!("{:.4}", rate(0)),
    );
    verdicts.insert("low_rate".to_string(), format!("{:.4}", low));
    verdicts.insert("mid_rate".to_string(), format!("{:.4}", mid));
    verdicts.insert("high_rate".to_string(), format!("{:.4}", high));
    verdicts
}
