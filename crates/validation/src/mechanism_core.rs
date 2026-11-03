//! 机制实验：属性扰动后回合响应方向。
//!
//! 每个实验对同一组种子跑两次：基准阵容与单属性提高后的阵容。指标按
//! 分子分母汇总后再比较，避免小样本比率噪声。裁决以种子为抽样单元做
//! 配对 bootstrap：区间整体在零的预期一侧才通过，跨零记证据不足。
//!
//! 这层替代旧引擎的 MECH 实验。旧实现在单次请求里同步跑上千次短跑，
//! 三百分秒不返回；这里把重实验留在离线命令，端点只读缓存工件。
//!
//! 哪些属性被排除、为什么，只写在 `data/calibration/known_insufficient.json`。

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
    Block,
    Steal,
}

impl MechanismAttribute {
    pub fn all() -> [Self; 13] {
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
            Self::Block,
            Self::Steal,
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
            Self::Block => "block",
            Self::Steal => "steal",
        }
    }

    pub fn metric(self) -> Metric {
        match self {
            Self::ShootingThree => Metric::ThreePointPct,
            Self::ShootingClose => Metric::RimPct,
            Self::ShootingMid => Metric::MidPct,
            Self::FreeThrow => Metric::FreeThrowPct,
            // 速度与加速决定推进与出手的节奇：更快的球队整场打出更多回合。
            Self::Speed | Self::Acceleration => Metric::Possessions,
            Self::Stamina => Metric::MinutesMax,
            Self::Strength => Metric::ScreenDisplacement,
            Self::OffensiveRebound => Metric::OffensiveReboundRate,
            Self::DefensiveRebound => Metric::DefensiveReboundRate,
            Self::DecisionIq => Metric::TurnoversPerPass,
            Self::Block => Metric::BlocksPerShot,
            Self::Steal => Metric::StealsPerPossession,
        }
    }

    pub fn direction(self) -> Direction {
        match self {
            Self::DecisionIq => Direction::Down,
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
                Self::Block => attributes.block = value,
                Self::Steal => attributes.steal = value,
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
    TurnoversPerPass,
    ScreenDisplacement,
    OffensiveReboundRate,
    DefensiveReboundRate,
    TurnoverRate,
    BlocksPerShot,
    StealsPerPossession,
    /// 主队单人最高出场分钟：耐力决定一个球员能留在场上的时长。
    MinutesMax,
    /// 有对抗篮板的归属份额：卡位加成只在这种篮板里生效。
    ContestedReboundShare,
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
            Self::TurnoversPerPass => "turnovers_per_pass",
            Self::ScreenDisplacement => "screen_displacement_ft",
            Self::OffensiveReboundRate => "offensive_rebound_rate",
            Self::DefensiveReboundRate => "defensive_rebound_rate",
            Self::TurnoverRate => "turnover_rate",
            Self::BlocksPerShot => "blocks_per_shot",
            Self::StealsPerPossession => "steals_per_possession",
            Self::MinutesMax => "minutes_max",
            Self::ContestedReboundShare => "contested_rebound_share",
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
            // 160 个子样本：能力扰动的真实效应量在 1%–5%，40 与 80 个
            // 样本的配对区间仍会跨零（防守篮板尤其），再翻倍才稳定。
            seeds: 160,
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
    /// `wiring`：指标直接来自消费者算式（接线检查）；
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
    /// 配对 bootstrap 的 5% 分位，与 `relative_change` 同向：正数表示上升。
    pub ci_low: f64,
    /// 配对 bootstrap 的 95% 分位，与 `relative_change` 同向：正数表示上升。
    pub ci_high: f64,
    /// 区间宽度，与 `relative_change` 同单位。
    pub interval_width: f64,
    /// 靠近零的边界与点估计的比值；只在通过的项上有值。比值小于一半
    /// 说明下界不到点估计的一半。
    pub lower_bound_ratio: Option<f64>,
    /// 边缘通过：区间宽度超过点估计本身，下界离零很近。
    pub marginal: bool,
    pub verdict: String,
    /// `not_applicable` 的结构性原因；其余裁决为空串。
    pub reason: String,
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

/// 已登记的排除项。声明只写一份：`data/calibration/known_insufficient.json`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownInsufficient {
    pub attribute: String,
    pub reason: String,
    /// `more_games`：效应存在、当前样本分辨不出，`required_games` 给出所需场次；
    /// `mechanism_change`：效应结构上接近零，放大场次解决不了。
    pub resolution: String,
    #[serde(default)]
    pub required_games: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct KnownInsufficientFile {
    entries: Vec<KnownInsufficient>,
}

/// 读登记表。`more_games` 必须带 `required_games`，否则加载直接失败。
pub fn load_known_insufficient(dir: &Path) -> Result<Vec<KnownInsufficient>, String> {
    let path = dir.join("known_insufficient.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path).map_err(|error| {
        format!(
            "known-insufficient table {} unreadable: {error}",
            path.display()
        )
    })?;
    let file: KnownInsufficientFile = serde_json::from_str(&text).map_err(|error| {
        format!(
            "known-insufficient table {} invalid: {error}",
            path.display()
        )
    })?;
    validate_known_entries(&file.entries)?;
    Ok(file.entries)
}

/// 校验登记项：`more_games` 必须给出所需场次，未知 resolution 直接拒绝。
pub fn validate_known_entries(entries: &[KnownInsufficient]) -> Result<(), String> {
    for entry in entries {
        match entry.resolution.as_str() {
            "more_games" => {
                if entry.required_games.is_none() {
                    return Err(format!(
                        "known-insufficient entry `{}` is `more_games` but has no required_games",
                        entry.attribute
                    ));
                }
            }
            "mechanism_change" => {}
            other => {
                return Err(format!(
                    "known-insufficient entry `{}` has unknown resolution `{other}`",
                    entry.attribute
                ))
            }
        }
    }
    Ok(())
}

fn declared(
    known: &[KnownInsufficient],
    attribute: MechanismAttribute,
) -> Option<&KnownInsufficient> {
    known
        .iter()
        .find(|entry| entry.attribute == attribute.key())
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
    let known = load_known_insufficient(calibration)?;
    experiment(&params, attribute, config, declared(&known, attribute))
}

pub fn run_mechanism_experiments_with_dir(
    config: MechanismConfig,
    calibration: &Path,
) -> Result<MechanismReport, String> {
    let params = Params::load(calibration)?;
    let known = load_known_insufficient(calibration)?;
    let mut results = Vec::new();
    for attribute in MechanismAttribute::all() {
        // 逐项输出：机制实验是长跑，崩在中间时至少要能看出是哪一项。
        let started = std::time::Instant::now();
        let result = experiment(&params, attribute, config, declared(&known, attribute))?;
        eprintln!(
            "MECHANISM {} {} rel={:.4} seeds={}/{} {:.1}s",
            attribute.key(),
            result.verdict,
            result.relative_change,
            result.seeds_in_expected_direction,
            result.seeds,
            started.elapsed().as_secs_f64()
        );
        results.push(result);
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
    declaration: Option<&KnownInsufficient>,
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
    // 一个样本，不假设逐场差值服从正态分布。区间与 `relative_change`
    // 同向：正数表示上升。
    let (ci_low, ci_high) = paired_bootstrap(&baseline_values, &perturbed_values, 2_000);
    // 把区间投影到预期方向：`low > 0` 表示整个区间都在预期一侧。结构上
    // 不响应扰动的属性先登记为 not_applicable，测量值仍然保留在记录里。
    let (low, high) = match attribute.direction() {
        Direction::Up => (ci_low, ci_high),
        Direction::Down => (-ci_high, -ci_low),
    };
    let judged = if low > 0.0 {
        "pass"
    } else if high < 0.0 {
        "defect"
    } else {
        "insufficient"
    };
    let (verdict, reason) = match declaration {
        Some(entry) if entry.resolution == "mechanism_change" => {
            ("not_applicable", entry.reason.clone())
        }
        _ => (judged, String::new()),
    };
    // 靠近零的那一侧边界与点估计的比值：区间对称时，比值小于一半等价于
    // 区间宽度超过点估计本身。只在通过的项上有意义。
    let lower_bound_ratio = if verdict == "pass" && relative.abs() > f64::EPSILON {
        Some(low / relative.abs())
    } else {
        None
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
        interval_width: ci_high - ci_low,
        lower_bound_ratio,
        marginal: verdict == "pass" && (ci_high - ci_low) > relative.abs(),
        verdict: verdict.to_string(),
        reason,
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
    // 步数上限是硬约束：没有它，一场走不完的比赛会一直堆事件，
    // 直到把内存吃光为止。超限就地报错，不让它静默跑下去。
    const STEP_LIMIT: usize = 400_000;
    // 事件上限：一场比赛的事件量有确定上界，越过它说明内核在原地打转。
    // 在这里就地报错，不让事件日志涨到把内存吃光。
    const EVENT_LIMIT: usize = 200_000;
    let mut steps = 0usize;
    while !world.finished && steps < STEP_LIMIT {
        world.step(TICK_SECONDS);
        steps += 1;
        if world.events.len() > EVENT_LIMIT {
            return Err(format!(
                "seed {seed} emitted {} events after {steps} steps",
                world.events.len()
            ));
        }
        if let Some(target) = possessions {
            if world.possession_log.len() as u32 >= target {
                break;
            }
        }
    }
    if !world.finished && possessions.is_none() {
        return Err(format!(
            "seed {seed} did not finish within {STEP_LIMIT} steps"
        ));
    }
    Ok(world.report())
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
        // 中投出手占主队全部出手的份额：分母是整队出手，不受中投小样本限制。
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
        Metric::ScreenDisplacement => {
            let values: Vec<f64> = report.events.iter().filter_map(|event| {
                if let nba_sim_core::action::Fact::Screen { team, displacement_ft, .. } = &event.fact {
                    (*team == TeamSide::Home).then_some(f64::from(*displacement_ft))
                } else {
                    None
                }
            }).collect();
            (values.iter().sum(), values.len().max(1) as f64)
        }
        Metric::OffensiveReboundRate => (home.oreb as f64, (home.fga - home.fgm).max(1) as f64),
        // 防守篮板率的分母是对手投丢的球：拿自己的防守篮板除以「自己的
        // 防守篮板 + 对手前场篮板」会把分子分母一起抬高，扰动被自己抵消。
        Metric::DefensiveReboundRate => (
            home.dreb as f64,
            (away.fga as i64 - away.fgm as i64).max(1) as f64,
        ),
        Metric::TurnoverRate => {
            let home_possessions = report
                .possessions
                .iter()
                .filter(|record| record.team == TeamSide::Home)
                .count()
                .max(1) as f64;
            (home.turnovers as f64, home_possessions)
        }
        Metric::TurnoversPerPass => {
            let home_passes: u32 = report
                .possessions
                .iter()
                .filter(|record| record.team == TeamSide::Home)
                .map(|record| record.passes)
                .sum();
            (home.turnovers as f64, home_passes.max(1) as f64)
        }
        // 主队封盖 ÷ 客队出手：封盖是对手出手上的防守产物。
        Metric::BlocksPerShot => {
            let away = report.box_score.team_totals(TeamSide::Away);
            (home.blocks as f64, away.fga.max(1) as f64)
        }
        // 主队抢断 ÷ 客队回合：抢断剥夺的是对手的回合。
        Metric::StealsPerPossession => {            let away_possessions = report
                .possessions
                .iter()
                .filter(|record| record.team == TeamSide::Away)
                .count()
                .max(1) as f64;
            (home.steals as f64, away_possessions)
        }
        Metric::MinutesMax => {
            let max_minutes = report
                .box_score
                .home
                .iter()
                .map(|line| line.minutes())
                .fold(0.0, f64::max);
            (max_minutes, 1.0)
        }
        Metric::ContestedReboundShare => (
            report.contested_rebounds[0] as f64,
            (report.contested_rebounds[0] + report.contested_rebounds[1]).max(1) as f64,
        )
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
        stats.push((high - low) / low);
    }
    if stats.is_empty() {
        return (0.0, 0.0);
    }
    stats.sort_by(|a, b| a.partial_cmp(b).expect("bootstrap statistics are finite"));
    let low_index = (stats.len() as f64 * 0.05) as usize;
    let high_index = ((stats.len() as f64 * 0.95) as usize).min(stats.len() - 1);
    (stats[low_index], stats[high_index])
}
