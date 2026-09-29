use glam::Vec2;
use nba_domain::{GameFlowState, GameRules, SubPhase};
use nba_engine::{MatchEngine, MatchSetup};
use nba_physics::BallTrajectoryKind;

#[test]
fn successful_catch_transfers_the_ball_without_a_speed_jump() {
    let rules = GameRules {
        tick_seconds: 0.04,
        ..Default::default()
    };
    let mut setup = MatchSetup::builtin(rules);
    setup.rules.resolve.base_rates.pass_success = 1.0;
    setup.rules.resolve.pass.openness_weight = 0.0;
    setup.rules.resolve.pass.passer_skill_weight = 0.0;
    setup.rules.resolve.pass.receiver_control_weight = 0.0;
    setup.rules.resolve.pass.catch_equilibrium_weight = 0.0;
    let mut engine = MatchEngine::with_setup(setup, 42);

    let receiver_id = "H_02";
    let receiver_position = Vec2::new(48.0, 25.0);
    let passer_position = Vec2::new(40.0, 25.0);
    for id in [
        "H_01", "H_02", "H_03", "H_04", "H_05", "A_01", "A_02", "A_03", "A_04", "A_05",
    ] {
        if let Some(player) = engine.physics_mut_for_test().get_player_mut(id) {
            player.pos_ft = if id == receiver_id {
                receiver_position
            } else if id == "H_01" {
                passer_position
            } else {
                Vec2::new(
                    5.0 + (id.as_bytes()[2] as f32 % 10.0),
                    5.0 + (id.as_bytes()[3] as f32 % 30.0),
                )
            };
            player.target_pos_ft = player.pos_ft;
            player.vel_ft = Vec2::ZERO;
            player.accel_ft = Vec2::ZERO;
            player.target_speed_ftps = 0.0;
            if id != receiver_id {
                player.is_locked_kinematics = true;
            }
        }
    }
    engine.set_ball_state_for_test(BallTrajectoryKind::Pass {
        from_pos: passer_position,
        from_z: 4.0,
        to_pos: receiver_position,
        target_id: receiver_id.to_string(),
        start_time: 0.0,
        duration: 0.2,
        peak_z: engine.rules().pass_peak_ft,
        inbound: false,
    });
    engine.set_last_passer_for_test(Some("H_01".to_string()));
    engine.set_game_flow_for_test(GameFlowState::LiveBall);
    engine.set_sub_phase_for_test(SubPhase::ActionExecution);
    engine.set_current_time_for_test(0.0);
    engine.set_ball_pos_for_test(passer_position, engine.rules().chest_height_ft);
    let mut previous = engine.ball_pos_3d();
    let speed_limit = engine.rules().ball_max_speed_ftps * 1.05;
    let mut saw_receive = false;
    let mut saw_transfer = false;
    let mut saw_held = false;
    let mut jumps = Vec::new();
    for tick_index in 1..=20 {
        engine.set_current_time_for_test(tick_index as f32 * engine.rules().tick_seconds);
        let tick = engine.step();
        let current = engine.ball_pos_3d();
        let speed = ((current.0 - previous.0).length_squared() + (current.1 - previous.1).powi(2))
            .sqrt()
            / engine.rules().tick_seconds;
        if speed > speed_limit {
            jumps.push(speed);
        }
        previous = current;
        let received = tick
            .frame
            .event_log
            .iter()
            .any(|event| event.kind == "PASS_RECEIVED");
        saw_receive |= received;
        if received {
            saw_transfer = matches!(
                engine.ball_state(),
                BallTrajectoryKind::ControlTransfer { carrier_id, .. } if carrier_id == receiver_id
            );
        }
        saw_held |= matches!(
            engine.ball_state(),
            BallTrajectoryKind::Held { carrier_id } if carrier_id == receiver_id
        );
        if saw_held {
            break;
        }
    }
    assert!(
        jumps.is_empty(),
        "ball transition speeds exceed {speed_limit:.2} ft/s: {jumps:?}"
    );
    assert!(saw_receive, "pass must publish PASS_RECEIVED");
    assert!(saw_transfer, "catch must enter ControlTransfer before Held");
    assert!(saw_held, "control transfer must complete into Held");
}

#[test]
fn pass_contact_emits_intercepted_without_arrival_events() {
    let rules = GameRules {
        tick_seconds: 0.04,
        ..Default::default()
    };
    let mut setup = MatchSetup::builtin(rules);
    setup.rules.resolve.base_rates.intercept_steal_slope = 100.0;
    setup.rules.resolve.base_rates.intercept_steal_floor = 1.0;
    setup.rules.resolve.base_rates.intercept_steal_ceiling = 1.0;
    setup.rules.resolve.base_rates.intercept_tip_floor = 0.0;
    setup.rules.resolve.base_rates.intercept_tip_ceiling = 0.0;
    setup.rules.resolve.base_rates.intercept_clearance_scale_ft = 20.0;
    let mut engine = MatchEngine::with_setup(setup, 42);
    engine.force_possession_for_test(nba_domain::Possession::Home);

    let passer_pos = Vec2::new(40.0, 25.0);
    let target_pos = Vec2::new(50.0, 25.0);
    let defender_pos = Vec2::new(45.0, 25.0);

    for id in [
        "H_01", "H_02", "H_03", "H_04", "H_05", "A_01", "A_02", "A_03", "A_04", "A_05",
    ] {
        if let Some(player) = engine.physics_mut_for_test().get_player_mut(id) {
            player.pos_ft = if id == "H_01" {
                passer_pos
            } else if id == "H_02" {
                target_pos
            } else if id == "A_01" {
                defender_pos
            } else {
                Vec2::new(
                    5.0 + (id.as_bytes()[2] as f32 % 10.0) * 4.0,
                    5.0 + (id.as_bytes()[3] as f32 % 10.0) * 4.0,
                )
            };
            player.target_pos_ft = player.pos_ft;
            player.vel_ft = Vec2::ZERO;
            player.accel_ft = Vec2::ZERO;
            player.target_speed_ftps = 0.0;
            player.attributes.vertical = 0.8;
            player.attributes.steal = 1.0;
            player.is_locked_kinematics = true;
        }
    }

    engine.set_ball_state_for_test(BallTrajectoryKind::Pass {
        from_pos: passer_pos,
        from_z: 4.0,
        to_pos: target_pos,
        target_id: "H_02".to_string(),
        start_time: 0.0,
        duration: 0.2,
        peak_z: engine.rules().pass_peak_ft,
        inbound: false,
    });
    engine.set_last_passer_for_test(Some("H_01".to_string()));
    engine.set_game_flow_for_test(GameFlowState::LiveBall);
    engine.set_sub_phase_for_test(SubPhase::ActionExecution);
    engine.set_current_time_for_test(0.0);
    engine.set_ball_pos_for_test(passer_pos, engine.rules().chest_height_ft);

    let mut saw_steal = false;
    let mut saw_drop = false;
    let mut saw_receive = false;
    let mut saw_tip = false;

    for tick_index in 1..=20 {
        engine.set_current_time_for_test(tick_index as f32 * engine.rules().tick_seconds);
        let tick = engine.step();
        for event in &tick.frame.event_log {
            match event.kind.as_str() {
                "STEAL" => saw_steal = true,
                "PASS_DROPPED" => saw_drop = true,
                "PASS_RECEIVED" => saw_receive = true,
                "PASS_TIPPED" => saw_tip = true,
                _ => {}
            }
        }
        if saw_steal {
            break;
        }
    }

    assert!(saw_steal, "interception must emit STEAL");
    assert!(!saw_receive, "intercepted pass cannot emit PASS_RECEIVED");
    assert!(!saw_drop, "intercepted pass cannot emit PASS_DROPPED");
    assert!(!saw_tip, "intercepted pass cannot emit PASS_TIPPED");
}

#[test]
fn pass_contact_emits_tipped_without_arrival_events() {
    let rules = GameRules {
        tick_seconds: 0.04,
        ..Default::default()
    };
    let mut setup = MatchSetup::builtin(rules);
    setup.rules.resolve.base_rates.intercept_steal_floor = 0.0;
    setup.rules.resolve.base_rates.intercept_steal_ceiling = 0.0;
    setup.rules.resolve.base_rates.intercept_steal_slope = 0.0;
    setup.rules.resolve.base_rates.intercept_tip_slope = 100.0;
    setup.rules.resolve.base_rates.intercept_tip_floor = 1.0;
    setup.rules.resolve.base_rates.intercept_tip_ceiling = 1.0;
    setup.rules.resolve.base_rates.intercept_clearance_scale_ft = 20.0;
    let mut engine = MatchEngine::with_setup(setup, 42);
    engine.force_possession_for_test(nba_domain::Possession::Home);

    let passer_pos = Vec2::new(40.0, 25.0);
    let target_pos = Vec2::new(50.0, 25.0);
    let defender_pos = Vec2::new(45.0, 25.0);

    for id in [
        "H_01", "H_02", "H_03", "H_04", "H_05", "A_01", "A_02", "A_03", "A_04", "A_05",
    ] {
        if let Some(player) = engine.physics_mut_for_test().get_player_mut(id) {
            player.pos_ft = if id == "H_01" {
                passer_pos
            } else if id == "H_02" {
                target_pos
            } else if id == "A_01" {
                defender_pos
            } else {
                Vec2::new(
                    5.0 + (id.as_bytes()[2] as f32 % 10.0) * 4.0,
                    5.0 + (id.as_bytes()[3] as f32 % 10.0) * 4.0,
                )
            };
            player.target_pos_ft = player.pos_ft;
            player.vel_ft = Vec2::ZERO;
            player.accel_ft = Vec2::ZERO;
            player.target_speed_ftps = 0.0;
            player.attributes.vertical = 0.8;
            player.attributes.steal = 1.0;
            player.is_locked_kinematics = true;
        }
    }

    engine.set_ball_state_for_test(BallTrajectoryKind::Pass {
        from_pos: passer_pos,
        from_z: 4.0,
        to_pos: target_pos,
        target_id: "H_02".to_string(),
        start_time: 0.0,
        duration: 0.2,
        peak_z: engine.rules().pass_peak_ft,
        inbound: false,
    });
    engine.set_last_passer_for_test(Some("H_01".to_string()));
    engine.set_game_flow_for_test(GameFlowState::LiveBall);
    engine.set_sub_phase_for_test(SubPhase::ActionExecution);
    engine.set_current_time_for_test(0.0);
    engine.set_ball_pos_for_test(passer_pos, engine.rules().chest_height_ft);

    let mut saw_steal = false;
    let mut saw_drop = false;
    let mut saw_receive = false;
    let mut saw_tip = false;

    for tick_index in 1..=20 {
        engine.set_current_time_for_test(tick_index as f32 * engine.rules().tick_seconds);
        let tick = engine.step();
        for event in &tick.frame.event_log {
            match event.kind.as_str() {
                "STEAL" => saw_steal = true,
                "PASS_DROPPED" => saw_drop = true,
                "PASS_RECEIVED" => saw_receive = true,
                "PASS_TIPPED" => saw_tip = true,
                _ => {}
            }
        }
        if saw_tip {
            break;
        }
    }

    assert!(saw_tip, "deflection must emit PASS_TIPPED");
    assert!(!saw_steal, "deflected pass cannot emit STEAL");
    assert!(!saw_receive, "deflected pass cannot emit PASS_RECEIVED");
    assert!(!saw_drop, "deflected pass cannot emit PASS_DROPPED");
}

#[test]
fn pass_arrival_emits_drop_without_contact_events() {
    let rules = GameRules {
        tick_seconds: 0.04,
        ..Default::default()
    };
    let mut setup = MatchSetup::builtin(rules);
    setup.rules.resolve.base_rates.pass_success = 0.0;
    setup.rules.resolve.pass.openness_weight = 0.0;
    setup.rules.resolve.pass.passer_skill_weight = 0.0;
    setup.rules.resolve.pass.receiver_control_weight = 0.0;
    setup.rules.resolve.pass.catch_equilibrium_weight = 0.0;
    let mut engine = MatchEngine::with_setup(setup, 42);
    engine.force_possession_for_test(nba_domain::Possession::Home);

    let passer_pos = Vec2::new(40.0, 25.0);
    let target_pos = Vec2::new(50.0, 25.0);

    for id in [
        "H_01", "H_02", "H_03", "H_04", "H_05", "A_01", "A_02", "A_03", "A_04", "A_05",
    ] {
        if let Some(player) = engine.physics_mut_for_test().get_player_mut(id) {
            player.pos_ft = if id == "H_01" {
                passer_pos
            } else if id == "H_02" {
                target_pos
            } else {
                Vec2::new(
                    5.0 + (id.as_bytes()[2] as f32 % 10.0) * 4.0,
                    5.0 + (id.as_bytes()[3] as f32 % 10.0) * 4.0,
                )
            };
            player.target_pos_ft = player.pos_ft;
            player.vel_ft = Vec2::ZERO;
            player.accel_ft = Vec2::ZERO;
            player.target_speed_ftps = 0.0;
            player.is_locked_kinematics = true;
        }
    }

    engine.set_ball_state_for_test(BallTrajectoryKind::Pass {
        from_pos: passer_pos,
        from_z: 4.0,
        to_pos: target_pos,
        target_id: "H_02".to_string(),
        start_time: 0.0,
        duration: 0.1,
        peak_z: engine.rules().pass_peak_ft,
        inbound: false,
    });
    engine.set_last_passer_for_test(Some("H_01".to_string()));
    engine.set_game_flow_for_test(GameFlowState::LiveBall);
    engine.set_sub_phase_for_test(SubPhase::ActionExecution);
    engine.set_current_time_for_test(0.0);
    engine.set_ball_pos_for_test(passer_pos, engine.rules().chest_height_ft);

    let mut saw_steal = false;
    let mut saw_drop = false;
    let mut saw_receive = false;
    let mut saw_tip = false;

    for tick_index in 1..=20 {
        engine.set_current_time_for_test(tick_index as f32 * engine.rules().tick_seconds);
        let tick = engine.step();
        for event in &tick.frame.event_log {
            match event.kind.as_str() {
                "STEAL" => saw_steal = true,
                "PASS_DROPPED" => saw_drop = true,
                "PASS_RECEIVED" => saw_receive = true,
                "PASS_TIPPED" => saw_tip = true,
                _ => {}
            }
        }
        if saw_drop {
            break;
        }
    }

    assert!(saw_drop, "unsuccessful pass must emit PASS_DROPPED");
    assert!(!saw_steal, "dropped pass cannot emit STEAL");
    assert!(!saw_receive, "dropped pass cannot emit PASS_RECEIVED");
    assert!(!saw_tip, "dropped pass cannot emit PASS_TIPPED");
}

#[test]
fn pass_outcomes_are_strictly_mutually_exclusive_across_seed_matrix() {
    for seed in [42, 1, 7, 100] {
        let mut engine = MatchEngine::new(seed);
        engine.set_scope("1q").expect("1q scope");

        let mut pass_active = false;
        let mut outcomes_for_current_pass = 0usize;

        while !engine.is_finished() {
            let tick = engine.step();
            for event in &tick.frame.event_log {
                if event.kind == "PASS" {
                    assert!(
                        !pass_active || outcomes_for_current_pass == 1,
                        "seed {seed}: previous pass must have settled before new pass begins"
                    );
                    pass_active = true;
                    outcomes_for_current_pass = 0;
                }
                if pass_active
                    && matches!(
                        event.kind.as_str(),
                        "PASS_RECEIVED" | "PASS_DROPPED" | "PASS_TIPPED" | "STEAL"
                    )
                {
                    outcomes_for_current_pass += 1;
                    pass_active = false;
                }
            }
            assert!(
                outcomes_for_current_pass <= 1,
                "seed {seed}: multiple mutually exclusive pass outcomes emitted in same resolution"
            );
        }
    }
}
