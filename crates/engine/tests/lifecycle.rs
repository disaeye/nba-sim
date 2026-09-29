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
use rayon::prelude::*;

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
fn test_period_break_freezes_game_time_and_dead_ball_setup_advances_it() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.period_break_seconds = 0.3;
    rules.league.regulation_periods = 2;
    let mut engine = MatchEngine::with_rules(203, rules);
    engine.set_game_flow_for_test(GameFlowState::Halftime);
    engine.set_period_break_elapsed_for_test(0.1);
    let break_time = engine.current_time();
    let break_tick = engine.step();
    assert_eq!(engine.current_time(), break_time);
    assert_eq!(break_tick.frame.t, break_time);
    assert_eq!(engine.game_flow(), GameFlowState::Halftime);

    engine.set_period_break_elapsed_for_test(engine.rules().period_break_seconds);
    let start_tick = engine.step();
    assert_eq!(engine.game_flow(), GameFlowState::DeadBall);
    assert!(start_tick.frame.t >= break_time);
    let dead_ball_time = engine.current_time();
    engine.step();
    assert!(engine.current_time() > dead_ball_time);
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
        aim_pos: Vec2::new(88.75, 25.0),
        start_time: engine.current_time(),
        duration: 10.0,
        is_three: false,
        peak_z: 15.0,
        make_probability: 0.5,
        contest_intensity: 0.0,
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
    // 种子间并行：八场完整比赛互不共享状态，并行只改变 wall time。
    let failures: Vec<String> = [999u64, 555, 6, 9, 11, 3, 15, 26]
        .par_iter()
        .map(|&seed| {
            let mut engine = MatchEngine::new(seed);
            engine.set_scope("full").unwrap();
            let mut ticks = 0usize;
            while !engine.is_finished() && ticks < 200_000 {
                engine.step();
                ticks += 1;
            }
            if engine.is_finished() {
                None
            } else {
                Some(format!(
                    "seed {} livelocked before GameEnd (ticks={}, flow={:?}, clock={:.1})",
                    seed,
                    ticks,
                    engine.game_flow(),
                    engine.game_clock()
                ))
            }
        })
        .flatten()
        .collect();
    assert!(
        failures.is_empty(),
        "full-scope livelock regressions: {failures:?}"
    );
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

#[test]
fn test_action_lifecycle_all_types_complete_and_unlock_kinematics() {
    use nba_domain::action_window::{ActionTimeWindow, ActionType};

    let types = [
        ActionType::ScreenSet,
        ActionType::ScreenRoll,
        ActionType::ScreenPop,
        ActionType::CutBackdoor,
        ActionType::Cut,
        ActionType::BoxOut,
        ActionType::Putback,
        ActionType::JumpShot,
        ActionType::Layup,
        ActionType::Dunk,
        ActionType::PassRelease,
        ActionType::CloseoutContest,
        ActionType::ReboundJump,
    ];

    for action_type in types {
        let mut engine = MatchEngine::new(101);
        let rules = engine.rules().clone();
        let player_id = "H_01";
        let start_t = engine.current_time();

        let window = match action_type {
            ActionType::ScreenSet => ActionTimeWindow::new_screen_set(player_id, start_t, &rules),
            ActionType::ScreenRoll => ActionTimeWindow::new_screen_roll(player_id, start_t, &rules),
            ActionType::ScreenPop => ActionTimeWindow::new_screen_pop(player_id, start_t, &rules),
            ActionType::CutBackdoor => {
                ActionTimeWindow::new_cut_backdoor(player_id, start_t, &rules)
            }
            ActionType::Cut => ActionTimeWindow::new_cut(player_id, start_t, &rules),
            ActionType::BoxOut => ActionTimeWindow::new_box_out(player_id, start_t, &rules),
            ActionType::Putback => ActionTimeWindow::new_putback(player_id, start_t, &rules),
            ActionType::JumpShot => ActionTimeWindow::new_jump_shot(player_id, start_t, &rules),
            ActionType::Layup => ActionTimeWindow::new_layup(player_id, start_t, &rules),
            ActionType::Dunk => ActionTimeWindow::new_dunk(player_id, start_t, &rules),
            ActionType::PassRelease => ActionTimeWindow::new_pass(player_id, start_t, &rules),
            ActionType::CloseoutContest => {
                ActionTimeWindow::new_closeout_contest(player_id, start_t, &rules)
            }
            ActionType::ReboundJump => {
                ActionTimeWindow::new_rebound_jump(player_id, start_t, &rules)
            }
        };

        let expects_locked = window.lock_kinematics;
        engine.start_action_window_for_test(player_id, window, None, None);
        assert_eq!(engine.active_windows_count_for_test(), 1);
        if expects_locked {
            assert!(
                engine.is_player_locked_for_test(player_id),
                "action {action_type:?} should lock kinematics on start"
            );
        }

        let started_events = engine.publish_events_for_test();
        assert!(
            started_events.iter().any(|e| e.kind == "ACTION_STARTED"),
            "action {action_type:?} should emit ActionStarted"
        );

        // Advance past prep into execution
        let exec_time = start_t + rules.screen_prep_seconds + rules.tick_seconds;
        engine.advance_action_windows_for_test(exec_time);
        let phase_events = engine.publish_events_for_test();
        assert!(
            phase_events
                .iter()
                .any(|e| e.kind == "ACTION_PHASE_CHANGED"),
            "action {action_type:?} should emit ActionPhaseChanged"
        );

        // Advance to full completion
        let finish_time = start_t + 10.0;
        engine.advance_action_windows_for_test(finish_time);
        let completed_events = engine.publish_events_for_test();
        assert!(
            completed_events
                .iter()
                .any(|e| e.kind == "ACTION_COMPLETED"),
            "action {action_type:?} should emit ActionCompleted"
        );

        assert_eq!(
            engine.active_windows_count_for_test(),
            0,
            "action {action_type:?} should be removed after completion"
        );
        assert!(
            !engine.is_player_locked_for_test(player_id),
            "action {action_type:?} must unlock kinematics after completion"
        );
    }
}

#[test]
fn test_action_lifecycle_single_uncompleted_action_per_player_tick() {
    use nba_domain::action_window::ActionTimeWindow;

    let mut engine = MatchEngine::new(102);
    let rules = engine.rules().clone();
    let player_id = "H_01";
    let start_t = engine.current_time();

    let window1 = ActionTimeWindow::new_screen_set(player_id, start_t, &rules);
    engine.start_action_window_for_test(player_id, window1, None, None);
    assert_eq!(engine.active_windows_count_for_test(), 1);
    assert!(engine.is_player_locked_for_test(player_id));

    // Start a second action window for the same player in the same tick
    let window2 = ActionTimeWindow::new_cut_backdoor(player_id, start_t, &rules);
    engine.start_action_window_for_test(player_id, window2, None, None);

    // Exactly one active window remains
    assert_eq!(
        engine.active_windows_count_for_test(),
        1,
        "player must have at most one active action window per tick"
    );
    // CutBackdoor does not lock kinematics, so superseded old window must have unlocked
    assert!(
        !engine.is_player_locked_for_test(player_id),
        "superseded locked window must unlock kinematics"
    );

    let events = engine.publish_events_for_test();
    assert!(
        events.iter().any(|e| e.kind == "ACTION_CANCELLED"),
        "superseded action should emit ActionCancelled"
    );
}

#[test]
fn test_action_lifecycle_cancellation_unlocks_kinematics() {
    use nba_domain::action_window::ActionTimeWindow;
    use nba_domain::event::ActionCancellationReason;

    let mut engine = MatchEngine::new(103);
    let rules = engine.rules().clone();
    let player_id = "H_02";
    let start_t = engine.current_time();

    let window = ActionTimeWindow::new_screen_set(player_id, start_t, &rules);
    engine.start_action_window_for_test(player_id, window, None, None);
    assert!(engine.is_player_locked_for_test(player_id));

    let cancelled =
        engine.cancel_action_window_for_test(player_id, ActionCancellationReason::PreemptedByFoul);
    assert!(cancelled);
    assert_eq!(engine.active_windows_count_for_test(), 0);
    assert!(
        !engine.is_player_locked_for_test(player_id),
        "cancellation must unlock kinematics"
    );

    let events = engine.publish_events_for_test();
    assert!(
        events.iter().any(|e| e.kind == "ACTION_CANCELLED"),
        "cancellation should emit ActionCancelled"
    );
}

#[test]
fn test_action_lifecycle_failure_unlocks_kinematics() {
    use nba_domain::action_window::ActionTimeWindow;
    use nba_domain::event::ActionFailureReason;

    let mut engine = MatchEngine::new(104);
    let rules = engine.rules().clone();
    let player_id = "H_03";
    let start_t = engine.current_time();

    let window = ActionTimeWindow::new_jump_shot(player_id, start_t, &rules);
    engine.start_action_window_for_test(player_id, window, None, None);
    assert!(engine.is_player_locked_for_test(player_id));

    let failed = engine.fail_action_window_for_test(player_id, ActionFailureReason::Blocked);
    assert!(failed);
    assert_eq!(engine.active_windows_count_for_test(), 0);
    assert!(
        !engine.is_player_locked_for_test(player_id),
        "failure must unlock kinematics"
    );

    let events = engine.publish_events_for_test();
    assert!(
        events.iter().any(|e| e.kind == "ACTION_FAILED"),
        "failure should emit ActionFailed"
    );
}
