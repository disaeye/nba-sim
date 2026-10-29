use std::path::{Path, PathBuf};

use glam::Vec2;
use nba_sim_core::action::Fact;
use nba_sim_core::ball;
use nba_sim_core::params::{workspace_root, RuntimeParams};
use nba_sim_core::resolve::{freethrow, rebound};
use nba_sim_core::state::{BallState, Phase};
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
    world.possession.team = TeamSide::Home;
    world.players[handle].pos = Vec2::new(30.0, 20.0);
    world.ball.state = BallState::Loose {
        pos: world.players[handle].pos,
        vel: Vec2::ZERO,
    };
    world.ball.pos = world.players[handle].pos;
    world.ball.z = 4.0;
    world.ball.vel = Vec2::ZERO;
    rebound::resolve(&mut world, "three", 24.0, true);
    assert!(world.pending_rebound.is_some());
    assert!(world.ball.holder.is_none());
    assert!(!world
        .events
        .iter()
        .any(|event| matches!(event.fact, Fact::Rebound { .. })));
    ball::step(&mut world, TICK_SECONDS);
    assert_eq!(world.ball.holder, Some(handle));
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
    freethrow::shoot(&mut world, 0, 1, 1, 0, false, false);
    assert!(world
        .pending_shot
        .as_ref()
        .is_some_and(|pending| pending.free_throw.is_some()));
    assert!(!world
        .events
        .iter()
        .any(|event| matches!(event.fact, Fact::FreeThrow { .. })));
    for _ in 0..200 {
        if !matches!(world.phase, Phase::ShotFlight { .. }) {
            break;
        }
        world.step(TICK_SECONDS);
    }
    assert!(world
        .events
        .iter()
        .any(|event| matches!(event.fact, Fact::FreeThrow { shooter: 0, .. })));
}
