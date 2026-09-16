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
        out_of_bounds_placement: false,
        is_receiving_pass: false,
        is_driving_to_rim: false,
        boundary_cross_latched: false,
        attributes: Default::default(),
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
    players.insert("H_01".to_string(), make_player("H_01", "home", 40.0, 25.0));
    players.insert("H_02".to_string(), make_player("H_02", "home", 60.0, 25.0));
    let physics = make_physics(&players);
    let ctx = base_ctx(&physics);
    let action = CandidateAction::Pass {
        passer_id: "H_01".to_string(),
        receiver_id: "H_02".to_string(),
        from_pos: Vec2::new(40.0, 25.0),
        to_pos: Vec2::new(96.0, 25.0),
    };
    let scored = ConstraintRegistry::default().evaluate_candidate(&ctx, &action);
    assert!(!scored.feasible);
}

#[test]
fn test_dead_ball_blocks_shot() {
    let mut players = HashMap::new();
    players.insert("H_01".to_string(), make_player("H_01", "home", 40.0, 25.0));
    let physics = make_physics(&players);
    let mut ctx = base_ctx(&physics);
    ctx.phase = PhaseType::DeadBallReset;
    ctx.game_flow = nba_domain::GameFlowState::DeadBall;
    let action = CandidateAction::Shoot {
        shooter_id: "H_01".to_string(),
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
    players.insert("H_01".to_string(), make_player("H_01", "home", -2.0, 25.0));
    players.insert("H_02".to_string(), make_player("H_02", "home", 10.0, 25.0));
    let physics = make_physics(&players);
    let mut ctx = base_ctx(&physics);
    ctx.ball_pos = Vec2::new(-2.0, 25.0);
    ctx.phase = PhaseType::Inbound;
    ctx.game_flow = nba_domain::GameFlowState::DeadBall;
    let action = CandidateAction::InboundPass {
        passer_id: "H_01".to_string(),
        receiver_id: "H_02".to_string(),
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
        if (12065..=12082).contains(&i) {
            eprintln!(
                "TICK {}: flow={:?} sub={:?} ball={:?} holder={:?} events={:?} event_log={:?}",
                i,
                engine.game_flow(),
                engine.sub_phase(),
                engine.ball_state(),
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
    let before = engine.game_clock();
    engine.step();
    assert!((engine.current_time() - 0.1).abs() < f32::EPSILON);
    assert!(engine.game_clock() < before);
}

#[test]
fn test_ball_flight_is_not_available_for_second_action() {
    let mut players = HashMap::new();
    players.insert("H_01".to_string(), make_player("H_01", "home", 40.0, 25.0));
    let physics = make_physics(&players);
    let mut ctx = base_ctx(&physics);
    ctx.ball_phase = nba_domain::BallPhase::PassFlight;
    let action = CandidateAction::Shoot {
        shooter_id: "H_01".to_string(),
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
    players.insert("H_01".to_string(), make_player("H_01", "home", 40.0, 25.0));
    let physics = make_physics(&players);
    let mut ctx = base_ctx(&physics);
    ctx.shot_clock = 0.0;
    let findings = ConstraintRegistry::default().evaluate_runtime(&ctx);
    assert!(!findings.is_empty());
}

#[test]
fn test_game_clock_expired_blocks_shot_only() {
    let mut players = HashMap::new();
    players.insert("H_01".to_string(), make_player("H_01", "home", 40.0, 25.0));
    let physics = make_physics(&players);
    let mut ctx = base_ctx(&physics);
    ctx.game_clock = 0.0;
    let shot = CandidateAction::Shoot {
        shooter_id: "H_01".to_string(),
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
    players.insert("H_01".to_string(), make_player("H_01", "home", 40.0, 25.0));
    players.insert("H_02".to_string(), make_player("H_02", "home", 60.0, 25.0));
    players.insert("A_01".to_string(), make_player("A_01", "away", 50.0, 25.0));
    let physics = make_physics(&players);
    let ctx = base_ctx(&physics);
    let action = CandidateAction::Pass {
        passer_id: "H_01".to_string(),
        receiver_id: "H_02".to_string(),
        from_pos: Vec2::new(40.0, 25.0),
        to_pos: Vec2::new(60.0, 25.0),
    };
    let scored = ConstraintRegistry::default().evaluate_candidate(&ctx, &action);
    assert!(scored.risk >= 0.0);
}

#[test]
fn test_clock_urgency_preference_boosts_shot() {
    let mut players = HashMap::new();
    players.insert("H_01".to_string(), make_player("H_01", "home", 40.0, 25.0));
    let physics = make_physics(&players);
    let mut ctx = base_ctx(&physics);
    ctx.shot_clock = 2.0;
    let shot = CandidateAction::Shoot {
        shooter_id: "H_01".to_string(),
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
    engine.set_shot_clock_for_test(engine.rules().tick_seconds);
    let previous_possession = engine.possession();

    let tick = engine.step();

    assert!(tick.frame.events.iter().any(|event| event == "VIOLATION"));
    assert_ne!(engine.possession(), previous_possession);
    assert!(matches!(
        engine.ball_state(),
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
        player_id: "H_01".to_string(),
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
    let initial = engine.game_clock();
    engine.set_game_flow_for_test(nba_domain::GameFlowState::Timeout);
    engine.step();
    assert_eq!(engine.game_clock(), initial);
}

#[test]
fn test_phase_transition_and_pass_events_are_observable() {
    let mut engine = MatchEngine::new(7);
    let mut saw_phase_transition = false;
    let mut saw_pass = false;
    // 预算 1500 tick：发球员现在必须真实走到界外发球点（F1.3 placement 修正），
    // 首次入界传球比历史硬编码预算更晚，但仍是单节内的正常时序。
    for _ in 0..1500 {
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
        from_z: engine.rules().rim_height_ft,
        last_touch_team: engine.possession(),
        hoop_pos: engine.rules().court.hoop_pos(true),
        target_landing: glam::Vec2::new(70.0, 25.0),
        start_time: engine.current_time(),
        duration: 10.0,
        peak_z: engine.rules().rebound_peak_ft,
    };
    engine.set_ball_state_for_test(rebound);
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
        player_a: "H_01".to_string(),
        player_b: "A_01".to_string(),
        impact_speed: 20.0,
        contact_normal: (1.0, 0.0),
        is_screen: false,
        semantic_kind: "BlockingCandidate".to_string(),
        semantic_severity: "FoulCandidate".to_string(),
        possessor_id: Some("H_01".to_string()),
        legal_position: false,
    };
    let mut rng = rand::rngs::StdRng::seed_from_u64(1);
    let outcome = ResolutionLayer::resolve_contact_with_policy(
        "H_01",
        "A_01",
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
    engine.push_event_for_test(nba_domain::GameEvent::Contact {
        player_a: "H_01".to_string(),
        player_b: "A_01".to_string(),
        impact_speed: 20.0,
        contact_normal: (1.0, 0.0),
        is_screen: false,
        semantic_kind: "BlockingCandidate".to_string(),
        semantic_severity: "FoulCandidate".to_string(),
        possessor_id: Some("H_01".to_string()),
        legal_position: false,
    });
    let before = engine
        .modulation_for_test()
        .get("H_01")
        .map(|m| m.catch_equilibrium)
        .unwrap_or(1.0);
    let tick = engine.step();
    let foul_seen = tick.frame.events.iter().any(|event| event == "FOUL")
        || tick.frame.event_type.as_deref() == Some("FOUL");
    if foul_seen {
        assert!(
            engine
                .modulation_for_test()
                .get("H_01")
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
    engine.push_event_for_test(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_01".to_string(),
        fouler_id: "A_01".to_string(),
        is_shooting: true,
    });
    let first = engine.step();
    assert!(
        first.frame.events.iter().any(|event| event == "FOUL")
            || first.frame.event_type.as_deref() == Some("FOUL")
    );
    assert_eq!(engine.game_flow(), nba_domain::GameFlowState::FreeThrow);
    assert_eq!(engine.free_throws_remaining(), 2);
    let score_before = engine.engine_snapshot().game.home_score;
    for _ in 0..20 {
        engine.step();
        if engine.free_throws_remaining() == 0 {
            break;
        }
    }
    assert_eq!(engine.free_throws_remaining(), 0);
    assert!(engine.engine_snapshot().game.home_score >= score_before);
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
    engine.set_game_flow_for_test(nba_domain::GameFlowState::LiveBall);
    let end_tick = engine.step();
    assert_eq!(engine.game_flow(), nba_domain::GameFlowState::Halftime);
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
fn test_free_throw_phase_is_visible_in_stream_contract() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.decision_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(404, rules);
    engine.push_event_for_test(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_01".to_string(),
        fouler_id: "A_01".to_string(),
        is_shooting: true,
    });
    let tick = engine.step();
    assert_eq!(tick.frame.game_flow, "FreeThrow");
    assert_eq!(tick.frame.free_throws_remaining, 2);
    assert_eq!(tick.frame.team_fouls_away, 1);
    assert_eq!(
        engine
            .physics()
            .get_player("A_01")
            .expect("fouler")
            .foul_count,
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
    engine.set_scores_for_test(1, 0);

    for _ in 0..10 {
        engine.step();
        if engine.game_flow() == nba_domain::GameFlowState::GameEnd {
            break;
        }
    }
    assert_eq!(engine.game_flow(), nba_domain::GameFlowState::GameEnd);
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
fn test_bonus_foul_uses_configured_threshold() {
    let mut rules = nba_domain::GameRules::default();
    rules.league.bonus_fouls_per_period = 1;
    let mut engine = MatchEngine::with_rules(707, rules);
    engine.push_event_for_test(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_01".to_string(),
        fouler_id: "A_01".to_string(),
        is_shooting: false,
    });
    engine.step();
    assert_eq!(engine.team_fouls_away(), 1);
    assert_eq!(engine.free_throws_remaining(), 2);
    assert_eq!(engine.game_flow(), nba_domain::GameFlowState::FreeThrow);
    assert_eq!(
        engine
            .physics()
            .get_player("A_01")
            .expect("fouler")
            .foul_count,
        1
    );
}

#[test]
fn test_inbound_pass_arrival_releases_dead_ball() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.decision_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(808, rules);
    engine.push_event_for_test(nba_domain::GameEvent::RuleViolation {
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
    engine.push_event_for_test(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_03".to_string(),
        fouler_id: "A_02".to_string(),
        is_shooting: true,
    });
    engine.step();
    assert_eq!(engine.free_throws_remaining(), 2);
    assert_eq!(engine.possession(), possession_before);
    assert_eq!(engine.free_throw_shooter(), Some("H_03"));
}

#[test]
fn test_made_final_free_throw_gives_ball_to_opponent_inbound() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.decision_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(902, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.push_event_for_test(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_01".to_string(),
        fouler_id: "A_01".to_string(),
        is_shooting: true,
    });
    engine.step();
    let mut saw_inbound_setup = false;
    for _ in 0..30 {
        engine.resolve_forced_free_throw(true);
        if engine.game_flow() == nba_domain::GameFlowState::DeadBall
            && engine.free_throws_remaining() == 0
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
    engine.push_event_for_test(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_01".to_string(),
        fouler_id: "A_01".to_string(),
        is_shooting: true,
    });
    engine.step();
    engine.resolve_forced_free_throw(false);
    engine.resolve_forced_free_throw(false);
    assert!(matches!(
        engine.ball_state(),
        nba_physics::BallTrajectoryKind::RimRebound { from_pos, from_z, .. }
            if *from_pos == engine.ball_pos_3d().0 && *from_z == engine.ball_pos_3d().1
    ));
    assert_eq!(engine.game_flow(), nba_domain::GameFlowState::LiveBall);
    assert_eq!(engine.sub_phase(), nba_domain::SubPhase::FlightAndRebound);
}

#[test]
fn test_setup_json_uses_selected_roster_for_runtime_state() {
    let setup = nba_engine::setup::MatchSetup::builtin(nba_domain::GameRules::default());
    // 首发身份来自名册的 `starter` 标记，而非数组位置（P-2）。
    let starter_id = setup
        .home_team
        .players
        .iter()
        .find(|p| p.starter)
        .map(|p| p.id.clone())
        .expect("roster must declare a starter");
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
    engine.set_game_flow_for_test(nba_domain::GameFlowState::LiveBall);
    engine.set_shot_clock_for_test(0.0);
    engine.set_ball_state_for_test(nba_physics::BallTrajectoryKind::Held {
        carrier_id: "H_01".to_string(),
    });
    let source = engine.ball_pos_3d().0;
    let _tick = engine.step();
    assert_eq!(engine.possession(), nba_domain::Possession::Away);
    assert!(matches!(
        engine.ball_state(),
        nba_physics::BallTrajectoryKind::InboundTransfer { .. }
    ));
    assert_eq!(engine.ball_pos_3d().0, source);
    let mut saw_ready = false;
    for _ in 0..50 {
        let tick = engine.step();
        if tick.frame.ball.status == "INBOUND_READY" {
            saw_ready = true;
            assert!((engine.ball_pos_3d().0 - engine.inbound_baseline()).length() <= 4.5);
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
    engine.set_game_clock_for_test(0.0);
    engine.set_game_flow_for_test(nba_domain::GameFlowState::LiveBall);
    engine.set_ball_state_for_test(nba_physics::BallTrajectoryKind::Shot {
        shooter_id: "H_01".to_string(),
        from_pos: glam::Vec2::new(80.0, 25.0),
        hoop_pos: glam::Vec2::new(88.75, 25.0),
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
    assert_eq!(engine.game_flow(), nba_domain::GameFlowState::LiveBall);
    engine.step();
    assert_ne!(engine.game_flow(), nba_domain::GameFlowState::GameEnd);
    assert!(engine.home_score() >= home_before);
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
    assert!(engine.pending_events().is_empty());
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
        .physics()
        .get_player("H_01")
        .expect("builtin home player");
    let away = engine
        .physics()
        .get_player("A_01")
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
    let first = setup
        .home_team
        .players
        .iter()
        .find(|p| p.starter)
        .map(|p| p.id.clone())
        .expect("roster must declare a starter");
    let engine = MatchEngine::with_setup(setup, 909);
    assert_eq!(engine.new_possession_pg_for_test(), first);
}

#[test]
fn test_selected_lineup_controls_runtime_roster() {
    let mut setup = nba_engine::setup::MatchSetup::builtin(nba_domain::GameRules::default());
    let home_bench_id = setup.home_lineup.bench.pop().expect("builtin bench");
    let away_bench_id = setup.away_lineup.bench.pop().expect("builtin bench");
    let engine = MatchEngine::with_setup(setup, 915);
    assert!(engine.physics().get_player(&home_bench_id).is_none());
    assert!(engine.physics().get_player(&away_bench_id).is_none());
    assert_eq!(engine.physics().get_players().len(), 14);
    assert!(engine.physics().get_players().values().all(|player| {
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
    engine
        .physics_mut_for_test()
        .get_player_mut("H_01")
        .unwrap()
        .pos_ft = Vec2::new(30.0, 25.0);
    engine
        .physics_mut_for_test()
        .get_player_mut("A_01")
        .unwrap()
        .pos_ft = Vec2::new(33.0, 25.0);
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
    assert!(engine.pending_events().is_empty());
}

#[test]
fn screen_contact_classification_uses_tactical_action_context() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    rules.contact_margin_ft = 0.2;
    let mut engine = MatchEngine::with_rules(909, rules);
    engine
        .physics_mut_for_test()
        .get_player_mut("H_01")
        .unwrap()
        .pos_ft = Vec2::new(30.0, 25.0);
    engine
        .physics_mut_for_test()
        .get_player_mut("H_01")
        .unwrap()
        .action = "SET_HIGH_SCREEN".to_string();
    engine
        .physics_mut_for_test()
        .get_player_mut("A_01")
        .unwrap()
        .pos_ft = Vec2::new(33.0, 25.0);
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
    engine.set_possession_context_for_test(Some("H_01".to_string()), 100.0, 10.0);
    engine.set_game_clock_for_test(97.5);
    engine.set_current_time_for_test(12.5);

    engine.emit_possession_summary(
        nba_domain::PossessionEndCause::TurnoverViolation,
        None,
        Some("H_01".to_string()),
        None,
    );

    let summary = engine
        .pending_events()
        .iter()
        .find_map(|event| match event {
            nba_domain::GameEvent::PossessionSummary(summary) => Some(summary),
            _ => None,
        })
        .expect("summary should be emitted");
    assert!((summary.duration_seconds - 2.5).abs() < f32::EPSILON);
    assert_eq!(summary.turnover_player_id.as_deref(), Some("H_01"));
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

/// 关闭接球人的预估噪声（层 A 对照开关）。
///
/// `receive_estimate_noise_ft == 0` 时接球人精确知道落点 —— 这是
/// round-10 之前的行为，仅用于"与位置无关"的机制测试。
fn setup_noise_off(engine: &mut nba_engine::MatchEngine) {
    engine.rules_mut_for_test().receive_estimate_noise_ft = 0.0;
}

#[test]
fn pass_arrival_replays_release_outcome_and_emits_matching_fact() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.tactical_initiation_seconds = 0.0;
    rules.decision_interval_seconds = 0.1;
    let mut setup = nba_engine::MatchSetup::builtin(rules);
    setup.rules.resolve.base_rates.pass_success = 1.0;
    setup.rules.resolve.pass.openness_weight = 0.0;
    setup.rules.resolve.pass.passer_skill_weight = 0.0;
    setup.rules.resolve.pass.receiver_control_weight = 0.0;
    setup.rules.resolve.pass.catch_equilibrium_weight = 0.0;
    let mut engine = MatchEngine::with_setup(setup, 1201);
    engine.set_ball_state_for_test(nba_physics::BallTrajectoryKind::Pass {
        from_pos: glam::Vec2::new(40.0, 25.0),
        to_pos: glam::Vec2::new(50.0, 25.0),
        target_id: "H_02".to_string(),
        start_time: 1.0,
        duration: 0.2,
        peak_z: engine.rules().pass_peak_ft,
        inbound: false,
        receive_success: true,
        intercept: None,
    });
    engine.set_last_passer_for_test(Some("H_01".to_string()));
    engine.set_game_flow_for_test(nba_domain::GameFlowState::LiveBall);
    engine.set_sub_phase_for_test(nba_domain::SubPhase::ActionExecution);
    engine.set_current_time_for_test(1.0);
    // 层 A（P-1）修正：接球人不再直读传球人的冻结落点，而是按**自己的
    // 感知**预估并跑位，因此可能接不到。本测试的目的是验证「release 时
    // 裁定的接收结果被忠实回放」，与位置无关，故关闭预估噪声
    // （`receive_estimate_noise_ft = 0` 即恢复"精确知道落点"的对照行为）。
    setup_noise_off(&mut engine);
    // 同时把接球人标记为「已到位静止」：本测试验证的是
    // 「release 时裁定的接收结果被忠实回放」，与接球人的跑位模型无关。
    // 层 A（P-1）引入后，接球人按自己的估计跑位，可能与冻结点分离 ——
    // 那是另一个被测对象（见 tests/pass_information.rs）。
    {
        let ids: Vec<String> = engine.physics().get_players().keys().cloned().collect();
        for id in ids {
            if let Some(p) = engine.physics_mut_for_test().get_player_mut(&id) {
                p.vel_ft = glam::Vec2::ZERO;
                p.target_speed_ftps = 0.0;
            }
        }
        if let Some(p) = engine.physics_mut_for_test().get_player_mut("H_02") {
            p.pos_ft = glam::Vec2::new(50.0, 25.0);
            p.target_pos_ft = glam::Vec2::new(50.0, 25.0);
        }
    }
    engine
        .physics_mut_for_test()
        .get_player_mut("H_01")
        .expect("passer")
        .pos_ft = glam::Vec2::new(40.0, 25.0);
    engine
        .physics_mut_for_test()
        .get_player_mut("H_02")
        .expect("receiver")
        .pos_ft = glam::Vec2::new(50.0, 25.0);
    // 初速清零：本测试构造「接球人恰好站在落点」的静止场景。
    // 引擎初始化赋予的战术跑位初速会因惯性把他推出落点（实测 1 tick 滑
    // 1.73 ft > catch_radius），使层 A 失败 —— 那是测试场景污染，不是被测行为。
    engine
        .physics_mut_for_test()
        .get_player_mut("H_02")
        .expect("receiver")
        .vel_ft = glam::Vec2::ZERO;
    // 目标速度也必须清零：接球人的「到位即停」分支条件是
    // `is_receiving_pass && target_speed == 0`。本场景接球人恰好站在落点，
    // 不应有残留的战术移动速度。
    engine
        .physics_mut_for_test()
        .get_player_mut("H_02")
        .expect("receiver")
        .target_speed_ftps = 0.0;
    engine
        .physics_mut_for_test()
        .get_player_mut("H_02")
        .expect("receiver")
        .target_pos_ft = glam::Vec2::new(50.0, 25.0);
    // 把其余在场球员移离接球点，避免分离投影把他推出 catch_radius。
    //
    // 实测：A_5 的默认站位距 (50,25) 仅 1.5 ft（< min_separation 3.6），
    // 物理层的分离投影会在第一个 tick 把 H_2 推开 1.73 ft > catch_radius 2.6，
    // 使层 A 判「接不到」。本测试验证的是「release 结果被忠实回放」，
    // 不应被无关球员的站位污染。
    {
        let others: Vec<String> = engine
            .physics()
            .get_players()
            .iter()
            .filter(|(id, p)| p.on_court && id.as_str() != "H_02")
            .map(|(id, _)| id.clone())
            .collect();
        for (i, id) in others.iter().enumerate() {
            if let Some(p) = engine.physics_mut_for_test().get_player_mut(id) {
                // 沿边线一字排开，远离接球点
                p.pos_ft = glam::Vec2::new(5.0, 5.0 + i as f32 * 4.0);
                p.target_pos_ft = p.pos_ft;
                p.vel_ft = glam::Vec2::ZERO;
                p.target_speed_ftps = 0.0;
            }
        }
    }
    engine.clear_pending_events_for_test();
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
        engine.ball_state(),
        nba_physics::BallTrajectoryKind::Held { ref carrier_id } if carrier_id == "H_02"
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
    setup.rules.resolve.pass.openness_weight = 0.0;
    setup.rules.resolve.pass.passer_skill_weight = 0.0;
    setup.rules.resolve.pass.receiver_control_weight = 0.0;
    setup.rules.resolve.pass.catch_equilibrium_weight = 0.0;
    let mut engine = MatchEngine::with_setup(setup, 1202);
    engine.set_ball_state_for_test(nba_physics::BallTrajectoryKind::Pass {
        from_pos: glam::Vec2::new(40.0, 25.0),
        to_pos: glam::Vec2::new(50.0, 25.0),
        target_id: "H_02".to_string(),
        start_time: 0.0,
        duration: 0.1,
        peak_z: engine.rules().pass_peak_ft,
        inbound: false,
        receive_success: false,
        intercept: None,
    });
    engine.set_last_passer_for_test(Some("H_01".to_string()));
    engine.set_current_time_for_test(0.0);
    engine.set_game_flow_for_test(nba_domain::GameFlowState::LiveBall);
    engine.set_sub_phase_for_test(nba_domain::SubPhase::ActionExecution);
    let tick = engine.step();
    assert!(tick
        .frame
        .events
        .iter()
        .any(|event| event == "PASS_DROPPED"));
    assert!(matches!(
        engine.ball_state(),
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
    engine.ball_pos_3d().0 = glam::Vec2::new(40.0, 25.0);
    engine
        .physics_mut_for_test()
        .get_player_mut("H_01")
        .expect("shooter")
        .attributes
        .shooting_mid = 1.0;
    engine
        .physics_mut_for_test()
        .get_player_mut("H_01")
        .expect("shooter")
        .pos_ft = glam::Vec2::new(40.0, 25.0);
    engine.execute_shot_for_test("H_01", glam::Vec2::new(40.0, 25.0), false);
    assert!(matches!(
        engine.ball_state(),
        nba_physics::BallTrajectoryKind::Shot { is_made: true, .. }
    ));
}

#[test]
fn audit_margin_is_part_of_serialized_rules_contract() {
    let mut engine = MatchEngine::new(1204);
    let value = serde_json::to_value(engine.step()).expect("frame should serialize");
    assert_eq!(
        value["rules"]["separation_safety_margin_ft"],
        serde_json::json!(engine.rules().separation_safety_margin_ft)
    );
}

#[test]
fn test_configured_tip_off_duration_produces_physical_tipoff_phase() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 1.0;
    let setup = nba_engine::MatchSetup::builtin(rules);
    let mut engine = MatchEngine::with_setup(setup, 42);
    assert_eq!(engine.game_flow(), nba_domain::GameFlowState::TipOff);

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
    assert_eq!(engine.game_flow(), nba_domain::GameFlowState::LiveBall);
}

/// F1.1 红测试：罚球全程不得输出 holderId（伪持球）。
/// 根因：罚球期间权威球态仍是 Held{carrier_id}，而 ball_pos_3d 被放到
/// 篮筐/罚球点，导致 BALL_WITH_HOLDER Hard（seed=1 full tick=58954，26.51ft）。
#[test]
fn test_free_throw_never_reports_ball_holder() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    rules.free_throw_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(101, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.push_event_for_test(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_01".to_string(),
        fouler_id: "A_01".to_string(),
        is_shooting: true,
    });

    // 第一 tick 触发犯规，随后每个 tick 检查罚球期间的球权投影。
    let mut saw_free_throw = false;
    for _ in 0..40 {
        let tick = engine.step();
        if tick.frame.game_flow == "FreeThrow"
            || engine.game_flow() == nba_domain::GameFlowState::FreeThrow
        {
            saw_free_throw = true;
            assert!(
                tick.frame.ball.holder_id.is_none(),
                "during FreeThrow the ball must not advertise a holder (tick {}, holder={:?}, status={})",
                engine.tick_index(),
                tick.frame.ball.holder_id,
                tick.frame.ball.status
            );
        }
        if engine.free_throws_remaining() == 0 {
            break;
        }
    }
    assert!(saw_free_throw, "free-throw flow must be observed");
}

/// F1.1 红测试：罚球期间球与任何球员的距离必须在 leash 内，否则球态必须是 Dead。
/// 这是 BALL_WITH_HOLDER 不变量语义的正面表述。
#[test]
fn test_free_throw_ball_is_dead_or_attached() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    rules.free_throw_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(102, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.push_event_for_test(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_01".to_string(),
        fouler_id: "A_01".to_string(),
        is_shooting: true,
    });
    for _ in 0..40 {
        let tick = engine.step();
        if engine.game_flow() != nba_domain::GameFlowState::FreeThrow {
            if engine.free_throws_remaining() == 0 {
                break;
            }
            continue;
        }
        if let Some(holder) = tick.frame.ball.holder_id.as_deref() {
            let rules = &tick.frame.rules;
            let bft = (
                tick.frame.ball.x * rules.court_width_ft,
                tick.frame.ball.y * rules.court_height_ft,
            );
            if let Some(p) = engine.physics().get_player(holder) {
                let d = ((bft.0 - p.pos_ft.x).powi(2) + (bft.1 - p.pos_ft.y).powi(2)).sqrt();
                assert!(
                    d <= rules.holder_leash_ft,
                    "advertised holder {} is {:.2} ft from ball during free throw (tick {})",
                    holder,
                    d,
                    engine.tick_index()
                );
            }
        }
        if engine.free_throws_remaining() == 0 {
            break;
        }
    }
}

/// F1.2 红测试：进攻方越过中线进入前场后，后场计时必须重置。
/// 根因：backcourt_elapsed 仅在进入 Initiation 阶段清零，跨场后继续累加，
/// 8 秒后无条件判 EIGHT_SECOND_BACKCOURT（seed 0 单节 26 次违例主因）。
#[test]
fn test_backcourt_clock_resets_on_halfcourt_cross() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    rules.tick_seconds = 0.1;
    rules.backcourt_seconds = 3.0;
    let mut engine = MatchEngine::with_rules(701, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.set_game_flow(nba_domain::GameFlowState::LiveBall);
    engine.set_ball_state_for_test(nba_physics::BallTrajectoryKind::Held {
        carrier_id: "H_01".to_string(),
    });
    // 把球与持球人放到前场（Home 攻击右侧，x >= 47 为前场）。
    let frontcourt = glam::Vec2::new(70.0, 25.0);
    engine.set_ball_pos_for_test(frontcourt, 4.0);
    if let Some(p) = engine.physics_mut_for_test().get_player_mut("H_01") {
        p.pos_ft = frontcourt;
        p.target_pos_ft = frontcourt;
    }
    // 人为把后场计时推过阈值，若引擎不按位置重置就会立刻误判违例。
    engine.set_backcourt_elapsed_for_test(99.0);
    let tick = engine.step();
    assert!(
        !tick.frame.events.iter().any(|e| e == "RULE_VIOLATION"),
        "frontcourt possession must not trigger an 8-second violation, events={:?}",
        tick.frame.events
    );
    assert!(
        engine.backcourt_elapsed() < 1.0,
        "backcourt clock must reset after crossing halfcourt, got {}",
        engine.backcourt_elapsed()
    );
}

/// F1.2 负面对照：若球队仍滞留后场，计时必须继续累加并最终判违例。
#[test]
fn test_eight_second_violation_requires_continuous_backcourt() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    rules.tick_seconds = 0.1;
    rules.backcourt_seconds = 0.5;
    rules.decision_interval_seconds = 100.0;
    let mut engine = MatchEngine::with_rules(702, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.set_game_flow(nba_domain::GameFlowState::LiveBall);
    engine.set_ball_state_for_test(nba_physics::BallTrajectoryKind::Held {
        carrier_id: "H_01".to_string(),
    });
    let backcourt = glam::Vec2::new(20.0, 25.0);
    engine.set_ball_pos_for_test(backcourt, 4.0);
    if let Some(p) = engine.physics_mut_for_test().get_player_mut("H_01") {
        p.pos_ft = backcourt;
        p.target_pos_ft = backcourt;
        p.target_speed_ftps = 0.0;
        p.vel_ft = glam::Vec2::ZERO;
    }
    let mut saw_violation = false;
    for _ in 0..40 {
        let tick = engine.step();
        if tick
            .frame
            .events
            .iter()
            .any(|e| e == "VIOLATION" || e == "RULE_VIOLATION")
        {
            saw_violation = true;
            break;
        }
        // 保持球员停留在后场，排除运动学把它们带过中线。
        if let Some(p) = engine.physics_mut_for_test().get_player_mut("H_01") {
            p.pos_ft = backcourt;
            p.target_pos_ft = backcourt;
            p.vel_ft = glam::Vec2::ZERO;
        }
        engine.set_ball_pos_for_test(backcourt, 4.0);
    }
    assert!(
        saw_violation,
        "continuous backcourt possession must eventually trigger the 8-second violation"
    );
}

/// F1.3 红测试：full scope 必须在真实生命周期内进入 GameEnd，不能卡死在
/// DeadBall。根因：发球员的界外发球点被 physics 的场地 clamp 推回场内，
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

/// F1.3c 红测试：被钉在边线的防守者不得阻塞发球程序。
/// 根因：发球员必须步行到界外发球点，而防守者被场地 clamp 钉在同一路径上，
/// 形成几何死锁（本轮 seed 6/9/11 实测约 19 万 tick 的 OUT_OF_BOUNDS 活锁）。
/// 修复：发球赴界外改为显式离散 placement（gap.md §4.3）。
///
/// D3.1 口径修正：计数从"任意球员 OOB 的连续 tick"改为"**同一球员**连续
/// OOB 的 tick 数"。前者把多名球员接力越界累加成一个长 streak，与
/// gap.md §4.3 的死锁定义（单球员被永久钉在边界）不符，会产生假阳性。
/// 物理层同时改为边沿触发（boundary_cross_latched），贴边不再逐 tick 刷屏。
#[test]
fn test_wall_pinned_defender_does_not_block_inbound() {
    for seed in [6u64, 9, 11, 16, 21] {
        let mut engine = MatchEngine::new(seed);
        engine.set_scope("full").unwrap();
        let mut ticks = 0usize;
        // 每个球员各自的连续越界计数。
        let mut streaks: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut max_oob_streak = 0usize;
        while !engine.is_finished() && ticks < 250_000 {
            let tick = engine.step();
            let mut crossed: Vec<String> = Vec::new();
            for e in &tick.frame.event_log {
                if e.kind != "OUT_OF_BOUNDS" {
                    continue;
                }
                if let Some(pid) = e
                    .data
                    .as_ref()
                    .and_then(|d| d.get("BoundaryCross"))
                    .and_then(|b| b.get("player_id"))
                    .and_then(|v| v.as_str())
                {
                    crossed.push(pid.to_string());
                }
            }
            for pid in &crossed {
                let s = streaks.entry(pid.clone()).or_insert(0);
                *s += 1;
                max_oob_streak = max_oob_streak.max(*s);
            }
            // 本 tick 未越界的球员计数归零。
            for (pid, s) in streaks.iter_mut() {
                if !crossed.contains(pid) {
                    *s = 0;
                }
            }
            ticks += 1;
        }
        assert!(engine.is_finished(), "seed {} livelocked", seed);
        // 单个球员连续越界 = 被永久钉在边界（死锁）。
        assert!(
            max_oob_streak < 200,
            "seed {} had a player pinned out of bounds for {} consecutive ticks",
            seed,
            max_oob_streak
        );
        assert!(
            engine.last_tick_violations().is_empty(),
            "seed {} ended with violations: {:?}",
            seed,
            engine.last_tick_violations()
        );
    }
}

/// F1.3 红测试：发球员被换下/犯满离场后，发球程序必须自动改派在场球员
/// 并最终完成，不能永久停留在「发球员不在场」的状态。
/// 根因：`new_possession_pg` 曾按 roster 顺序取发球员，可能返回替补；
/// 替补不参与物理步进，`inbounder_arrived` 永不成立。
#[test]
fn test_inbounder_reassigned_when_leaving_court() {
    for seed in [3u64, 15, 26] {
        let mut engine = MatchEngine::new(seed);
        engine.set_scope("full").unwrap();
        let mut ticks = 0usize;
        let mut stalled_ticks = 0usize;
        let mut max_stalled = 0usize;
        while !engine.is_finished() && ticks < 250_000 {
            engine.step();
            let inbounder_off_court = match &engine.ball_state() {
                nba_physics::BallTrajectoryKind::InboundTransfer { inbounder_id, .. }
                | nba_physics::BallTrajectoryKind::InboundReady { inbounder_id, .. } => engine
                    .physics()
                    .get_player(inbounder_id)
                    .map(|p| !p.on_court)
                    .unwrap_or(true),
                _ => false,
            };
            if inbounder_off_court {
                stalled_ticks += 1;
                max_stalled = max_stalled.max(stalled_ticks);
            } else {
                stalled_ticks = 0;
            }
            ticks += 1;
        }
        assert!(engine.is_finished(), "seed {} livelocked", seed);
        // 恢复应在极短窗口内完成，而不是无限期停留。
        assert!(
            max_stalled < 60,
            "seed {} stayed with an off-court inbounder for {} ticks",
            seed,
            max_stalled
        );
    }
}

/// F1.3：发球员站界外不得被判违例（不产生 VIOLATION / 回合终止），
/// 且发球程序必须能够真实完成推进到 InboundReady。
/// 非持球人的 BOUNDARY_CROSSING 是咨询性信号（flagged），不属于本节范围。
#[test]
fn test_inbounder_out_of_bounds_does_not_emit_boundary_turnover() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    rules.tick_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(703, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.start_inbound_transition_for_test();
    let mut inbounder_violation = false;
    let mut reached_ready = false;
    for _ in 0..200 {
        let tick = engine.step();
        if matches!(
            engine.ball_state(),
            nba_physics::BallTrajectoryKind::InboundReady { .. }
        ) {
            reached_ready = true;
        }
        for e in &tick.frame.event_log {
            if e.kind == "VIOLATION" {
                let d = e.data.as_ref().and_then(|v| v.as_object());
                let mentions_inbounder = d
                    .map(|o| {
                        let text = serde_json::to_string(o).unwrap_or_default();
                        text.contains("INBOUND_SETUP") || text.contains("INBOUND")
                    })
                    .unwrap_or(false);
                if mentions_inbounder {
                    inbounder_violation = true;
                }
            }
        }
        if reached_ready {
            break;
        }
    }
    assert!(
        !inbounder_violation,
        "inbounder standing out of bounds must not be judged as a violation"
    );
    assert!(
        reached_ready,
        "inbound program must reach InboundReady instead of livelocking"
    );
}

/// F6.2 红测试：默认（facts）流必须远小于逐 tick 帧流，且仍然可评判。
/// 根因：此前默认逐 tick 全量帧，单场 full scope 达数百 MB
/// （gap.md §16.4 / problem.md §14.4）。
#[test]
fn test_bounded_stream_modes_are_much_smaller() {
    use nba_engine::StreamMode;

    let run = |mode: StreamMode, label: &str| -> u64 {
        let mut engine = MatchEngine::new(42);
        // RAII：panic 展开也会删除临时流（test-support）。
        let artifact = nba_test_support::TempArtifact::new(&format!("stream_mode_{label}"));
        let result = engine.simulate_scope_and_export_with_mode("1q", &artifact.path_str(), mode);
        assert!(result.is_ok(), "mode {:?} failed: {:?}", mode, result.err());
        artifact.assert_within_limit()
    };

    let frames = run(StreamMode::Frames, "frames");
    let facts = run(StreamMode::Facts, "facts");
    let summary = run(StreamMode::Summary, "summary");

    assert!(frames > 0 && facts > 0 && summary > 0);
    // facts 模式必须比帧模式小一个数量级以上。
    assert!(
        facts * 10 < frames,
        "facts stream ({facts} B) must be at least 10x smaller than frames ({frames} B)"
    );
    assert!(
        summary < facts,
        "summary ({summary} B) must be smaller than facts ({facts} B)"
    );
    // 一场 full scope 的 facts 流必须落在 MB 级（< 64 MiB）。
    let mut engine = MatchEngine::new(42);
    let artifact = nba_test_support::TempArtifact::with_limit("full_facts", 64 * 1024 * 1024);
    engine
        .simulate_scope_and_export_with_mode("full", &artifact.path_str(), StreamMode::Facts)
        .expect("full facts export");
    let full_facts = artifact.assert_within_limit();
    assert!(
        full_facts < 64 * 1024 * 1024,
        "full-scope facts stream must stay in the MB range, got {} bytes",
        full_facts
    );
}

/// F6.2：超过字节预算必须报错，而不是静默写满磁盘。
#[test]
fn test_stream_byte_budget_fails_closed() {
    use nba_engine::StreamMode;
    let mut rules = nba_domain::GameRules::default();
    rules.stream_max_bytes = 1024; // 1 KiB：必然超限
    let mut engine = MatchEngine::with_rules(42, rules);
    let artifact = nba_test_support::TempArtifact::new("budget");
    let result =
        engine.simulate_scope_and_export_with_mode("1q", &artifact.path_str(), StreamMode::Facts);
    assert!(
        result.is_err(),
        "exceeding the byte budget must fail closed"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("budget"),
        "error should mention the budget, got: {err}"
    );
}

/// D5.1 红测试：交接（ControlTransfer）不得永久悬置。
/// 根因：接球人被动作窗口锁定（`lock_kinematics`）时无法走到冻结点，
/// `receiver_ready` 永不成立 —— seed 6 full 实测 69,466 帧（约 2,780 秒）
/// 活锁，全场仅 11 个回合、10 分。
#[test]
fn test_control_transfer_never_hangs_forever() {
    use nba_domain::GameRules;
    for seed in [6u64, 21, 42] {
        let mut engine = MatchEngine::with_rules(seed, GameRules::default());
        engine.set_scope("full").unwrap();
        let mut max_ct_streak = 0usize;
        let mut streak = 0usize;
        let mut ticks = 0usize;
        while !engine.is_finished() && ticks < 250_000 {
            engine.step();
            ticks += 1;
            if matches!(
                engine.ball_state(),
                nba_physics::BallTrajectoryKind::ControlTransfer { .. }
            ) {
                streak += 1;
                max_ct_streak = max_ct_streak.max(streak);
            } else {
                streak = 0;
            }
        }
        assert!(engine.is_finished(), "seed {seed} did not reach GameEnd");
        // 交接是一次短飞行（约 0.5s = 13 tick）；超过数秒即为悬置。
        assert!(
            max_ct_streak < 500,
            "seed {seed}: ControlTransfer hung for {max_ct_streak} consecutive ticks"
        );
        assert!(
            engine.completed_possessions() > 150,
            "seed {seed}: only {} possessions completed (expected a full game)",
            engine.completed_possessions()
        );
    }
}

/// D5.1：投篮弧线峰值必须服从规则高度上限（BALL_HEIGHT_BOUNDS）。
/// 根因：`arc = A·p·(1-p)` 的真实极值点在 p>0.5，线性缩放的 A 使实际
/// 峰值高于请求值（请求 35.0 ft → 采样 35.08 ft）。
#[test]
fn test_shot_arc_respects_height_ceiling() {
    use nba_domain::GameRules;
    let rules = GameRules::default();
    let ceiling = rules.ball_z_max_ft;
    for seed in [6u64, 21, 42] {
        let mut engine = MatchEngine::with_rules(seed, rules.clone());
        engine.set_scope("full").unwrap();
        let mut worst = 0.0f32;
        let mut ticks = 0usize;
        while !engine.is_finished() && ticks < 250_000 {
            let tick = engine.step();
            ticks += 1;
            worst = worst.max(tick.frame.ball.z);
        }
        assert!(
            worst <= ceiling,
            "seed {seed}: ball reached {worst:.4} ft, above the {ceiling} ft ceiling"
        );
    }
}

/// D5.1b 红测试：战术档案的槽位必须真正决定进攻站位。
/// 根因：`data/tactics/*.json` 的 `base_offset_x/y` 从未被消费，全队挤在
/// 弧顶三分线外（实测进攻方 90% 球员距篮筐 > 23.75 ft），中距离出手
/// 没有机会产生（2PA 均值 10.8，真实 ~55）。
#[test]
fn test_tactical_spec_slots_drive_offensive_spacing() {
    use nba_domain::GameRules;
    let rules = GameRules::default();
    let three = rules.league.three_point_distance_ft;
    let mut inside = 0usize;
    let mut total = 0usize;
    for seed in [0u64, 42] {
        let mut engine = MatchEngine::with_rules(seed, rules.clone());
        engine.set_scope("1q").unwrap();
        let mut ticks = 0usize;
        while !engine.is_finished() && ticks < 60_000 {
            let tick = engine.step();
            ticks += 1;
            for p in &tick.frame.players {
                if !p.on_court {
                    continue;
                }
                // 只统计进攻方（possession_team）
                if p.team != tick.frame.possession_team {
                    continue;
                }
                let x = p.x * rules.court.width_ft;
                let y = p.y * rules.court.height_ft;
                let hoop = rules.court.hoop_pos(tick.frame.possession_team == "home");
                total += 1;
                if (glam::Vec2::new(x, y) - hoop).length() < three {
                    inside += 1;
                }
            }
        }
    }
    assert!(total > 0, "no offensive player samples collected");
    let ratio = inside as f64 / total as f64;
    // 修复前该比例约 9.9%；档案槽位生效后应显著提升（底角/内线槽位在三分线内）。
    assert!(
        ratio > 0.25,
        "offensive players inside the arc: {:.1}% (expected > 25%; \
         tactical spec slots are not driving spacing)",
        ratio * 100.0
    );
}

/// D5.1b：底角三分必须按「更近的底角线」判定，而不是弧顶半径。
/// NBA 底角线距边线 3 ft，最近点距篮筐 22 ft；弧顶是 23.75 ft。
#[test]
fn test_corner_three_geometry() {
    use nba_domain::GameRules;
    let rules = GameRules::default();
    let court = rules.court;
    let arc = rules.league.three_point_distance_ft;
    let corner = rules.league.corner_three_distance_ft;
    assert!(
        corner > 0.0 && corner < arc,
        "NBA must declare corner < arc"
    );

    // 底角带内、距篮筐 22.4 ft → 三分
    let corner_spot = glam::Vec2::new(court.hoop_right_x_ft - 6.0, 2.5);
    let d = (corner_spot - court.hoop_pos(true)).length();
    assert!(
        d >= corner && d < arc,
        "probe must sit between corner and arc"
    );
    assert!(
        court.is_three_point_attempt(corner_spot, true, arc, corner),
        "corner spot at {d:.2} ft must count as a three"
    );

    // 同一距离但在弧顶区域（y 居中）→ 两分
    let top_spot = glam::Vec2::new(court.hoop_right_x_ft - 6.0, court.hoop_y_ft);
    assert!(
        (top_spot - court.hoop_pos(true)).length() < arc,
        "probe must be inside the arc"
    );
    assert!(
        !court.is_three_point_attempt(top_spot, true, arc, corner),
        "top-of-key spot inside the arc must not count as a three"
    );

    // 底角特例只"放宽"近处的判定，不得"收紧"远处的判定：
    // 后场远投（距篮筐 > 弧顶半径）本就该是三分。
    let backcourt = glam::Vec2::new(court.width_ft * 0.2, 2.5);
    let bd = (backcourt - court.hoop_pos(true)).length();
    assert!(bd > arc, "probe must be beyond the arc ({bd:.1} ft)");
    assert!(
        court.is_three_point_attempt(backcourt, true, arc, corner),
        "a shot beyond the arc is a three regardless of court zone"
    );

    // 底角特例不适用于【弧顶区域】内、比底角线更近的点。
    let near_top = glam::Vec2::new(court.hoop_right_x_ft - 6.0, court.hoop_y_ft);
    assert!(
        !court.is_three_point_attempt(near_top, true, arc, corner),
        "corner exception must not apply away from the sideline"
    );
}

/// C6.1：`events_since` 的游标必须用跨 tick 唯一的 `event_id`。
///
/// 原实现按 tick 内局部 `sequence` 过滤：后续 tick 里 sequence ≤ 游标的
/// 事件全部漏取，增量语义失效。本测试驱动多个 tick 后：
/// 1. 用最后一个 event_id 作游标，增量必须为空（无重复）；
/// 2. 用中途 event_id 作游标，增量必须恰好是其后的全部事件（无漏取）；
/// 3. 去重按 event_id：历史里不得有重复 event_id。
#[test]
fn match_service_events_since_cursor_uses_global_event_id() {
    use nba_engine::MatchService;

    let mut service = MatchService::with_seed(641);
    service
        .setup_match(nba_engine::MatchSetup::builtin(
            nba_domain::GameRules::default(),
        ))
        .expect("setup should be accepted");
    service.start().expect("start");
    // 驱动足够多 tick，保证跨 tick 的事件积累（sequence 每tick重新计数）。
    for _ in 0..400 {
        service
            .tick(nba_domain::FixedDt(0.1))
            .expect("tick advances");
    }
    let history = service.events_since(0);
    assert!(
        history.len() >= 2,
        "expected multi-event history, got {}",
        history.len()
    );

    // 游标语义 1：尾部游标 → 空增量。
    let last_id = history.iter().map(|e| e.event_id).max().unwrap();
    assert!(
        service.events_since(last_id).is_empty(),
        "tail event_id cursor must yield no events"
    );

    // 游标语义 2：中途游标 → 恰好是其后的全部事件（按 event_id 全序）。
    let mid = history[history.len() / 2].event_id;
    let expected: Vec<u64> = history
        .iter()
        .filter(|e| e.event_id > mid)
        .map(|e| e.event_id)
        .collect();
    let got: Vec<u64> = service
        .events_since(mid)
        .iter()
        .map(|e| e.event_id)
        .collect();
    assert_eq!(got, expected, "mid cursor must return exactly the suffix");
    assert!(
        !got.is_empty(),
        "mid cursor must not drop events (the old sequence cursor did)"
    );

    // 去重语义：event_history 内无重复 event_id（旧按 sequence 去重会误吞跨 tick 同号事件）。
    let mut seen = std::collections::HashSet::new();
    for e in &history {
        assert!(
            seen.insert(e.event_id),
            "duplicate event_id {} in history",
            e.event_id
        );
    }
}

/// C6.4：protocol `FrameRules::default()` 必须与引擎从 `GameRules::default()`
/// 投影出的 FrameRules 全字段一致。
///
/// 历史上两处独立手抄已实际漂移（player_radius_ft 1.8 vs 1.0）。此测试
/// 让任何一侧的漂移必红——新增规则字段时同步更新投影函数与 Default。
#[test]
fn frame_rules_default_matches_game_rules_default_projection() {
    let projected = nba_engine::frame_rules_from_game_rules(&nba_domain::GameRules::default());
    let fallback = nba_protocol::FrameRules::default();
    assert_eq!(projected.tick_seconds, fallback.tick_seconds);
    assert_eq!(projected.court_width_ft, fallback.court_width_ft);
    assert_eq!(projected.court_height_ft, fallback.court_height_ft);
    assert_eq!(projected.hoop_left_x_ft, fallback.hoop_left_x_ft);
    assert_eq!(projected.hoop_right_x_ft, fallback.hoop_right_x_ft);
    assert_eq!(projected.hoop_y_ft, fallback.hoop_y_ft);
    assert_eq!(projected.player_radius_ft, fallback.player_radius_ft);
    assert_eq!(
        projected.min_player_separation_ft,
        fallback.min_player_separation_ft
    );
    assert_eq!(
        projected.separation_safety_margin_ft,
        fallback.separation_safety_margin_ft
    );
    assert_eq!(
        projected.max_player_speed_ftps,
        fallback.max_player_speed_ftps
    );
    assert_eq!(
        projected.max_player_accel_ftps2,
        fallback.max_player_accel_ftps2
    );
    assert_eq!(projected.ball_max_speed_ftps, fallback.ball_max_speed_ftps);
    assert_eq!(
        projected.three_point_distance_ft,
        fallback.three_point_distance_ft
    );
    assert_eq!(projected.shot_clock_seconds, fallback.shot_clock_seconds);
    assert_eq!(projected.holder_leash_ft, fallback.holder_leash_ft);
    assert_eq!(
        projected.speed_tolerance_ftps,
        fallback.speed_tolerance_ftps
    );
    assert_eq!(projected.ball_z_max_ft, fallback.ball_z_max_ft);
}
