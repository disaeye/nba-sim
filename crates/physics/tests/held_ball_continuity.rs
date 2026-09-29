use glam::Vec2;
use nba_domain::GameRules;
use nba_physics::{BallTrajectoryKind, BallisticsEngine, PlayerPhysicsState};
use std::collections::HashMap;

#[test]
fn held_ball_position_respects_the_ball_speed_envelope() {
    let rules = GameRules::default();
    let carrier = PlayerPhysicsState {
        id: "H_01".to_string(),
        jersey: "1".to_string(),
        team: "home".to_string(),
        pos_ft: Vec2::new(40.0, 25.0),
        vel_ft: Vec2::new(12.0, 0.0),
        accel_ft: Vec2::ZERO,
        target_pos_ft: Vec2::new(40.0, 25.0),
        target_speed_ftps: 12.0,
        max_speed_ftps: 20.0,
        max_accel_ftps2: 35.0,
        on_court: true,
        action: "Run".to_string(),
        slot: "PG".to_string(),
        morale: "Normal".to_string(),
        stamina: 100.0,
        max_stamina: 100.0,
        foul_count: 0,
        locomotion: nba_physics::LocomotionState::Sprinting,
        facing_dir: Vec2::X,
        ball_orientation: nba_domain::action_window::BallOrientation::FaceUp,
        turn_decel_timer: 0.0,
        is_locked_kinematics: false,
        out_of_bounds_placement: false,
        is_receiving_pass: false,
        is_driving_to_rim: false,
        boundary_cross_latched: false,
        attributes: nba_domain::PlayerAttributes::default(),
        tendencies: nba_domain::PlayerTendencies::default(),
    };
    let players = HashMap::from([(carrier.id.clone(), carrier)]);
    let state = BallTrajectoryKind::Held {
        carrier_id: "H_01".to_string(),
    };
    let mut previous = BallisticsEngine::sample_ball_position(&state, 0.0, &players, &rules);
    for tick in 1..=600 {
        let time = tick as f32 * rules.tick_seconds;
        let current = BallisticsEngine::sample_ball_position(&state, time, &players, &rules);
        let speed = ((current.0 - previous.0).length_squared() + (current.1 - previous.1).powi(2))
            .sqrt()
            / rules.tick_seconds;
        assert!(
            speed <= rules.ball_max_speed_ftps + rules.invariant_speed_tolerance_ftps,
            "held-ball speed {speed:.2} ft/s exceeds limit at t={time:.2}"
        );
        assert!(
            (current.0 - players["H_01"].pos_ft).length() <= rules.invariant_holder_leash_ft,
            "held-ball position exceeded the carrier leash at t={time:.2}"
        );
        assert!(
            current.1 >= rules.ball_holder_height_ft * 0.70
                && current.1 <= rules.ball_holder_height_ft,
            "held-ball height escaped the dribble range at t={time:.2}"
        );
        previous = current;
    }
}
