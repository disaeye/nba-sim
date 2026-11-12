//! 停表跳球：双方同时控制球时的争抢动作与协议投影。
//!
//! 方案 §6.5：篮板归属只能在实际控制成立时确定。点球未控制期间继续争抢。
//! 双方同时控制球时，裁判抛球重新开始，争抢动作因此有一个明确的终态。

mod common;

use glam::Vec2;
use nba_sim_core::action::{Fact, TeamSide};
use nba_sim_core::resolve::rebound::PendingRebound;
use nba_sim_core::state::{Phase, PossessionKind, MAX_ON_COURT};
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};

fn held_ball_world() -> World {
    let params = Params::load(&nba_sim_core::params::workspace_calibration_dir())
        .expect("calibration tables load");
    let mut world = World::new(
        MatchSetup::builtin(nba_domain::rules::GameRules::default()),
        params,
        233,
    );
    let offense = world.on_court(TeamSide::Home).as_slice()[0];
    let defense = world.on_court(TeamSide::Away).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == offense || index == defense;
        player.vel = Vec2::ZERO;
    }
    let ball_spot = Vec2::new(62.0, 25.0);
    // 两人到球的距离只差五分之一英尺：同时够到，谁也没有先控制。
    world.players[offense].pos = ball_spot + Vec2::new(5.0, 0.0);
    world.players[defense].pos = ball_spot + Vec2::new(0.0, 5.05);
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball = nba_sim_core::ball::BallBody::loose(ball_spot, Vec2::ZERO);
    world.ball.z = world.rules.ball_radius_ft;
    world.pending_rebound = Some(PendingRebound {
        offense: TeamSide::Home,
        zone: "test",
        shot_distance: 0.0,
        rim_contact: false,
        started_at: world.now() - 3.0,
        turnover: false,
        poke_by: None,
        pass_by: None,
        box_outs: [usize::MAX; MAX_ON_COURT],
        box_out_count: 0,
    });
    world
}

#[test]
fn a_held_ball_starts_a_dive_action_for_both_players() {
    let mut world = held_ball_world();
    nba_sim_core::ball::step(&mut world, TICK_SECONDS);
    // 双方同时控制：回合按双方控球结清，跳球程序开始。
    assert!(
        world.events.iter().any(|event| matches!(
            &event.fact,
            Fact::PossessionEnded {
                end_reason: nba_domain::event::PossessionEndCause::HeldBall,
                ..
            }
        )),
        "双方同时控制必须按双方控球结清回合"
    );
    assert!(
        world.pending_jump_ball.is_some(),
        "停表后必须进入跳球准备"
    );
    // 两人各自有一个争抢动作。
    let dives = world
        .players
        .iter()
        .filter(|player| {
            player.action.as_ref().is_some_and(|runtime| {
                matches!(
                    runtime.intent.kind,
                    nba_sim_core::action::ActionKind::Rebound(
                        nba_sim_core::action::ReboundKind::HeldBall
                    )
                )
            })
        })
        .count();
    assert_eq!(dives, 2, "双方同时控制时两人各自有一个争抢动作");
}

#[test]
fn the_held_ball_dive_reaches_the_protocol_projection() {
    let mut world = held_ball_world();
    nba_sim_core::ball::step(&mut world, TICK_SECONDS);
    let tick = world.stream_tick();
    let identities: Vec<(String, String, String, String)> = tick
        .frame
        .players
        .iter()
        .flat_map(|player| player.active_actions.iter())
        .map(|action| {
            (
                action.family.clone(),
                action.kind.clone(),
                action.variant.clone(),
                action.role.clone(),
            )
        })
        .collect();
    assert!(
        identities.contains(&(
            "ACTION-REBOUND".to_string(),
            "rebound".to_string(),
            "held_ball".to_string(),
            "primary".to_string(),
        )),
        "双方同时控制的争抢必须出现在协议投影里：{identities:?}"
    );
}
