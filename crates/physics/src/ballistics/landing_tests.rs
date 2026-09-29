use super::*;
use nba_domain::GameRules;

#[cfg(test)]
fn receiver(pos: Vec2, vel: Vec2) -> PlayerPhysicsState {
    PlayerPhysicsState {
        id: "T_1".to_string(),
        jersey: "1".to_string(),
        team: "home".to_string(),
        pos_ft: pos,
        vel_ft: vel,
        accel_ft: Vec2::ZERO,
        target_pos_ft: pos,
        target_speed_ftps: 0.0,
        max_speed_ftps: 20.0,
        max_accel_ftps2: 35.0,
        on_court: true,
        action: "Run".to_string(),
        slot: "S".to_string(),
        morale: "Normal".to_string(),
        stamina: 100.0,
        max_stamina: 100.0,
        foul_count: 0,
        locomotion: crate::movement::LocomotionState::Idle,
        facing_dir: Vec2::X,
        turn_decel_timer: 0.0,
        is_locked_kinematics: false,
        out_of_bounds_placement: false,
        is_receiving_pass: false,
        is_driving_to_rim: false,
        boundary_cross_latched: false,
        attributes: nba_domain::PlayerAttributes::default(),
        tendencies: nba_domain::PlayerTendencies::default(),
    }
}

#[test]
fn solve_landing_is_a_fixed_point() {
    let rules = GameRules::default();
    for (pos, vel) in [
        (Vec2::new(50.0, 25.0), Vec2::new(-13.0, 0.1)),
        (Vec2::new(64.0, 40.0), Vec2::new(-8.0, 6.0)),
        (Vec2::new(30.0, 20.0), Vec2::new(6.0, -4.0)),
    ] {
        let passer = Vec2::new(97.0, 25.0);
        let r = receiver(pos, vel);
        let (landing, t) = BallisticsEngine::solve_pass_landing(passer, &r, &rules);
        let t_from_landing = rules.pass_duration((landing - passer).length(), false);
        assert!(
            (t_from_landing - t).abs() < 1e-3,
            "fixed point must hold: t={t} vs duration(|L-p|)={t_from_landing}"
        );
    }
}

#[test]
fn lead_never_exceeds_braking_reach() {
    let rules = GameRules::default();
    let v0 = 18.0f32;
    let r = receiver(Vec2::new(60.0, 25.0), Vec2::new(-v0, 0.0));
    let passer = Vec2::new(97.0, 25.0);
    let (landing, t) = BallisticsEngine::solve_pass_landing(passer, &r, &rules);
    let led = (landing - r.pos_ft).length();
    let accel = rules.max_player_accel_ftps2;
    let t_brake = v0 / accel;
    let reachable = if t <= t_brake {
        v0 * t - 0.5 * accel * t * t
    } else {
        v0 * t_brake - 0.5 * accel * t_brake * t_brake
    };
    assert!(
        led <= reachable + 1e-3,
        "lead {led:.3} must not exceed braking reach {reachable:.3}"
    );
    assert!(led > 0.0, "a moving receiver must get a positive lead");
}

#[test]
fn stationary_receiver_gets_no_lead() {
    let rules = GameRules::default();
    let r = receiver(Vec2::new(60.0, 25.0), Vec2::ZERO);
    let passer = Vec2::new(97.0, 25.0);
    let (landing, t) = BallisticsEngine::solve_pass_landing(passer, &r, &rules);
    assert!((landing - r.pos_ft).length() < 1e-4);
    assert!((t - rules.pass_duration((r.pos_ft - passer).length(), false)).abs() < 1e-4);
}

#[test]
fn lead_time_tracks_distance_not_a_constant() {
    let rules = GameRules::default();
    let passer = Vec2::new(10.0, 25.0);
    let mut times = Vec::new();
    for d in [10.0f32, 25.0, 40.0, 55.0] {
        let r = receiver(Vec2::new(10.0 + d, 25.0), Vec2::new(-14.0, 0.0));
        let (_landing, t) = BallisticsEngine::solve_pass_landing(passer, &r, &rules);
        times.push(t);
    }
    assert!(
        times.windows(2).all(|w| w[1] >= w[0] - 1e-4),
        "flight time must grow with distance: {times:?}"
    );
    assert!(
        times.last().unwrap() - times.first().unwrap() > 0.3,
        "flight time must vary materially with distance (not a constant): {times:?}"
    );
}

#[test]
fn free_ball_contact_respects_radius_reach_and_priority() {
    let rules = GameRules::default();
    let at = |id: &str, cm: u16, vertical: f32, pos: Vec2, on_court: bool| {
        (id.to_string(), cm, vertical, pos, on_court)
    };
    let nearby = vec![at("H_01", 200, 0.5, Vec2::new(95.0 + 1.2, 25.0), true)];
    let hit =
        BallisticsEngine::free_ball_player_contact(Vec2::new(95.0, 25.0), 3.0, &nearby, &rules);
    assert!(hit.is_some(), "ball inside radius and below reach must hit");
    let hit_high =
        BallisticsEngine::free_ball_player_contact(Vec2::new(95.0, 25.0), 4.0, &nearby, &rules);
    assert!(
        hit_high.is_none(),
        "ball above the reach ceiling must pass over"
    );
    let hit_far = BallisticsEngine::free_ball_player_contact(
        Vec2::new(95.0, 25.0),
        3.0,
        &[at("H_01", 200, 0.5, Vec2::new(95.0 + 2.0, 25.0), true)],
        &rules,
    );
    assert!(
        hit_far.is_none(),
        "ball outside the horizontal radius must miss"
    );
    let two = vec![
        at("H_02", 200, 0.5, Vec2::new(95.0 + 0.8, 25.0), true),
        at("H_01", 200, 0.5, Vec2::new(95.0 + 1.2, 25.0), true),
    ];
    let (nearest, _) =
        BallisticsEngine::free_ball_player_contact(Vec2::new(95.0, 25.0), 3.0, &two, &rules)
            .expect("two candidates must hit");
    assert_eq!(nearest, "H_02", "closer candidate must win");
    let tied = vec![
        at("H_02", 200, 0.5, Vec2::new(95.0 - 1.2, 25.0), true),
        at("H_01", 200, 0.5, Vec2::new(95.0 + 1.2, 25.0), true),
    ];
    let (tied_id, _) =
        BallisticsEngine::free_ball_player_contact(Vec2::new(95.0, 25.0), 3.0, &tied, &rules)
            .expect("tied candidates must hit");
    assert_eq!(tied_id, "H_01", "equal distance must tie-break by id order");
    let bench = vec![at("H_01", 200, 0.5, Vec2::new(95.0, 25.0), false)];
    let hit_bench =
        BallisticsEngine::free_ball_player_contact(Vec2::new(95.0, 25.0), 3.0, &bench, &rules);
    assert!(
        hit_bench.is_none(),
        "bench players must not collide with the ball"
    );
}
