//! 时间类违例：后场八秒。
//!
//! 规则读 `GameRules::backcourt_seconds`：控球方在后场停满秒数即违例，
//! 停表并把球权交给对方。

use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_sim_core::action::{Fact, TurnoverKind};
use nba_sim_core::params::{workspace_calibration_dir, Params};
use nba_sim_core::state::{Phase, PossessionKind};
use nba_sim_core::world::{World, TICK_SECONDS};
use nba_sim_core::{MatchSetup, TeamSide};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

#[test]
fn a_backcourt_possession_that_never_advances_is_an_eight_second_violation() {
    let mut world = world(307);
    let holder = world.on_court(TeamSide::Home).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == holder;
        player.vel = Vec2::ZERO;
    }
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(holder);
    // Home 向右进攻：x < 中线就是后场。
    world.players[holder].pos = Vec2::new(30.0, 25.0);
    world.players[holder].target = world.players[holder].pos;
    world.ball.pos = world.players[holder].pos;
    world.ball.z = world.rules.ball_radius_ft;
    // 持球人自己不动：不给决策层把它推进前场的机会。
    world.players[holder].schedule.next_redecide_at = world.now() + 1_000.0;

    let mut violation = None;
    for _ in 0..(f64::from(world.rules.backcourt_seconds) / TICK_SECONDS) as usize + 40 {
        let before = world.events.len();
        world.step(TICK_SECONDS);
        for event in &world.events[before..] {
            if matches!(
                &event.fact,
                Fact::Turnover {
                    kind: TurnoverKind::EightSecond,
                    ..
                }
            ) {
                violation = Some(world.now());
            }
        }
        if violation.is_some() {
            break;
        }
    }
    assert!(violation.is_some(), "后场停满八秒必须判违例");
    match world.phase() {
        Phase::DeadBall { team, .. } => assert_eq!(team, TeamSide::Away, "八秒违例后由对方发球"),
        other => panic!("八秒违例后必须停表换边，实际 {other:?}"),
    }
    assert_eq!(
        world.possession.id, 0,
        "违例之后旧回合必须结清"
    );
}
