use nba_sim_core::action::{Fact, TurnoverKind};

mod common;

#[test]
fn probe_turnover_kinds() {
    let mut counts: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    let mut games = 0;
    let mut possessions = 0;
    for seed in 1..=3u64 {
        let report = common::short_world(seed).run();
        games += 1;
        possessions += report.possessions.len();
        for event in &report.events {
            if let Fact::Turnover { kind, .. } = &event.fact {
                let name = match kind {
                    TurnoverKind::BadPass => "bad_pass",
                    TurnoverKind::LostBall => "lost_ball",
                    TurnoverKind::OffensiveFoul => "offensive_foul",
                    TurnoverKind::Travel => "travel",
                    TurnoverKind::DoubleDribble => "double_dribble",
                    TurnoverKind::ShotClock => "shot_clock",
                    TurnoverKind::OutOfBounds => "out_of_bounds",
                    TurnoverKind::CloselyGuarded => "closely_guarded",
                    TurnoverKind::EightSecond => "eight_second",
                    TurnoverKind::Backcourt => "backcourt",
                };
                *counts.entry(name.to_string()).or_default() += 1;
            }
        }
    }
    eprintln!(
        "games={games} possessions={possessions} per_game={:.1} kinds={counts:?}",
        possessions as f64 / games as f64
    );
}
