use std::collections::BTreeMap;

use nba_sim_core::action::PassKind;
use nba_sim_core::params::{workspace_calibration_dir, RuntimeParams};
use nba_sim_core::world::MatchSetup;
use serde_json::json;

#[test]
fn possession_end_reasons_are_visible_for_calibration() {
    let params = RuntimeParams::load(&workspace_calibration_dir()).expect("runtime tables");
    let mut reasons: BTreeMap<&'static str, u32> = BTreeMap::new();
    let mut fga = 0u32;
    let mut shots = 0u32;
    let mut made = 0u32;
    let mut oreb = 0u32;
    let mut actions: BTreeMap<&'static str, u32> = BTreeMap::new();
    let mut zones: BTreeMap<&'static str, u32> = BTreeMap::new();
    let mut creations: BTreeMap<&'static str, u32> = BTreeMap::new();
    let mut turnover_kinds: BTreeMap<&'static str, u32> = BTreeMap::new();
    let mut steals = 0u32;
    let mut passes_completed = 0u32;
    let mut passes_released = 0u32;
    let mut lob_passes = 0u32;
    let mut cut_catches = 0u32;
    let mut drive_total = 0u32;
    let mut drive_reached = 0u32;
    let mut drive_separation = 0.0f64;
    let mut shot_phases: BTreeMap<&'static str, u32> = BTreeMap::new();
    let mut rebound_opportunities = 0u32;
    let mut contestable_rebounds = 0u32;
    let mut second_chances = 0u32;
    let mut durations: Vec<f64> = Vec::new();
    for seed in 1..=9 {
        let report = nba_sim_core::run_game_with_setup(
            seed,
            params.clone(),
            MatchSetup::builtin(nba_domain::GameRules::default()),
        );
        for record in &report.possessions {
            *reasons.entry(record.ended_by).or_default() += 1;
            durations.push(record.duration);
        }
        for line in report.box_score.home.iter().chain(report.box_score.away.iter()) {
            fga += line.fga;
            made += line.fgm;
            oreb += line.oreb;
        }
        for sample in &report.shot_samples {
            *shot_phases.entry(sample.phase).or_default() += 1;
        }
        for event in &report.events {
            match &event.fact {
                nba_sim_core::Fact::Shot { zone, kind, .. } => {
                    shots += 1;
                    *zones.entry(zone).or_default() += 1;
                    *creations.entry(kind.creation()).or_default() += 1;
                }
                nba_sim_core::Fact::Drive {
                    reached,
                    separation_ft,
                    ..
                } => {
                    *actions.entry("drive").or_default() += 1;
                    drive_total += 1;
                    if *reached {
                        drive_reached += 1;
                    }
                    drive_separation += f64::from(*separation_ft);
                }
                nba_sim_core::Fact::Cut { arrived, .. } => {
                    *actions.entry("cut").or_default() += 1;
                    if *arrived {
                        cut_catches += 1;
                    }
                }
                nba_sim_core::Fact::PostUp { .. } => *actions.entry("post").or_default() += 1,
                nba_sim_core::Fact::Screen { .. } => *actions.entry("screen").or_default() += 1,
                nba_sim_core::Fact::Pass {
                    completed, kind, ..
                } => {
                    *actions.entry("pass").or_default() += 1;
                    if *kind == PassKind::Lob {
                        lob_passes += 1;
                    }
                    if *completed {
                        passes_completed += 1
                    } else {
                        passes_released += 1
                    }
                }
                nba_sim_core::Fact::Turnover { kind, .. } => {
                    *turnover_kinds.entry(kind.as_str()).or_default() += 1;
                }
                nba_sim_core::Fact::Steal { .. } => steals += 1,
                nba_sim_core::Fact::ReboundOpportunity {
                    contestable,
                    ..
                } => {
                    rebound_opportunities += 1;
                    contestable_rebounds += u32::from(*contestable);
                }
                nba_sim_core::Fact::SecondChanceStarted { .. } => second_chances += 1,
                _ => {}
            }
        }
    }
    durations.sort_by(f64::total_cmp);
    let p50 = durations.get(durations.len() / 2).copied().unwrap_or_default();
    let total_possessions = durations.len() as f64;
    let audit = json!({
        "seed_start": 1,
        "games": 9,
        "possessions": durations.len(),
        "possession_duration_p50_seconds": p50,
        "actions": actions,
        "action_generation_per_possession": actions
            .iter()
            .map(|(key, value)| ((*key).to_string(), f64::from(*value) / total_possessions))
            .collect::<BTreeMap<String, f64>>(),
        "drive": {
            "total": drive_total,
            "reached": drive_reached,
            "arrival_rate": f64::from(drive_reached) / f64::from(drive_total).max(f64::MIN_POSITIVE),
            "mean_separation_ft": drive_separation / f64::from(drive_total).max(f64::MIN_POSITIVE),
        },
        "cut": {
            "lob_passes": lob_passes,
            "catches": cut_catches,
        },
        "passes": {
            "released": passes_released,
            "completed": passes_completed,
        },
        "source_share": creations
            .iter()
            .map(|(key, value)| ((*key).to_string(), f64::from(*value) / f64::from(shots).max(f64::MIN_POSITIVE)))
            .collect::<BTreeMap<String, f64>>(),
        "zone_share": zones
            .iter()
            .map(|(key, value)| ((*key).to_string(), f64::from(*value) / f64::from(shots).max(f64::MIN_POSITIVE)))
            .collect::<BTreeMap<String, f64>>(),
        "shot_phases": shot_phases,
        "rebound_opportunities": rebound_opportunities,
        "contestable_rebounds": contestable_rebounds,
        "second_chance_possessions": second_chances,
        "end_reasons": reasons,
        "turnover_kinds": turnover_kinds,
        "steals": steals,
        "shots": shots,
        "fga": fga,
        "made": made,
        "oreb": oreb,
    });
    let dir = workspace_calibration_dir()
        .parent()
        .expect("data directory")
        .join("..")
        .join(".work");
    std::fs::create_dir_all(&dir).expect("work directory");
    let path = dir.join("s8-action-audit.json");
    std::fs::write(&path, serde_json::to_string_pretty(&audit).expect("audit json"))
        .expect("audit artifact");
    println!("wrote {}", path.display());
    println!("{}", serde_json::to_string_pretty(&audit).expect("audit json"));
}
