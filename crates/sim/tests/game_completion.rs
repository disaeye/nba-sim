mod common;

use nba_sim_core::action::Fact;

#[test]
fn representative_nba_games_finish_and_emit_final_events() {
    for seed in [1, 23] {
        let mut world = common::short_world(seed);
        let report = world.run();
        assert!(world.finished, "seed {seed} did not reach a final state");
        assert!(report.ticks < 30_000, "seed {seed} exceeded the tick limit");
        assert_ne!(report.home_score, report.away_score, "seed {seed} ended tied");
        assert!(!report.possessions.is_empty(), "seed {seed} recorded no possessions");
        assert!(
            report.events.iter().any(|event| matches!(event.fact, Fact::GameEnd)),
            "seed {seed} omitted GameEnd"
        );
    }
}
