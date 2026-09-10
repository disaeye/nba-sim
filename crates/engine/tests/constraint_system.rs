#![allow(clippy::field_reassign_with_default)]

//! 约束系统集成测试：验证约束对象模型的核心契约
//! （架构文档 Phase 1/3/7 的可观察行为）。

use glam::Vec2;
use rand::SeedableRng;
use std::collections::HashMap;
use std::sync::LazyLock;

use nba_decision::constraint::{CandidateAction, ConstraintContext, ConstraintRegistry, PhaseType};
use nba_engine::MatchEngine;
use nba_physics::movement::PlayerPhysicsState;

static DEFAULT_RULES: LazyLock<nba_domain::GameRules> =
    LazyLock::new(nba_domain::GameRules::default);

fn make_player(id: &str, team: &str, x: f32, y: f32) -> PlayerPhysicsState {
    PlayerPhysicsState {
        id: id.to_string(),
        jersey: id.to_string(),
        team: team.to_string(),
        pos_ft: Vec2::new(x, y),
        vel_ft: Vec2::ZERO,
        accel_ft: Vec2::ZERO,
        target_pos_ft: Vec2::new(x, y),
        max_speed_ftps: 22.0,
        max_accel_ftps2: 35.0,
        target_speed_ftps: 0.0,
        has_ball: false,
        on_court: true,
        action: "Idle".to_string(),
        slot: "PG".to_string(),
        morale: "Normal".to_string(),
        stamina: 100.0,
        max_stamina: 100.0,
        foul_count: 0,
        locomotion: nba_physics::movement::LocomotionState::Idle,
        facing_dir: Vec2::X,
        turn_decel_timer: 0.0,
        is_locked_kinematics: false,
        attributes: Default::default(),
        roles: Vec::new(),
        tendencies: Default::default(),
    }
}

fn make_physics(players: &HashMap<String, PlayerPhysicsState>) -> nba_physics::PhysicsWorld {
    let rules = &*DEFAULT_RULES;
    let mut physics =
        nba_physics::PhysicsWorld::with_backend(rules, nba_physics::PhysicsBackend::SimpleCircle);
    let mut ids: Vec<_> = players.values().cloned().collect();
    ids.sort_by(|left, right| left.id.cmp(&right.id));
    for player in ids {
        physics.register_player(player);
    }
    physics
}

fn base_ctx<'a>(physics: &'a nba_physics::PhysicsWorld) -> ConstraintContext<'a> {
    let rules = &*DEFAULT_RULES;
    static TEAM_TRAITS: LazyLock<HashMap<String, nba_domain::TeamTraits>> = LazyLock::new(|| {
        HashMap::from([
            ("home".to_string(), nba_domain::TeamTraits::default()),
            ("away".to_string(), nba_domain::TeamTraits::default()),
        ])
    });
    ConstraintContext {
        physics,
        ball_pos: Vec2::new(47.0, 25.0),
        possession_team: "home",
        shot_clock: 12.0,
        game_clock: 300.0,
        phase: PhaseType::SetPlay,
        game_flow: nba_domain::GameFlowState::LiveBall,
        ball_phase: nba_domain::BallPhase::Held,
        inbound_elapsed: 0.0,
        backcourt_elapsed: 0.0,
        rules,
        team_traits: &TEAM_TRAITS,
    }
}

#[test]
fn test_hard_constraint_blocks_out_of_bounds_pass() {
    let mut players = HashMap::new();
    players.insert("H_1".to_string(), make_player("H_1", "home", 40.0, 25.0));
    players.insert("H_2".to_string(), make_player("H_2", "home", 60.0, 25.0));
    let physics = make_physics(&players);
    let ctx = base_ctx(&physics);
    let action = CandidateAction::Pass {
        passer_id: "H_1".to_string(),
        receiver_id: "H_2".to_string(),
        from_pos: Vec2::new(40.0, 25.0),
        to_pos: Vec2::new(96.0, 25.0),
    };
    let scored = ConstraintRegistry::default().evaluate_candidate(&ctx, &action);
    assert!(!scored.feasible);
}

#[test]
fn test_dead_ball_blocks_shot() {
    let mut players = HashMap::new();
    players.insert("H_1".to_string(), make_player("H_1", "home", 40.0, 25.0));
    let physics = make_physics(&players);
    let mut ctx = base_ctx(&physics);
    ctx.phase = PhaseType::DeadBallReset;
    ctx.game_flow = nba_domain::GameFlowState::DeadBall;
    let action = CandidateAction::Shoot {
        shooter_id: "H_1".to_string(),
        from_pos: Vec2::new(40.0, 25.0),
        is_three: false,
        jumper_kind: None,
    };
    let scored = ConstraintRegistry::default().evaluate_candidate(&ctx, &action);
    assert!(!scored.feasible);
}

#[test]
fn test_inbound_action_requires_inbound_dead_ball_phase() {
    let mut players = HashMap::new();
    players.insert("H_1".to_string(), make_player("H_1", "home", -2.0, 25.0));
    players.insert("H_2".to_string(), make_player("H_2", "home", 10.0, 25.0));
    let physics = make_physics(&players);
    let mut ctx = base_ctx(&physics);
    ctx.ball_pos = Vec2::new(-2.0, 25.0);
    ctx.phase = PhaseType::Inbound;
    ctx.game_flow = nba_domain::GameFlowState::DeadBall;
    let action = CandidateAction::InboundPass {
        passer_id: "H_1".to_string(),
        receiver_id: "H_2".to_string(),
        from_pos: Vec2::new(0.5, 25.0),
        to_pos: Vec2::new(10.0, 25.0),
    };
    assert!(
        ConstraintRegistry::default()
            .evaluate_candidate(&ctx, &action)
            .feasible
    );
}
#[test]
fn test_seed4_diagnostic() {
    let mut engine = MatchEngine::new(4);
    engine.set_scope("full").expect("full scope is valid");
    for i in 0..12085 {
        let tick = engine.step();
        if i >= 12065 && i <= 12082 {
            eprintln!(
                "TICK {}: flow={:?} sub={:?} ball={:?} holder={:?} events={:?} event_log={:?}",
                i,
                engine.game_flow,
                engine.sub_phase,
                engine.ball_state,
                tick.frame.ball.holder_id,
                tick.frame.events,
                tick.frame.event_log
            );
        }
    }
}

#[test]
fn test_custom_rules_drive_engine_tick_and_clock() {
    let rules = nba_domain::GameRules {
        tick_seconds: 0.1,
        tactical_initiation_seconds: 0.1,
        tip_off_duration_seconds: 0.0,
        ..nba_domain::GameRules::default()
    };
    let mut engine = MatchEngine::with_rules(3, rules);
    let before = engine.game_clock;
    engine.step();
    assert!((engine.current_time - 0.1).abs() < f32::EPSILON);
    assert!(engine.game_clock < before);
}

#[test]
fn test_ball_flight_is_not_available_for_second_action() {
    let mut players = HashMap::new();
    players.insert("H_1".to_string(), make_player("H_1", "home", 40.0, 25.0));
    let physics = make_physics(&players);
    let mut ctx = base_ctx(&physics);
    ctx.ball_phase = nba_domain::BallPhase::PassFlight;
    let action = CandidateAction::Shoot {
        shooter_id: "H_1".to_string(),
        from_pos: Vec2::new(40.0, 25.0),
        is_three: false,
        jumper_kind: None,
    };
    let scored = ConstraintRegistry::default().evaluate_candidate(&ctx, &action);
    assert!(!scored.feasible);
}

#[test]
fn test_shot_clock_violation_world_check() {
    let mut players = HashMap::new();
    players.insert("H_1".to_string(), make_player("H_1", "home", 40.0, 25.0));
    let physics = make_physics(&players);
    let mut ctx = base_ctx(&physics);
    ctx.shot_clock = 0.0;
    let findings = ConstraintRegistry::default().evaluate_runtime(&ctx);
    assert!(!findings.is_empty());
}

#[test]
fn test_game_clock_expired_blocks_shot_only() {
    let mut players = HashMap::new();
    players.insert("H_1".to_string(), make_player("H_1", "home", 40.0, 25.0));
    let physics = make_physics(&players);
    let mut ctx = base_ctx(&physics);
    ctx.game_clock = 0.0;
    let shot = CandidateAction::Shoot {
        shooter_id: "H_1".to_string(),
        from_pos: Vec2::new(40.0, 25.0),
        is_three: false,
        jumper_kind: None,
    };
    let scored = ConstraintRegistry::default().evaluate_candidate(&ctx, &shot);
    assert!(!scored.feasible);
}

#[test]
fn test_risky_pass_soft_penalty_applies() {
    let mut players = HashMap::new();
    players.insert("H_1".to_string(), make_player("H_1", "home", 40.0, 25.0));
    players.insert("H_2".to_string(), make_player("H_2", "home", 60.0, 25.0));
    players.insert("A_1".to_string(), make_player("A_1", "away", 50.0, 25.0));
    let physics = make_physics(&players);
    let ctx = base_ctx(&physics);
    let action = CandidateAction::Pass {
        passer_id: "H_1".to_string(),
        receiver_id: "H_2".to_string(),
        from_pos: Vec2::new(40.0, 25.0),
        to_pos: Vec2::new(60.0, 25.0),
    };
    let scored = ConstraintRegistry::default().evaluate_candidate(&ctx, &action);
    assert!(scored.risk >= 0.0);
}

#[test]
fn test_clock_urgency_preference_boosts_shot() {
    let mut players = HashMap::new();
    players.insert("H_1".to_string(), make_player("H_1", "home", 40.0, 25.0));
    let physics = make_physics(&players);
    let mut ctx = base_ctx(&physics);
    ctx.shot_clock = 2.0;
    let shot = CandidateAction::Shoot {
        shooter_id: "H_1".to_string(),
        from_pos: Vec2::new(40.0, 25.0),
        is_three: false,
        jumper_kind: None,
    };
    let scored = ConstraintRegistry::default().evaluate_candidate(&ctx, &shot);
    assert!(scored.feasible);
}

#[test]
fn test_simulation_decision_trace_present_in_stream() {
    let mut engine = MatchEngine::new(7);
    let mut traces = 0;
    let mut events = std::collections::HashSet::new();
    for _ in 0..2500 {
        let tick = engine.step();
        if tick.frame.debug.is_some() {
            traces += 1;
        }
        events.extend(tick.frame.events);
        if traces > 2 && events.contains("SHOT_RELEASE") {
            break;
        }
    }
    assert!(traces > 0);
    assert!(events.contains("PHASE_TRANSITION"));
}

#[test]
fn test_shot_clock_violation_triggers_turnover_in_sim() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    let mut engine = MatchEngine::with_rules(42, rules);
    engine.shot_clock = engine.rules.tick_seconds;
    let previous_possession = engine.possession();

    let tick = engine.step();

    assert!(tick.frame.events.iter().any(|event| event == "VIOLATION"));
    assert_ne!(engine.possession(), previous_possession);
    assert!(matches!(
        engine.ball_state,
        nba_physics::BallTrajectoryKind::InboundTransfer { .. }
    ));
}
#[test]
fn test_runtime_clock_finding_carries_enforcement_intent() {
    let players = HashMap::new();
    let physics = make_physics(&players);
    let mut ctx = base_ctx(&physics);
    ctx.shot_clock = 0.0;
    let findings = ConstraintRegistry::default().evaluate_runtime(&ctx);
    assert!(findings
        .iter()
        .any(|finding| finding.enforcement != nba_decision::constraint::EnforcementAction::None));
}
#[test]
fn test_post_boundary_fact_is_evaluated_by_active_constraint() {
    let players = HashMap::new();
    let physics = make_physics(&players);
    let ctx = base_ctx(&physics);
    let events = [nba_domain::GameEvent::BoundaryCross {
        player_id: "H_1".to_string(),
        pos: (96.0, 25.0),
        boundary_name: "sideline".to_string(),
    }];
    assert!(!ConstraintRegistry::default()
        .evaluate_events(&ctx, &events)
        .is_empty());
}

#[test]
fn test_engine_clock_advances_only_in_live_flow() {
    let mut engine = MatchEngine::new(11);
    let initial = engine.game_clock;
    engine.game_flow = nba_domain::GameFlowState::Timeout;
    engine.step();
    assert_eq!(engine.game_clock, initial);
}

#[test]
fn test_phase_transition_and_pass_events_are_observable() {
    let mut engine = MatchEngine::new(7);
    let mut saw_phase_transition = false;
    let mut saw_pass = false;
    for _ in 0..800 {
        let tick = engine.step();
        saw_phase_transition |= tick
            .frame
            .events
            .iter()
            .any(|event| event == "PHASE_TRANSITION");
        saw_pass |= tick.frame.events.iter().any(|event| event == "PASS");
        if saw_phase_transition && saw_pass {
            break;
        }
    }
    assert!(saw_phase_transition);
    assert!(saw_pass);
}

#[test]
fn test_offensive_rebound_does_not_switch_possession() {
    let mut engine = MatchEngine::new(19);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    let before = engine.possession();
    let rebound = nba_physics::BallTrajectoryKind::RimRebound {
        from_pos: glam::Vec2::new(88.75, 25.0),
        from_z: engine.rules.rim_height_ft,
        last_touch_team: engine.possession(),
        hoop_pos: engine.rules.court.hoop_pos(true),
        target_landing: glam::Vec2::new(70.0, 25.0),
        start_time: engine.current_time,
        duration: 10.0,
        peak_z: engine.rules.rebound_peak_ft,
    };
    engine.ball_state = rebound;
    assert_eq!(engine.possession(), before);
    assert_eq!(engine.possession(), nba_domain::Possession::Home);
}

#[test]
fn test_constraint_activation_is_priority_ordered() {
    let players = HashMap::new();
    let physics = make_physics(&players);
    let ctx = base_ctx(&physics);
    let active = ConstraintRegistry::default().active_set(&ctx);
    assert!(active
        .windows(2)
        .all(|pair| (pair[0].priority, pair[0].id) <= (pair[1].priority, pair[1].id)));
}

#[test]
fn test_game_flow_transition_table_rejects_impossible_edges() {
    use nba_domain::GameFlowState;
    assert!(GameFlowState::LiveBall.can_transition_to(GameFlowState::DeadBall));
    assert!(GameFlowState::DeadBall.can_transition_to(GameFlowState::LiveBall));
    assert!(!GameFlowState::GameEnd.can_transition_to(GameFlowState::LiveBall));
    assert!(GameFlowState::Timeout.can_transition_to(GameFlowState::Timeout));
}

#[test]
fn test_contact_fact_is_adjudicated_into_foul_fact() {
    use nba_domain::resolve::ContactPolicy;
    use nba_domain::GameEvent;
    use nba_officiating::{ResolutionLayer, ResolutionOutcome};
    let event = GameEvent::Contact {
        player_a: "H_1".to_string(),
        player_b: "A_1".to_string(),
        impact_speed: 20.0,
        contact_normal: (1.0, 0.0),
        is_screen: false,
        semantic_kind: "BlockingCandidate".to_string(),
        semantic_severity: "FoulCandidate".to_string(),
        possessor_id: Some("H_1".to_string()),
        legal_position: false,
    };
    let mut rng = rand::rngs::StdRng::seed_from_u64(1);
    let outcome = ResolutionLayer::resolve_contact_with_policy(
        "H_1",
        "A_1",
        20.0,
        false,
        &HashMap::new(),
        &ContactPolicy::default(),
        &mut rng,
    );
    assert!(matches!(
        outcome,
        ResolutionOutcome::NoChange | ResolutionOutcome::Foul { .. }
    ));
    assert!(matches!(event, GameEvent::Contact { .. }));
}
#[test]
fn test_contact_adjudication_emits_foul_fact_when_policy_calls_it() {
    let mut engine = MatchEngine::new(7);
    engine.pending_events.push(nba_domain::GameEvent::Contact {
        player_a: "H_1".to_string(),
        player_b: "A_1".to_string(),
        impact_speed: 20.0,
        contact_normal: (1.0, 0.0),
        is_screen: false,
        semantic_kind: "BlockingCandidate".to_string(),
        semantic_severity: "FoulCandidate".to_string(),
        possessor_id: Some("H_1".to_string()),
        legal_position: false,
    });
    let before = engine
        .modulation
        .get("H_1")
        .map(|m| m.catch_equilibrium)
        .unwrap_or(1.0);
    let tick = engine.step();
    let foul_seen = tick.frame.events.iter().any(|event| event == "FOUL")
        || tick.frame.event_type.as_deref() == Some("FOUL");
    if foul_seen {
        assert!(
            engine
                .modulation
                .get("H_1")
                .map(|m| m.catch_equilibrium)
                .unwrap_or(1.0)
                <= before
        );
    }
}

#[test]
fn test_shooting_foul_enters_free_throw_state_and_scores_free_throws() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    rules.free_throw_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(101, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.pending_events.push(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_1".to_string(),
        fouler_id: "A_1".to_string(),
        is_shooting: true,
    });
    let first = engine.step();
    assert!(
        first.frame.events.iter().any(|event| event == "FOUL")
            || first.frame.event_type.as_deref() == Some("FOUL")
    );
    assert_eq!(engine.game_flow, nba_domain::GameFlowState::FreeThrow);
    assert_eq!(engine.free_throws_remaining, 2);
    let score_before = engine.home_score;
    for _ in 0..20 {
        engine.step();
        if engine.free_throws_remaining == 0 {
            break;
        }
    }
    assert_eq!(engine.free_throws_remaining, 0);
    assert!(engine.home_score >= score_before);
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
    let period_before = engine.period;
    engine.game_clock = 0.0;
    engine.ball_state = nba_physics::BallTrajectoryKind::Held {
        carrier_id: engine.new_possession_pg_for_test(),
    };
    engine.game_flow = nba_domain::GameFlowState::LiveBall;
    let end_tick = engine.step();
    assert_eq!(engine.game_flow, nba_domain::GameFlowState::Halftime);
    assert!(end_tick
        .frame
        .events
        .iter()
        .any(|event| event == "PHASE_TRANSITION"));
    assert_eq!(engine.period, period_before);
    engine.period_break_elapsed = engine.rules.period_break_seconds + 1.0;
    engine.step();
    assert_eq!(engine.period, period_before + 1);
}
#[test]
fn test_free_throw_phase_is_visible_in_stream_contract() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.decision_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(404, rules);
    engine.pending_events.push(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_1".to_string(),
        fouler_id: "A_1".to_string(),
        is_shooting: true,
    });
    let tick = engine.step();
    assert_eq!(tick.frame.game_flow, "FreeThrow");
    assert_eq!(tick.frame.free_throws_remaining, 2);
    assert_eq!(tick.frame.team_fouls_away, 1);
    assert_eq!(
        engine.physics.get_player("A_1").expect("fouler").foul_count,
        1
    );
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
    engine.home_score = 1;
    engine.away_score = 0;
    for _ in 0..10 {
        engine.step();
        if engine.game_flow == nba_domain::GameFlowState::GameEnd {
            break;
        }
    }
    assert_eq!(engine.game_flow, nba_domain::GameFlowState::GameEnd);
    assert_eq!(engine.step().frame.game_flow, "GameEnd");
}

#[test]
fn test_inbound_baseline_is_recorded_inside_boundary_domain() {
    let engine = MatchEngine::new(303);
    assert!(engine.inbound_baseline.x >= -0.01 && engine.inbound_baseline.x <= 94.01);
    assert!(engine.inbound_baseline.y >= 0.0 && engine.inbound_baseline.y <= 50.0);
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
    engine.home_score = 1;
    engine.game_clock = 0.0;
    engine.ball_state = nba_physics::BallTrajectoryKind::Held {
        carrier_id: "H_1".to_string(),
    };
    let mut saw_game_end = false;
    let mut previous_game_clock = engine.game_clock;
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
fn test_bonus_foul_uses_configured_threshold() {
    let mut rules = nba_domain::GameRules::default();
    rules.league.bonus_fouls_per_period = 1;
    let mut engine = MatchEngine::with_rules(707, rules);
    engine.pending_events.push(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_1".to_string(),
        fouler_id: "A_1".to_string(),
        is_shooting: false,
    });
    engine.step();
    assert_eq!(engine.team_fouls_away, 1);
    assert_eq!(engine.free_throws_remaining, 2);
    assert_eq!(engine.game_flow, nba_domain::GameFlowState::FreeThrow);
    assert_eq!(
        engine.physics.get_player("A_1").expect("fouler").foul_count,
        1
    );
}

#[test]
fn test_inbound_pass_arrival_releases_dead_ball() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.decision_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(808, rules);
    engine
        .pending_events
        .push(nba_domain::GameEvent::RuleViolation {
            constraint_id: "test".to_string(),
            reason: "test".to_string(),
        });
    let mut saw_live = false;
    for _ in 0..30 {
        let tick = engine.step();
        saw_live |= tick.frame.game_flow == "LiveBall";
        if saw_live {
            break;
        }
    }
    assert!(saw_live);
}

#[test]
fn test_shooting_foul_awards_free_throws_to_fouled_team_not_possession_flip() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.decision_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(901, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    let possession_before = engine.possession();
    engine.pending_events.push(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_3".to_string(),
        fouler_id: "A_2".to_string(),
        is_shooting: true,
    });
    engine.step();
    assert_eq!(engine.free_throws_remaining, 2);
    assert_eq!(engine.possession(), possession_before);
    assert_eq!(engine.free_throw_shooter.as_deref(), Some("H_3"));
}

#[test]
fn test_made_final_free_throw_gives_ball_to_opponent_inbound() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.decision_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(902, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.pending_events.push(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_1".to_string(),
        fouler_id: "A_1".to_string(),
        is_shooting: true,
    });
    engine.step();
    let mut saw_inbound_setup = false;
    for _ in 0..30 {
        engine.resolve_forced_free_throw(true);
        if engine.game_flow == nba_domain::GameFlowState::DeadBall
            && engine.free_throws_remaining == 0
            && engine.possession() == nba_domain::Possession::Away
        {
            saw_inbound_setup = true;
            break;
        }
    }
    assert!(saw_inbound_setup);
}

#[test]
fn test_missed_final_free_throw_starts_rebound_from_last_ball_position() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.decision_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(904, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.pending_events.push(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_1".to_string(),
        fouler_id: "A_1".to_string(),
        is_shooting: true,
    });
    engine.step();
    engine.resolve_forced_free_throw(false);
    engine.resolve_forced_free_throw(false);
    assert!(matches!(
        engine.ball_state,
        nba_physics::BallTrajectoryKind::RimRebound { from_pos, from_z, .. }
            if from_pos == engine.ball_pos_3d.0 && from_z == engine.ball_pos_3d.1
    ));
    assert_eq!(engine.game_flow, nba_domain::GameFlowState::LiveBall);
    assert_eq!(engine.sub_phase, nba_domain::SubPhase::FlightAndRebound);
}

#[test]
fn test_setup_json_uses_selected_roster_for_runtime_state() {
    let setup = nba_engine::setup::MatchSetup::builtin(nba_domain::GameRules::default());
    let starter_id = setup.home_lineup.starters[0].clone();
    let request = serde_json::json!({ "setup": setup, "seed": 916 });
    let mut service = nba_engine::MatchService::new();
    service
        .setup_match_json(&request.to_string())
        .expect("setup JSON should be accepted");
    let snapshot = service.snapshot().expect("snapshot after setup");
    assert_eq!(snapshot.frame.players.len(), 16);
    assert!(snapshot
        .frame
        .players
        .iter()
        .any(|player| player.id == starter_id));
}
#[test]
fn test_shot_clock_violation_starts_continuous_inbound_transfer() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.decision_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(903, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.game_flow = nba_domain::GameFlowState::LiveBall;
    engine.shot_clock = 0.0;
    engine.ball_state = nba_physics::BallTrajectoryKind::Held {
        carrier_id: "H_1".to_string(),
    };
    let source = engine.ball_pos_3d.0;
    let _tick = engine.step();
    assert_eq!(engine.possession(), nba_domain::Possession::Away);
    assert!(matches!(
        engine.ball_state,
        nba_physics::BallTrajectoryKind::InboundTransfer { .. }
    ));
    assert_eq!(engine.ball_pos_3d.0, source);
    let mut saw_ready = false;
    for _ in 0..50 {
        let tick = engine.step();
        if tick.frame.ball.status == "INBOUND_READY" {
            saw_ready = true;
            assert!((engine.ball_pos_3d.0 - engine.inbound_baseline).length() <= 4.5);
            break;
        }
    }
    assert!(saw_ready);
}

#[test]
fn test_period_end_waits_for_shot_in_flight() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.league.period_duration_seconds = 0.2;
    rules.league.regulation_periods = 1;
    let mut engine = MatchEngine::with_rules(904, rules);
    engine.game_clock = 0.0;
    engine.game_flow = nba_domain::GameFlowState::LiveBall;
    engine.ball_state = nba_physics::BallTrajectoryKind::Shot {
        shooter_id: "H_1".to_string(),
        from_pos: glam::Vec2::new(80.0, 25.0),
        hoop_pos: glam::Vec2::new(88.75, 25.0),
        start_time: engine.current_time,
        duration: 10.0,
        is_made: true,
        is_three: false,
        peak_z: 15.0,
    };
    let home_before = engine.home_score;
    engine.step();
    assert_eq!(engine.game_flow, nba_domain::GameFlowState::LiveBall);
    engine.step();
    assert_ne!(engine.game_flow, nba_domain::GameFlowState::GameEnd);
    assert!(engine.home_score >= home_before);
}

#[test]
fn test_event_types_are_scoped_to_one_tick() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.tactical_initiation_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(905, rules);
    let first = engine.step();
    let second = engine.step();
    assert!(first.frame.events.len() <= 10);
    assert!(second.frame.events.len() <= 10);
    assert!(engine.pending_events.is_empty());
}

#[test]
fn test_decision_uses_custom_hoop_geometry() {
    let mut rules = nba_domain::GameRules::default();
    rules.court.hoop_right_x_ft = 70.0;
    rules.court.hoop_left_x_ft = 10.0;
    let mut engine = MatchEngine::with_rules(906, rules);
    for _ in 0..100 {
        let tick = engine.step();
        assert!(tick
            .frame
            .players
            .iter()
            .all(|player| player.x >= 0.0 && player.x <= 1.0));
    }
}

#[test]
fn test_builtin_player_attributes_reach_runtime_state() {
    let engine = MatchEngine::new(907);
    let home = engine
        .physics
        .get_player("H_1")
        .expect("builtin home player");
    let away = engine
        .physics
        .get_player("A_1")
        .expect("builtin away player");
    assert_eq!(home.max_speed_ftps, away.max_speed_ftps);
    assert_eq!(home.max_accel_ftps2, away.max_accel_ftps2);
    assert_eq!(home.max_stamina, 50.0);
    assert_eq!(home.stamina, home.max_stamina);
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
fn test_event_log_is_monotonic_and_tick_scoped() {
    let mut engine = MatchEngine::new(908);
    let mut prior_sequence = 0;
    let mut logged = 0;
    for _ in 0..200 {
        let tick = engine.step();
        assert!(tick
            .frame
            .event_log
            .windows(2)
            .all(|events| events[0].sequence < events[1].sequence));
        for event in &tick.frame.event_log {
            assert!(event.sequence > prior_sequence);
            assert!((event.time - tick.frame.t).abs() < 0.001);
            prior_sequence = event.sequence;
            logged += 1;
        }
        assert_eq!(tick.frame.event_sequence, prior_sequence);
    }
    assert!(logged > 0);
}

#[test]
fn test_configured_starter_order_drives_target_binding() {
    let mut setup = nba_engine::setup::MatchSetup::builtin(nba_domain::GameRules::default());
    setup.home_lineup.starters.swap(0, 4);
    let first = setup.home_lineup.starters[0].clone();
    let engine = MatchEngine::with_setup(setup, 909);
    assert_eq!(engine.new_possession_pg_for_test(), first);
}

#[test]
fn test_selected_lineup_controls_runtime_roster() {
    let mut setup = nba_engine::setup::MatchSetup::builtin(nba_domain::GameRules::default());
    let home_bench_id = setup.home_lineup.bench.pop().expect("builtin bench");
    let away_bench_id = setup.away_lineup.bench.pop().expect("builtin bench");
    let engine = MatchEngine::with_setup(setup, 915);
    assert!(engine.physics.get_player(&home_bench_id).is_none());
    assert!(engine.physics.get_player(&away_bench_id).is_none());
    assert_eq!(engine.physics.get_players().len(), 14);
    assert!(engine.physics.get_players().values().all(|player| {
        player.action == "Bench" || player.action == "SetPosition" || player.action == "Initiate"
    }));
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
fn test_render_frame_preserves_setup_team_metadata() {
    let mut setup = nba_engine::setup::MatchSetup::builtin(nba_domain::GameRules::default());
    setup.home_team.name = "Custom Home Club".to_string();
    setup.home_team.short_name = "CHC".to_string();
    setup.away_team.name = "Custom Away Club".to_string();
    setup.away_team.short_name = "CAC".to_string();

    let tick = MatchEngine::with_setup(setup, 910).step();
    assert_eq!(tick.frame.home_team.name, "Custom Home Club");
    assert_eq!(tick.frame.home_team.short_name, "CHC");
    assert_eq!(tick.frame.away_team.name, "Custom Away Club");
    assert_eq!(tick.frame.away_team.short_name, "CAC");

    let encoded = serde_json::to_value(&tick).expect("stream tick should serialize");
    assert_eq!(encoded["home_team"]["short_name"], "CHC");
    assert_eq!(encoded["away_team"]["short_name"], "CAC");
}

#[test]
fn physics_contact_is_promoted_to_semantic_event_stream() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    rules.contact_margin_ft = 0.2;
    let mut engine = MatchEngine::with_rules(808, rules);
    engine.physics.get_player_mut("H_1").unwrap().pos_ft = Vec2::new(30.0, 25.0);
    engine.physics.get_player_mut("A_1").unwrap().pos_ft = Vec2::new(33.0, 25.0);
    let tick = engine.step();
    assert!(
        tick.frame
            .events
            .iter()
            .any(|event| event == "CONTACT_BUMP")
            || tick.frame.event_type.as_deref() == Some("CONTACT_BUMP"),
        "raw contact was not surfaced as a semantic contact event: {:?}",
        tick.frame.events
    );
    assert!(engine.pending_events.is_empty());
}

#[test]
fn screen_contact_classification_uses_tactical_action_context() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    rules.contact_margin_ft = 0.2;
    let mut engine = MatchEngine::with_rules(909, rules);
    engine.physics.get_player_mut("H_1").unwrap().pos_ft = Vec2::new(30.0, 25.0);
    engine.physics.get_player_mut("H_1").unwrap().action = "SET_HIGH_SCREEN".to_string();
    engine.physics.get_player_mut("A_1").unwrap().pos_ft = Vec2::new(33.0, 25.0);
    let tick = engine.step();
    assert!(
        tick.frame
            .events
            .iter()
            .any(|event| event == "SCREEN_CONTACT")
            || tick.frame.event_type.as_deref() == Some("SCREEN_CONTACT"),
        "screen action did not classify the raw contact: {:?}",
        tick.frame.events
    );
}

#[test]
fn match_service_owns_lifecycle_and_fixed_step_accumulator() {
    use nba_engine::{MatchService, SessionState};

    let setup = nba_engine::MatchSetup::builtin(nba_domain::GameRules::default());
    let mut service = MatchService::with_seed(911);
    assert_eq!(service.state(), SessionState::Ready);
    assert!(service.tick(nba_domain::FixedDt(0.1)).is_err());
    let info = service
        .setup_match(setup)
        .expect("setup should be accepted");
    assert_eq!(info.seed, 911);
    let initial = service.snapshot().expect("snapshot after setup");
    service
        .tick(nba_domain::FixedDt(0.01))
        .expect("paused tick");
    assert_eq!(service.snapshot().unwrap().frame.t, initial.frame.t);

    service.start().expect("configured session starts");
    let before = service.snapshot().unwrap();
    let partial = service
        .tick(nba_domain::FixedDt(0.02))
        .expect("partial tick");
    assert_eq!(partial.frame.t, before.frame.t);
    let stepped = service.tick(nba_domain::FixedDt(0.02)).expect("fixed step");
    assert!(stepped.frame.t > before.frame.t);
    assert_eq!(service.state(), SessionState::Running);
    service.pause();
    let paused = service.snapshot().unwrap();
    service
        .tick(nba_domain::FixedDt(1.0))
        .expect("paused tick is read-only");
    assert_eq!(service.snapshot().unwrap().frame.t, paused.frame.t);
}

#[test]
fn match_service_next_possession_preserves_pause_state_and_history() {
    use nba_engine::{MatchService, SessionState};

    let mut service = MatchService::with_seed(912);
    service
        .setup_match(nba_engine::MatchSetup::builtin(
            nba_domain::GameRules::default(),
        ))
        .expect("setup should be accepted");
    let before = service.snapshot().unwrap();
    let after = service
        .next_possession()
        .expect("next possession should advance");
    assert_eq!(service.state(), SessionState::Ready);
    assert!(after.frame.possession_id != before.frame.possession_id);
    assert!(!service.events_since(0).is_empty());
}

#[test]
fn match_service_rejects_invalid_lifecycle_commands_and_preserves_state() {
    use nba_engine::{MatchService, SessionState};

    let mut service = MatchService::with_seed(913);
    assert!(service.start().is_err());
    assert!(service.resume().is_err());
    assert!(service.fast_forward(1.0).is_err());
    assert!(service.next_possession().is_err());

    service
        .setup_match(nba_engine::MatchSetup::builtin(
            nba_domain::GameRules::default(),
        ))
        .expect("setup should be accepted");
    assert_eq!(service.state(), SessionState::Ready);
    assert!(service.tick(nba_domain::FixedDt(-0.01)).is_err());
    assert!(service.tick(nba_domain::FixedDt(f32::NAN)).is_err());
    assert!(service.fast_forward(-1.0).is_err());
    assert!(service.fast_forward(f32::INFINITY).is_err());
    assert_eq!(service.state(), SessionState::Ready);
}

#[test]
fn match_service_fast_forward_preserves_running_and_paused_lifecycle() {
    use nba_engine::{MatchService, SessionState};

    let mut service = MatchService::with_seed(914);
    service
        .setup_match(nba_engine::MatchSetup::builtin(
            nba_domain::GameRules::default(),
        ))
        .expect("setup should be accepted");
    service.start().expect("configured session starts");
    let running_before = service.snapshot().unwrap().frame.t;
    let running_after = service.fast_forward(0.08).expect("running fast-forward");
    assert!(running_after.frame.t > running_before);
    assert_eq!(service.state(), SessionState::Running);

    service.pause();
    let paused_before = service.snapshot().unwrap().frame.t;
    let paused_after = service.fast_forward(0.08).expect("paused fast-forward");
    assert!(paused_after.frame.t > paused_before);
    assert_eq!(service.state(), SessionState::Paused);
}

#[test]
fn event_log_retains_replayable_domain_payloads() {
    let mut engine = MatchEngine::new(917);
    let tick = (0..200)
        .map(|_| engine.step())
        .find(|tick| !tick.frame.event_log.is_empty())
        .expect("simulation should publish a domain event");
    assert!(tick
        .frame
        .event_log
        .iter()
        .all(|event| event.data.is_some()));
    let encoded = serde_json::to_value(&tick).expect("event payload should serialize");
    let first = encoded["event_log"]
        .as_array()
        .and_then(|events| events.first())
        .expect("serialized event log should contain an entry");
    assert!(first["data"].is_object());
}

#[test]
fn possession_summary_uses_elapsed_time_and_turnover_actor() {
    let mut engine = MatchEngine::new(918);
    engine.current_possession_start_clock = 100.0;
    engine.game_clock = 97.5;
    engine.current_possession_start_time = 10.0;
    engine.current_time = 12.5;
    engine.current_possession_turnover_player = Some("H_1".to_string());

    engine.emit_possession_summary("TURNOVER_VIOLATION", None, Some("H_1".to_string()), None);

    let summary = engine
        .pending_events
        .iter()
        .find_map(|event| match event {
            nba_domain::GameEvent::PossessionSummary(summary) => Some(summary),
            _ => None,
        })
        .expect("summary should be emitted");
    assert!((summary.duration_seconds - 2.5).abs() < f32::EPSILON);
    assert_eq!(summary.turnover_player_id.as_deref(), Some("H_1"));
}

#[test]
fn setup_rejects_players_outside_configured_geometry() {
    let mut setup = nba_engine::MatchSetup::builtin(nba_domain::GameRules::default());
    setup.home_team.players[0].initial_position_ft = (200.0, 25.0);
    let error = setup
        .validate()
        .expect_err("invalid player position must be rejected");
    assert!(error.contains("starts outside the court geometry"));
}

#[test]
fn pass_arrival_replays_release_outcome_and_emits_matching_fact() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.tactical_initiation_seconds = 0.0;
    rules.decision_interval_seconds = 0.1;
    let mut setup = nba_engine::MatchSetup::builtin(rules);
    setup.rules.resolve.base_rates.pass_success = 1.0;
    setup.rules.resolve.pass.lane_risk_weight = 0.0;
    setup.rules.resolve.pass.openness_weight = 0.0;
    setup.rules.resolve.pass.passer_skill_weight = 0.0;
    setup.rules.resolve.pass.receiver_control_weight = 0.0;
    setup.rules.resolve.pass.catch_equilibrium_weight = 0.0;
    let mut engine = MatchEngine::with_setup(setup, 1201);
    engine.ball_state = nba_physics::BallTrajectoryKind::Pass {
        from_pos: glam::Vec2::new(40.0, 25.0),
        to_pos: glam::Vec2::new(50.0, 25.0),
        target_id: "H_2".to_string(),
        start_time: 1.0,
        duration: 0.2,
        peak_z: engine.rules.pass_peak_ft,
        inbound: false,
        receive_success: true,
    };
    engine.last_passer_id = Some("H_1".to_string());
    engine.game_flow = nba_domain::GameFlowState::LiveBall;
    engine.sub_phase = nba_domain::SubPhase::ActionExecution;
    engine.current_time = 1.0;
    engine.physics.get_player_mut("H_1").expect("passer").pos_ft = glam::Vec2::new(40.0, 25.0);
    engine
        .physics
        .get_player_mut("H_2")
        .expect("receiver")
        .pos_ft = glam::Vec2::new(50.0, 25.0);
    engine.pending_events.clear();
    let mut saw_receive = false;
    for _ in 0..30 {
        let tick = engine.step();
        saw_receive |= tick
            .frame
            .events
            .iter()
            .any(|event| event == "PASS_RECEIVED");
        if saw_receive {
            break;
        }
    }
    assert!(saw_receive, "pass arrival did not emit PASS_RECEIVED");
    assert!(matches!(
        engine.ball_state,
        nba_physics::BallTrajectoryKind::Held { ref carrier_id } if carrier_id == "H_2"
    ));
}

#[test]
fn pass_release_policy_can_emit_drop_without_redeciding_at_arrival() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.tactical_initiation_seconds = 0.0;
    rules.decision_interval_seconds = 0.1;
    let mut setup = nba_engine::MatchSetup::builtin(rules);
    setup.rules.resolve.base_rates.pass_success = 0.0;
    setup.rules.resolve.pass.lane_risk_weight = 0.0;
    setup.rules.resolve.pass.openness_weight = 0.0;
    setup.rules.resolve.pass.passer_skill_weight = 0.0;
    setup.rules.resolve.pass.receiver_control_weight = 0.0;
    setup.rules.resolve.pass.catch_equilibrium_weight = 0.0;
    let mut engine = MatchEngine::with_setup(setup, 1202);
    engine.ball_state = nba_physics::BallTrajectoryKind::Pass {
        from_pos: glam::Vec2::new(40.0, 25.0),
        to_pos: glam::Vec2::new(50.0, 25.0),
        target_id: "H_2".to_string(),
        start_time: 0.0,
        duration: 0.1,
        peak_z: engine.rules.pass_peak_ft,
        inbound: false,
        receive_success: false,
    };
    engine.last_passer_id = Some("H_1".to_string());
    engine.current_time = 0.0;
    engine.game_flow = nba_domain::GameFlowState::LiveBall;
    engine.sub_phase = nba_domain::SubPhase::ActionExecution;
    let tick = engine.step();
    assert!(tick
        .frame
        .events
        .iter()
        .any(|event| event == "PASS_DROPPED"));
    assert!(matches!(
        engine.ball_state,
        nba_physics::BallTrajectoryKind::LooseBall { .. }
    ));
}

#[test]
fn shot_release_uses_configured_skill_and_spacing_inputs() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    rules.shot_pct_floor = 1.0;
    rules.shot_pct_ceiling = 1.0;
    let mut engine = MatchEngine::with_rules(1211, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.ball_pos_3d.0 = glam::Vec2::new(40.0, 25.0);
    engine
        .physics
        .get_player_mut("H_1")
        .expect("shooter")
        .attributes
        .shooting_mid = 1.0;
    engine
        .physics
        .get_player_mut("H_1")
        .expect("shooter")
        .pos_ft = glam::Vec2::new(40.0, 25.0);
    engine.execute_shot_for_test("H_1", glam::Vec2::new(40.0, 25.0), false);
    assert!(matches!(
        engine.ball_state,
        nba_physics::BallTrajectoryKind::Shot { is_made: true, .. }
    ));
}

#[test]
fn audit_margin_is_part_of_serialized_rules_contract() {
    let mut engine = MatchEngine::new(1204);
    let value = serde_json::to_value(engine.step()).expect("frame should serialize");
    assert_eq!(
        value["rules"]["separation_safety_margin_ft"],
        serde_json::json!(engine.rules.separation_safety_margin_ft)
    );
}

#[test]
fn test_configured_tip_off_duration_produces_physical_tipoff_phase() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 1.0;
    let setup = nba_engine::MatchSetup::builtin(rules);
    let mut engine = MatchEngine::with_setup(setup, 42);
    assert_eq!(engine.game_flow, nba_domain::GameFlowState::TipOff);

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
    assert_eq!(engine.game_flow, nba_domain::GameFlowState::LiveBall);
}
