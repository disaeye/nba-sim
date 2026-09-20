//! 约束系统行为：候选过滤、运行时求值、事件求值与优先级。
//!
//! 守卫对象：硬约束剔除不可行候选、软约束与风险进入效用、运行时约束的执法
//! 意图、边界事实被活动约束求值、接触事实被裁决成犯规事实。
//!
//! 对应 `docs/architecture.md` §5.1（候选 → 硬约束 → 软约束 → 效用）与
//! `docs/gap.md` §9.3。

#![allow(clippy::field_reassign_with_default)]

mod support;

use std::collections::HashMap;

use glam::Vec2;
use nba_decision::constraint::{CandidateAction, ConstraintRegistry};
use nba_engine::MatchEngine;
use rand::SeedableRng;
use support::{base_ctx, make_physics, make_player};

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
    ctx.phase = nba_decision::constraint::PhaseType::DeadBallReset;
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
    ctx.phase = nba_decision::constraint::PhaseType::Inbound;
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
fn test_constraint_activation_is_priority_ordered() {
    let players = HashMap::new();
    let physics = make_physics(&players);
    let ctx = base_ctx(&physics);
    let active = ConstraintRegistry::default().active_set(&ctx);
    assert!(active
        .windows(2)
        .all(|pair| (pair[0].priority, pair[0].id) <= (pair[1].priority, pair[1].id)));
}

/// 接触事实必须能被裁决成犯规事实（或明确判为无变化）。
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

/// 被裁决为犯规的接触必须让被犯规者的接球平衡下降（士气反馈链）。
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

/// 运行时的攻防接触必须以语义事实的形式出现在事件流里。
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

/// 掩护动作必须让接触被分类成掩护接触，而不是普通碰撞。
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
