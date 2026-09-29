use std::collections::HashMap;

use glam::Vec2;
use nba_decision::constraint::ConstraintContext;
use nba_decision::modulation::CoachStrategy;
use nba_decision::pipeline::{DecisionSystem, OnBallDecisionContext};
use nba_decision::play_executor::evaluate_active_play;
use nba_decision::play_selector::PlaySelectionContext;
use nba_domain::play::{
    DecisionActionFamily, PlayCarrierPreference, PlayInhibition, PlayInhibitionMode, PlayKind,
    PlayPredicate, PlaySpec, PlayTrigger,
};
use nba_domain::{
    BallPhase, GameFlowState, GameRules, PhaseType, PlayerAttributes, PlayerTendencies, TeamTraits,
};
use nba_physics::movement::{LocomotionState, PlayerPhysicsState};
use nba_physics::{PhysicsBackend, PhysicsWorld};
use rand::rngs::StdRng;
use rand::SeedableRng;

fn play_effects() -> (PlaySpec, PlaySelectionContext) {
    let spec = PlaySpec {
        schema_version: 1,
        id: "decision_effects".to_string(),
        name_zh: "决策效果测试".to_string(),
        kind: PlayKind::Play,
        carrier_preferences: vec![PlayCarrierPreference {
            action_family: DecisionActionFamily::Drive,
            bonus: f32::from(1u8),
        }],
        inhibitions: vec![PlayInhibition {
            action_family: DecisionActionFamily::Shoot,
            mode: PlayInhibitionMode::Soft {
                penalty: f32::from(1u8),
            },
        }],
        rules: Vec::new(),
        triggers: vec![PlayTrigger {
            when: vec![PlayPredicate::CarrierPossed],
            window_seconds: f32::from(6u8),
            cooldown_seconds: f32::from(10u8),
        }],
    };
    let selection_context = PlaySelectionContext {
        possession_ticks: 1,
        shot_clock_seconds: f32::from(14u8),
        tick_seconds: f32::from(1u8),
        carrier_possed: true,
        halfcourt: true,
        carrier_dist_to_hoop_ft: f32::from(20u8),
        screen_established: false,
        corner_left_occupied: false,
        corner_right_occupied: false,
        help_shading_off_stable: false,
        weak_side_vacated_stable: false,
    };
    (spec, selection_context)
}

fn player_state(rules: &GameRules) -> PlayerPhysicsState {
    let pos_ft = Vec2::new(
        rules.court.width_ft / f32::from(2u8),
        rules.court.height_ft / f32::from(2u8),
    );
    PlayerPhysicsState {
        id: "carrier".to_string(),
        jersey: "carrier".to_string(),
        team: "home".to_string(),
        pos_ft,
        vel_ft: Vec2::ZERO,
        accel_ft: Vec2::ZERO,
        target_pos_ft: pos_ft,
        target_speed_ftps: f32::from(0u8),
        max_speed_ftps: rules.max_player_speed_ftps,
        max_accel_ftps2: rules.max_player_accel_ftps2,
        on_court: true,
        action: "Idle".to_string(),
        slot: "PG".to_string(),
        morale: "Normal".to_string(),
        stamina: f32::from(100u8),
        max_stamina: f32::from(100u8),
        foul_count: 0,
        locomotion: LocomotionState::Idle,
        facing_dir: Vec2::X,
        ball_orientation: nba_domain::action_window::BallOrientation::FaceUp,
        turn_decel_timer: f32::from(0u8),
        is_locked_kinematics: false,
        out_of_bounds_placement: false,
        is_receiving_pass: false,
        is_driving_to_rim: false,
        boundary_cross_latched: false,
        attributes: PlayerAttributes::default(),
        tendencies: PlayerTendencies::default(),
    }
}

fn decision_probabilities(
    effect_weight: f32,
    spec: &PlaySpec,
    play_context: &PlaySelectionContext,
) -> (f32, f32) {
    let mut rules = GameRules::default();
    rules.decision.play_effect_weight = effect_weight;
    let mut physics = PhysicsWorld::with_backend(&rules, PhysicsBackend::SimpleCircle);
    physics.register_player(player_state(&rules));
    let team_traits = HashMap::from([
        ("home".to_string(), TeamTraits::default()),
        ("away".to_string(), TeamTraits::default()),
    ]);
    let context = ConstraintContext {
        physics: &physics,
        ball_pos: Vec2::new(
            rules.court.width_ft / f32::from(2u8),
            rules.court.height_ft / f32::from(2u8),
        ),
        possession_team: "home",
        shot_clock: rules.league.shot_clock_seconds,
        game_clock: rules.league.period_duration_seconds,
        phase: PhaseType::SetPlay,
        game_flow: GameFlowState::LiveBall,
        ball_phase: BallPhase::Held,
        ball_holder_id: Some("carrier"),
        inbound_elapsed: f32::from(0u8),
        backcourt_elapsed: f32::from(0u8),
        rules: &rules,
        team_traits: &team_traits,
        possession_had_shot: false,
        putback_rebounder_id: None,
        possession_elapsed_seconds: f32::from(0u8),
        lane_dwell_seconds: f32::from(0u8),
        lane_dwell_player_id: None,
        frontcourt_established: false,
        last_touch_player_id: None,
        pass_target_pos: None,
    };
    let play = evaluate_active_play(spec, play_context);
    let decision = OnBallDecisionContext {
        constraint_context: &context,
        carrier_id: "carrier",
        stamina: f32::from(1u8),
        morale_bias: f32::from(0u8),
        coach: &CoachStrategy::default(),
        active_play: Some(&play),
    };
    let mut rng = StdRng::seed_from_u64(42);
    let output = DecisionSystem::with_weights(rules.decision.clone())
        .decide_on_ball_with_play(&decision, &mut rng)
        .expect("live-ball decision must have feasible candidates");
    let probability = |prefix: &str| {
        output
            .trace
            .probabilities
            .iter()
            .find(|(label, _)| label.starts_with(prefix))
            .map(|(_, value)| *value)
            .expect("decision trace must include requested action family")
    };
    (probability("DRIVE("), probability("SHOOT("))
}

#[test]
fn putback_rebounder_has_no_non_shooting_dwell_candidate() {
    let rules = GameRules::default();
    let mut player = player_state(&rules);
    player.pos_ft = rules.court.hoop_pos(true);
    player.target_pos_ft = player.pos_ft;
    let mut physics = PhysicsWorld::with_backend(&rules, PhysicsBackend::SimpleCircle);
    physics.register_player(player);
    let team_traits = HashMap::from([
        ("home".to_string(), TeamTraits::default()),
        ("away".to_string(), TeamTraits::default()),
    ]);
    let context = ConstraintContext {
        physics: &physics,
        ball_pos: rules.court.hoop_pos(true),
        possession_team: "home",
        shot_clock: rules.league.offensive_rebound_shot_clock_seconds,
        game_clock: rules.league.period_duration_seconds,
        phase: PhaseType::SetPlay,
        game_flow: GameFlowState::LiveBall,
        ball_phase: BallPhase::Held,
        ball_holder_id: Some("carrier"),
        inbound_elapsed: 0.0,
        backcourt_elapsed: 0.0,
        rules: &rules,
        team_traits: &team_traits,
        possession_had_shot: true,
        putback_rebounder_id: Some("carrier"),
        possession_elapsed_seconds: 5.0,
        lane_dwell_seconds: f32::from(0u8),
        lane_dwell_player_id: None,
        frontcourt_established: false,
        last_touch_player_id: None,
        pass_target_pos: None,
    };
    let decision = OnBallDecisionContext {
        constraint_context: &context,
        carrier_id: "carrier",
        stamina: 1.0,
        morale_bias: 0.0,
        coach: &CoachStrategy::default(),
        active_play: None,
    };
    let mut rng = StdRng::seed_from_u64(9);
    let output = DecisionSystem::new()
        .decide_on_ball_with_play(&decision, &mut rng)
        .expect("putback decision must retain a feasible shot candidate");
    assert!(matches!(
        output.action,
        nba_decision::constraint::CandidateAction::Shoot { .. }
    ));
    assert!(output.trace.probabilities.iter().all(|(label, _)| {
        !label.starts_with("DWELL(")
            && !label.starts_with("DRIVE(")
            && !label.starts_with("PASS(")
            && !label.starts_with("TRIPLE_THREAT_JAB(")
            && !label.starts_with("POST_UP(")
    }));
}

#[test]
fn play_effect_weight_scales_preferences_and_soft_inhibitions() {
    let (spec, play_context) = play_effects();
    let without_effects = decision_probabilities(f32::from(0u8), &spec, &play_context);
    let with_effects = decision_probabilities(f32::from(1u8), &spec, &play_context);

    assert!(
        with_effects.0 > without_effects.0,
        "Drive preference must increase Drive probability when its rule weight increases"
    );
    assert!(
        with_effects.1 < without_effects.1,
        "Shoot soft inhibition must decrease Shoot probability when its rule weight increases"
    );
}
