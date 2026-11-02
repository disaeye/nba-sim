//! 动作链内核的验证入口。
//!
//! 旧验证体系逐回合读取旧引擎的观测事实；本模块为新内核提供同一位置的
//! 判定入口：四层判定加箱体守恒。判定单元是动作与回合，聚合统计只在
//! 分布层出现。

use std::collections::BTreeMap;
use std::path::Path;

use crate::mechanism_core::MechanismReport;
use nba_sim_core::action::Fact;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{gate, GameReport, MatchSetup, Params, PlayerLine, TeamSide, World};
use serde::{Deserialize, Serialize};

pub use crate::mechanism_core::{load_known_insufficient, KnownInsufficient};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerCheck {
    pub game: u64,
    pub points_reconcile: bool,
    pub attempts_reconcile: bool,
    pub rebounds_reconcile: bool,
    pub turnovers_reconcile: bool,
    pub minutes_reconcile: bool,
    pub fouls_reconcile: bool,
    pub possessions_reconcile: bool,
    pub player_stats_reconcile: bool,
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
    /// 分布层需要完整比赛结果，才能与赛季基准比较。
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
                || !game.ledger.fouls_reconcile
                || !game.ledger.possessions_reconcile
                || !game.ledger.player_stats_reconcile
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

/// 验证器与引擎共用规则档案，确保边界条件采用同一规则版本。
pub fn rule_profile() -> nba_domain::GameRules {
    nba_domain::GameRules::default()
}

/// 跑一组种子并产出判定报告。
pub fn run_core_validation(seed_start: u64, games: u64) -> Result<CoreValidationReport, String> {
    if games == 0 {
        return Err("core validation requires at least one game".to_string());
    }
    run_core_validation_with_dir(seed_start, games, &workspace_calibration_dir())
}

/// 以指定比赛范围运行核心验证。
pub fn run_core_validation_scoped(
    seed_start: u64,
    games: u64,
    scope: ValidationScope,
) -> Result<CoreValidationReport, String> {
    run_core_validation_scoped_with_dir(seed_start, games, scope, &workspace_calibration_dir())
}

/// 以指定校准目录运行核心验证。
pub fn run_core_validation_with_dir(
    seed_start: u64,
    games: u64,
    calibration_dir: &Path,
) -> Result<CoreValidationReport, String> {
    run_core_validation_scoped_with_dir(seed_start, games, ValidationScope::Full, calibration_dir)
}

/// 在指定校准目录与比赛范围运行核心验证。
pub fn run_core_validation_scoped_with_dir(
    seed_start: u64,
    games: u64,
    scope: ValidationScope,
    calibration_dir: &Path,
) -> Result<CoreValidationReport, String> {
    if games == 0 {
        return Err("core validation requires at least one game".to_string());
    }
    let params = Params::load(calibration_dir)?;
    let mut results = Vec::new();
    let mut merged = GameReport::empty();
    let mut geometric_violations = 0;
    let mut context_violations = 0;
    for offset in 0..games {
        let seed = seed_start + offset;
        let mut report = run_scoped_game(seed, &params, scope);
        let ledger = check_ledger_scoped(seed, &report, scope.is_full());
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
        if scope.is_full() {
            merged.merge_for_gate(&report);
        } else {
            geometric_violations += report
                .violations
                .iter()
                .filter(|violation| violation.starts_with("geometry:"))
                .count();
            context_violations += report
                .violations
                .iter()
                .filter(|violation| violation.starts_with("context:"))
                .count();
        }
        drop_event_stream(&mut report);
    }
    let mut gate = if scope.is_full() {
        gate::evaluate_merged(&merged, &params)
    } else {
        gate::GateReport::default()
    };
    if !scope.is_full() {
        gate.geometric_violations = geometric_violations;
        gate.context_violations = context_violations;
    }
    let ledger_failures = results
        .iter()
        .filter(|result| {
            !result.ledger.points_reconcile
                || !result.ledger.attempts_reconcile
                || !result.ledger.rebounds_reconcile
                || !result.ledger.turnovers_reconcile
                || !result.ledger.minutes_reconcile
                || !result.ledger.fouls_reconcile
                || !result.ledger.possessions_reconcile
                || !result.ledger.player_stats_reconcile
        })
        .count();
    let geometry_violations = gate.geometric_violations;
    let context_violations = gate.context_violations;
    let passed = ledger_failures == 0
        && geometry_violations == 0
        && context_violations == 0
        && (!scope.is_full() || gate.passed);
    Ok(CoreValidationReport {
        scope: scope.as_str(),
        distribution_applicable: scope.is_full(),
        geometry_violations,
        context_violations,
        ledger_failures,
        ticks: results.iter().map(|result| result.ticks).sum(),
        events: results.iter().map(|result| result.events).sum(),
        open_business_instances: results
            .iter()
            .map(|result| result.open_business_instances)
            .sum(),
        passed,
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
            while !world.finished && world.clock.period() < 2 {
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

/// 验证完成后释放过程流，降低批次峰值内存。
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

pub fn check_ledger(seed: u64, report: &GameReport) -> LedgerCheck {
    check_ledger_scoped(seed, report, true)
}

fn check_ledger_scoped(seed: u64, report: &GameReport, require_closed: bool) -> LedgerCheck {
    let mut points_reconcile = true;
    let mut attempts_reconcile = true;
    let mut rebounds_reconcile = true;
    let mut turnovers_reconcile = true;
    let mut minutes_reconcile = true;
    let mut fouls_reconcile = true;
    let mut possessions_reconcile = true;
    let mut player_stats_reconcile = true;
    let mut detail = String::new();

    let reconstructed = reconstruct(report);
    if !report.events.is_empty() || !report.possessions.is_empty() {
        if let Err(error) = audit_possession_events(report, require_closed) {
            possessions_reconcile = false;
            detail.push_str(&format!("possession event ledger: {error}; "));
        }
    }
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
    let box_personal_fouls = home.fouls + away.fouls;
    if reconstructed.player_fouls_total != box_personal_fouls {
        fouls_reconcile = false;
        detail.push_str(&format!(
            "event fouls {} != box score {}; ",
            reconstructed.player_fouls_total, box_personal_fouls
        ));
    }
    let box_shooting_fouls = home.shooting_fouls + away.shooting_fouls;
    if reconstructed.shooting_foul_total != box_shooting_fouls {
        fouls_reconcile = false;
        detail.push_str(&format!(
            "event shooting fouls {} != box score {}; ",
            reconstructed.shooting_foul_total, box_shooting_fouls
        ));
    }
    let box_and_ones = home.and_one + away.and_one;
    if reconstructed.and_ones_total != box_and_ones {
        fouls_reconcile = false;
        detail.push_str(&format!(
            "event and-one fouls {} != box score {}; ",
            reconstructed.and_ones_total, box_and_ones
        ));
    }
    let box_blocks = home.blocks + away.blocks;
    if reconstructed.blocks != box_blocks {
        player_stats_reconcile = false;
        detail.push_str(&format!(
            "event blocks {} != box score {}; ",
            reconstructed.blocks, box_blocks
        ));
    }
    let mut event_rebounds = BTreeMap::new();
    for (player, (offensive, defensive)) in &reconstructed.player_rebounds {
        event_rebounds.insert(*player, offensive + defensive);
    }
    let player_ledgers: [(&BTreeMap<usize, u32>, fn(&PlayerLine) -> u32, &str); 15] = [
        (&reconstructed.player_points, |line| line.points, "points"),
        (&reconstructed.player_assists, |line| line.assists, "assists"),
        (&reconstructed.player_free_throw_attempts, |line| line.fta, "free-throw attempts"),
        (&reconstructed.player_free_throw_makes, |line| line.ftm, "free-throw makes"),
        (&reconstructed.personal_fouls, |line| line.fouls, "personal fouls"),
        (&reconstructed.player_field_goal_attempts, |line| line.fga, "field-goal attempts"),
        (&reconstructed.player_field_goal_makes, |line| line.fgm, "field-goal makes"),
        (&reconstructed.player_three_attempts, |line| line.tpa, "three-point attempts"),
        (&reconstructed.player_three_makes, |line| line.tpm, "three-point makes"),
        (&reconstructed.shooting_fouls, |line| line.shooting_fouls, "shooting fouls"),
        (&reconstructed.and_ones, |line| line.and_one, "and-one"),
        (&reconstructed.player_steals, |line| line.steals, "steals"),
        (&reconstructed.player_turnovers, |line| line.turnovers, "turnovers"),
        (&reconstructed.player_blocks, |line| line.blocks, "blocks"),
        (&event_rebounds, PlayerLine::rebounds, "rebounds"),
    ];
    for (values, field, label) in player_ledgers {
        assert_player_ledger(report, values, field, label, &mut player_stats_reconcile, &mut detail);
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
            detail.push_str(&format!("{:?} player points {} != team points {}; ", team, player_points, totals.points));
        }
        let event_stats = reconstructed.team_stats[World::team_index(team)];
        let expected_stats = [
            totals.points,
            totals.fgm,
            totals.fga,
            totals.tpm,
            totals.tpa,
            totals.ftm,
            totals.fta,
            totals.oreb,
            totals.dreb,
            totals.assists,
            totals.steals,
            totals.blocks,
            totals.turnovers,
            totals.fouls,
            totals.shooting_fouls,
            totals.and_one,
        ];
        if event_stats != expected_stats {
            player_stats_reconcile = false;
            detail.push_str(&format!(
                "{:?} team stats {:?} != box score {:?}; ",
                team, event_stats, expected_stats
            ));
        }
        let regulation_periods = report.periods.min(report.regulation_periods);
        let overtime_periods = report.periods.saturating_sub(report.regulation_periods);
        let game_seconds = regulation_periods as f64 * report.period_duration_seconds as f64
            + overtime_periods as f64 * report.overtime_duration_seconds as f64;
        let maximum_ticks = (game_seconds / report.tick_seconds as f64).ceil() as u64 + 1;
        let team_ticks = lines
            .iter()
            .map(|line| (line.seconds / report.tick_seconds as f64).round() as u64)
            .sum::<u64>();
        if team_ticks > maximum_ticks.saturating_mul(5) {
            minutes_reconcile = false;
            detail.push_str(&format!(
                "{:?} player-time ticks {} exceed the game roster capacity {}; ",
                team,
                team_ticks,
                maximum_ticks.saturating_mul(5)
            ));
        }
    }
    if detail.is_empty() {
        detail.push_str("conserved");
    }
    LedgerCheck {
        game: seed,
        points_reconcile,
        attempts_reconcile,
        rebounds_reconcile,
        turnovers_reconcile,
        minutes_reconcile,
        fouls_reconcile,
        possessions_reconcile,
        player_stats_reconcile,
        detail,
    }
}

#[derive(Default)]
pub struct Reconstructed {
    pub points: u32,
    pub fga: u32,
    pub fgm: u32,
    pub fta: u32,
    pub ftm: u32,
    pub rebounds: u32,
    pub turnovers: u32,
    pub steals: u32,
    pub blocks: u32,
    pub player_fouls_total: u32,
    pub shooting_foul_total: u32,
    pub and_ones_total: u32,
    pub player_stats_total: u32,
    pub team_stats: [[u32; 16]; 2],
    pub player_points: BTreeMap<usize, u32>,
    pub player_assists: BTreeMap<usize, u32>,
    pub player_free_throw_attempts: BTreeMap<usize, u32>,
    pub player_free_throw_makes: BTreeMap<usize, u32>,
    pub player_steals: BTreeMap<usize, u32>,
    pub player_blocks: BTreeMap<usize, u32>,
    pub personal_fouls: BTreeMap<usize, u32>,
    pub shooting_fouls: BTreeMap<usize, u32>,
    pub and_ones: BTreeMap<usize, u32>,
    pub player_turnovers: BTreeMap<usize, u32>,
    pub player_rebounds: BTreeMap<usize, (u32, u32)>,
    pub player_field_goal_attempts: BTreeMap<usize, u32>,
    pub player_field_goal_makes: BTreeMap<usize, u32>,
    pub player_three_attempts: BTreeMap<usize, u32>,
    pub player_three_makes: BTreeMap<usize, u32>,
}

/// 从事实流独立重建比赛总量。
pub fn reconstruct(report: &GameReport) -> Reconstructed {
    let mut totals = Reconstructed::default();
    for event in &report.events {
        match &event.fact {
            Fact::Shot { shooter, is_three, blocked_by, made, points, assisted_by, .. } => {
                let shooting_team = player_team_index(report, *shooter);
                totals.fga += 1;
                totals.team_stats[shooting_team][2] += 1;
                *totals.player_field_goal_attempts.entry(*shooter).or_default() += 1;
                if *is_three { totals.team_stats[shooting_team][4] += 1; *totals.player_three_attempts.entry(*shooter).or_default() += 1; }
                if let Some(blocker) = blocked_by { totals.blocks += 1; totals.team_stats[player_team_index(report, *blocker)][11] += 1; *totals.player_blocks.entry(*blocker).or_default() += 1; }
                if *made { totals.fgm += 1; totals.points += *points as u32; totals.team_stats[shooting_team][0] += u32::from(*points); totals.team_stats[shooting_team][1] += 1; *totals.player_field_goal_makes.entry(*shooter).or_default() += 1; if *is_three { totals.team_stats[shooting_team][3] += 1; *totals.player_three_makes.entry(*shooter).or_default() += 1; } *totals.player_points.entry(*shooter).or_default() += u32::from(*points); }
                if let Some(passer) = assisted_by.filter(|_| *made) { totals.team_stats[player_team_index(report, passer)][9] += 1; *totals.player_assists.entry(passer).or_default() += 1; }
            }
            Fact::FreeThrow { shooter, made, .. } => {
                let team = player_team_index(report, *shooter); totals.fta += 1; totals.team_stats[team][6] += 1; *totals.player_free_throw_attempts.entry(*shooter).or_default() += 1;
                if *made { totals.ftm += 1; totals.points += 1; totals.team_stats[team][0] += 1; totals.team_stats[team][5] += 1; *totals.player_free_throw_makes.entry(*shooter).or_default() += 1; *totals.player_points.entry(*shooter).or_default() += 1; }
            }
            Fact::Rebound { player, offensive, .. } => { totals.rebounds += 1; let team = player_team_index(report, *player); let counts = totals.player_rebounds.entry(*player).or_default(); if *offensive { totals.team_stats[team][7] += 1; counts.0 += 1; } else { totals.team_stats[team][8] += 1; counts.1 += 1; } }
            Fact::Turnover { player, .. } => { totals.turnovers += 1; totals.team_stats[player_team_index(report, *player)][12] += 1; *totals.player_turnovers.entry(*player).or_default() += 1; }
            Fact::Steal { player, .. } => { totals.steals += 1; totals.team_stats[player_team_index(report, *player)][10] += 1; *totals.player_steals.entry(*player).or_default() += 1; }
            Fact::Foul { fouler, fouled, kind, and_one, .. } => { totals.player_fouls_total += 1; totals.team_stats[player_team_index(report, *fouler)][13] += 1; if nba_domain::rules::counts_toward_personal_limit(*kind) { *totals.personal_fouls.entry(*fouler).or_default() += 1; } if matches!(kind, nba_domain::event::FoulKind::Shooting) { totals.shooting_foul_total += 1; totals.team_stats[player_team_index(report, *fouler)][14] += 1; *totals.shooting_fouls.entry(*fouler).or_default() += 1; } if *and_one { totals.and_ones_total += 1; totals.team_stats[player_team_index(report, *fouled)][15] += 1; *totals.and_ones.entry(*fouled).or_default() += 1; } }
            _ => {}
        }
    }
    totals
}

fn audit_possession_events(report: &GameReport, require_closed: bool) -> Result<(), String> {
    use nba_sim_core::action::Fact;
    use std::collections::BTreeMap;

    let mut active: Option<(u64, TeamSide)> = None;
    let mut started = BTreeMap::new();
    let mut ended = BTreeMap::new();
    for event in &report.events {
        match &event.fact {
            Fact::PossessionStarted { team, possession_id, .. } => {
                if *possession_id == 0 || started.insert(*possession_id, *team).is_some() {
                    return Err(format!("possession {possession_id} has a zero or duplicate start"));
                }
                if let Some((active_id, _)) = active {
                    return Err(format!("possession {active_id} remained active when {possession_id} started"));
                }
                active = Some((*possession_id, *team));
            }
            Fact::PossessionEnded { team, possession_id, end_reason, .. } => {
                if active != Some((*possession_id, *team)) {
                    return Err(format!("possession end {possession_id} does not match the active identity"));
                }
                if ended.insert(*possession_id, *end_reason).is_some() {
                    return Err(format!("possession {possession_id} ended more than once"));
                }
                active = None;
            }
            _ => {}
        }
    }
    let active_count = usize::from(active.is_some());
    if require_closed {
        if active.is_some() {
            return Err(format!("completed report ends with active possession {:?}", active));
        }
        if started.len() != report.possessions.len() || ended.len() != report.possessions.len() {
            return Err(format!(
                "{} starts, {} ends, {} records",
                started.len(),
                ended.len(),
                report.possessions.len()
            ));
        }
    } else if started.len() != ended.len() + active_count
        || ended.len() != report.possessions.len()
    {
        return Err(format!(
            "{} starts, {} ends, {} records, {} active",
            started.len(),
            ended.len(),
            report.possessions.len(),
            active_count
        ));
    }
    for record in &report.possessions {
        if started.get(&record.id) != Some(&record.team) || ended.get(&record.id).is_none() {
            return Err(format!("record {} has no matching start and end events", record.id));
        }
    }
    Ok(())
}

fn assert_player_ledger(report: &GameReport, reconstructed: &BTreeMap<usize, u32>, field: fn(&PlayerLine) -> u32, label: &str, player_stats_reconcile: &mut bool, detail: &mut String) {
    let mut seen = BTreeMap::new();
    for (handle, value) in reconstructed { seen.insert(*handle, *value); }
    for handle in 0..report.box_score.home.len() + report.box_score.away.len() {
        let line = if handle < report.box_score.home.len() { &report.box_score.home[handle] } else { &report.box_score.away[handle - report.box_score.home.len()] };
        let observed = seen.remove(&handle).unwrap_or_default();
        let expected = field(line);
        if observed != expected { *player_stats_reconcile = false; detail.push_str(&format!("player {handle} {label}: event {observed} != box score {expected}; ")); }
    }
    assert!(seen.is_empty(), "reconstructed player ledger contains an unknown player");
}

fn player_team_index(report: &GameReport, player: usize) -> usize {
    if player < report.box_score.home.len() { 0 } else { assert!(player - report.box_score.home.len() < report.box_score.away.len(), "event references a player outside the box score"); 1 }
}

/// 批次判定总览：把四层判定与机制实验合成一份可读结论。
pub fn certification_artifact(
    report: &CoreValidationReport,
    mechanism: Option<&MechanismReport>,
    commit: &str,
    known_insufficient: &[KnownInsufficient],
) -> serde_json::Value {
    let mut defects = Vec::new();
    let mut insufficient = Vec::new();
    let mut known = Vec::new();
    let mut new_insufficient = Vec::new();
    let mut not_applicable = Vec::new();
    let mut marginal = Vec::new();
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
                    Some(ratio) => format!("mechanism:{}（下界比 {ratio:.2}）", result.attribute),
                    None => format!("mechanism:{}", result.attribute),
                });
            }
        }
    }
    for attribute in &insufficient {
        match known_insufficient.iter().find(|entry| entry.attribute == *attribute) {
            Some(entry) if entry.resolution == "more_games" => {
                known.push(format!("mechanism:{attribute}"));
            }
            Some(entry) => not_applicable.push(format!("mechanism:{}（{}）", attribute, entry.reason)),
            None => new_insufficient.push(format!("mechanism:{attribute}")),
        }
    }
    let status = if !defects.is_empty() || !new_insufficient.is_empty() {
        "defects_found"
    } else if !known.is_empty() {
        "insufficient_evidence"
    } else {
        "pass"
    };
    let games = report.games.len();
    let clean_games = report.games.iter().filter(|game| {
        game.ledger.points_reconcile
            && game.ledger.attempts_reconcile
            && game.ledger.rebounds_reconcile
            && game.ledger.turnovers_reconcile
            && game.ledger.minutes_reconcile
            && game.ledger.fouls_reconcile
            && game.ledger.possessions_reconcile
            && game.ledger.player_stats_reconcile
    }).count();
    let evaluated: std::collections::BTreeSet<&str> = report.gate.layers.iter()
        .map(|layer| layer.criterion.as_str()).collect();
    let sampled_gate = REGISTERED_GATE_CRITERIA.iter()
        .filter(|name| evaluated.contains(*name)).count();
    let unregistered: Vec<String> = evaluated.iter()
        .filter(|name| !REGISTERED_GATE_CRITERIA.contains(name))
        .map(|name| (*name).to_string()).collect();
    let mechanism_attributes = mechanism.map(|value| value.results.len()).unwrap_or(0);
    let registered = REGISTERED_GATE_CRITERIA.len()
        + crate::mechanism_core::MechanismAttribute::all().len();
    let sampled = sampled_gate + mechanism_attributes;
    serde_json::json!({
        "certification_status": status,
        "commit": commit,
        "games": games,
        "per_game_ledger": { "passed": clean_games, "total": games },
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
            "not_applicable": not_applicable.len(),
        },
    })
}

/// 已登记的判定准则。
pub const REGISTERED_GATE_CRITERIA: [&str; 37] = [
    "zone_share_rim", "zone_share_near", "zone_share_mid", "zone_share_three",
    "creation_share_drive", "creation_share_cut", "creation_share_post",
    "creation_share_pull_up", "creation_share_putback", "creation_share_transition",
    "creation_share_set", "turnover_per_possession", "fga_per_possession",
    "fta_per_game", "points_per_game", "oreb_per_game", "possessions_per_game",
    "blocks_per_game", "steals_per_game", "shooting_fouls_per_game", "and_one_per_game",
    "early_contested_rate", "usage_top1_over_fifth", "assisted_fg_spread", "ping_pong_rate",
    "minutes_max", "shot_clock_buckets_complete", "defender_distance_buckets_complete",
    "post_scorer_three_share", "stationary_shooter_three_share", "play_activation_rate",
    "play_window_gap", "substitutions_per_game", "pass_chain_mean", "screen_read_under_shoot",
    "screen_read_over_drive", "shot_points_by_clock",
];

/// 逐准则的观测值与基准，供批次统计工件消费。
pub fn batch_statistical_artifact(
    report: &CoreValidationReport,
    params: &Params,
) -> serde_json::Value {
    let games = report.games.len().max(1) as f64;
    let points: f64 = report.games.iter()
        .map(|game| (game.home_score + game.away_score) as f64).sum();
    let possessions: f64 = report.games.iter()
        .map(|game| game.possessions as f64).sum();
    let criteria: Vec<serde_json::Value> = report.gate.layers.iter().map(|layer| {
        let band = params.policy.band(&layer.criterion);
        serde_json::json!({
            "criterion": layer.criterion,
            "observed": layer.observed,
            "target": layer.target,
            "band": { "lower": band.lower, "upper": band.upper },
            "relative_error": layer.relative_error,
            "passed": layer.passed,
        })
    }).collect();
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
