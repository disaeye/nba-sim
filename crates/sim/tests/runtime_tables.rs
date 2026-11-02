use std::path::{Path, PathBuf};

use glam::Vec2;
use nba_sim_core::action::Fact;
use nba_sim_core::ball;
use nba_sim_core::params::{workspace_root, RuntimeParams};
use nba_sim_core::resolve::{freethrow, rebound};
use nba_sim_core::state::{BallState, Phase, PossessionKind};
use nba_sim_core::world::{MatchSetup, World, TICK_SECONDS};
use nba_sim_core::TeamSide;

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("fixture directory");
    for entry in std::fs::read_dir(from).expect("fixture source") {
        let entry = entry.expect("fixture entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("fixture type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).expect("fixture copy");
        }
    }
}

fn runtime_only_dir() -> PathBuf {
    let root = workspace_root().join(".work").join("runtime-only");
    let calibration = root.join("calibration");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&calibration).expect("calibration fixture");
    for name in ["policy", "formations", "shot_distance"] {
        std::fs::copy(
            workspace_root()
                .join("data/calibration")
                .join(format!("{name}.json")),
            calibration.join(format!("{name}.json")),
        )
        .expect("runtime table copy");
    }
    copy_dir(
        &workspace_root().join("data/tactics"),
        &root.join("tactics"),
    );
    calibration
}

#[test]
fn production_entry_runs_without_result_tables() {
    let calibration = runtime_only_dir();
    assert!(!calibration.join("shot_make.json").exists());
    assert!(!calibration.join("rebound.json").exists());
    let params = RuntimeParams::load(&calibration).expect("runtime tables load without results");
    let setup = MatchSetup::builtin(nba_domain::GameRules::default());
    let report = nba_sim_core::run_game_with_setup(11, params, setup);
    assert!(!report.possessions.is_empty());
    assert!(!report.events.is_empty());
}

#[test]
fn rebound_stays_loose_until_a_player_controls_it() {
    let mut world = World::new(
        MatchSetup::builtin(nba_domain::GameRules::default()),
        RuntimeParams::embedded(),
        5,
    );
    let handle = 0;
    world.players[handle].on_court = true;
    world.sync_on_court();
    world.set_phase(Phase::Live);
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.players[handle].pos = Vec2::new(30.0, 20.0);
    world.players[handle].target = world.players[handle].pos;
    world.players[handle].target_partner = usize::MAX;
    world.ball.release(BallState::Loose {
        pos: world.players[handle].pos,
        vel: Vec2::ZERO,
    });
    world.ball.pos = world.players[handle].pos;
    world.ball.z = 4.0;
    world.ball.vel = Vec2::ZERO;
    rebound::resolve(&mut world, "three", 24.0, true);
    assert!(world.pending_rebound.is_some());
    assert!(world.ball.holder().is_none());
    assert!(!world
        .events
        .iter()
        .any(|event| matches!(event.fact, Fact::Rebound { .. })));
    ball::step(&mut world, TICK_SECONDS);
    assert_eq!(world.ball.holder(), Some(handle));
    assert!(world
        .events
        .iter()
        .any(|event| matches!(event.fact, Fact::Rebound { player, .. } if player == handle)));
}

#[test]
fn free_throw_is_resolved_by_ball_flight_not_by_a_table_draw() {
    let mut world = World::new(
        MatchSetup::builtin(nba_domain::GameRules::default()),
        RuntimeParams::embedded(),
        7,
    );
    world.players[0].pos = Vec2::new(70.0, 25.0);
    let foul_id = world.new_foul_id();
    freethrow::shoot(&mut world, 0, 1, 1, 0, false, false, foul_id);
    assert!(world
        .pending_shot
        .as_ref()
        .is_some_and(|pending| pending.free_throw.is_some()));
    assert!(matches!(world.phase(), Phase::FreeThrowFlight { .. }));
    let game_clock = world.clock.game_clock();
    let shot_clock = world.clock.shot_clock();
    let mut steps = 0;
    while !world
        .events
        .iter()
        .any(|event| matches!(event.fact, Fact::FreeThrow { shooter: 0, .. }))
    {
        world.step(TICK_SECONDS);
        steps += 1;
        assert!(steps < 200, "free-throw flight did not resolve");
    }
    assert_eq!(world.clock.game_clock(), game_clock);
    assert_eq!(world.clock.shot_clock(), shot_clock);
    assert!(matches!(
        world.phase(),
        Phase::DeadBall { .. } | Phase::Live
    ));
}

#[test]
fn pass_flight_consumes_both_live_clocks() {
    use nba_sim_core::action::{ActionIntent, ActionKind, PassKind};
    use nba_sim_core::resolve::pass;

    let mut world = World::new(
        MatchSetup::builtin(nba_domain::GameRules::default()),
        RuntimeParams::embedded(),
        11,
    );
    world.players[0].pos = Vec2::new(60.0, 25.0);
    world.players[1].pos = Vec2::new(75.0, 25.0);
    for &defender in world.on_court(TeamSide::Away).as_slice() {
        world.players[defender].pos = Vec2::new(20.0, 45.0);
    }
    world.possession.team = TeamSide::Home;
    world.ball.set_held(0);
    world.set_phase(Phase::Live);
    let intent = ActionIntent {
        actor: 0,
        kind: ActionKind::Pass(PassKind::Chest),
        target: Some(1),
        from_ft: world.players[0].pos,
        to_ft: world.players[1].pos,
        start_time: world.now(),
        duration: 0.1,
    };
    pass::resolve(&mut world, &intent, PassKind::Chest);
    let game_clock = world.clock.game_clock();
    let shot_clock = world.clock.shot_clock();
    world.step(TICK_SECONDS);
    assert_eq!(world.clock.game_clock(), game_clock - TICK_SECONDS);
    assert_eq!(world.clock.shot_clock(), shot_clock - TICK_SECONDS);
    assert!(matches!(
        world.phase(),
        Phase::PassFlight { .. } | Phase::Live
    ));
}

#[test]
fn an_eligible_missed_final_free_throw_enters_live_rebound_state() {
    let mut world = World::new(
        MatchSetup::builtin(nba_domain::GameRules::default()),
        RuntimeParams::embedded(),
        17,
    );
    let shooter = 0;
    let team = world.players[shooter].team;
    world.possession.team = team;
    world.possession.id = 1;
    world.players[shooter].pos = Vec2::new(70.0, 25.0);
    world.clock.set_shot_clock(9.0);
    let foul_id = world.new_foul_id();
    freethrow::shoot(&mut world, shooter, 1, 1, 0, false, false, foul_id);
    let pending = world.pending_shot.as_mut().expect("free throw pending");
    pending.result = Some(nba_sim_core::resolve::shot::ShotResult::Missed { rim_contact: true });
    let pending = world.pending_shot.take().expect("free throw pending");
    let meta = pending.free_throw.expect("free throw metadata");
    freethrow::finish(&mut world, pending, meta);

    assert!(world.pending_rebound.is_some());
    assert!(world.ball.holder().is_none());
    assert!(matches!(world.phase(), Phase::Live));
    assert_eq!(world.clock.shot_clock(), 14.0);
    assert_eq!(world.possession.id, 1);
}
