//! 四层判定：几何合法性、语境合法性、分布比较、反事实。
//!
//! 前两层在结算时逐动作记录，后两层对批次统计做比较。聚合统计只作为投影
//! 呈现，判定单元始终是动作与回合。

use serde::{Deserialize, Serialize};

use crate::params::Params;
use crate::world::GameReport;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerResult {
    pub layer: String,
    pub criterion: String,
    pub observed: f64,
    pub target: f64,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GateReport {
    pub geometric_violations: usize,
    pub context_violations: usize,
    pub layers: Vec<LayerResult>,
    pub passed: bool,
}

impl GateReport {
    pub fn failed(&self) -> usize {
        self.layers.iter().filter(|layer| !layer.passed).count()
    }
}

/// 第三层：把批次分布与校准表比较。带宽取真实赛季的采样波动，
/// 相对误差超过 15% 记为不通过。
pub fn evaluate(report: &GameReport, params: &Params) -> GateReport {
    let shots: u32 = report.shot_zone_counts.iter().flatten().sum();
    let mut gate = GateReport {
        geometric_violations: report
            .violations
            .iter()
            .filter(|violation| violation.starts_with("geometry:"))
            .count(),
        context_violations: report
            .violations
            .iter()
            .filter(|violation| violation.starts_with("context:"))
            .count(),
        ..Default::default()
    };

    let bands = &params.policy;
    let mut push = |criterion: &str, observed: f64, target: f64| {
        let band = bands.band(criterion);
        let passed = target > 0.0 && (observed / target - 1.0).abs() <= band;
        gate.layers.push(LayerResult {
            layer: "distribution".to_string(),
            criterion: criterion.to_string(),
            observed,
            target,
            passed,
            detail: format!(
                "relative error {:.3} (band {:.3})",
                (observed / target - 1.0).abs(),
                band
            ),
        });
    };

    if shots > 0 {
        let zone_names = ["rim", "near", "mid", "three"];
        for (index, name) in zone_names.iter().enumerate() {
            let target = *params.shot_profile.zone_share.get(*name).unwrap_or(&0.0) as f64;
            let observed: u32 = report.shot_zone_counts.iter().map(|team| team[index]).sum();
            push(
                &format!("zone_share_{name}"),
                observed as f64 / shots as f64,
                target,
            );
        }
        let creation_names = [
            "drive",
            "cut",
            "post",
            "pull_up",
            "putback",
            "transition",
            "set",
        ];
        for (index, name) in creation_names.iter().enumerate() {
            let target = *params
                .shot_profile
                .creation_share
                .get(*name)
                .unwrap_or(&0.0) as f64;
            push(
                &format!("creation_share_{name}"),
                report.creation_counts[index] as f64 / shots as f64,
                target,
            );
        }
    }

    let possessions = report.possessions.len().max(1) as f64;
    let games = report.games.max(1) as f64;
    let turnovers = report
        .possessions
        .iter()
        .filter(|record| record.ended_by.starts_with("turnover"))
        .count() as f64;
    push(
        "turnover_per_possession",
        turnovers / possessions,
        params.possession.league.turnover_per_possession as f64,
    );

    let league = &params.possession.league;
    let fga = report.box_score.team_totals(crate::TeamSide::Home).fga
        + report.box_score.team_totals(crate::TeamSide::Away).fga;
    push(
        "fga_per_possession",
        fga as f64 / possessions,
        league.fga_per_possession as f64,
    );
    let fta = report.box_score.team_totals(crate::TeamSide::Home).fta
        + report.box_score.team_totals(crate::TeamSide::Away).fta;
    push(
        "fta_per_game",
        fta as f64 / games,
        league.fta_per_game as f64,
    );
    let points = (report.home_score + report.away_score) as f64;
    push(
        "points_per_game",
        points / games,
        league.points_per_game as f64,
    );
    let oreb = report.box_score.team_totals(crate::TeamSide::Home).oreb
        + report.box_score.team_totals(crate::TeamSide::Away).oreb;
    push(
        "oreb_per_game",
        oreb as f64 / games,
        (league.oreb_per_possession * league.possessions_per_game) as f64,
    );
    push(
        "possessions_per_game",
        possessions / games,
        league.possessions_per_game as f64,
    );

    gate.passed = gate.geometric_violations == 0
        && gate.context_violations == 0
        && gate.layers.iter().all(|layer| layer.passed);
    gate
}

/// 多场聚合后再判分布，避免单场采样噪声把判定打穿。
pub fn evaluate_batch(reports: &[GameReport], params: &Params) -> GateReport {
    let mut merged = GameReport::empty();
    for report in reports {
        merged.merge(report);
    }
    evaluate(&merged, params)
}
