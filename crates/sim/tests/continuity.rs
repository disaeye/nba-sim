use nba_domain::rules::GameRules;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::{BallState, Phase};
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

#[test]
fn every_tick_respects_displacement_and_acceleration_bounds() {
    let mut world = world(23);
    let dt = TICK_SECONDS as f32;
    let max_speed = world.rules.max_player_speed_ftps;
    let max_accel = world.rules.max_player_accel_ftps2;
    let tolerance = 0.01;
    for _ in 0..20_000 {
        let phase_before = world.phase;
        let before: Vec<_> = world
            .players
            .iter()
            .map(|player| (player.on_court, player.pos, player.vel))
            .collect();
        world.step(TICK_SECONDS);
        let inbound = matches!(phase_before, Phase::Inbound { .. })
            || matches!(world.phase, Phase::Inbound { .. });
        for (handle, (was_on, pos, vel)) in before.into_iter().enumerate() {
            let player = &world.players[handle];
            if !was_on || !player.on_court || inbound {
                continue;
            }
            let displacement = (player.pos - pos).length();
            assert!(
                displacement <= max_speed * dt + tolerance,
                "player {handle} moved {displacement:.3} ft in one tick (limit {:.3})",
                max_speed * dt
            );
            let change = (player.vel - vel).length();
            assert!(
                change <= max_accel * dt + tolerance,
                "player {handle} changed velocity by {change:.3} ft/s in one tick (limit {:.3})",
                max_accel * dt
            );
        }
        match world.holder() {
            Some(handle) => {
                assert!(
                    matches!(world.ball.state, BallState::Held),
                    "holder is set but ball is not held"
                );
                assert!(
                    world.players[handle].on_court,
                    "holder {handle} is not on court"
                );
            }
            None => assert!(
                !matches!(world.ball.state, BallState::Held),
                "ball is held but no holder is set"
            ),
        }
        if world.finished {
            break;
        }
    }
}

#[test]
fn frame_projection_has_no_single_tick_jumps() {
    let mut world = world(29);
    let width = world.rules.court.width_ft;
    let height = world.rules.court.height_ft;
    let limit = world.rules.max_player_speed_ftps * TICK_SECONDS as f32 + 0.01;
    let mut previous: std::collections::BTreeMap<String, (f32, f32)> =
        std::collections::BTreeMap::new();
    for _ in 0..5_000 {
        let frame = world.stream_tick().frame;
        if frame.phase.contains("Inbound") {
            previous.clear();
            world.step(TICK_SECONDS);
            continue;
        }
        for player in &frame.players {
            let current = (player.x * width, player.y * height);
            if let Some(old) = previous.insert(player.id.clone(), current) {
                let dx = current.0 - old.0;
                let dy = current.1 - old.1;
                let distance = (dx * dx + dy * dy).sqrt();
                assert!(
                    distance <= limit,
                    "player {} jumped {distance:.3} ft between frames (limit {limit:.3})",
                    player.id
                );
            }
        }
        world.step(TICK_SECONDS);
    }
}
