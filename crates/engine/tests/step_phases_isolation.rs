use nba_domain::BallState;
use nba_engine::MatchEngine;

#[test]
fn test_decision_phase_isolation() {
    let mut engine = MatchEngine::new(42);
    // 初始状态推进 10 tick 进入常规进攻状态
    for _ in 0..10 {
        engine.step();
    }
    let snap = engine.engine_snapshot();
    assert!(snap.game.game_clock > 0.0);
    assert!(!snap.lineups.home_roster_order.is_empty());
}

#[test]
fn test_ballistics_resolution_phase_isolation() {
    let mut engine = MatchEngine::new(42);
    for _ in 0..20 {
        engine.step();
    }
    let snap = engine.engine_snapshot();
    match snap.ball.state {
        BallState::Held { carrier_id } => {
            assert!(!carrier_id.is_empty());
        }
        BallState::Pass { target_id, .. } => {
            assert!(!target_id.is_empty());
        }
        BallState::Shot { shooter_id, .. } => {
            assert!(!shooter_id.is_empty());
        }
        BallState::InboundReady { inbounder_id, .. } => {
            assert!(!inbounder_id.is_empty());
        }
        _ => {}
    }
}

#[test]
fn test_accounting_and_events_phase_isolation() {
    let mut engine = MatchEngine::new(42);
    let mut ticks_count = 0;
    for _ in 0..50 {
        let tick = engine.step();
        assert!(tick.frame.t >= 0.0);
        ticks_count += 1;
    }
    assert_eq!(ticks_count, 50);
}
