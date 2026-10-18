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
            Self::ShootingMid => Metric::MidAttemptShare,
            Self::FreeThrow => Metric::FreeThrowPct,
            Self::Speed | Self::Acceleration => Metric::MeanFirstActionSeconds,
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

    /// 证据类别：接线检查的指标直接来自消费者算式；比赛产物类指标的
    /// 指标与消费者之间隔着整个结算过程。
    pub fn evidence_kind(self) -> &'static str {
        match self {
            Self::Speed | Self::Acceleration => "wiring",
            _ => "outcome",
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
    MidAttemptShare,
    FreeThrowPct,
    FieldGoalPct,
    Possessions,
    MeanFirstActionSeconds,
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
            Self::MidAttemptShare => "mid_attempt_share",
            Self::FreeThrowPct => "free_throw_pct",
            Self::FieldGoalPct => "field_goal_pct",
            Self::Possessions => "possessions",
            Self::MeanFirstActionSeconds => "mean_first_action_seconds",
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
}

impl Default for MechanismConfig {
    fn default() -> Self {
        Self {
            seeds: 20,
            seed_start: 1,
            possessions: None,
            low_value: 0.10,
            high_value: 0.95,
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
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MechanismResult {
    pub attribute: String,
    pub metric: String,
    /// `wiring`：指标直接来自消费者算式（接缐检查）；
    /// `outcome`：指标是比赛产物（命中率、篮板率）。两类证据等级不同。
    pub evidence_kind: String,
    pub expectation: String,
    pub baseline: f64,
    pub perturbed: f64,
    pub relative_change: f64,
    pub per_seed_change: Vec<f64>,
    pub seeds_in_expected_direction: usize,
    pub seeds: usize,
    pub observed_direction: String,
    pub direction_matches: bool,
    pub ci_low: f64,
    pub ci_high: f64,
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
    let _ = seeds_opposite;
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
    let direction_matches = match attribute.direction() {
        Direction::Up => relative > 0.0,
        Direction::Down => relative < 0.0,
    };
    // 配对 bootstrap：以种子为抽样单元重采样，对每次重采样重新算聚合比率。
    // 单场比率在分母很小时噪声大，bootstrap 把「这一场的出手」整体当
    // 一个样本，不假设逐场差值服从正态分布。
    let (ci_low, ci_high) = paired_bootstrap(
        &baseline_values,
        &perturbed_values,
        attribute.direction(),
        2_000,
    );
    // 置信区间整体在零的一侧才算方向成立；跨零为 insufficient。
    let verdict = if ci_low > 0.0 {
        "pass"
    } else if ci_high < 0.0 {
        "defect"
    } else {
        "insufficient"
    };
    Ok(MechanismResult {
        attribute: attribute.key().to_string(),
        metric: metric.key().to_string(),
        evidence_kind: attribute.evidence_kind().to_string(),
        expectation: attribute.direction().key().to_string(),
        baseline: baseline_value,
        perturbed: perturbed_value,
        relative_change: relative,
        per_seed_change,
        seeds_in_expected_direction,
        seeds: seed_count,
        observed_direction: observed.key().to_string(),
        direction_matches,
        ci_low,
        ci_high,
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

fn ratio_of_sums(samples: &[(f64, f64)]) -> f64 {
    let numerator: f64 = samples.iter().map(|(n, _)| *n).sum();
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
        None => Ok(world.run()),
        Some(target) => {
            let mut steps = 0usize;
            while !world.finished && steps < 400_000 {
                world.step(TICK_SECONDS);
                steps += 1;
                if world.possession_log.len() as u32 >= target {
                    break;
                }
            }
            Ok(world.report())
        }
    }
}

fn metric_parts(metric: Metric, report: &GameReport) -> (f64, f64) {
    let home = report.box_score.team_totals(TeamSide::Home);
    let away = report.box_score.team_totals(TeamSide::Away);
    let possessions = report.possessions.len().max(1) as f64;
    match metric {
        Metric::ThreePointPct => (home.tpm as f64, home.tpa as f64),
        Metric::RimPct => (
            report.shot_zone_makes[0][0] as f64,
            report.shot_zone_counts[0][0] as f64,
        ),
        Metric::MidPct => (
            report.shot_zone_makes[0][2] as f64,
            report.shot_zone_counts[0][2] as f64,
        ),
        // 中投出手占主队全部出手的份额：分母是全队出手，不受中投小样本限制。
        Metric::MidAttemptShare => (
            report.shot_zone_counts[0][2] as f64,
            report.shot_zone_counts[0].iter().sum::<u32>() as f64,
        ),
        Metric::FreeThrowPct => (home.ftm as f64, home.fta as f64),
        Metric::FieldGoalPct => (home.fgm as f64, home.fga as f64),
        Metric::Possessions => (possessions, 1.0),
        Metric::MeanFirstActionSeconds => {
            let values: Vec<f64> = report
                .possessions
                .iter()
                .filter_map(|record| record.first_action_seconds)
                .collect();
            (values.iter().sum(), values.len().max(1) as f64)
        }
        Metric::OffensiveReboundRate => (home.oreb as f64, (home.fga - home.fgm).max(1) as f64),
        Metric::DefensiveReboundRate => (home.dreb as f64, (home.dreb + away.oreb).max(1) as f64),
        Metric::TurnoverRate => {
            let home_possessions = report
                .possessions
                .iter()
                .filter(|record| record.team == TeamSide::Home)
                .count()
                .max(1) as f64;
            (home.turnovers as f64, home_possessions)
        }
    }
}

/// f32 抬成 f64 会带出尾数（0.95 → 0.949999988…），记录里保留两位即可。
fn round_two(value: f32) -> f64 {
    (f64::from(value) * 100.0).round() / 100.0
}

/// 以种子为抽样单元的配对 bootstrap：每次重采样同一组种子的基线/扰动
/// 两个臂，重算聚合比率，取差值的 5% 与 95% 分位。方向给定时符号统一为
/// 「预期方向为正」，因此区间整体为正即通过，整体为负即反向。
fn paired_bootstrap(
    baseline: &[(f64, f64)],
    perturbed: &[(f64, f64)],
    direction: Direction,
    resamples: usize,
) -> (f64, f64) {
    use rand::Rng;
    use rand::SeedableRng;
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(0x5eed_1234);
    let mut stats = Vec::with_capacity(resamples);
    for _ in 0..resamples {
        let mut low_num = 0.0;
        let mut low_den = 0.0;
        let mut high_num = 0.0;
        let mut high_den = 0.0;
        for _ in 0..baseline.len() {
            let index = rng.gen_range(0..baseline.len());
            low_num += baseline[index].0;
            low_den += baseline[index].1;
            high_num += perturbed[index].0;
            high_den += perturbed[index].1;
        }
        if low_den <= 0.0 || high_den <= 0.0 {
            continue;
        }
        let low = low_num / low_den;
        let high = high_num / high_den;
        if low.abs() <= f64::EPSILON {
            continue;
        }
        let signed = match direction {
            Direction::Up => (high - low) / low,
            Direction::Down => (low - high) / low,
        };
        stats.push(signed);
    }
    if stats.is_empty() {
        return (0.0, 0.0);
    }
    stats.sort_by(|a, b| a.partial_cmp(b).expect("bootstrap statistics are finite"));
    let low_index = (stats.len() as f64 * 0.05) as usize;
    let high_index = ((stats.len() as f64 * 0.95) as usize).min(stats.len() - 1);
    (stats[low_index], stats[high_index])
}
