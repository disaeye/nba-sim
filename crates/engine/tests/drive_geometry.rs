use glam::Vec2;
use nba_domain::action_window::DribbleMoveKind;
use nba_domain::{GameEvent, GameFlowState, Possession, SubPhase};
use nba_engine::{MatchEngine, MatchSetup};
use nba_physics::ballistics::BallTrajectoryKind;

fn configured_drive(
    defender_positions: &[(f32, f32)],
    dribbler_skill: f32,
    defender_skill: f32,
    seed: u64,
) -> (MatchEngine, Vec2, Vec2) {
    let rules = nba_domain::GameRules {
        tactical_initiation_seconds: 0.0,
        ..Default::default()
    };
    let mut setup = MatchSetup::builtin(rules);
    setup.rules.resolve.base_rates.foul_on_drive_rate = 0.0;
    setup.rules.tactics.drive_speed_ratio = 1.0;
    let mut engine = MatchEngine::with_setup(setup, seed);
    engine.force_possession_for_test(Possession::Home);
    engine.set_game_flow_for_test(GameFlowState::LiveBall);
    engine.set_sub_phase_for_test(SubPhase::ActionExecution);
    let from_pos = Vec2::new(30.0, 25.0);
    let target_pos = Vec2::new(40.0, 25.0);
    let home_id = "H_01";
    let away_ids: Vec<String> = engine
        .physics()
        .get_players()
        .keys()
        .filter(|id| id.starts_with("A_"))
        .cloned()
        .collect();
    let mut away_ids = away_ids;
    away_ids.sort();
    for (index, id) in away_ids.iter().enumerate() {
        let player = engine
            .physics_mut_for_test()
            .get_player_mut(id)
            .expect("away player exists");
        player.on_court = index < defender_positions.len();
        player.vel_ft = Vec2::ZERO;
        player.target_speed_ftps = 0.0;
        player.attributes.defense_perimeter = defender_skill;
        player.attributes.agility = defender_skill;
        player.attributes.strength = defender_skill;
        if let Some((x, y)) = defender_positions.get(index) {
            player.pos_ft = Vec2::new(*x, *y);
            player.target_pos_ft = player.pos_ft;
        }
    }
    let driver = engine
        .physics_mut_for_test()
        .get_player_mut(home_id)
        .expect("home driver exists");
    driver.on_court = true;
    driver.pos_ft = from_pos;
    driver.target_pos_ft = from_pos;
    driver.vel_ft = Vec2::ZERO;
    driver.max_speed_ftps = 22.0;
    driver.max_accel_ftps2 = 35.0;
    driver.attributes.ball_handling = dribbler_skill;
    driver.attributes.agility = dribbler_skill;
    driver.attributes.strength = dribbler_skill;
    driver.attributes.finishing = dribbler_skill;
    engine.set_ball_state_for_test(BallTrajectoryKind::Held {
        carrier_id: home_id.to_string(),
    });
    engine.clear_pending_events_for_test();
    (engine, from_pos, target_pos)
}

fn execute_drive(engine: &mut MatchEngine, from_pos: Vec2, target_pos: Vec2) {
    engine.execute_drive_for_test(
        "H_01",
        from_pos,
        target_pos,
        Some(DribbleMoveKind::DirectDrive),
    );
}

fn initiated_target(engine: &MatchEngine) -> Vec2 {
    engine
        .pending_events()
        .iter()
        .find_map(|event| match event {
            GameEvent::DriveInitiated { target_pos, .. } => {
                Some(Vec2::new(target_pos.0, target_pos.1))
            }
            _ => None,
        })
        .expect("drive initiation event is present")
}

#[test]
fn open_direct_route_succeeds_without_a_probability_draw() {
    let (mut engine, from_pos, target_pos) = configured_drive(&[], 0.5, 0.5, 9);
    execute_drive(&mut engine, from_pos, target_pos);
    assert!(matches!(
        engine.ball_state(),
        BallTrajectoryKind::Drive { .. }
    ));
}

#[test]
fn close_route_blocker_requires_a_successful_bypass_path() {
    let (mut engine, from_pos, target_pos) = configured_drive(&[(35.0, 25.0)], 0.5, 0.5, 11);
    execute_drive(&mut engine, from_pos, target_pos);
    assert!(matches!(
        engine.ball_state(),
        BallTrajectoryKind::Drive { .. }
    ));
    assert_ne!(initiated_target(&engine), target_pos);
}

#[test]
fn two_sided_blockers_break_every_reachable_route() {
    let (mut engine, from_pos, target_pos) =
        configured_drive(&[(34.0, 23.2), (34.0, 26.8)], 0.5, 0.5, 13);
    execute_drive(&mut engine, from_pos, target_pos);
    assert!(matches!(
        engine.ball_state(),
        BallTrajectoryKind::Drive { .. }
    ));
    assert_eq!(initiated_target(&engine), target_pos);
}

#[test]
fn directional_change_moves_the_bypass_to_the_clear_side() {
    let (mut upper_clear, from_pos, target_pos) =
        configured_drive(&[(35.0, 25.0), (35.0, 30.8)], 0.9, 0.5, 17);
    execute_drive(&mut upper_clear, from_pos, target_pos);
    let upper_target = initiated_target(&upper_clear);
    assert!(upper_target.y < from_pos.y);

    let (mut lower_clear, from_pos, target_pos) =
        configured_drive(&[(35.0, 25.0), (35.0, 19.2)], 0.9, 0.5, 19);
    execute_drive(&mut lower_clear, from_pos, target_pos);
    let lower_target = initiated_target(&lower_clear);
    assert!(lower_target.y > from_pos.y);
}

#[test]
fn contact_advantage_changes_route_adjudication() {
    let positions = [(35.0, 25.0), (34.0, 19.2)];
    let (mut strong_driver, from_pos, target_pos) = configured_drive(&positions, 0.95, 0.2, 23);
    execute_drive(&mut strong_driver, from_pos, target_pos);
    assert_ne!(initiated_target(&strong_driver), target_pos);

    let (mut strong_defender, from_pos, target_pos) = configured_drive(&positions, 0.2, 0.95, 29);
    execute_drive(&mut strong_defender, from_pos, target_pos);
    assert_eq!(initiated_target(&strong_defender), target_pos);
}

#[test]
fn drive_outcome_chains_to_drive_initiated_parent_event() {
    let (mut engine, from_pos, target_pos) = configured_drive(&[], 0.95, 0.1, 42);
    execute_drive(&mut engine, from_pos, target_pos);

    let mut initiated_id = None;
    let mut outcome_parent_id = None;
    let mut saw_outcome = false;

    for _ in 0..40 {
        let tick = engine.step();
        for event in &tick.frame.event_log {
            if event.kind == "DRIVE_INITIATED" {
                initiated_id = Some(event.event_id);
            }
            if event.kind == "DRIVE_REACHED" || event.kind == "DRIVE_STOPPED" {
                saw_outcome = true;
                outcome_parent_id = event.parent_event_id;
            }
        }
        if saw_outcome {
            break;
        }
    }

    assert!(saw_outcome, "drive must conclude with an outcome event");
    assert!(initiated_id.is_some(), "drive must publish DRIVE_INITIATED");
    assert_eq!(
        outcome_parent_id, initiated_id,
        "drive outcome must chain to DRIVE_INITIATED parent event"
    );
}
