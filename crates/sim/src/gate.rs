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
    /// 相对误差 `observed / target - 1`；分布层的判定量与工件记录同一份。
    #[serde(default)]
    pub relative_error: f64,
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
        let error = if target > 0.0 {
            observed / target - 1.0
        } else {
            0.0
        };
        let passed = target > 0.0 && error >= band.lower && error <= band.upper;
        gate.layers.push(LayerResult {
            layer: "distribution".to_string(),
            criterion: criterion.to_string(),
            observed,
            target,
            relative_error: error,
            passed,
            detail: format!(
                "relative error {error:.3} (band {:.3}..{:.3})",
                band.lower, band.upper
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
    // 基准按「真正走到篮板的投失」计算：犯规导致的投失不产生篮板，
    // 用 FGA−FGM 做分母会把犯规率的变化误算成篮板能力的变化。
    let reboundable_misses = report.rebound_attempts as f64 / games;
    push(
        "oreb_per_game",
        oreb as f64 / games,
        league.oreb_per_miss as f64 * reboundable_misses,
    );
    push(
        "possessions_per_game",
        possessions / games,
        league.possessions_per_game as f64,
    );
    let home_line = report.box_score.team_totals(crate::TeamSide::Home);
    let away_line = report.box_score.team_totals(crate::TeamSide::Away);
    push(
        "blocks_per_game",
        (home_line.blocks + away_line.blocks) as f64 / games,
        league.blocks_per_game as f64,
    );
    push(
        "steals_per_game",
        (home_line.steals + away_line.steals) as f64 / games,
        league.steals_per_game as f64,
    );
    push(
        "shooting_fouls_per_game",
        (home_line.shooting_fouls + away_line.shooting_fouls) as f64 / games,
        league.shooting_fouls_per_game as f64,
    );
    push(
        "and_one_per_game",
        (home_line.and_one + away_line.and_one) as f64 / games,
        league.and_one_per_game as f64,
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
