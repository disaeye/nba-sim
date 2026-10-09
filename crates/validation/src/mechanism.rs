//! 机制实验框架（validation.md §7）。
//!
//! §7.1 直接机制实验的执行载体在 `mech.rs`（检查点分叉 + 校验专用扰动 +
//! 具名随机键配对，引擎侧 `experimental_fork` / `apply_validation_perturbation`
//! / `reseed_validation_rng` 已落地）；本模块保留 §7.2 全场反馈实验
//! （固定赛程，每次只改一个输入，整场配对，以比赛为单位计算差值区间）
//! 与共享的观测快照结构。

use serde::{Deserialize, Serialize};

use crate::stats::{BootstrapConfig, DifferenceInterval, PairedGameValues};

/// 机制实验类型（validation.md §7.1/§7.2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentKind {
    /// 决策前检查点扰动（需要引擎 checkpoint API）。
    Checkpoint,
    /// 完整比赛配对反馈（当前可执行）。
    FullGame,
}

/// 一次输入扰动的声明（validation.md §7.3 输入覆盖目录的实验单元）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerturbationSpec {
    /// 实验标识（如 `MECH-S01-close-low`）。
    pub experiment_id: String,
    /// 对应准则叶项（如 `MECH-S01`）。
    pub criterion_id: String,
    pub kind: ExperimentKind,
    /// 扰动对象：属性/倾向/档案字段路径。
    pub input_path: String,
    /// 扰动方向（`low`/`mid`/`high` 三水平）。
    pub level: String,
    /// 水平值（合法域内的值，进入生产初始化路径）。
    pub value: f64,
    /// 预期受影响的观测量（方向门登记）。
    pub expected_channel: String,
    /// 预期方向（`increase`/`decrease`/`none`）。
    pub expected_direction: String,
    /// 方向门的最小效果（validation.md §7.2 事前定义）。
    pub minimum_effect: f64,
    /// 不应变化的量的等效区间宽。
    pub neutral_delta: f64,
}

impl PerturbationSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.experiment_id.is_empty() || self.criterion_id.is_empty() {
            return Err("experiment and criterion ids are required".to_string());
        }
        if !self.value.is_finite() || !self.minimum_effect.is_finite() {
            return Err("perturbation values must be finite".to_string());
        }
        Ok(())
    }
}

/// 一场配对比赛的观测值（validation.md §7.2 比赛为单位）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedGameObservation {
    pub game_index: u32,
    /// 基线运行的观测量。
    pub baseline: GameMetricsSnapshot,
    /// 扰动运行的观测量。
    pub perturbed: GameMetricsSnapshot,
}

/// 单场机器可累积的观测量快照。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GameMetricsSnapshot {
    pub points: u32,
    pub possessions: u32,
    pub field_goal_attempts: u32,
    pub field_goal_makes: u32,
    pub three_point_attempts: u32,
    pub free_throw_attempts: u32,
    pub turnovers: u32,
    pub fouls: u32,
    pub passes: u64,
    pub shots_released: u64,
}

impl GameMetricsSnapshot {
    /// 配对差值序列（validation.md §7.2 机会数、动作数、条件结果与全队输出）。
    pub fn paired_differences(&self, baseline: &Self) -> Vec<(&'static str, f64)> {
        [
            (
                "points",
                f64::from(self.points) - f64::from(baseline.points),
            ),
            (
                "possessions",
                f64::from(self.possessions) - f64::from(baseline.possessions),
            ),
            (
                "fga",
                f64::from(self.field_goal_attempts) - f64::from(baseline.field_goal_attempts),
            ),
            (
                "fgm",
                f64::from(self.field_goal_makes) - f64::from(baseline.field_goal_makes),
            ),
            (
                "three_pa",
                f64::from(self.three_point_attempts) - f64::from(baseline.three_point_attempts),
            ),
            (
                "fta",
                f64::from(self.free_throw_attempts) - f64::from(baseline.free_throw_attempts),
            ),
            (
                "tov",
                f64::from(self.turnovers) - f64::from(baseline.turnovers),
            ),
            ("passes", self.passes as f64 - baseline.passes as f64),
            (
                "shots_released",
                self.shots_released as f64 - baseline.shots_released as f64,
            ),
        ]
        .into_iter()
        .collect()
    }
}

/// 一次实验的结果（validation.md §7.2 总效应 + 差值区间）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentResult {
    pub experiment_id: String,
    pub criterion_id: String,
    pub kind: ExperimentKind,
    /// 每个观测通道的差值区间与判定。
    pub channels: Vec<ChannelResult>,
    /// 支持结论的比赛对数。
    pub game_pairs: u32,
}

/// 单个观测通道的结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelResult {
    pub channel: String,
    pub interval: DifferenceInterval,
    /// 方向门判定：`pass`/`defect`/`insufficient`。
    pub direction_verdict: String,
}

impl ExperimentResult {
    /// 汇总配对观测为实验结果（validation.md §7.2 正式检验用全部比赛）。
    pub fn summarize(
        spec: &PerturbationSpec,
        observations: &[PairedGameObservation],
        config: &BootstrapConfig,
    ) -> Self {
        let mut channels: Vec<ChannelResult> = Vec::new();
        if observations.is_empty() {
            channels.push(ChannelResult {
                channel: spec.expected_channel.clone(),
                interval: DifferenceInterval {
                    lower: f64::NEG_INFINITY,
                    upper: f64::INFINITY,
                },
                direction_verdict: "insufficient".to_string(),
            });
            return Self {
                experiment_id: spec.experiment_id.clone(),
                criterion_id: spec.criterion_id.clone(),
                kind: spec.kind,
                channels,
                game_pairs: 0,
            };
        }
        // 按通道聚合配对差值。
        let mut per_channel: std::collections::BTreeMap<&'static str, Vec<PairedGameValues>> =
            Default::default();
        for observation in observations {
            let diffs = observation
                .perturbed
                .paired_differences(&observation.baseline);
            for (channel, difference) in diffs {
                per_channel
                    .entry(channel)
                    .or_default()
                    .push(PairedGameValues {
                        game_id: format!("pair-{}", observation.game_index),
                        simulated: difference,
                        reference: 0.0,
                    });
            }
        }
        for (channel, samples) in per_channel {
            let target = channel == spec.expected_channel.as_str();
            let allowed = if target {
                spec.minimum_effect
            } else {
                spec.neutral_delta
            };
            let (interval, verdict) =
                crate::stats::compare_mean_difference(&samples, allowed, config);
            let direction = if target {
                match verdict {
                    crate::stats::EquivalenceVerdict::Pass => "pass".to_string(),
                    crate::stats::EquivalenceVerdict::Defect => "defect".to_string(),
                    crate::stats::EquivalenceVerdict::InsufficientEvidence => {
                        "insufficient".to_string()
                    }
                }
            } else {
                // 非预期通道：要求等效（validation.md §7.2 不应变化的量）。
                match verdict {
                    crate::stats::EquivalenceVerdict::Pass => "neutral_ok".to_string(),
                    crate::stats::EquivalenceVerdict::Defect => "neutral_violated".to_string(),
                    crate::stats::EquivalenceVerdict::InsufficientEvidence => {
                        "insufficient".to_string()
                    }
                }
            };
            channels.push(ChannelResult {
                channel: channel.to_string(),
                interval,
                direction_verdict: direction,
            });
        }
        Self {
            experiment_id: spec.experiment_id.clone(),
            criterion_id: spec.criterion_id.clone(),
            kind: spec.kind,
            channels,
            game_pairs: observations.len() as u32,
        }
    }
}
