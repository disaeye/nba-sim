//! 五维聚合诊断：把一次批量运行的过程量按五个维度聚合成可回归的报告。
//!
//! 五个维度对应引擎的五类可观测过程——决策可解释性、情境敏感性、
//! 战术分层、运动与惯性、规则与账本。指标上下界与探测参数全部来自
//! `data/diagnostics/five_dimension.json`，报告字段固定、取值只依赖
//! 种子区间，因此可以直接当回归基线比对。

use std::path::Path;

use serde::{Deserialize, Serialize};

use nba_domain::rules::GameRules;
use nba_sim_core::action::Fact;
use nba_sim_core::endgame::{leverage, win_probability_at};
use nba_sim_core::params::{workspace_calibration_dir, workspace_root};
use nba_sim_core::physical::Physical;
use nba_sim_core::state::CandidateTrace;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, TeamSide, World};

pub const CONFIG_PATH: &str = "data/diagnostics/five_dimension_bands.json";

#[derive(Debug, Clone, Deserialize)]
pub struct Probe {
    pub leverage_remaining_seconds: f64,
    pub leverage_step_seconds: f64,
    pub leverage_sweep_steps: u32,
    pub leverage_sweep_step_seconds: f64,
    pub win_probability_remaining_seconds: f64,
    pub win_probability_max_deficit: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Tolerance {
    pub probability_sum: f32,
    pub monotone: f32,
    pub win_monotone: f64,
    pub velocity: f32,
    pub acceleration: f32,
    pub momentum: f32,
    pub teleport: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BandSpec {
    pub dimension: String,
    pub key: String,
    pub lower: f64,
    pub upper: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EndgameWindow {
    pub period: u32,
    pub game_clock_seconds: f64,
    pub three_point_zone: u8,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiagnosticConfig {
    pub probe: Probe,
    pub endgame: EndgameWindow,
    pub tolerance: Tolerance,
    pub events_per_thousand: f64,
    pub bands: Vec<BandSpec>,
}

impl DiagnosticConfig {
    pub fn load() -> Result<Self, String> {
        let path = workspace_root().join(CONFIG_PATH);
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        serde_json::from_str(&text).map_err(|error| format!("parse {}: {error}", path.display()))
    }

    fn band(&self, dimension: &str, key: &str) -> (f64, f64) {
        let spec = self
            .bands
            .iter()
            .find(|spec| spec.dimension == dimension && spec.key == key)
            .unwrap_or_else(|| panic!("五维诊断配置缺少 {dimension}.{key} 的上下界"));
        (spec.lower, spec.upper)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metric {
    pub key: String,
    pub value: f64,
    pub lower: f64,
    pub upper: f64,
    pub passed: bool,
}

impl Metric {
    fn measure(config: &DiagnosticConfig, dimension: &str, key: &str, value: f64) -> Self {
        let (lower, upper) = config.band(dimension, key);
        Self {
            key: key.to_string(),
            value,
            lower,
            upper,
            passed: value.is_finite() && value >= lower && value <= upper,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dimension {
    pub name: String,
    pub metrics: Vec<Metric>,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiveDimensionReport {
    pub seeds: Vec<u64>,
    pub dimensions: Vec<Dimension>,
    pub passed: bool,
}

impl FiveDimensionReport {
    pub fn dimension(&self, name: &str) -> Option<&Dimension> {
        self.dimensions.iter().find(|d| d.name == name)
    }

    pub fn failed_metrics(&self) -> Vec<String> {
        let mut failures = Vec::new();
        for dimension in &self.dimensions {
            for metric in &dimension.metrics {
                if !metric.passed {
                    failures.push(format!(
                        "{}/{} = {:.4} 不在 [{:.4}, {:.4}]",
                        dimension.name, metric.key, metric.value, metric.lower, metric.upper
                    ));
                }
            }
        }
        failures
    }
}

#[derive(Default)]
struct Tally {
    decisions: u32,
    well_formed: u32,
    candidate_sum: f64,
    beta_sum: f64,
    candidates: u64,
    finite_candidates: u64,
    families: Vec<u32>,
    traces_with_bonus: u32,
    passes: u32,
    shots: u32,
    drives: u32,
    post_ups: u32,
    plays: u32,
    possessions: u32,
    endgame_shots: u32,
    endgame_threes: u32,
    player_ticks: u64,
    speed_violations: u64,
    accel_violations: u64,
    momentum_violations: u64,
    events: u64,
    violations: u64,
    games: u32,
    finished: u32,
    reconciled: u32,
}

fn finite_utility(candidate: &CandidateTrace) -> bool {
    let utility = &candidate.utility;
    utility.epv.sum().is_finite()
        && utility.variance.is_finite()
        && utility.clock_cost.is_finite()
        && utility.foul_cost.is_finite()
        && utility.tactical_bonus.is_finite()
        && candidate.total.is_finite()
        && candidate.probability.is_finite()
}

fn accumulate(tally: &mut Tally, world: &World, config: &DiagnosticConfig) {
    let unity = f32::from(1u8);
    for trace in &world.decision_traces {
        tally.decisions += 1;
        let sum: f32 = trace.candidates.iter().map(|c| c.probability).sum();
        if trace.candidates.len() >= usize::from(2u8)
            && (sum - unity).abs() <= config.tolerance.probability_sum
        {
            tally.well_formed += 1;
        }
        tally.candidate_sum += trace.candidates.len() as f64;
        tally.beta_sum += f64::from(trace.beta);
        for candidate in &trace.candidates {
            tally.candidates += 1;
            if finite_utility(candidate) {
                tally.finite_candidates += 1;
            }
        }
        if trace
            .candidates
            .get(trace.chosen)
            .is_some_and(|candidate| candidate.utility.tactical_bonus != f32::default())
        {
            tally.traces_with_bonus += 1;
        }
    }
    for sample in &world.decision_samples {
        let slot = sample.family as usize;
        if slot >= tally.families.len() {
            tally.families.resize(slot + 1, u32::default());
        }
        tally.families[slot] += 1;
    }
    for event in &world.events {
        tally.events += 1;
        match &event.fact {
            Fact::Pass { .. } => tally.passes += 1,
            Fact::Shot { .. } => tally.shots += 1,
            Fact::Drive { .. } => tally.drives += 1,
            Fact::PostUp { .. } => tally.post_ups += 1,
            Fact::PlayActivated { .. } => tally.plays += 1,
            _ => {}
        }
    }
    tally.possessions += world.possession_log.len() as u32;
    for sample in &world.shot_samples {
        if sample.period >= config.endgame.period
            && sample.game_clock <= config.endgame.game_clock_seconds
        {
            tally.endgame_shots += 1;
            if sample.zone == config.endgame.three_point_zone {
                tally.endgame_threes += 1;
            }
        }
    }
    tally.violations += world.violations.len() as u64;
    tally.games += 1;
    if world.finished {
        tally.finished += 1;
    }
    let mut reconciled = true;
    for team in [TeamSide::Home, TeamSide::Away] {
        // 账本对账用箱体积分与比分：回合日志只覆盖回合内得分，
        // 技术犯规罚球与节末压哨不在同一个回合里。
        let lines = match team {
            TeamSide::Home => &world.box_score.home,
            TeamSide::Away => &world.box_score.away,
        };
        let points: u32 = lines.iter().map(|line| line.points).sum();
        if points != world.score[World::team_index(team)] {
            reconciled = false;
        }
    }
    if reconciled {
        tally.reconciled += 1;
    }
}

fn entropy_bits(counts: &[u32]) -> f64 {
    let total: u32 = counts.iter().sum();
    if total == 0 {
        return f64::default();
    }
    let total = f64::from(total);
    let mut entropy = f64::default();
    for count in counts {
        if *count == 0 {
            continue;
        }
        let p = f64::from(*count) / total;
        entropy -= p * p.log2();
    }
    entropy
}

fn dimension(name: &str, metrics: Vec<Metric>) -> Dimension {
    let passed = metrics.iter().all(|metric| metric.passed);
    Dimension {
        name: name.to_string(),
        metrics,
        passed,
    }
}

fn share(part: u64, total: u64) -> f64 {
    if total == 0 {
        return f64::default();
    }
    part as f64 / total as f64
}

fn mean(sum: f64, count: u64) -> f64 {
    if count == 0 {
        return f64::default();
    }
    sum / count as f64
}

pub fn run_five_dimension(seed_start: u64, games: u64) -> Result<FiveDimensionReport, String> {
    if games == 0 {
        return Err("five-dimension report requires at least one game".to_string());
    }
    let config = DiagnosticConfig::load()?;
    let params = Params::load(&workspace_calibration_dir())?;
    let tolerance = &config.tolerance;
    let mut tally = Tally::default();
    let mut seeds = Vec::new();
    let dt = TICK_SECONDS as f32;
    for offset in 0..games {
        let seed = seed_start + offset;
        seeds.push(seed);
        let mut world = World::new(
            MatchSetup::builtin(GameRules::default()),
            params.clone(),
            seed,
        );
        let mut previous_velocity: Vec<_> = world.players.iter().map(|player| player.vel).collect();
        let mut previous_position: Vec<_> = world.players.iter().map(|player| player.pos).collect();
        let mut previous_distance: Vec<_> = world
            .players
            .iter()
            .map(|player| (player.target - player.pos).length())
            .collect();
        let mut previous_target: Vec<_> = world.players.iter().map(|player| player.target).collect();
        let arrival = world.params.policy.g("motion_arrival_epsilon_ft");
        let momentum_cap = world.params.policy.w("momentum_cap");
        while !world.finished {
            world.step(TICK_SECONDS);
            let policy = world.params.policy.clone();
            for (index, player) in world.players.iter().enumerate() {
                if !player.on_court {
                    continue;
                }
                let physical = Physical::of(player, &policy);
                tally.player_ticks += 1;
                if player.vel.length() > physical.speed_ftps + tolerance.velocity {
                    tally.speed_violations += 1;
                }
                // 只判定「真的由移动积分产生位移、且不是到位停步」的帧：
                // 摆位重置与换人是瞬移，目标点当帧变化时也可能伴随重新摆位。
                let travelled = player.pos - previous_position[index];
                if (travelled - player.vel * dt).length() <= tolerance.teleport
                    && previous_distance[index] > arrival
                    && (player.target - previous_target[index]).length() <= tolerance.teleport
                {
                    let change = (player.vel - previous_velocity[index]).length();
                    if change > physical.accel_ftps2 * dt + tolerance.acceleration {
                        tally.accel_violations += 1;
                    }
                }
                previous_velocity[index] = player.vel;
                previous_position[index] = player.pos;
                previous_distance[index] = (player.target - player.pos).length();
                previous_target[index] = player.target;
            }
            if world
                .momentum
                .iter()
                .any(|value| *value > momentum_cap + tolerance.momentum)
            {
                tally.momentum_violations += 1;
            }
        }
        accumulate(&mut tally, &world, &config);
    }

    // 情境敏感性用同一份参数做合成扫描：时间步长越小，两点的 γ 越接近；
    // 分差越大，本方的胜率越高。
    let probe = World::new(
        MatchSetup::builtin(GameRules::default()),
        params.clone(),
        seed_start,
    );
    let near = leverage(&probe, 3, config.probe.leverage_remaining_seconds).variance_appetite;
    let next = leverage(
        &probe,
        3,
        config.probe.leverage_remaining_seconds + config.probe.leverage_step_seconds,
    )
    .variance_appetite;
    let leverage_gap = f64::from((near - next).abs());
    let mut monotone_steps = u32::default();
    let mut monotone_ok = u32::default();
    let mut previous = f32::INFINITY;
    for step in 1..=config.probe.leverage_sweep_steps {
        let remaining = f64::from(step) * config.probe.leverage_sweep_step_seconds;
        let magnitude = leverage(&probe, 3, remaining).variance_appetite.abs();
        if step > 1 {
            monotone_steps += 1;
            if magnitude <= previous + tolerance.monotone {
                monotone_ok += 1;
            }
        }
        previous = magnitude;
    }
    let mut win_steps = u32::default();
    let mut win_ok = u32::default();
    let mut previous_win = f64::NEG_INFINITY;
    for delta in 0..=config.probe.win_probability_max_deficit {
        let probability = win_probability_at(
            &probe,
            TeamSide::Home,
            f64::from(delta),
            config.probe.win_probability_remaining_seconds,
            true,
        );
        if delta > 0 {
            win_steps += 1;
            if probability >= previous_win - tolerance.win_monotone {
                win_ok += 1;
            }
        }
        previous_win = probability;
    }

    let games = u64::from(tally.games);
    let decisions = u64::from(tally.decisions);
    let ticks = tally.player_ticks;
    let decision = dimension(
        "decision",
        vec![
            Metric::measure(
                &config,
                "decision",
                "trace_coverage",
                share(u64::from(tally.well_formed), decisions),
            ),
            Metric::measure(
                &config,
                "decision",
                "mean_candidates",
                mean(tally.candidate_sum, decisions),
            ),
            Metric::measure(
                &config,
                "decision",
                "decomposition_finite_share",
                share(tally.finite_candidates, tally.candidates),
            ),
            Metric::measure(
                &config,
                "decision",
                "mean_beta",
                mean(tally.beta_sum, decisions),
            ),
        ],
    );
    let situation = dimension(
        "situation",
        vec![
            Metric::measure(&config, "situation", "leverage_time_step_gap", leverage_gap),
            Metric::measure(
                &config,
                "situation",
                "leverage_magnitude_monotone_share",
                share(u64::from(monotone_ok), u64::from(monotone_steps)),
            ),
            Metric::measure(
                &config,
                "situation",
                "win_probability_monotone_share",
                share(u64::from(win_ok), u64::from(win_steps)),
            ),
            Metric::measure(
                &config,
                "situation",
                "endgame_three_share",
                share(
                    u64::from(tally.endgame_threes),
                    u64::from(tally.endgame_shots),
                ),
            ),
        ],
    );
    let tactics = dimension(
        "tactics",
        vec![
            Metric::measure(
                &config,
                "tactics",
                "play_activation_share",
                share(u64::from(tally.plays), u64::from(tally.possessions)),
            ),
            Metric::measure(
                &config,
                "tactics",
                "action_family_entropy_bits",
                entropy_bits(&tally.families),
            ),
            Metric::measure(
                &config,
                "tactics",
                "pass_share",
                share(
                    u64::from(tally.passes),
                    u64::from(tally.passes + tally.shots + tally.drives + tally.post_ups),
                ),
            ),
            Metric::measure(
                &config,
                "tactics",
                "tactical_bonus_share",
                share(u64::from(tally.traces_with_bonus), decisions),
            ),
        ],
    );
    let motion = dimension(
        "motion",
        vec![
            Metric::measure(
                &config,
                "motion",
                "speed_cap_compliance",
                share(ticks - tally.speed_violations, ticks),
            ),
            Metric::measure(
                &config,
                "motion",
                "accel_cap_compliance",
                share(ticks - tally.accel_violations, ticks),
            ),
            Metric::measure(
                &config,
                "motion",
                "momentum_cap_compliance",
                share(ticks - tally.momentum_violations, ticks),
            ),
            Metric::measure(
                &config,
                "motion",
                "reaction_delay_seconds",
                f64::from(params.policy.g("defense_reaction_delay_seconds")),
            ),
        ],
    );
    let rules = dimension(
        "rules",
        vec![
            Metric::measure(
                &config,
                "rules",
                "violations_per_1k_events",
                share(tally.violations, tally.events) * config.events_per_thousand,
            ),
            Metric::measure(
                &config,
                "rules",
                "finished_game_share",
                share(u64::from(tally.finished), games),
            ),
            Metric::measure(
                &config,
                "rules",
                "score_reconcile_share",
                share(u64::from(tally.reconciled), games),
            ),
            Metric::measure(
                &config,
                "rules",
                "events_per_game",
                mean(tally.events as f64, games),
            ),
        ],
    );

    let dimensions = vec![decision, situation, tactics, motion, rules];
    let passed = dimensions.iter().all(|dimension| dimension.passed);
    Ok(FiveDimensionReport {
        seeds,
        dimensions,
        passed,
    })
}

pub fn write_five_dimension(
    report: &FiveDimensionReport,
    dir: &Path,
) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("five_dimension.json");
    let body = serde_json::to_string_pretty(report)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    std::fs::write(&path, format!("{body}\n"))?;
    Ok(path)
}
