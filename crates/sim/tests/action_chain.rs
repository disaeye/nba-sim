use glam::Vec2;
use nba_sim_core::action::{ActionIntent, ActionKind, Fact};
use nba_sim_core::params::RuntimeParams;
use nba_sim_core::resolve::drive;
use nba_sim_core::resolve::pass::resolve_cut;
use nba_sim_core::state::Phase;
use nba_sim_core::world::{MatchSetup, World, TICK_SECONDS};
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
    world.possession.team = TeamSide::Home;
    world.phase = Phase::Live;
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
fn cut_uses_lob_flight_and_received_fact_before_control() {
    let mut world = two_player_world();
    let intent = ActionIntent {
        actor: 0,
        kind: ActionKind::Cut,
        target: Some(1),
        from_ft: world.players[0].pos,
        to_ft: world.hoop_for(TeamSide::Home),
        start_time: world.now(),
        duration: 1.0,
    };
    resolve_cut(&mut world, &intent);
    assert!(world.ball.holder.is_none());
    assert!(world.pass_in_flight.is_some());
    assert!(matches!(world.phase, Phase::PassFlight { .. }));
    assert!(!world
        .events
        .iter()
        .any(|event| matches!(event.fact, Fact::Cut { .. })));
    for _ in 0..100 {
        world.step(TICK_SECONDS);
        if world.ball.holder == Some(1) {
            break;
        }
    }
    assert_eq!(world.ball.holder, Some(1));
    assert!(world
        .events
        .iter()
        .any(|event| matches!(event.fact, Fact::Cut { arrived: true, .. })));
}
