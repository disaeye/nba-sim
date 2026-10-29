//! 动作链内核的验证入口。
//!
//! 旧验证体系逐回合读取旧引擎的观测事实；本模块为新内核提供同一位置的
//! 判定入口：四层判定加箱体守恒。判定单元是动作与回合，聚合统计只在
//! 分布层出现。

use std::path::Path;

use nba_sim_core::action::Fact;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{gate, GameReport, MatchSetup, Params, TeamSide, World};
use serde::{Deserialize, Serialize};

pub use crate::mechanism_core::{load_known_insufficient, KnownInsufficient};

use crate::mechanism_core::MechanismReport;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerCheck {
    pub game: u64,
    pub points_reconcile: bool,
    pub attempts_reconcile: bool,
    pub rebounds_reconcile: bool,
    pub turnovers_reconcile: bool,
    pub minutes_reconcile: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreGameResult {
    pub seed: u64,
    pub home_score: u32,
    pub away_score: u32,
    pub possessions: usize,
    pub ticks: u64,
    pub events: u64,
    pub open_business_instances: u64,
    pub box_score: nba_sim_core::BoxScore,
    pub ledger: LedgerCheck,
    pub violations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreValidationReport {
    pub games: Vec<CoreGameResult>,
    pub scope: String,
    /// 分布层是否适用：只有整场比赛才与赛季基准可比。
    pub distribution_applicable: bool,
    pub gate: gate::GateReport,
    pub geometry_violations: usize,
    pub context_violations: usize,
    pub ledger_failures: usize,
    pub ticks: u64,
    pub events: u64,
    pub open_business_instances: u64,
    pub passed: bool,
}

impl CoreValidationReport {
    pub fn failed_criteria(&self) -> Vec<String> {
        let mut failures = Vec::new();
        if self.geometry_violations > 0 {
            failures.push(format!("geometry violations: {}", self.geometry_violations));
        }
        if self.context_violations > 0 {
            failures.push(format!("context violations: {}", self.context_violations));
        }
        for layer in self.gate.layers.iter().filter(|layer| !layer.passed) {
            failures.push(format!(
                "{}: observed {:.3} target {:.3}",
                layer.criterion, layer.observed, layer.target
            ));
        }
        for game in &self.games {
            if !game.ledger.points_reconcile
                || !game.ledger.attempts_reconcile
                || !game.ledger.rebounds_reconcile
                || !game.ledger.turnovers_reconcile
                || !game.ledger.minutes_reconcile
            {
                failures.push(format!("ledger: seed {} {}", game.seed, game.ledger.detail));
            }
        }
        failures
    }
}

/// 判定范围。分布层需要整场比赛；几何、语境与账本对部分比赛同样成立。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationScope {
    Full,
    Quarter,
    Possessions(u32),
}

impl ValidationScope {
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "full" => Self::Full,
            "1q" | "quarter" => Self::Quarter,
            other => other
                .trim_end_matches('p')
                .parse::<u32>()
                .map(Self::Possessions)
                .unwrap_or(Self::Full),
        }
    }

    pub fn is_full(self) -> bool {
        matches!(self, Self::Full)
    }

    pub fn as_str(self) -> String {
        match self {
            Self::Full => "full".to_string(),
            Self::Quarter => "1q".to_string(),
            Self::Possessions(count) => format!("{count}p"),
        }
    }
}

/// 验证器与引擎共用的规则档案。验证器不另建一套阈值。
pub fn rule_profile() -> nba_domain::GameRules {
    nba_domain::GameRules::default()
}

/// 跑一组种子并产出判定报告。单场调用用于面板「对屏幕这一场判定」；
/// 批次结论由 `validate-core` 的 32 场口径负责。
pub fn run_core_validation(seed_start: u64, games: u64) -> Result<CoreValidationReport, String> {
    if games == 0 {
        return Err("core validation requires at least one game".to_string());
    }
    run_core_validation_with_dir(seed_start, games, &workspace_calibration_dir())
}

pub fn run_core_validation_scoped(
    seed_start: u64,
    games: u64,
    scope: ValidationScope,
) -> Result<CoreValidationReport, String> {
    run_core_validation_scoped_with_dir(seed_start, games, scope, &workspace_calibration_dir())
}

pub fn run_core_validation_with_dir(
    seed_start: u64,
    games: u64,
    calibration: &Path,
) -> Result<CoreValidationReport, String> {
    run_core_validation_scoped_with_dir(seed_start, games, ValidationScope::Full, calibration)
}

pub fn run_core_validation_scoped_with_dir(
    seed_start: u64,
    games: u64,
    scope: ValidationScope,
    calibration: &Path,
) -> Result<CoreValidationReport, String> {
    if games == 0 {
        return Err("core validation requires at least one game".to_string());
    }
    let params = Params::load(calibration)?;
    let mut results = Vec::new();
    // 聚合报告按场累加：每场跑完就并入并放掉，批次不会把整批事件挂在堆上。
    let mut merged = GameReport::empty();
    for offset in 0..games {
        let seed = seed_start + offset;
        let mut report = run_scoped_game(seed, &params, scope);
        let ledger = check_ledger(seed, &report);
        results.push(CoreGameResult {
            seed,
            home_score: report.home_score,
            away_score: report.away_score,
            possessions: report.possessions.len(),
            ticks: report.ticks,
            events: report.events.len() as u64,
            open_business_instances: report.open_business_instances,
            box_score: report.box_score.clone(),
            ledger,
            violations: report.violations.clone(),
        });
        merged.merge_for_gate(&report);
        // 账本对账、违规计数与分布判定都已用完这一场的事件流。
        drop_event_stream(&mut report);
    }
    // 分布层只在整场比赛口径下判定；部分比赛的得分、回合数与区域份额
    // 与赛季基准不可比。
    let gate = if scope.is_full() {
        gate::evaluate_merged(&merged, &params)
    } else {
        gate::GateReport::default()
    };
    let ledger_failures = results
        .iter()
        .filter(|result| {
            !result.ledger.points_reconcile
                || !result.ledger.attempts_reconcile
                || !result.ledger.rebounds_reconcile
                || !result.ledger.turnovers_reconcile
                || !result.ledger.minutes_reconcile
        })
        .count();
    Ok(CoreValidationReport {
        scope: scope.as_str(),
        distribution_applicable: scope.is_full(),
        geometry_violations: gate.geometric_violations,
        context_violations: gate.context_violations,
        ledger_failures,
        ticks: results.iter().map(|result| result.ticks).sum(),
        events: results.iter().map(|result| result.events).sum(),
        open_business_instances: results
            .iter()
            .map(|result| result.open_business_instances)
            .sum(),
        passed: ledger_failures == 0
            && gate.geometric_violations == 0
            && gate.context_violations == 0
            && (!scope.is_full() || gate.passed),
        games: results,
        gate,
    })
}

fn run_scoped_game(seed: u64, params: &Params, scope: ValidationScope) -> GameReport {
    let setup = MatchSetup::builtin(rule_profile());
    let mut world = World::new(setup, params.clone(), seed);
    match scope {
        ValidationScope::Full => world.run(),
        ValidationScope::Quarter => {
            while !world.finished && world.clock.period < 2 {
                world.step(TICK_SECONDS);
            }
            world.report()
        }
        ValidationScope::Possessions(target) => {
            while !world.finished && world.possession_log.len() < target as usize {
                world.step(TICK_SECONDS);
            }
            world.report()
        }
    }
}

/// 账本对账需要事实流；判定完成后把流放掉。
///
/// 判定批次只保留每场的箱体、回合与统计。事实流、决策快照与逐次样本在
/// 判定之外没有消费者，全部留在报告里会让 32 场批次吃掉数百兆。
/// 几何与语境违规由结算时写入 violations，分布层只读箱体与聚合计数，
/// 账本对账在流放掉之前完成。
fn drop_event_stream(report: &mut GameReport) {
    report.events.clear();
    report.events.shrink_to_fit();
    report.decision_traces.clear();
    report.decision_traces.shrink_to_fit();
    report.decision_records.clear();
    report.decision_records.shrink_to_fit();
    report.decision_samples.clear();
    report.decision_samples.shrink_to_fit();
}

fn check_ledger(seed: u64, report: &GameReport) -> LedgerCheck {
    let mut points_reconcile = true;
    let mut attempts_reconcile = true;
    let mut rebounds_reconcile = true;
    let mut turnovers_reconcile = true;
    let mut minutes_reconcile = true;
    let mut detail = String::new();

    let reconstructed = reconstruct(report);
    let box_points = report.box_score.team_totals(TeamSide::Home).points
        + report.box_score.team_totals(TeamSide::Away).points;
    if reconstructed.points != box_points {
        points_reconcile = false;
        detail.push_str(&format!(
            "event stream points {} != box score points {}; ",
            reconstructed.points, box_points
        ));
    }
    let home = report.box_score.team_totals(TeamSide::Home);
    let away = report.box_score.team_totals(TeamSide::Away);
    if reconstructed.fga != home.fga + away.fga || reconstructed.fta != home.fta + away.fta {
        attempts_reconcile = false;
        detail.push_str(&format!(
            "event stream attempts {:?}/{:?} != box score {:?}/{:?}; ",
            reconstructed.fga,
            reconstructed.fta,
            home.fga + away.fga,
            home.fta + away.fta
        ));
    }
    let box_rebounds = home.oreb + home.dreb + away.oreb + away.dreb;
    if reconstructed.rebounds != box_rebounds {
        rebounds_reconcile = false;
        detail.push_str(&format!(
            "event stream rebounds {} != box score {}; ",
            reconstructed.rebounds, box_rebounds
        ));
    }
    let box_turnovers = home.turnovers + away.turnovers;
    if reconstructed.turnovers != box_turnovers {
        turnovers_reconcile = false;
        detail.push_str(&format!(
            "event stream turnovers {} != box score {}; ",
            reconstructed.turnovers, box_turnovers
        ));
    }

    for team in [TeamSide::Home, TeamSide::Away] {
        let lines = match team {
            TeamSide::Home => &report.box_score.home,
            TeamSide::Away => &report.box_score.away,
        };
        let totals = report.box_score.team_totals(team);
        let player_points: u32 = lines.iter().map(|line| line.points).sum();
        if player_points != totals.points {
            points_reconcile = false;
            detail.push_str(&format!(
                "{:?} player points {} != team points {}; ",
                team, player_points, totals.points
            ));
        }
        let expected = 2 * (totals.fgm - totals.tpm) + 3 * totals.tpm + totals.ftm;
        if expected != totals.points {
            points_reconcile = false;
            detail.push_str(&format!(
                "{:?} shot ledger {} != team points {}; ",
                team, expected, totals.points
            ));
        }
        if totals.fgm > totals.fga || totals.ftm > totals.fta || totals.tpm > totals.fga {
            attempts_reconcile = false;
            detail.push_str(&format!("{:?} attempts exceed makes; ", team));
        }
        let minutes: f64 = lines.iter().map(|line| line.minutes()).sum();
        let period = nba_domain::GameRules::default()
            .league
            .period_duration_seconds as f64;
        let ceiling = 4.0 * period / 60.0 * 5.0 + 60.0;
        if minutes > ceiling {
            minutes_reconcile = false;
            detail.push_str(&format!(
                "{:?} total minutes {:.1} exceed the roster ceiling {:.1}; ",
                team, minutes, ceiling
            ));
        }
    }
    if detail.is_empty() {
        detail.push_str("conserved");
    }
    let _ = seed;
    LedgerCheck {
        game: seed,
        points_reconcile,
        attempts_reconcile,
        rebounds_reconcile,
        turnovers_reconcile,
        minutes_reconcile,
        detail,
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Reconstructed {
    pub points: u32,
    pub fga: u32,
    pub fgm: u32,
    pub fta: u32,
    pub ftm: u32,
    pub rebounds: u32,
    pub turnovers: u32,
    pub assists: u32,
}

/// 从事实流独立重建比赛总量。重建只读事实，不读箱体，
/// 因此两侧一旦分叉就说明有一侧在说谎。
pub fn reconstruct(report: &GameReport) -> Reconstructed {
    let mut totals = Reconstructed::default();
    for event in &report.events {
        match &event.fact {
            Fact::Shot {
                made,
                points,
                assisted_by,
                ..
            } => {
                totals.fga += 1;
                if *made {
                    totals.fgm += 1;
                    totals.points += *points as u32;
                }
                if assisted_by.is_some() && *made {
                    totals.assists += 1;
                }
            }
            Fact::FreeThrow { made, .. } => {
                totals.fta += 1;
                if *made {
                    totals.ftm += 1;
                    totals.points += 1;
                }
            }
            Fact::Rebound { .. } => totals.rebounds += 1,
            Fact::Turnover { .. } => totals.turnovers += 1,
            _ => {}
        }
    }
    totals
}

/// 批次判定总览：把四层判定与机制实验合成一份可读结论。
///
/// 状态三态与旧认证保持同一词汇：`pass`、`defects_found`、
/// `insufficient_evidence`。判定面覆盖统计的是实际评估过的准则数。
pub fn certification_artifact(
    report: &CoreValidationReport,
    mechanism: Option<&MechanismReport>,
    commit: &str,
    known_insufficient: &[KnownInsufficient],
) -> serde_json::Value {
    let mut defects: Vec<String> = Vec::new();
    let mut insufficient: Vec<String> = Vec::new();
    let mut known: Vec<String> = Vec::new();
    let mut new_insufficient: Vec<String> = Vec::new();
    let mut not_applicable: Vec<String> = Vec::new();
    let mut marginal: Vec<String> = Vec::new();
    for layer in &report.gate.layers {
        if !layer.passed {
            defects.push(layer.criterion.clone());
        }
    }
    if report.geometry_violations > 0 {
        defects.push("geometry_legality".to_string());
    }
    if report.context_violations > 0 {
        defects.push("context_legality".to_string());
    }
    if report.ledger_failures > 0 {
        defects.push("box_score_ledger".to_string());
    }
    if let Some(mechanism) = mechanism {
        for result in &mechanism.results {
            match result.verdict.as_str() {
                "defect" => defects.push(format!("mechanism:{}", result.attribute)),
                "insufficient" => insufficient.push(result.attribute.clone()),
                "not_applicable" => not_applicable.push(format!(
                    "mechanism:{}（{}）",
                    result.attribute, result.reason
                )),
                _ => {}
            }
            if result.marginal {
                marginal.push(match result.lower_bound_ratio {
                    Some(ratio) => {
                        format!("mechanism:{}（下界比 {ratio:.2}）", result.attribute)
                    }
                    None => format!("mechanism:{}", result.attribute),
                });
            }
        }
    }
    for attribute in &insufficient {
        match known_insufficient
            .iter()
            .find(|entry| entry.attribute == *attribute)
        {
            Some(entry) if entry.resolution == "more_games" => {
                known.push(format!("mechanism:{attribute}"))
            }
            Some(entry) => {
                not_applicable.push(format!("mechanism:{}（{}）", attribute, entry.reason))
            }
            None => new_insufficient.push(format!("mechanism:{attribute}")),
        }
    }
    // 已知不足只记录；未登记的不足按缺陷处理，否则新出现的不足会被
    // 「已知清单」掩盖。结构上不适用的准则单独记录，不挂不足。
    let status = if !defects.is_empty() || !new_insufficient.is_empty() {
        "defects_found"
    } else if !known.is_empty() {
        "insufficient_evidence"
    } else {
        "pass"
    };
    let games = report.games.len();
    let clean_games = report
        .games
        .iter()
        .filter(|game| {
            game.ledger.points_reconcile
                && game.ledger.attempts_reconcile
                && game.ledger.rebounds_reconcile
                && game.ledger.turnovers_reconcile
                && game.ledger.minutes_reconcile
        })
        .count();
    // 覆盖统计以声明的清单为基准：新增判据却未登记时，uncovered 会
    // 从零变为正数；已评估但未登记的准则单独列在 unregistered。
    let evaluated: std::collections::BTreeSet<&str> = report
        .gate
        .layers
        .iter()
        .map(|layer| layer.criterion.as_str())
        .collect();
    let sampled_gate = REGISTERED_GATE_CRITERIA
        .iter()
        .filter(|name| evaluated.contains(*name))
        .count();
    let unregistered: Vec<String> = evaluated
        .iter()
        .filter(|name| !REGISTERED_GATE_CRITERIA.contains(name))
        .map(|name| (*name).to_string())
        .collect();
    let mechanism_attributes = mechanism.map(|report| report.results.len()).unwrap_or(0);
    let not_applicable_count = not_applicable.len();
    let registered =
        REGISTERED_GATE_CRITERIA.len() + crate::mechanism_core::MechanismAttribute::all().len();
    let sampled = sampled_gate + mechanism_attributes;
    serde_json::json!({
        "certification_status": status,
        "commit": commit,
        "games": games,
        "per_game_ledger": {
            "passed": clean_games,
            "total": games,
        },
        "criteria": {
            "defects": defects,
            "known_insufficient": known,
            "new_insufficient": new_insufficient,
            "not_applicable": not_applicable,
            "marginal": marginal,
        },
        "coverage": {
            "registered": registered,
            "sampled": sampled,
            "uncovered": registered.saturating_sub(sampled),
            "unregistered": unregistered,
            "gate_criteria": REGISTERED_GATE_CRITERIA.len(),
            "mechanism_attributes": mechanism_attributes,
            "not_applicable": not_applicable_count,
        },
    })
}

/// 已登记的判定准则。新增判据必须在此登记，否则 `coverage.uncovered`
/// 会暴露缺口。
///
/// 分布层 21 条与过程层 16 条都在此列：过程判据同样是一次判定，
/// 未登记就等于覆盖统计看不出它已经被评估过。
pub const REGISTERED_GATE_CRITERIA: [&str; 37] = [
    "zone_share_rim",
    "zone_share_near",
    "zone_share_mid",
    "zone_share_three",
    "creation_share_drive",
    "creation_share_cut",
    "creation_share_post",
    "creation_share_pull_up",
    "creation_share_putback",
    "creation_share_transition",
    "creation_share_set",
    "turnover_per_possession",
    "fga_per_possession",
    "fta_per_game",
    "points_per_game",
    "oreb_per_game",
    "possessions_per_game",
    "blocks_per_game",
    "steals_per_game",
    "shooting_fouls_per_game",
    "and_one_per_game",
    // 过程层：设计边界类判据，与分布层同样一次判定一次登记。
    "early_contested_rate",
    "usage_top1_over_fifth",
    "assisted_fg_spread",
    "ping_pong_rate",
    "minutes_max",
    "shot_clock_buckets_complete",
    "defender_distance_buckets_complete",
    "post_scorer_three_share",
    "stationary_shooter_three_share",
    "play_activation_rate",
    "play_window_gap",
    "substitutions_per_game",
    "pass_chain_mean",
    "screen_read_under_shoot",
    "screen_read_over_drive",
    "shot_points_by_clock",
];

/// 逐准则的观测值与基准，供批次统计工件消费。
pub fn batch_statistical_artifact(
    report: &CoreValidationReport,
    params: &Params,
) -> serde_json::Value {
    let games = report.games.len().max(1) as f64;
    let points: f64 = report
        .games
        .iter()
        .map(|game| (game.home_score + game.away_score) as f64)
        .sum();
    let possessions: f64 = report
        .games
        .iter()
        .map(|game| game.possessions as f64)
        .sum();
    let criteria: Vec<serde_json::Value> = report
        .gate
        .layers
        .iter()
        .map(|layer| {
            let band = params.policy.band(&layer.criterion);
            serde_json::json!({
                "criterion": layer.criterion,
                "observed": layer.observed,
                "target": layer.target,
                "band": { "lower": band.lower, "upper": band.upper },
                "relative_error": layer.relative_error,
                "passed": layer.passed,
            })
        })
        .collect();
    serde_json::json!({
        "games": report.games.len(),
        "criteria": criteria,
        "totals": {
            "points_per_game": points / games,
            "possessions_per_game": possessions / games,
            "geometry_violations": report.geometry_violations,
            "context_violations": report.context_violations,
            "ledger_failures": report.ledger_failures,
        },
    })
}
