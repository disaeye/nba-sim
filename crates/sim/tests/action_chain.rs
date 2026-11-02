use glam::Vec2;
use nba_sim_core::action::{
    ActionIntent, ActionIntentRole, ActionKind, ActionRuntime, Fact,
};
use nba_sim_core::params::RuntimeParams;
use nba_sim_core::resolve::drive;
use nba_sim_core::resolve::pass::resolve_cut;
use nba_sim_core::state::Phase;
use nba_sim_core::world::{MatchSetup, World, TICK_SECONDS};
use nba_domain::action_window::ActionPhase;
use nba_sim_core::TeamSide;

fn two_player_world() -> World {
    let mut world = World::new(
        MatchSetup::builtin(nba_domain::GameRules::default()),
        RuntimeParams::embedded(),
        13,
    );
    for player in world.players.iter_mut() {
        player.on_court = false;
    }
    world.players[0].on_court = true;
    world.players[1].on_court = true;
    let defender = world
        .players
        .iter()
        .position(|player| player.team != TeamSide::Home)
        .expect("away defender");
    world.players[defender].on_court = true;
    world.sync_on_court();
    world.possession.team = TeamSide::Home;
    world.set_phase(Phase::Live);
    world.players[0].pos = Vec2::new(60.0, 25.0);
    world.players[1].pos = Vec2::new(76.0, 25.0);
    // 防守人不在高吊球的路线上：这条测试检验的是「高吊球飞行 → 接球 →
    // PassReceived 改变控制权」这条机制，不是「球会不会被断」。
    world.players[defender].pos = Vec2::new(66.0, 32.0);
    world.players[0].target = Vec2::new(88.8, 25.0);
    world.ball.set_held(0);
    world
}

#[test]
fn action_runtime_tracks_configured_phase_boundaries() {
    let intent = ActionIntent {
        actor: 0,
        kind: ActionKind::Pass(nba_sim_core::action::PassKind::Chest),
        target: Some(1),
        from_ft: Vec2::new(10.0, 10.0),
        to_ft: Vec2::new(20.0, 10.0),
        start_time: 2.0,
        duration: 0.9,
    };
    let mut runtime = ActionRuntime::new(
        1,
        1,
        intent,
        0,
        ActionIntentRole::Primary,
        ActionPhase::Preparation,
        0.2,
        0.3,
        0.4,
    );
    assert_eq!(runtime.phase_at(2.0), None);
    assert_eq!(runtime.phase_at(2.2), Some(ActionPhase::Execution));
    runtime.phase = ActionPhase::Execution;
    assert_eq!(runtime.phase_at(2.5), Some(ActionPhase::FollowThrough));
    runtime.phase = ActionPhase::FollowThrough;
    assert_eq!(runtime.phase_at(2.9), None);
}

#[test]
fn shared_action_runtime_assigns_one_interaction_to_each_participant() {
    let mut world = two_player_world();
    let intent = ActionIntent {
        actor: 0,
        kind: ActionKind::Screen(nba_sim_core::action::ScreenKind::OnBall),
        target: Some(1),
        from_ft: world.players[0].pos,
        to_ft: Vec2::new(70.0, 25.0),
        start_time: world.now(),
        duration: 1.0,
    };
    world.screen_defender = Some(2);
    world.events.clear();
    assert!(world.start_action(
        intent,
        &[
            (0, ActionIntentRole::Primary),
            (1, ActionIntentRole::ScreenPartner),
        ],
    ));
    let handler = world.players[0].action.as_ref().expect("handler action");
    let screener = world.players[1].action.as_ref().expect("screener action");
    assert_eq!(handler.action_id, screener.action_id);
    assert_eq!(handler.interaction_id, screener.interaction_id);
    assert_eq!(handler.player, 0);
    assert_eq!(screener.player, 1);
    assert_eq!(handler.role, ActionIntentRole::Primary);
    assert_eq!(screener.role, ActionIntentRole::ScreenPartner);
    let starts = world
        .events
        .iter()
        .filter(|event| matches!(event.fact, Fact::ActionStarted { .. }))
        .count();
    assert_eq!(starts, 2);
    for _ in 0..3 {
        world.step(TICK_SECONDS);
    }
    assert!(matches!(
        world.players[0].action.as_ref().map(|runtime| runtime.phase),
        Some(ActionPhase::Execution)
    ));
    let phase_changes = world
        .events
        .iter()
        .filter(|event| matches!(event.fact, Fact::ActionPhaseChanged { .. }))
        .count();
    assert_eq!(phase_changes, 2);
}

#[test]
fn drive_resolution_reports_actual_arrival_and_separation() {
    let mut world = two_player_world();
    let intent = ActionIntent {
        actor: 0,
        kind: ActionKind::Drive,
        target: None,
        from_ft: world.players[0].pos,
        to_ft: world.hoop_for(TeamSide::Home),
        start_time: world.now(),
        duration: 1.0,
    };
    drive::resolve(&mut world, &intent);
    let event = world
        .events
        .iter()
        .rev()
        .find(|event| matches!(event.fact, Fact::Drive { .. }))
        .expect("drive fact");
    match event.fact {
        Fact::Drive {
            reached,
            distance_ft,
            separation_ft,
            ..
        } => {
            assert!(!reached);
            assert!(distance_ft > 20.0);
            assert!(separation_ft < 10.0);
        }
        _ => unreachable!(),
    }
}

#[test]
fn live_shot_starts_parallel_closeout_and_changes_the_on_ball_target() {
    let mut world = two_player_world();
    let defender = world
        .players
        .iter()
        .position(|player| player.team == TeamSide::Away && player.on_court)
        .expect("away defender");
    world.players[0].pos = Vec2::new(60.0, 25.0);
    world.players[0].target = world.players[0].pos;
    world.players[defender].pos = Vec2::new(60.0, 32.0);
    world.players[defender].target = world.players[defender].pos;
    world.players[defender].defense_responsibility =
        nba_sim_core::state::DefenseResponsibility::OnBall;
    world.possession.team = TeamSide::Home;
    world.possession.matchup[0] = defender;
    world.ball.set_held(0);
    let intent = ActionIntent {
        actor: 0,
        kind: ActionKind::Shoot(nba_sim_core::action::ShotKind::CatchShoot),
        target: None,
        from_ft: world.players[0].pos,
        to_ft: world.players[0].pos,
        start_time: world.now(),
        duration: 1.2,
    };
    world.rules.jump_shot_prep_seconds = 0.02;
    world.rules.jump_shot_exec_seconds = 0.02;
    world.rules.jump_shot_follow_seconds = 1.16;
    world.players[defender].data.height_cm = 208;
    world.players[defender].data.attributes.vertical = 1.0;
    // 本测试看的是扑防几何与干扰强度：防守人不具备封盖能力，出手必须飞出去。
    world.players[defender].data.attributes.block = 0.0;
    assert!(world.start_action(intent, &[(0, ActionIntentRole::Primary)]));
    let offense_action_id = world.players[0]
        .action
        .as_ref()
        .expect("offense action")
        .action_id;
    let initial_distance = world.players[defender].pos.distance(world.players[0].pos);
    let baseline_model = nba_sim_core::shot_quality::evaluate(
        &world,
        0,
        nba_sim_core::action::ShotKind::CatchShoot,
        nba_sim_core::state::BallPhase::Catch,
        world.players[0].pos,
    );
    world.step(TICK_SECONDS);
    let runtime = world.defensive_actions[defender]
        .as_ref()
        .expect("defensive runtime runs beside the shot");
    assert!(runtime.action_id > offense_action_id);
    assert!(runtime.interaction_id > 0);
    assert!(matches!(runtime.phase, ActionPhase::Preparation));
    assert!(world.events.iter().any(|event| matches!(
        event.fact,
        Fact::DefensiveActionStarted { defender: event_defender, shooter: 0, .. }
            if event_defender == defender
    )));
    for _ in 0..4 {
        world.step(TICK_SECONDS);
    }
    let target_distance = world.defensive_actions[defender]
        .as_ref()
        .expect("closeout runtime")
        .target_position
        .distance(world.players[0].pos);
    assert!(target_distance >= world.rules.min_player_separation_ft - 0.25);
    assert!(
        target_distance <= world.rules.min_player_separation_ft + 2.5,
        "target distance {target_distance}; target {:?}; shooter {:?}; hoop {:?}; responsibility {:?}; runtime {:?}",
        world.defensive_actions[defender]
            .as_ref()
            .expect("closeout runtime")
            .target_position,
        world.players[0].pos,
        world.hoop_for(TeamSide::Home),
        world.players[defender].defense_responsibility,
        world.defensive_actions[defender]
            .as_ref()
            .map(|runtime| (runtime.phase, runtime.target_position))
    );
    let basketward = (world.hoop_for(TeamSide::Home) - world.players[0].pos).normalize_or_zero();
    let target_direction = (world.defensive_actions[defender]
        .as_ref()
        .expect("closeout runtime")
        .target_position
        - world.players[0].pos)
        .normalize_or_zero();
    assert!(
        target_direction.dot(basketward) > 0.8,
        "target {:?}; shooter {:?}; basketward {:?}; direction {:?}",
        world.players[defender].target,
        world.players[0].pos,
        basketward,
        target_direction
    );
    assert!(
        world.players[defender].pos.distance(world.players[0].pos) < initial_distance,
        "defender {:?}; shooter {:?}; target {:?}; initial distance {initial_distance}; runtime {:?}",
        world.players[defender].pos,
        world.players[0].pos,
        world.defensive_actions[defender]
            .as_ref()
            .expect("closeout runtime")
            .target_position,
        world.defensive_actions[defender]
            .as_ref()
            .map(|runtime| (runtime.phase, runtime.target_position))
    );
    while world.clock.simulation_time() < 0.8 {
        world.step(TICK_SECONDS);
    }
    let closeout_model = nba_sim_core::shot_quality::evaluate(
        &world,
        0,
        nba_sim_core::action::ShotKind::CatchShoot,
        nba_sim_core::state::BallPhase::Catch,
        world.players[0].pos,
    );
    assert!(
        closeout_model.contest > baseline_model.contest,
        "closeout contest {} must exceed baseline {}",
        closeout_model.contest,
        baseline_model.contest
    );
    assert!(world.defensive_actions[defender].is_some());
    let completed_actions = world
        .events
        .iter()
        .filter(|event| matches!(
            event.fact,
            Fact::DefensiveActionCompleted { defender: event_defender, .. }
                if event_defender == defender
        ))
        .count();
    assert_eq!(completed_actions, 1);
    assert!(world.events.iter().any(|event| matches!(
        event.fact,
        Fact::DefensiveActionPhaseChanged {
            defender: event_defender,
            to: ActionPhase::Execution,
            ..
        } if event_defender == defender
    )));
    while world.clock.simulation_time() < 1.3 {
        world.step(TICK_SECONDS);
    }
    assert!(world.interactions.is_empty());
    assert!(matches!(world.phase(), Phase::ShotFlight { .. }));
    match world.pending_shot.as_ref() {
        Some(pending_shot) => assert!(
            pending_shot.contest > baseline_model.contest,
            "shot contest {} must exceed baseline {}",
            pending_shot.contest,
            baseline_model.contest
        ),
        None => {
            // 扑防到位后手真的碰到了球：这次出手被封盖，同样的几何后果。
            assert!(
                world.events.iter().any(|event| matches!(
                    event.fact,
                    Fact::BallContact { kind: "hand", entity: Some(who), .. } if who == defender
                )),
                "出手已经结束就必须是防守人封到了球"
            );
        }
    }
    let started_actions = world
        .events
        .iter()
        .filter(|event| matches!(
            event.fact,
            Fact::DefensiveActionStarted { defender: event_defender, .. }
                if event_defender == defender
        ))
        .count();
    assert_eq!(started_actions, 1);
    assert!(world.events.iter().any(|event| matches!(
        event.fact,
        Fact::DefensiveActionCancelled {
            defender: event_defender,
            reason: "shot_action_ended",
            ..
        } if event_defender == defender
    )));
}

#[test]
fn cut_uses_lob_flight_and_received_fact_before_control() {
    let mut world = two_player_world();
    let intent = ActionIntent {
        actor: 0,
        kind: ActionKind::Cut(nba_sim_core::action::CutKind::Basket),
        target: Some(1),
        from_ft: world.players[0].pos,
        to_ft: world.hoop_for(TeamSide::Home),
        start_time: world.now(),
        duration: 1.0,
    };
    resolve_cut(&mut world, &intent);
    assert!(world.ball.holder().is_none());
    assert!(world.pass_in_flight.is_some());
    assert!(matches!(world.phase(), Phase::PassFlight { .. }));
    assert!(!world
        .events
        .iter()
        .any(|event| matches!(event.fact, Fact::Cut { .. })));
    for _ in 0..100 {
        world.step(TICK_SECONDS);
        if world.ball.holder() == Some(1) {
            break;
        }
    }
    assert_eq!(world.ball.holder(), Some(1));
    assert!(world
        .events
        .iter()
        .any(|event| matches!(event.fact, Fact::Cut { arrived: true, .. })));
}
