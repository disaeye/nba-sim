//! 动作链内核的验证入口。
//!
//! 旧验证体系逐回合读取旧引擎的观测事实；本模块为新内核提供同一位置的
//! 判定入口：四层判定加箱体守恒。判定单元是动作与回合，聚合统计只在
//! 分布层出现。

use std::path::Path;

use nba_sim_core::action::Fact;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{gate, run_game, GameReport, Params, TeamSide};
use serde::{Deserialize, Serialize};

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
    pub box_score: nba_sim_core::BoxScore,
    pub ledger: LedgerCheck,
    pub violations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreValidationReport {
    pub games: Vec<CoreGameResult>,
    pub gate: gate::GateReport,
    pub geometry_violations: usize,
    pub context_violations: usize,
    pub ledger_failures: usize,
    pub passed: bool,
}

impl CoreValidationReport {
    pub fn failed_criteria(&self) -> Vec<String> {
        let mut failures = Vec::new();
        if self.geometry_violations > 0 {
            failures.push(format!(
                "geometry violations: {}",
                self.geometry_violations
            ));
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

/// 跑一组种子并产出判定报告。样本少于两场时直接失败，避免单场噪声被当成结论。
pub fn run_core_validation(seed_start: u64, games: u64) -> Result<CoreValidationReport, String> {
    if games < 2 {
        return Err("core validation requires at least two games".to_string());
    }
    run_core_validation_with_dir(seed_start, games, &workspace_calibration_dir())
}

pub fn run_core_validation_with_dir(
    seed_start: u64,
    games: u64,
    calibration: &Path,
) -> Result<CoreValidationReport, String> {
    let params = Params::load(calibration)?;
    let mut reports = Vec::new();
    let mut results = Vec::new();
    for offset in 0..games {
        let seed = seed_start + offset;
        let report = run_game(seed, calibration);
        let ledger = check_ledger(seed, &report);
        results.push(CoreGameResult {
            seed,
            home_score: report.home_score,
            away_score: report.away_score,
            possessions: report.possessions.len(),
            box_score: report.box_score.clone(),
            ledger,
            violations: report.violations.clone(),
        });
        reports.push(report);
    }
    let gate = gate::evaluate_batch(&reports, &params);
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
        geometry_violations: gate.geometric_violations,
        context_violations: gate.context_violations,
        ledger_failures,
        passed: gate.passed && ledger_failures == 0,
        games: results,
        gate,
    })
}

fn check_ledger(seed: u64, report: &GameReport) -> LedgerCheck {    let mut points_reconcile = true;
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
) -> serde_json::Value {
    let mut defects: Vec<String> = Vec::new();
    let mut insufficient: Vec<String> = Vec::new();
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
                "insufficient" => {
                    insufficient.push(format!("mechanism:{}", result.attribute))
                }
                _ => {}
            }
        }
    }
    let status = if !defects.is_empty() {
        "defects_found"
    } else if !insufficient.is_empty() {
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
    let gate_criteria = report.gate.layers.len();
    let mechanism_attributes = mechanism.map(|report| report.results.len()).unwrap_or(0);
    serde_json::json!({
        "certification_status": status,
        "commit": commit,
        "games": games,
        "layer1_flow": {
            "scenarios_pass": clean_games,
            "scenarios_total": games,
        },
        "layer3_5_distribution": {
            "defects": defects,
            "insufficient": insufficient,
        },
        "branch_coverage": {
            "sampled": gate_criteria + mechanism_attributes,
            "scenario_only": 0,
            "uncovered": 0,
            "gate_criteria": gate_criteria,
            "mechanism_attributes": mechanism_attributes,
        },
    })
}

/// 逐准则的观测值与基准，供批次统计工件消费。
pub fn batch_statistical_artifact(report: &CoreValidationReport) -> serde_json::Value {
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
            serde_json::json!({
                "criterion": layer.criterion,
                "observed": layer.observed,
                "target": layer.target,
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
