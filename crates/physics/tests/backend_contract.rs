use glam::Vec2;
use nba_domain::GameRules;
use nba_physics::{
    EntityFilter, LocomotionState, PhysicsBackend, PhysicsWorld, PlayerPhysicsState,
};
use rand::SeedableRng;

fn player(id: &str, team: &str, pos: Vec2) -> PlayerPhysicsState {
    PlayerPhysicsState {
        id: id.to_string(),
        jersey: id.to_string(),
        team: team.to_string(),
        pos_ft: pos,
        vel_ft: Vec2::ZERO,
        accel_ft: Vec2::ZERO,
        target_pos_ft: pos,
        target_speed_ftps: 0.0,
        max_speed_ftps: 22.0,
        max_accel_ftps2: 35.0,
        has_ball: false,
        on_court: true,
        action: "Idle".to_string(),
        slot: "PG".to_string(),
        morale: "Normal".to_string(),
        stamina: 100.0,
        max_stamina: 100.0,
        foul_count: 0,
        locomotion: LocomotionState::Idle,
        facing_dir: Vec2::X,
        ball_orientation: nba_domain::action_window::BallOrientation::FaceUp,
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

#[test]
fn spatial_backends_share_query_contract() {
    let rules = GameRules::default();
    for backend in [PhysicsBackend::Rapier, PhysicsBackend::SimpleCircle] {
        let mut world = PhysicsWorld::with_backend(&rules, backend);
        world.register_player(player("H_01", "home", Vec2::new(20.0, 25.0)));
        world.register_player(player("A_01", "away", Vec2::new(24.0, 25.0)));
        world.register_player(player("A_02", "away", Vec2::new(40.0, 25.0)));

        assert_eq!(world.backend(), backend);
        assert_eq!(
            world.query_nearby(
                Vec2::new(20.0, 25.0),
                5.0,
                &EntityFilter::OpposingTeam("home".to_string()),
            ),
            vec!["A_01".to_string()]
        );
        let hit = world
            .cast_capsule(
                Vec2::new(15.0, 25.0),
                Vec2::new(30.0, 25.0),
                0.0,
                &EntityFilter::OpposingTeam("home".to_string()),
            )
            .expect("capsule should hit A_1");
        assert_eq!(hit.entity_id, "A_01");
        let ray = world
            .raycast(
                Vec2::new(15.0, 25.0),
                Vec2::X,
                20.0,
                &EntityFilter::Team("away".to_string()),
            )
            .expect("ray should hit A_1");
        assert_eq!(ray.entity_id, "A_01");
    }
}

#[test]
fn kinematic_step_never_exceeds_configured_speed() {
    let rules = GameRules {
        max_player_speed_ftps: 10.0,
        max_player_accel_ftps2: 20.0,
        ..GameRules::default()
    };
    let mut world = PhysicsWorld::with_backend(&rules, PhysicsBackend::SimpleCircle);
    world.register_player(player("H_01", "home", Vec2::new(20.0, 25.0)));
    world.set_player_target(
        "H_01",
        Vec2::new(80.0, 25.0),
        100.0,
        "SPRINT",
        "Wing",
        "Normal",
    );
    for _ in 0..100 {
        let previous = world.get_player("H_01").unwrap().pos_ft;
        world.step(nba_domain::FixedDt(rules.tick_seconds));
        let current = world.get_player("H_01").unwrap();
        let speed = (current.pos_ft - previous).length() / rules.tick_seconds;
        assert!(
            speed <= rules.max_player_speed_ftps + 1e-4,
            "speed {speed} exceeded {}",
            rules.max_player_speed_ftps
        );
    }
}

#[test]
fn inbound_transfer_duration_uses_inbound_speed_policy() {
    let rules = GameRules::default();
    let from = Vec2::new(88.75, 25.0);
    let baseline = Vec2::new(0.0, 25.0);
    let state = nba_physics::BallTrajectoryKind::InboundTransfer {
        from_pos: from,
        from_z: rules.rim_height_ft,
        baseline_pos: baseline,
        inbounder_id: "A_01".to_string(),
        start_time: 0.0,
        duration: rules.pass_duration((baseline - from).length(), true),
    };
    let players = std::collections::HashMap::new();
    let at_start =
        nba_physics::BallisticsEngine::sample_ball_position(&state, 0.0, &players, &rules);
    let at_end = nba_physics::BallisticsEngine::sample_ball_position(
        &state,
        rules.pass_duration((baseline - from).length(), true),
        &players,
        &rules,
    );
    assert_eq!(at_start, (from, rules.rim_height_ft));
    assert_eq!(at_end, (baseline, rules.chest_height_ft));
    assert!(
        rules.pass_duration((baseline - from).length(), true) >= rules.min_pass_duration_seconds
    );
}

#[test]
fn shot_samples_stay_within_configured_speed_envelope() {
    let rules = GameRules::default();
    let from = Vec2::new(15.4, 25.0);
    let hoop = Vec2::new(88.75, 25.0);
    let peak_z = rules.shot_peak_base_ft + (hoop - from).length() * rules.shot_peak_distance_factor;
    let duration =
        nba_physics::BallisticsEngine::shot_duration((hoop - from).length(), peak_z, &rules);
    let state = nba_physics::BallTrajectoryKind::Shot {
        shooter_id: "H_01".to_string(),
        from_pos: from,
        hoop_pos: hoop,
        start_time: 0.0,
        duration,
        is_made: true,
        is_three: true,
        peak_z,
        // 本测试只关心弹道采样，不涉及犯规事实。
        fouled: false,
        fouler_id: None,
    };
    let players = std::collections::HashMap::new();
    let mut previous =
        nba_physics::BallisticsEngine::sample_ball_position(&state, 0.0, &players, &rules);
    let mut max_speed: f32 = 0.0;
    let samples = (duration / rules.tick_seconds).ceil() as usize;
    for index in 1..=samples {
        let time = (index as f32 * rules.tick_seconds).min(duration);
        let current =
            nba_physics::BallisticsEngine::sample_ball_position(&state, time, &players, &rules);
        let delta = current.0 - previous.0;
        let speed = (delta.length_squared() + (current.1 - previous.1).powi(2)).sqrt()
            / (time - ((index - 1) as f32 * rules.tick_seconds).min(duration)).max(f32::EPSILON);
        max_speed = max_speed.max(speed);
        previous = current;
    }
    assert!(
        max_speed <= rules.ball_max_speed_ftps + 1e-3,
        "shot speed {max_speed} exceeded {}",
        rules.ball_max_speed_ftps
    );
}

#[test]
fn rebound_samples_start_at_configured_contact_point() {
    let rules = GameRules::default();
    let from = Vec2::new(84.0, 24.0);
    let landing = Vec2::new(72.0, 30.0);
    let state = nba_physics::BallTrajectoryKind::RimRebound {
        from_pos: from,
        from_z: rules.rim_height_ft,
        hoop_pos: Vec2::new(88.75, 25.0),
        target_landing: landing,
        start_time: 5.0,
        duration: 1.0,
        peak_z: rules.rim_height_ft + 1.5,
        last_touch_team: nba_domain::Possession::Home,
        last_touch_player: None,
    };
    let players = std::collections::HashMap::new();
    // 抛体化（第一步）：起点在触筐高度，终点触地（z=0），
    // 途中服从重力——不再是「线性插值到胸口高」。
    assert_eq!(
        nba_physics::BallisticsEngine::sample_ball_position(&state, 5.0, &players, &rules),
        (from, rules.rim_height_ft)
    );
    assert_eq!(
        nba_physics::BallisticsEngine::sample_ball_position(&state, 6.0, &players, &rules),
        (landing, 0.0)
    );
    // 中途采样服从重力：z(t) = z0 + vz0·t − g/2·t²，vz0 由两端反解。
    let (mid_xy, mid_z) =
        nba_physics::BallisticsEngine::sample_ball_position(&state, 5.5, &players, &rules);
    let t = 0.5_f32;
    let vz0 = (0.0 - rules.rim_height_ft + 0.5 * rules.ball_gravity_ftps2 * 1.0) / 1.0;
    let expected_z = rules.rim_height_ft + vz0 * t - 0.5 * rules.ball_gravity_ftps2 * t * t;
    assert!(
        (mid_z - expected_z).abs() < 1e-3,
        "mid z {mid_z} vs {expected_z}"
    );
    assert!(mid_xy.x < from.x, "xy still interpolating toward landing");
}

#[test]
fn bank_shot_contact_lies_on_board_plane_and_lands_in_front() {
    let rules = GameRules::default();
    let court = rules.court;
    let hoop = court.hoop_pos(true);
    let board_x = court.width_ft - rules.backboard_offset_from_baseline_ft;
    let mut rng = rand::rngs::StdRng::seed_from_u64(7);
    for dist in [6.0f32, 14.0, 25.0] {
        let origin = Vec2::new(hoop.x - dist, hoop.y + 3.0);
        // 探针：出手 → 筐延长线穿过板面且弦外推 z 处于板高内。
        assert!(
            nba_physics::BallisticsEngine::compute_backboard_contact_probe(origin, hoop, &rules),
            "overpowered shot from {dist} ft must probe as a bank attempt"
        );
        let peak = (rules.shot_peak_base_ft + dist * rules.shot_peak_distance_factor)
            .min(rules.ball_z_max_ft);
        let flight = nba_physics::BallisticsEngine::shot_duration(dist, peak, &rules);
        let spot = nba_physics::BallisticsEngine::compute_rebound_landing_bank(
            origin, hoop, flight, &mut rng, &rules,
        );
        // 触点在板平面上，高度与横向都在板范围内。
        assert!(
            (spot.contact_pos.x - board_x).abs() < 1e-3,
            "contact x {} not on board plane {board_x}",
            spot.contact_pos.x
        );
        assert!(
            spot.contact_z >= rules.backboard_bottom_height_ft
                && spot.contact_z <= rules.backboard_top_height_ft,
            "contact z {} outside board span",
            spot.contact_z
        );
        assert!(
            (spot.contact_pos.y - court.hoop_y_ft).abs() <= rules.backboard_width_ft / 2.0,
            "contact y {} outside board width",
            spot.contact_pos.y
        );
        // 落点在板前（场内侧），且球到地时仍在场内。
        assert!(
            spot.landing_pos.x < board_x,
            "landing {} must be in front of the board",
            spot.landing_pos.x
        );
        assert!(
            court.contains(spot.landing_pos, 0.0),
            "landing {:?} left the court",
            spot.landing_pos
        );
    }

    // 探针负例：短距离上篮（弦外推 z 低于板底）不走板通道。
    let close = Vec2::new(hoop.x - 2.0, hoop.y);
    assert!(
        !nba_physics::BallisticsEngine::compute_backboard_contact_probe(close, hoop, &rules),
        "close shot must stay on the rim-channel path"
    );
}

#[test]
fn collision_braking_respects_acceleration_envelope() {
    let rules = GameRules {
        tick_seconds: 0.04,
        max_player_speed_ftps: 16.0,
        max_player_accel_ftps2: 20.0,
        min_player_separation_ft: 3.6,
        ..GameRules::default()
    };
    let mut world = PhysicsWorld::with_backend(&rules, PhysicsBackend::SimpleCircle);
    world.register_player(player("H_01", "home", Vec2::new(30.0, 25.0)));
    world.register_player(player("A_01", "away", Vec2::new(36.0, 25.0)));
    world.set_player_target(
        "H_01",
        Vec2::new(40.0, 25.0),
        16.0,
        "DRIVE",
        "BallHandler",
        "Normal",
    );
    world.set_player_target(
        "A_01",
        Vec2::new(26.0, 25.0),
        16.0,
        "CLOSEOUT",
        "Defender",
        "Normal",
    );

    let mut previous_velocity = std::collections::HashMap::new();
    for _ in 0..80 {
        let before = world
            .get_players()
            .iter()
            .map(|(id, player)| (id.clone(), player.vel_ft))
            .collect::<std::collections::HashMap<_, _>>();
        world.step(nba_domain::FixedDt(rules.tick_seconds));
        for (id, current) in world.get_players() {
            let prior = previous_velocity.get(id).copied().unwrap_or(before[id]);
            let acceleration = (current.vel_ft - prior).length() / rules.tick_seconds;
            assert!(
                acceleration <= rules.max_player_accel_ftps2 + 1e-3,
                "{id} acceleration {acceleration} exceeded {}",
                rules.max_player_accel_ftps2
            );
        }
        previous_velocity = world
            .get_players()
            .iter()
            .map(|(id, player)| (id.clone(), player.vel_ft))
            .collect();
    }
}

#[test]
fn braking_and_traction_are_continuous_and_use_both_backends() {
    let baseline_rules = GameRules {
        tick_seconds: 0.04,
        max_player_speed_ftps: 12.0,
        max_player_accel_ftps2: 30.0,
        max_player_braking_accel_ftps2: 18.0,
        max_player_lateral_accel_ftps2: 16.0,
        min_player_separation_ft: 3.6,
        ..GameRules::default()
    };
    let mut slippery_rules = baseline_rules.clone();
    slippery_rules.max_player_braking_accel_ftps2 = 5.0;
    slippery_rules.max_player_lateral_accel_ftps2 = 4.0;

    for backend in [PhysicsBackend::Rapier, PhysicsBackend::SimpleCircle] {
        let stopping_distance = |rules: &GameRules| {
            let mut world = PhysicsWorld::with_backend(rules, backend);
            let mut runner = player("H_01", "home", Vec2::new(30.0, 25.0));
            runner.vel_ft = Vec2::new(8.0, 0.0);
            runner.target_pos_ft = runner.pos_ft;
            runner.attributes.acceleration = 1.0;
            runner.attributes.agility = 1.0;
            world.register_player(runner);
            for _ in 0..100 {
                world.step(nba_domain::FixedDt(rules.tick_seconds));
                if world.get_player("H_01").unwrap().vel_ft.length() <= 1e-4 {
                    break;
                }
            }
            world.get_player("H_01").unwrap().pos_ft.x - 30.0
        };
        let firm_stop = stopping_distance(&baseline_rules);
        let low_grip_stop = stopping_distance(&slippery_rules);
        assert!(
            low_grip_stop > firm_stop,
            "{backend:?}: lower braking must increase stopping distance: {low_grip_stop} vs {firm_stop}"
        );

        let mut world = PhysicsWorld::with_backend(&slippery_rules, backend);
        let mut runner = player("H_01", "home", Vec2::new(30.0, 25.0));
        runner.vel_ft = Vec2::new(8.0, 0.0);
        runner.attributes.acceleration = 1.0;
        runner.attributes.agility = 1.0;
        world.register_player(runner);
        world.set_player_target(
            "H_01",
            Vec2::new(30.0, 55.0),
            12.0,
            "SPRINT",
            "Wing",
            "Normal",
        );
        let mut previous_velocity = Vec2::new(8.0, 0.0);
        let mut previous_projection = previous_velocity.dot(Vec2::Y);
        let mut observed_lateral_velocity = false;
        for _ in 0..30 {
            world.step(nba_domain::FixedDt(slippery_rules.tick_seconds));
            let current = world.get_player("H_01").unwrap();
            let delta_velocity = current.vel_ft - previous_velocity;
            let acceleration = delta_velocity.length() / slippery_rules.tick_seconds;
            assert!(
                acceleration <= slippery_rules.max_player_accel_ftps2 + 1e-3,
                "{backend:?}: acceleration {acceleration} exceeded configured limit"
            );
            assert!(
                current.vel_ft.length() <= slippery_rules.max_player_speed_ftps + 1e-4,
                "{backend:?}: speed exceeded configured limit"
            );
            let projection = current.vel_ft.dot(Vec2::Y);
            assert!(
                projection + 1e-4 >= previous_projection,
                "{backend:?}: lateral velocity reversed direction: {projection} < {previous_projection}"
            );
            observed_lateral_velocity |= projection > 1e-3;
            previous_projection = projection;
            previous_velocity = current.vel_ft;
        }
        assert!(
            observed_lateral_velocity,
            "{backend:?}: turn produced no lateral slip"
        );

        let lateral_velocity_after = |rules: &GameRules| {
            let mut world = PhysicsWorld::with_backend(rules, backend);
            let mut runner = player("H_01", "home", Vec2::new(30.0, 25.0));
            runner.vel_ft = Vec2::new(8.0, 0.0);
            runner.attributes.acceleration = 1.0;
            runner.attributes.agility = 1.0;
            world.register_player(runner);
            world.set_player_target(
                "H_01",
                Vec2::new(30.0, 55.0),
                12.0,
                "SPRINT",
                "Wing",
                "Normal",
            );
            for _ in 0..10 {
                world.step(nba_domain::FixedDt(rules.tick_seconds));
            }
            world.get_player("H_01").unwrap().vel_ft.dot(Vec2::Y)
        };
        assert!(
            lateral_velocity_after(&slippery_rules) < lateral_velocity_after(&baseline_rules),
            "{backend:?}: lower lateral traction must increase turning slip"
        );

        let mut world = PhysicsWorld::with_backend(&baseline_rules, backend);
        let mut runner = player("H_01", "home", Vec2::new(30.0, 25.0));
        runner.vel_ft = Vec2::new(8.0, 0.0);
        runner.attributes.acceleration = 1.0;
        runner.attributes.agility = 1.0;
        world.register_player(runner);
        world.set_player_target(
            "H_01",
            Vec2::new(1.8, 25.0),
            12.0,
            "SPRINT",
            "Wing",
            "Normal",
        );
        let mut previous_x_velocity = 8.0;
        for _ in 0..45 {
            world.step(nba_domain::FixedDt(baseline_rules.tick_seconds));
            let current = world.get_player("H_01").unwrap();
            assert!(
                current.vel_ft.x <= previous_x_velocity + 1e-4,
                "{backend:?}: reverse direction did not decelerate monotonically"
            );
            assert!(
                (current.vel_ft.x - previous_x_velocity).abs()
                    <= baseline_rules.max_player_accel_ftps2 * baseline_rules.tick_seconds + 1e-3,
                "{backend:?}: reverse direction changed velocity beyond the acceleration envelope"
            );
            previous_x_velocity = current.vel_ft.x;
        }
        assert!(
            previous_x_velocity < 0.0,
            "{backend:?}: runner never reversed direction"
        );
    }
}

#[test]
fn collision_solver_preserves_separation_when_players_approach() {
    let rules = GameRules {
        tick_seconds: 0.04,
        max_player_speed_ftps: 16.0,
        max_player_accel_ftps2: 35.0,
        min_player_separation_ft: 3.6,
        ..GameRules::default()
    };
    let mut world = PhysicsWorld::with_backend(&rules, PhysicsBackend::SimpleCircle);
    world.register_player(player("H_01", "home", Vec2::new(30.0, 25.0)));
    world.register_player(player("A_01", "away", Vec2::new(36.0, 25.0)));
    world.set_player_target(
        "H_01",
        Vec2::new(40.0, 25.0),
        16.0,
        "DRIVE",
        "BallHandler",
        "Normal",
    );
    world.set_player_target(
        "A_01",
        Vec2::new(26.0, 25.0),
        16.0,
        "CLOSEOUT",
        "Defender",
        "Normal",
    );

    for step in 0..80 {
        world.step(nba_domain::FixedDt(rules.tick_seconds));
        let home = world.get_player("H_01").unwrap();
        let away = world.get_player("A_01").unwrap();
        let distance = home.pos_ft.distance(away.pos_ft);
        assert!(
            distance + 1e-3 >= rules.min_player_separation_ft,
            "step {step}: players crossed the configured separation: {distance}; home pos/vel {:?}/{:?}, away pos/vel {:?}/{:?}",
            home.pos_ft,
            home.vel_ft,
            away.pos_ft,
            away.vel_ft
        );
    }
}

#[test]
fn contact_facts_are_emitted_once_per_contact_episode() {
    let rules = GameRules {
        contact_margin_ft: 0.2,
        ..GameRules::default()
    };
    for backend in [PhysicsBackend::Rapier, PhysicsBackend::SimpleCircle] {
        let mut world = PhysicsWorld::with_backend(&rules, backend);
        world.register_player(player("H_01", "home", Vec2::new(30.0, 25.0)));
        world.register_player(player("A_01", "away", Vec2::new(33.0, 25.0)));
        world.step(nba_domain::FixedDt(rules.tick_seconds));
        let first = world.drain_contacts();
        assert_eq!(first.len(), 1, "{backend:?} should report the new contact");
        assert_eq!(first[0].entity_a, "A_01");
        assert_eq!(first[0].entity_b, "H_01");
        assert_eq!(first[0].entity_a_action.as_deref(), Some("Idle"));
        assert_eq!(first[0].entity_b_action.as_deref(), Some("Idle"));

        world.step(nba_domain::FixedDt(rules.tick_seconds));
        assert!(
            world.drain_contacts().is_empty(),
            "{backend:?} repeated contact"
        );
    }
}

#[test]
fn separation_contract_survives_turning_approach() {
    let rules = GameRules {
        tick_seconds: 0.04,
        max_player_speed_ftps: 16.0,
        max_player_accel_ftps2: 35.0,
        min_player_separation_ft: 3.6,
        ..GameRules::default()
    };
    let mut world = PhysicsWorld::with_backend(&rules, PhysicsBackend::SimpleCircle);
    world.register_player(player("H_01", "home", Vec2::new(30.0, 25.0)));
    world.register_player(player("A_01", "away", Vec2::new(36.0, 25.0)));
    world.set_player_target(
        "H_01",
        Vec2::new(38.0, 29.0),
        16.0,
        "DRIVE",
        "BallHandler",
        "Normal",
    );
    world.set_player_target(
        "A_01",
        Vec2::new(28.0, 21.0),
        16.0,
        "CLOSEOUT",
        "Defender",
        "Normal",
    );
    for _ in 0..120 {
        world.step(nba_domain::FixedDt(rules.tick_seconds));
        let home = world.get_player("H_01").unwrap();
        let away = world.get_player("A_01").unwrap();
        assert!(home.pos_ft.distance(away.pos_ft) + 1e-3 >= rules.min_player_separation_ft);
    }
}

#[test]
fn semantic_queries_use_selected_backend_contract() {
    let rules = GameRules::default();
    for backend in [PhysicsBackend::Rapier, PhysicsBackend::SimpleCircle] {
        let mut world = PhysicsWorld::with_backend(&rules, backend);
        world.register_player(player("H_01", "home", Vec2::new(40.0, 25.0)));
        world.register_player(player("H_02", "home", Vec2::new(52.0, 25.0)));
        world.register_player(player("A_01", "away", Vec2::new(46.0, 25.0)));
        let corridor = world.pass_corridor(
            Vec2::new(40.0, 25.0),
            Vec2::new(52.0, 25.0),
            rules.pass_corridor_radius_ft,
            "H_01",
            "H_02",
        );
        assert!(corridor.is_blocked);
        assert_eq!(corridor.nearest_interceptor_id.as_deref(), Some("A_01"));
        let openness = world.openness("H_01");
        assert!(openness.contest_intensity > 0.0);
    }
}
