use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_sim_core::action::{ActionIntent, ActionKind, Fact, PassKind, ShotKind};
use nba_sim_core::ball::{self, PassInFlight};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::{BallState, Phase};
use nba_sim_core::world::{World, TICK_SECONDS};
use nba_sim_core::{MatchSetup, Params, TeamSide};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

fn contacts(world: &World) -> Vec<&'static str> {
    world
        .events
        .iter()
        .filter_map(|event| match event.fact {
            Fact::BallContact { kind, .. } => Some(kind),
            _ => None,
        })
        .collect()
}

#[test]
fn shot_result_stays_pending_until_the_ball_resolves_it() {
    let mut world = world(41);
    let shooter = world.on_court(TeamSide::Home).as_slice()[0];
    let team = world.players[shooter].team;
    let hoop = world.hoop_for(team);
    let spot = hoop + Vec2::new(-18.0, 0.0);
    world.players[shooter].pos = spot;
    world.players[shooter].target = spot;
    world.players[shooter].facing = (hoop - spot).normalize_or_zero();
    world.phase = Phase::Live;
    world.ball.set_held(shooter);
    let intent = ActionIntent {
        actor: shooter,
        kind: ActionKind::Shoot(ShotKind::CatchShoot),
        target: None,
        from_ft: spot,
        to_ft: spot,
        start_time: world.now(),
        duration: 0.2,
    };
    nba_sim_core::resolve::shot::resolve(&mut world, &intent, ShotKind::CatchShoot);
    let pending = world.pending_shot.as_ref().expect("pending shot");
    assert!(pending.result.is_none(), "result was decided at release");
    for _ in 0..200 {
        world.wall += TICK_SECONDS;
        ball::step(&mut world, TICK_SECONDS);
        if world
            .pending_shot
            .as_ref()
            .is_some_and(|pending| pending.result.is_some())
        {
            break;
        }
    }
    assert!(
        world
            .pending_shot
            .as_ref()
            .is_some_and(|pending| pending.result.is_some()),
        "shot never resolved"
    );
    let contacts = contacts(&world);
    assert!(
        contacts
            .iter()
            .any(|kind| matches!(*kind, "rim_swish" | "rim" | "floor" | "hand" | "player")),
        "no contact fact: {contacts:?}"
    );
    assert!(
        world.events.iter().any(|event| matches!(
            &event.fact,
            Fact::BallContact { time, .. } if time.is_finite()
        )),
        "contact facts must carry a time"
    );
}

#[test]
fn pass_is_caught_or_intercepted_by_swept_contact() {
    let mut world = world(43);
    let offense: Vec<usize> = world.on_court(TeamSide::Home).as_slice().to_vec();
    let defense: Vec<usize> = world.on_court(TeamSide::Away).as_slice().to_vec();
    let from = offense[0];
    let to = offense[1];
    let defender = defense[0];
    world.players[from].pos = Vec2::new(20.0, 25.0);
    world.players[to].pos = Vec2::new(40.0, 25.0);
    world.players[defender].pos = Vec2::new(30.0, 25.0);
    world.players[from].vel = Vec2::ZERO;
    world.players[to].vel = Vec2::ZERO;
    world.players[defender].vel = Vec2::ZERO;
    world.players[from].facing = Vec2::X;
    world.pass_in_flight = Some(PassInFlight {
        pass_id: 1,
        from,
        to,
        kind: PassKind::Chest,
    });
    let rules = world.rules.clone();
    world.ball.release_pass(crate::ball::PassRelease {
        rules: &rules,
        from_player: &world.players[from].clone(),
        to_player: &world.players[to].clone(),
        now: world.now(),
        pass_id: 1,
        from,
        to,
        kind: PassKind::Chest,
        landing: None,
        flight: None,
    });
    for _ in 0..200 {
        world.wall += TICK_SECONDS;
        ball::step(&mut world, TICK_SECONDS);
        if world.pass_result.is_some() {
            break;
        }
    }
    assert!(
        world.pass_result.is_some(),
        "pass never reached a contact: {:?}",
        world.ball.state
    );
    assert!(
        world
            .events
            .iter()
            .any(|event| matches!(event.fact, Fact::BallContact { .. })),
        "pass contact produced no fact"
    );
}

#[test]
fn a_fast_ball_does_not_tunnel_through_a_defender() {
    let mut world = world(47);
    let offense: Vec<usize> = world.on_court(TeamSide::Home).as_slice().to_vec();
    let defense: Vec<usize> = world.on_court(TeamSide::Away).as_slice().to_vec();
    let from = offense[0];
    let to = offense[1];
    let defender = defense[0];
    world.players[from].pos = Vec2::new(20.0, 25.0);
    world.players[to].pos = Vec2::new(70.0, 25.0);
    // 传球只允许手部接触；把手放到球路上，才能验证高速球不会穿过手。
    world.players[defender].pos = Vec2::new(19.6, 25.0);
    world.players[defender].facing = Vec2::X;
    world.ball.pos = Vec2::new(20.0, 25.0);
    world.ball.z = 4.5;
    world.ball.vel = Vec2::new(85.0, 0.0);
    world.ball.vz = 0.0;
    world.ball.holder = None;
    world.ball.state = BallState::Pass {
        pass_id: 2,
        from,
        to,
        kind: PassKind::Chest,
        from_pos: Vec2::new(20.0, 25.0),
        to_pos: Vec2::new(70.0, 25.0),
        start: world.now(),
        duration: 0.8,
    };
    world.pass_in_flight = Some(PassInFlight {
        pass_id: 2,
        from,
        to,
        kind: PassKind::Chest,
    });
    world.wall += TICK_SECONDS;
    ball::step(&mut world, TICK_SECONDS);
    assert!(
        world
            .events
            .iter()
            .any(|event| matches!(event.fact, Fact::BallContact { .. })),
        "the fast ball produced no contact fact"
    );
}

#[test]
fn floor_rim_and_backboard_contacts_are_distinct() {
    let mut world = world(53);
    let floor_start = world.events.len();
    world.ball.pos = Vec2::new(20.0, 25.0);
    world.ball.z = 0.5;
    world.ball.vel = Vec2::new(2.0, 0.0);
    world.ball.vz = -12.0;
    world.ball.holder = None;
    world.ball.state = BallState::Loose {
        pos: world.ball.pos,
        vel: world.ball.vel,
    };
    world.wall += TICK_SECONDS;
    ball::step(&mut world, TICK_SECONDS);
    assert!(
        world.events[floor_start..]
            .iter()
            .any(|event| matches!(event.fact, Fact::BallContact { .. })),
        "floor contact produced no fact"
    );

    let backboard_start = world.events.len();
    world.ball.pos = Vec2::new(2.0, 25.0);
    world.ball.z = 11.0;
    world.ball.vel = Vec2::new(60.0, 0.0);
    world.ball.vz = 0.0;
    world.ball.state = BallState::Pass {
        pass_id: 3,
        from: 0,
        to: 1,
        kind: PassKind::Chest,
        from_pos: world.ball.pos,
        to_pos: Vec2::new(70.0, 25.0),
        start: world.now(),
        duration: 0.8,
    };
    world.pass_in_flight = None;
    world.wall += 0.1;
    ball::step(&mut world, 0.1);
    assert!(
        world.events[backboard_start..]
            .iter()
            .any(|event| matches!(event.fact, Fact::BallContact { .. })),
        "backboard contact produced no fact"
    );

    let rim_start = world.events.len();
    let hoop = world.hoop_for(world.players[0].team);
    world.ball.pos = hoop + Vec2::new(0.85, 0.0);
    world.ball.z = 10.1;
    world.ball.vel = Vec2::new(-2.0, 0.0);
    world.ball.vz = -1.0;
    world.ball.state = BallState::Shot {
        shot_id: 9,
        shooter: 0,
        from_pos: world.ball.pos,
        target: hoop,
        start: world.now(),
        aperture_ft: 0.0,
        aim_offset: Vec2::ZERO,
    };
    world.pass_in_flight = None;
    world.wall += TICK_SECONDS;
    ball::step(&mut world, TICK_SECONDS);
    assert!(
        world.events[rim_start..].iter().any(|event| matches!(
            event.fact,
            Fact::BallContact { kind: "rim", .. }
        )),
        "rim contact produced no fact"
    );
    assert!(
        matches!(world.ball.state, BallState::Loose { .. }),
        "rim contact must turn the ball loose"
    );
}

#[test]
fn frame_ball_height_and_velocity_are_finite_and_bounded() {
    let mut world = world(59);
    for _ in 0..20_000 {
        let frame = world.stream_tick().frame;
        let ball = &frame.ball;
        assert!(ball.x.is_finite() && ball.y.is_finite() && ball.z.is_finite());
        assert!(ball.vx.is_finite() && ball.vy.is_finite() && ball.vz.is_finite());
        assert!(
            ball.z <= world.rules.ball_z_max_ft + 1.0,
            "ball height {} exceeds the bound",
            ball.z
        );
        let speed = (ball.vx * ball.vx + ball.vy * ball.vy + ball.vz * ball.vz).sqrt();
        assert!(
            speed <= world.rules.ball_max_speed_ftps + 8.0,
            "ball speed {speed} exceeds the envelope"
        );
        world.step(TICK_SECONDS);
        if world.finished {
            break;
        }
    }
}
