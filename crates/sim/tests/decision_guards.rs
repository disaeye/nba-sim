//! 批5决策层守卫：传球拦截、快攻出手、三分频率与贴防干扰。
//!
//! 统计按 event_id 去重，从真实比赛的报告读取，不靠注入。
//! 口径：快攻用进攻钟（出手时已用 ≤6s），远距用出手点到篮筐距离，
//! 干扰用事实流里的被盖/贴防 contest。

mod common;

use nba_domain::rules::GameRules;
use nba_sim_core::action::Fact;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{MatchSetup, Params, World};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(
        MatchSetup::builtin(GameRules::default()),
        params,
        seed,
    )
}

#[derive(Default)]
struct DecisionStats {
    games: u64,
    interceptions: u64,
    shots: u64,
    quick_shots: u64,
    long_shots: u64,
    contests: Vec<f32>,
}

fn measure(seed: u64) -> DecisionStats {
    let mut world = world(seed);
    let mut stats = DecisionStats::default();
    let report = world.run();
    stats.games += 1;
    let mut ids = std::collections::BTreeSet::new();
    for event in &report.events {
        if !ids.insert(event.event_id) {
            continue;
        }
        match &event.fact {
            Fact::BallContact { kind: "interception", .. } => stats.interceptions += 1,
            Fact::Shot { contest, distance_ft, .. } => {
                stats.shots += 1;
                stats.contests.push(*contest);
                if *distance_ft >= 22.0 {
                    stats.long_shots += 1;
                }
            }
            _ => {}
        }
    }
    // 快攻出手：出手时进攻钟显示已用不超过 6 秒（24 → 18+）。
    stats.quick_shots = report
        .shot_clock_at_release
        .iter()
        .filter(|remaining| 24.0 - **remaining <= 6.0)
        .count() as u64;
    stats
}

#[test]
fn decision_guards_hold_in_complete_matches() {
    let mut total = DecisionStats::default();
    for seed in 1..=12u64 {
        let s = measure(seed);
        total.games += s.games;
        total.interceptions += s.interceptions;
        total.shots += s.shots;
        total.quick_shots += s.quick_shots;
        total.long_shots += s.long_shots;
        total.contests.extend(s.contests);
    }
    let interceptions_per_game = total.interceptions as f64 / total.games as f64;
    let quick = total.quick_shots as f64 / total.shots.max(1) as f64;
    let long = total.long_shots as f64 / total.shots.max(1) as f64;
    total.contests.sort_by(|a, b| a.total_cmp(b));
    let p50 = total.contests
        .get(total.contests.len() / 2)
        .copied()
        .unwrap_or(0.0);
    eprintln!(
        "interceptions/game={:.2} quick={:.3} long={:.3} contest_p50={:.3} shots={}",
        interceptions_per_game, quick, long, p50, total.shots
    );
    assert!(
        interceptions_per_game > 0.0,
        "飞行传球拦截必须由完整比赛自发产生：{interceptions_per_game:.2}/场"
    );
    assert!(quick >= 0.12, "6 秒内出手占比={quick:.3}，低于 12%");
    assert!(long <= 0.40, "22ft 以外出手占比={long:.3}，高于 40%");
    assert!(p50 > 0.05, "出手 contest 中位数={p50:.3}，贴防几何没有提供干扰");
}
