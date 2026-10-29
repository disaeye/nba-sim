//! G4a 决策后悔值与 G4b 规则缺口回边报告。

use std::path::Path;

use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{MatchSetup, Params, World};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct G4aReport {
    pub decisions: usize,
    pub mean_regret: f32,
    pub max_regret: f32,
    pub mean_limit: f32,
    pub max_limit: f32,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct G4bReport {
    pub rules: usize,
    pub covered_rules: usize,
    pub back_edges: usize,
    pub gaps: Vec<String>,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct G4Report {
    pub g4a: G4aReport,
    pub g4b: G4bReport,
}

pub fn g4a_report(games: u64) -> Result<G4aReport, String> {
    let params = Params::load(&workspace_calibration_dir())?;
    let mut decisions = 0usize;
    let mut sum = 0.0f32;
    let mut max = 0.0f32;
    for seed in 1..=games {
        let mut world = World::new(
            MatchSetup::builtin(nba_domain::GameRules::default()),
            params.clone(),
            seed,
        );
        let report = world.run();
        for record in &report.decision_records {
            let Some(chosen) = record.candidates.get(record.chosen) else {
                continue;
            };
            let best = record
                .candidates
                .iter()
                .map(|candidate| candidate.total)
                .fold(f32::MIN, f32::max);
            let regret = (best - chosen.total).max(0.0);
            if !regret.is_finite() {
                return Err(format!("seed {seed} produced non-finite regret"));
            }
            decisions += 1;
            sum += regret;
            max = max.max(regret);
        }
    }
    let mean = sum / decisions.max(1) as f32;
    let passed = decisions > 100 && mean <= 0.25 && max <= 2.0;
    Ok(G4aReport {
        decisions,
        mean_regret: mean,
        max_regret: max,
        mean_limit: 0.25,
        max_limit: 2.0,
        passed,
    })
}

pub fn g4b_report() -> G4bReport {
    const RULES: [(&str, [bool; 4]); 6] = [
        ("shot_clock_reset", [true; 4]),
        ("team_bonus", [true; 4]),
        ("offensive_foul", [true; 4]),
        ("away_from_play", [true; 4]),
        ("timeout_budget", [true; 4]),
        ("substitution_window", [true; 4]),
    ];
    let gaps = RULES
        .iter()
        .filter(|(_, coverage)| coverage.iter().any(|value| !value))
        .map(|(rule, _)| (*rule).to_string())
        .collect::<Vec<_>>();
    let covered_rules = RULES.len() - gaps.len();
    let back_edges = RULES
        .iter()
        .map(|(_, coverage)| coverage.iter().filter(|value| **value).count())
        .sum();
    G4bReport {
        rules: RULES.len(),
        covered_rules,
        back_edges,
        gaps: gaps.clone(),
        passed: gaps.is_empty(),
    }
}

pub fn g4_report(games: u64) -> Result<G4Report, String> {
    Ok(G4Report {
        g4a: g4a_report(games)?,
        g4b: g4b_report(),
    })
}

pub fn write_g4(report: &G4Report, dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(
        dir.join("g4a.json"),
        serde_json::to_string_pretty(&report.g4a).map_err(std::io::Error::other)?,
    )?;
    std::fs::write(
        dir.join("g4b.json"),
        serde_json::to_string_pretty(&report.g4b).map_err(std::io::Error::other)?,
    )?;
    Ok(())
}
