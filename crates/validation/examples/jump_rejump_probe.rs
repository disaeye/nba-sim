//! 诊断停表跳球的接应人到位情况。

use nba_sim_core::action::Fact;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::world::{World, TICK_SECONDS};
use nba_sim_core::{MatchSetup, Params, TeamSide};
use nba_domain::league::LeagueProfile;
use nba_domain::rules::GameRules;

fn nba_world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("params");
    let mut rules = GameRules::default();
    rules.league = LeagueProfile::nba();
    World::new(MatchSetup::builtin(rules), params, seed)
}

fn main() {
    let mut world = nba_world(902);
    let home = world.on_court(TeamSide::Home).as_slice()[0];
    let away = world.on_court(TeamSide::Away).as_slice()[0];
    world.stage_held_ball(home, away);
    let pending = world.pending_jump_ball.as_ref().expect("pending").clone();
    println!("home={} away={} receivers={:?}", pending.home, pending.away, pending.receivers);
    for step in 0..400 {
        world.step(TICK_SECONDS);
        if step % 30 == 0 {
            let pending = world.pending_jump_ball.as_ref();
            println!("step {step}: phase={:?} pending={:?}", world.phase(), pending.map(|p| (p.waited, p.ready_at)));
            if let Some(p) = pending {
                for (handle, spot) in &p.spots {
                    println!(
                        "   handle {handle}: pos={:?} d={:.2} target={:?}",
                        world.players[*handle].pos,
                        (world.players[*handle].pos - *spot).length(),
                        world.players[*handle].target
                    );
                }
            }
        }
        if world
            .events
            .iter()
            .any(|event| matches!(event.fact, Fact::JumpBall { .. }))
        {
            println!("tossed at step {step}");
            break;
        }
    }
    let taps: Vec<_> = world
        .events
        .iter()
        .filter_map(|event| match &event.fact {
            Fact::BallContact { kind, .. } if *kind == "jump_ball_tap" => Some(*kind),
            _ => None,
        })
        .collect();
    println!("tap contacts: {taps:?}");
}
