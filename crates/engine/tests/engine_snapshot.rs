use nba_domain::Possession;
use nba_engine::MatchEngine;

#[test]
fn test_engine_snapshot_view_projection() {
    let mut engine = MatchEngine::new(42);

    // 1. 验证只读投影能够正确读取各个状态族
    let initial_time = {
        let snap = engine.engine_snapshot();
        assert_eq!(snap.game.period, 1);
        assert_eq!(snap.game.home_score, 0);
        assert_eq!(snap.game.away_score, 0);
        assert_eq!(snap.lineups.home_roster_order.len(), 5);
        assert_eq!(snap.lineups.away_roster_order.len(), 5);
        assert_eq!(snap.rules.court.width_ft, 94.0);
        assert!(matches!(
            snap.game.possession,
            Possession::Home | Possession::Away
        ));
        assert!(!snap.ball.active_carrier_or_focus_id.is_empty());

        // 验证物理世界球员查询
        let first_home_player = &snap.lineups.home_roster_order[0];
        let p_state = snap.get_player(first_home_player);
        assert!(p_state.is_some());
        snap.game.current_time
    };

    // 2. 模拟步进
    for _ in 0..10 {
        engine.step();
    }

    // 3. 再次获取新 snapshot
    let snap2 = engine.engine_snapshot();
    assert!(snap2.game.current_time > initial_time);
}
