use nba_domain::GameRules;
use nba_engine::MatchEngine;

#[test]
fn probe_seed42_2000() {
    let mut engine = MatchEngine::with_rules(42, GameRules::default());
    let mut prev_status = String::new();
    let mut pending_ticks = 0u64;
    let mut max_pending = 0u64;
    for i in 0..400u64 {
        let tick = engine.step();
        if engine.pending_release_shooter_for_test().is_some() {
            pending_ticks += 1;
            max_pending = max_pending.max(pending_ticks);
        } else {
            pending_ticks = 0;
        }
        let status = tick.frame.ball.status.clone();
        if status != prev_status {
            println!("tick {i}: ball -> {status} (pending={})", engine.pending_release_shooter_for_test().is_some());
            prev_status = status;
        }
    }
    println!("max consecutive pending ticks: {max_pending}");
}
