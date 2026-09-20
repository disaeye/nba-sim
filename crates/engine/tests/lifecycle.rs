//! 生命周期行为：宏观阶段迁移、节末与终场、跳球时长、运行作用域。
//!
//! 守卫对象：`GameFlowState` 的迁移合法性、时钟推进只发生在活球、节末等待
//! 在飞行中的投篮、终场粘滞、跳球时长的物理表现、作用域请求的解析与完成。
//!
//! 这些断言对应 `docs/architecture.md` §3 与 `docs/dev/gap.md` §4
//! （权威时间、生命周期边界与终态处理）。

#![allow(clippy::field_reassign_with_default)]

mod support;

use glam::Vec2;
use nba_domain::GameFlowState;
use nba_engine::MatchEngine;

#[test]
fn test_engine_clock_advances_only_in_live_flow() {
    let mut engine = MatchEngine::new(11);
    let initial = engine.game_clock();
    engine.set_game_flow_for_test(GameFlowState::Timeout);
    engine.step();
    assert_eq!(engine.game_clock(), initial);
}

#[test]
fn test_game_flow_transition_table_rejects_impossible_edges() {
    assert!(GameFlowState::LiveBall.can_transition_to(GameFlowState::DeadBall));
    assert!(GameFlowState::DeadBall.can_transition_to(GameFlowState::LiveBall));
    assert!(!GameFlowState::GameEnd.can_transition_to(GameFlowState::LiveBall));
    assert!(GameFlowState::Timeout.can_transition_to(GameFlowState::Timeout));
}

#[test]
fn test_period_expiration_reaches_break_then_next_period() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.league.period_duration_seconds = 0.1;
    rules.league.overtime_duration_seconds = 0.1;
    rules.period_break_seconds = 0.1;
    rules.tactical_initiation_seconds = 0.1;
    rules.league.regulation_periods = 2;
    let mut engine = MatchEngine::with_rules(202, rules);
    let period_before = engine.engine_snapshot().game.period;
    engine.set_game_clock_for_test(0.0);
    engine.set_ball_state_for_test(nba_physics::BallTrajectoryKind::Held {
        carrier_id: engine.new_possession_pg_for_test(),
    });
    engine.set_game_flow_for_test(GameFlowState::LiveBall);
    let end_tick = engine.step();
    assert_eq!(engine.game_flow(), GameFlowState::Halftime);
    assert!(end_tick
        .frame
        .events
        .iter()
        .any(|event| event == "PHASE_TRANSITION"));
    assert_eq!(engine.period(), period_before);
    engine.set_period_break_elapsed_for_test(engine.rules().period_break_seconds + 1.0);
    engine.step();
    assert_eq!(engine.period(), period_before + 1);
}

#[test]
fn test_game_end_is_sticky_after_regulation_winner() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    rules.tick_seconds = 0.1;
    rules.league.period_duration_seconds = 0.1;
    rules.period_break_seconds = 0.1;
    rules.league.regulation_periods = 1;
    let mut engine = MatchEngine::with_rules(505, rules);
    engine.set_scores_for_test(1, 0);

    for _ in 0..10 {
        engine.step();
        if engine.game_flow() == GameFlowState::GameEnd {
            break;
        }
    }
    assert_eq!(engine.game_flow(), GameFlowState::GameEnd);
    assert_eq!(engine.step().frame.game_flow, "GameEnd");
}

#[test]
fn test_inbound_baseline_is_recorded_inside_boundary_domain() {
    let engine = MatchEngine::new(303);
    assert!(engine.inbound_baseline().x >= -0.01 && engine.inbound_baseline().x <= 94.01);
    assert!(engine.inbound_baseline().y >= 0.0 && engine.inbound_baseline().y <= 50.0);
}

#[test]
fn test_lifecycle_stream_preserves_event_order_and_clock_invariants() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.league.period_duration_seconds = 0.3;
    rules.period_break_seconds = 0.1;
    rules.tactical_initiation_seconds = 0.1;
    rules.decision_interval_seconds = 0.1;
    rules.league.regulation_periods = 1;
    let mut engine = MatchEngine::with_rules(606, rules);
    engine.set_scores_for_test(1, 0);
    engine.set_game_clock_for_test(0.0);
    engine.set_ball_state_for_test(nba_physics::BallTrajectoryKind::Held {
        carrier_id: "H_01".to_string(),
    });
    let mut saw_game_end = false;
    let mut previous_game_clock = engine.game_clock();
    for _ in 0..20 {
        let tick = engine.step();
        assert!(
            tick.frame.t_game <= previous_game_clock + 0.001
                || tick.frame.game_flow == "QuarterEnd"
                || tick.frame.game_flow == "GameEnd"
        );
        previous_game_clock = tick.frame.t_game;
        if tick.frame.game_flow == "GameEnd" {
            saw_game_end = true;
            break;
        }
    }
    assert!(saw_game_end);
}

#[test]
fn test_period_end_waits_for_shot_in_flight() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.league.period_duration_seconds = 0.2;
    rules.league.regulation_periods = 1;
    let mut engine = MatchEngine::with_rules(904, rules);
    engine.set_game_clock_for_test(0.0);
    engine.set_game_flow_for_test(GameFlowState::LiveBall);
    engine.set_ball_state_for_test(nba_physics::BallTrajectoryKind::Shot {
        shooter_id: "H_01".to_string(),
        from_pos: Vec2::new(80.0, 25.0),
        hoop_pos: Vec2::new(88.75, 25.0),
        start_time: engine.current_time(),
        duration: 10.0,
        is_made: true,
        is_three: false,
        peak_z: 15.0,
        // 该场景测试的是节末时钟行为，不涉及投篮犯规
        // （`BallState::Shot` 自 v62 起携带犯规事实）。
        fouled: false,
        fouler_id: None,
    });
    let home_before = engine.home_score();
    engine.step();
    assert_eq!(engine.game_flow(), GameFlowState::LiveBall);
    engine.step();
    assert_ne!(engine.game_flow(), GameFlowState::GameEnd);
    assert!(engine.home_score() >= home_before);
}

/// 全场必须在真实生命周期内进入终场，不能卡在死球。
///
/// 根因（F1.3）：发球员的界外发球点被 physics 的场地 clamp 推回场内，
/// `inbounder_arrived` 永不成立，`InboundTransfer` 无法推进（gap.md §6.3）。
#[test]
fn test_full_scope_reaches_game_end_without_deadball_livelock() {
    // 三个独立活锁根因分别由不同 seed 触发：555/999（发球点被 clamp）、
    // 6/9/11（分离投影推回发球员）、3/15/26（发球员被指派到替补）。
    for seed in [999u64, 555, 6, 9, 11, 3, 15, 26] {
        let mut engine = MatchEngine::new(seed);
        engine.set_scope("full").unwrap();
        let mut ticks = 0usize;
        while !engine.is_finished() && ticks < 200_000 {
            engine.step();
            ticks += 1;
        }
        assert!(
            engine.is_finished(),
            "seed {} livelocked before GameEnd (ticks={}, flow={:?}, clock={:.1})",
            seed,
            ticks,
            engine.game_flow(),
            engine.game_clock()
        );
    }
}

#[test]
fn test_configured_tip_off_duration_produces_physical_tipoff_phase() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 1.0;
    let setup = nba_engine::MatchSetup::builtin(rules);
    let mut engine = MatchEngine::with_setup(setup, 42);
    assert_eq!(engine.game_flow(), GameFlowState::TipOff);

    let mut saw_tipoff = false;
    let mut saw_tipoff_secured = false;
    for _ in 0..50 {
        let tick = engine.step();
        if tick.frame.events.iter().any(|e| e == "TIPOFF") {
            saw_tipoff = true;
            assert!(tick.frame.ball.z > 4.0);
        }
        if tick.frame.events.iter().any(|e| e == "TIPOFF_SECURED") {
            saw_tipoff_secured = true;
            break;
        }
    }
    assert!(
        saw_tipoff,
        "should emit TIPOFF events during configured tip off duration"
    );
    assert!(
        saw_tipoff_secured,
        "should emit TIPOFF_SECURED after duration completes"
    );
    assert_eq!(engine.game_flow(), GameFlowState::LiveBall);
}

#[test]
fn test_scoped_completion_emits_settled_terminal_frame_and_stays_sticky() {
    let mut engine = MatchEngine::new(42);
    engine.set_scope("1p").expect("valid possession scope");

    let terminal = (0..900)
        .map(|_| engine.step())
        .find(|tick| tick.frame.simulation_complete)
        .expect("one possession should complete");
    assert_eq!(terminal.frame.completed_possessions, 1);
    assert_eq!(terminal.frame.target_possessions, 1);
    assert_eq!(terminal.frame.ball.status, "DEAD");
    assert_eq!(terminal.frame.game_flow, "DeadBall");
    assert_eq!(terminal.frame.phase, "DeadBallReset");
    assert!(terminal.frame.ball.holder_id.is_none());

    let terminal_time = terminal.frame.t;
    let terminal_score = terminal.frame.score.clone();
    let next = engine.step();
    assert_eq!(next.frame.t, terminal_time);
    assert_eq!(next.frame.score.home, terminal_score.home);
    assert_eq!(next.frame.score.away, terminal_score.away);
    assert_eq!(next.frame.completed_possessions, 1);
    assert!(next.frame.event_log.is_empty());
    assert!(next.frame.event_type.is_none());
    assert_eq!(next.frame.ball.status, "DEAD");
}

#[test]
fn test_scope_parser_rejects_malformed_requests() {
    assert!(MatchEngine::parse_scope("bad").is_err());
    assert!(MatchEngine::parse_scope("-1p").is_err());
    assert!(MatchEngine::parse_scope("0p").is_err());
}

#[test]
fn test_scope_parser_uses_configured_period_budget() {
    let mut rules = nba_domain::GameRules::default();
    rules.league.regulation_periods = 2;
    rules.estimated_possessions_per_period = 11;
    assert_eq!(
        MatchEngine::parse_scope_with_rules("1q", &rules)
            .expect("quarter scope")
            .0,
        11
    );
    assert_eq!(
        MatchEngine::parse_scope_with_rules("full", &rules)
            .expect("full-game scope")
            .0,
        22
    );
}
