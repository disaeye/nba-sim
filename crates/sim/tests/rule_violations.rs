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

#[test]
fn a_loose_ball_over_the_sideline_hands_possession_to_the_opponent() {
    let mut world = world(317);
    let handler = world.on_court(TeamSide::Home).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == handler;
        player.vel = Vec2::ZERO;
    }
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.players[handler].pos = Vec2::new(10.0, 25.0);
    world.players[handler].target = world.players[handler].pos;
    // 进攻人把球碰出边线：最后触球是进攻方，按违例交出球权。
    world.last_touch = Some(handler);
    world.ball =
        nba_sim_core::ball::BallBody::loose(Vec2::new(2.0, 25.0), Vec2::new(-12.0, 0.0));

    let mut contact = false;
    for _ in 0..40 {
        let before = world.events.len();
        world.step(TICK_SECONDS);
        for event in &world.events[before..] {
            if matches!(
                &event.fact,
                Fact::BallContact {
                    kind: "out_of_bounds",
                    ..
                }
            ) {
                contact = true;
            }
        }
        if contact {
            break;
        }
    }
    assert!(contact, "球越过边线必须停表");
    assert!(
        world.events.iter().any(|event| matches!(
            &event.fact,
            Fact::Turnover {
                kind: TurnoverKind::OutOfBounds,
                ..
            }
        )),
        "进攻方碰出界必须记违例"
    );
    assert_eq!(world.possession.id, 0, "出界之后旧回合必须结清");
    match world.phase() {
        Phase::DeadBall { team, .. } => assert_eq!(team, TeamSide::Away, "出界后由对方发球"),
        other => panic!("出界后必须停表换边，实际 {other:?}"),
    }
    // 发球地点是出界点：等停表结束，发球人的目标点必须落在边线那一侧。
    for _ in 0..200 {
        if matches!(world.phase(), Phase::Inbound { .. }) {
            break;
        }
        world.step(TICK_SECONDS);
    }
    let passer = world.inbound_passer.expect("发球人必须已确定");
    let target = world.players[passer].target;
    assert!(
        target.x <= 4.0,
        "出界后的发球点必须在边线一侧：{target:?}"
    );
    assert!(
        (target.y - 25.0).abs() <= 3.0,
        "发球点必须留在出界处的纵向位置：{target:?}"
    );
}
