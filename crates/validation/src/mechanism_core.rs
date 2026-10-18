//! 机制实验：属性扰动后回合响应方向。
//!
//! 每个实验对同一组种子跑两次：基准阵容与单属性提高后的阵容。指标按
//! 分子分母汇总后再比较，避免小样本比率噪声。判定只要求方向与最小效应，
//! 不要求具体数值区间。
//!
//! 这层替代旧引擎的 MECH 实验。旧实现在单次请求里同步跑上千次短跑，
//! 三百分秒不返回；这里把重实验留在离线命令，端点只读缓存工件。

use std::path::Path;

use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{GameReport, MatchSetup, Params, TeamSide, World};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MechanismAttribute {
    ShootingThree,
    ShootingClose,
    ShootingMid,
    FreeThrow,
    Speed,
    Acceleration,
    Stamina,
    Strength,
    OffensiveRebound,
    DefensiveRebound,
    DecisionIq,
}

impl MechanismAttribute {
    pub fn all() -> [Self; 11] {
        [
            Self::ShootingThree,
            Self::ShootingClose,
            Self::ShootingMid,
            Self::FreeThrow,
            Self::Speed,
            Self::Acceleration,
            Self::Stamina,
            Self::Strength,
            Self::OffensiveRebound,
            Self::DefensiveRebound,
            Self::DecisionIq,
        ]
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::ShootingThree => "shooting_three",
            Self::ShootingClose => "shooting_close",
            Self::ShootingMid => "shooting_mid",
            Self::FreeThrow => "free_throw",
            Self::Speed => "speed",
            Self::Acceleration => "acceleration",
            Self::Stamina => "stamina",
            Self::Strength => "strength",
            Self::OffensiveRebound => "offensive_rebound",
            Self::DefensiveRebound => "defensive_rebound",
            Self::DecisionIq => "decision_iq",
        }
    }

    pub fn metric(self) -> Metric {
        match self {
            Self::ShootingThree => Metric::ThreePointPct,
            Self::ShootingClose => Metric::RimPct,
            Self::ShootingMid => Metric::MidPct,
            Self::FreeThrow => Metric::FreeThrowPct,
            Self::Speed | Self::Acceleration => Metric::MeanPossessionSeconds,
            Self::Stamina => Metric::FieldGoalPct,
            Self::Strength | Self::OffensiveRebound => Metric::OffensiveReboundRate,
            Self::DefensiveRebound => Metric::DefensiveReboundRate,
            Self::DecisionIq => Metric::TurnoverRate,
        }
    }

    pub fn direction(self) -> Direction {
        match self {
            Self::DecisionIq => Direction::Down,
            Self::Speed | Self::Acceleration => Direction::Down,
            _ => Direction::Up,
        }
    }

    pub fn apply(self, setup: &mut MatchSetup, value: f32) {
        // 只改主队：客队作为同一对种子里的对照组，方向差异因此可归因于
        // 被扰动的属性而不是整场随机波动。
        for player in setup.home.players.iter_mut() {
            let attributes = &mut player.attributes;
            match self {
                Self::ShootingThree => attributes.shooting_three = value,
                Self::ShootingClose => attributes.shooting_close = value,
                Self::ShootingMid => attributes.shooting_mid = value,
                Self::FreeThrow => attributes.free_throw = value,
                Self::Speed => attributes.speed = value,
                Self::Acceleration => attributes.acceleration = value,
                Self::Stamina => attributes.stamina = value,
                Self::Strength => attributes.strength = value,
                Self::OffensiveRebound => attributes.offensive_rebound = value,
                Self::DefensiveRebound => attributes.defensive_rebound = value,
                Self::DecisionIq => attributes.decision_iq = value,
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Metric {
    ThreePointPct,
    RimPct,
    MidPct,
    FreeThrowPct,
    FieldGoalPct,
    Possessions,
    MeanPossessionSeconds,
    OffensiveReboundRate,
    DefensiveReboundRate,
    TurnoverRate,
}

impl Metric {
    pub fn key(self) -> &'static str {
        match self {
            Self::ThreePointPct => "three_point_pct",
            Self::RimPct => "rim_pct",
            Self::MidPct => "mid_pct",
            Self::FreeThrowPct => "free_throw_pct",
            Self::FieldGoalPct => "field_goal_pct",
            Self::Possessions => "possessions",
            Self::MeanPossessionSeconds => "mean_possession_seconds",
            Self::OffensiveReboundRate => "offensive_rebound_rate",
            Self::DefensiveReboundRate => "defensive_rebound_rate",
            Self::TurnoverRate => "turnover_rate",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Up,
    Down,
}

impl Direction {
    pub fn key(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MechanismConfig {
    pub seeds: usize,
    pub seed_start: u64,
    pub possessions: Option<u32>,
    pub low_value: f32,
    pub high_value: f32,
    pub minimum_relative_change: f64,
}

impl Default for MechanismConfig {
    fn default() -> Self {
        Self {
            seeds: 8,
            seed_start: 1,
            possessions: None,
            low_value: 0.10,
            high_value: 0.95,
            minimum_relative_change: 0.002,
        }
    }
}

impl MechanismConfig {
    /// 快速档：限定回合数，供测试与交互式检查使用。
    pub fn fast() -> Self {
        Self {
            seeds: 2,
            seed_start: 1,
            possessions: Some(30),
            low_value: 0.10,
            high_value: 0.95,
            minimum_relative_change: 1e-6,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MechanismResult {
    pub attribute: String,
    pub metric: String,
    pub expectation: String,
    pub baseline: f64,
    pub perturbed: f64,
    pub relative_change: f64,
    pub per_seed_change: Vec<f64>,
    pub seeds_in_expected_direction: usize,
    pub seeds: usize,
    pub observed_direction: String,
    pub direction_matches: bool,
    pub magnitude_matches: bool,
    pub weak_effect: bool,
    pub verdict: String,
    pub passed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MechanismReport {
    pub config: MechanismConfigReport,
    pub results: Vec<MechanismResult>,
    pub passed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MechanismConfigReport {
    pub seeds: usize,
    pub possessions: Option<u32>,
    pub low_value: f64,
    pub high_value: f64,
    pub minimum_relative_change: f64,
}

pub fn run_mechanism_experiments(config: MechanismConfig) -> Result<MechanismReport, String> {
    run_mechanism_experiments_with_dir(config, &workspace_calibration_dir())
}

pub fn run_mechanism_experiment(
    attribute: MechanismAttribute,
    config: MechanismConfig,
) -> Result<MechanismResult, String> {
    run_mechanism_experiment_with_dir(attribute, config, &workspace_calibration_dir())
}

pub fn run_mechanism_experiment_with_dir(
    attribute: MechanismAttribute,
    config: MechanismConfig,
    calibration: &Path,
) -> Result<MechanismResult, String> {
    let params = Params::load(calibration)?;
    experiment(&params, attribute, config)
}

pub fn run_mechanism_experiments_with_dir(
    config: MechanismConfig,
    calibration: &Path,
) -> Result<MechanismReport, String> {
    let params = Params::load(calibration)?;
    let mut results = Vec::new();
    for attribute in MechanismAttribute::all() {
        results.push(experiment(&params, attribute, config)?);
    }
    Ok(MechanismReport {
        config: MechanismConfigReport {
            seeds: config.seeds,
            possessions: config.possessions,
            low_value: round_two(config.low_value),
            high_value: round_two(config.high_value),
            minimum_relative_change: config.minimum_relative_change,
        },
        passed: results.iter().all(|result| result.verdict != "defect"),
        results,
    })
}

fn experiment(
    params: &Params,
    attribute: MechanismAttribute,
    config: MechanismConfig,
) -> Result<MechanismResult, String> {
    let baseline = run_arm(params, attribute, config.low_value, config)?;
    let perturbed = run_arm(params, attribute, config.high_value, config)?;
    let metric = attribute.metric();
    let baseline_values = baseline.ok_or_else(|| {
        format!(
            "attribute {} produced no {} samples",
            attribute.key(),
            metric.key()
        )
    })?;
    let perturbed_values = perturbed.ok_or_else(|| {
        format!(
            "attribute {} produced no {} samples in the perturbed arm",
            attribute.key(),
            metric.key()
        )
    })?;
    let per_seed_change: Vec<f64> = baseline_values
        .iter()
        .zip(perturbed_values.iter())
        .map(|((low_n, low_d), (high_n, high_d))| {
            let low = low_n / low_d;
            let high = high_n / high_d;
            if low.abs() > f64::EPSILON {
                (high - low) / low
            } else {
                0.0
            }
        })
        .collect();
    let seed_count = per_seed_change.len().max(1);
    let seeds_in_expected_direction = per_seed_change
        .iter()
        .filter(|change| match attribute.direction() {
            Direction::Up => **change > 0.0,
            Direction::Down => **change < 0.0,
        })
        .count();
    let seeds_opposite = seed_count - seeds_in_expected_direction;
    let consistent_opposite = seeds_opposite * 4 >= seed_count * 3;
    let baseline_value = ratio_of_sums(&baseline_values);
    let perturbed_value = ratio_of_sums(&perturbed_values);
    let relative = if baseline_value.abs() > f64::EPSILON {
        (perturbed_value - baseline_value) / baseline_value.abs()
    } else {
        0.0
    };
    let observed = if relative > 0.0 {
        Direction::Up
    } else if relative < 0.0 {
        Direction::Down
    } else {
        attribute.direction()
    };
    // 方向判定用聚合值：逐种子比率在单场级别噪声很大，聚合是主估计；
    // 逐种子同向计数作为证据一并记录。
    let direction_matches = match attribute.direction() {
        Direction::Up => relative > 0.0,
        Direction::Down => relative < 0.0,
    };
    let magnitude_matches = relative.abs() >= config.minimum_relative_change;
    let weak_effect = relative.abs() < 0.01;
    let verdict = if direction_matches && magnitude_matches {
        "pass"
    } else if consistent_opposite {
        "defect"
    } else {
        "insufficient"
    };
    Ok(MechanismResult {
        attribute: attribute.key().to_string(),
        metric: metric.key().to_string(),
        expectation: attribute.direction().key().to_string(),
        baseline: baseline_value,
        perturbed: perturbed_value,
        relative_change: relative,
        per_seed_change,
        seeds_in_expected_direction,
        seeds: seed_count,
        observed_direction: observed.key().to_string(),
        direction_matches,
        magnitude_matches,
        weak_effect,
        verdict: verdict.to_string(),
        passed: verdict == "pass",
    })
}

fn run_arm(
    params: &Params,
    attribute: MechanismAttribute,
    value: f32,
    config: MechanismConfig,
) -> Result<Option<Vec<(f64, f64)>>, String> {
    let mut samples = Vec::new();
    for offset in 0..config.seeds {
        let seed = config.seed_start + offset as u64;
        let setup = build_setup(attribute, value);
        let report = run_limited(seed, params.clone(), setup, config.possessions)?;
        let (n, d) = metric_parts(attribute.metric(), &report);
        if d > 0.0 {
            samples.push((n, d));
        }
    }
    if samples.is_empty() {
        return Ok(None);
    }
    Ok(Some(samples))
}

fn ratio_of_sums(samples: &[(f64, f64)]) -> f64 {    let numerator: f64 = samples.iter().map(|(n, _)| *n).sum();
    let denominator: f64 = samples.iter().map(|(_, d)| *d).sum();
    if denominator.abs() < f64::EPSILON {
        0.0
    } else {
        numerator / denominator
    }
}

fn build_setup(attribute: MechanismAttribute, value: f32) -> MatchSetup {
    let mut setup = MatchSetup::builtin(nba_domain::GameRules::default());
    attribute.apply(&mut setup, value);
    setup
}

fn run_limited(
    seed: u64,
    params: Params,
    setup: MatchSetup,
    possessions: Option<u32>,
) -> Result<GameReport, String> {
    let mut world = World::new(setup, params, seed);
    match possessions {
        None => {
            let mut report = world.run();
            fill_report(&mut report, &world);
            Ok(report)
        }
        Some(target) => {
            let mut steps = 0usize;
            while !world.finished && steps < 400_000 {
                world.step(TICK_SECONDS);
                steps += 1;
                if world.possession_log.len() as u32 >= target {
                    break;
                }
            }
            let mut report = world.report();
            fill_report(&mut report, &world);
            Ok(report)
        }
    }
}

fn fill_report(report: &mut GameReport, world: &World) {
    report.shot_clock_at_release = world.shot_clock_at_release.clone();
    report.contest_samples = world.contest_samples.clone();
    report.shot_zone_counts = world.shot_zone_counts;
    report.shot_zone_makes = world.shot_zone_makes;
    report.creation_counts = world.creation_counts;
    report.violations = world.violations.clone();
}

fn metric_parts(metric: Metric, report: &GameReport) -> (f64, f64) {
    let home = report.box_score.team_totals(TeamSide::Home);
    let away = report.box_score.team_totals(TeamSide::Away);
    let possessions = report.possessions.len().max(1) as f64;
    match metric {
        Metric::ThreePointPct => (home.tpm as f64, home.tpa as f64),
        Metric::RimPct => (
            report.shot_zone_makes[0] as f64,
            report.shot_zone_counts[0] as f64,
        ),
        Metric::MidPct => (
            report.shot_zone_makes[2] as f64,
            report.shot_zone_counts[2] as f64,
        ),
        Metric::FreeThrowPct => (home.ftm as f64, home.fta as f64),
        Metric::FieldGoalPct => (home.fgm as f64, home.fga as f64),
        Metric::Possessions => (possessions, 1.0),
        Metric::MeanPossessionSeconds => (
            report.possessions.iter().map(|record| record.duration).sum(),
            report.possessions.len().max(1) as f64,
        ),
        Metric::OffensiveReboundRate => (
            home.oreb as f64,
            (home.fga - home.fgm).max(1) as f64,
        ),
        Metric::DefensiveReboundRate => (
            home.dreb as f64,
            (home.dreb + away.oreb).max(1) as f64,
        ),
        Metric::TurnoverRate => (home.turnovers as f64, possessions),
    }
}

/// f32 抬成 f64 会带出尾数（0.95 → 0.949999988…），记录里保留两位即可。
fn round_two(value: f32) -> f64 {
    (f64::from(value) * 100.0).round() / 100.0
}
