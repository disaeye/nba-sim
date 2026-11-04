//! 后场推进计时：八秒违例与「踢球」重置。
//!
//! NBA Rule 10 §VIII：八秒规则与它的例外。例外一：防守方「踢」球或
//! 拳击球之后，进攻方重新得到完整的八秒。
//!
//! 判定落在 `World::check_backcourt`。

mod common;

use glam::Vec2;
use nba_sim_core::action::{Fact, TurnoverKind};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::{PossessionKind, Phase};
use nba_sim_core::world::{World, TICK_SECONDS};
use nba_sim_core::{MatchSetup, Params, TeamSide};
use nba_domain::rules::GameRules;

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

/// 一个持球人在后场的夹具：Home 向右进攻，x < 中线就是后场。
///
/// 返回 (世界, 持球人, 防守人)。
fn backcourt_fixture(seed: u64) -> (World, usize, usize) {
    let mut world = world(seed);
    let holder = world.on_court(TeamSide::Home).as_slice()[0];
    // 只有持球人在场：决策层没有别的人可传，夹具因此稳定。
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == holder;
        player.vel = Vec2::ZERO;
    }
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(holder);
    world.players[holder].pos = Vec2::new(30.0, 25.0);
    world.players[holder].target = world.players[holder].pos;
    world.ball.pos = world.players[holder].pos;
    world.ball.z = world.rules.ball_radius_ft;
    // 持球人自己不动：不给决策层把它推进前场的机会。
    world.players[holder].schedule.next_redecide_at = world.now() + 1_000.0;
    // 「踢球」的防守人：他不在这份夹具的场上，但他的球队仍然算数。
    let defender = (0..world.players.len())
        .find(|handle| world.players[*handle].team == TeamSide::Away)
        .expect("the defence must have a player");
    (world, holder, defender)
}

#[test]
fn a_backcourt_possession_that_never_advances_is_an_eight_second_violation() {
    let (mut world, _, _) = backcourt_fixture(1501);
    let mut violation = false;
    for _ in 0..(f64::from(world.rules.backcourt_seconds) / TICK_SECONDS) as usize + 40 {
        world.step(TICK_SECONDS);
        if world.events.iter().any(|event| {
            matches!(
                &event.fact,
                Fact::Turnover {
                    kind: TurnoverKind::EightSecond,
                    ..
                }
            )
        }) {
            violation = true;
            break;
        }
    }
    assert!(violation, "后场停满八秒必须判违例");
    match world.phase() {
        Phase::DeadBall { team, .. } => assert_eq!(team, TeamSide::Away, "八秒违例后由对方发球"),
        other => panic!("八秒违例后必须停表换边，实际 {other:?}"),
    }
}

#[test]
fn a_defensive_kick_restarts_the_eight_second_count() {
    let (mut world, holder, defender) = backcourt_fixture(1502);
    // 先让八秒计时走起来。
    world.step(TICK_SECONDS);
    world.step(TICK_SECONDS);
    let started = world
        .possession
        .backcourt_started_game_clock
        .expect("八秒计时必须已经开始");
    // 防守人踢球：球被碰出去，进攻方重新得到完整的八秒（Rule 10 §VIII.1）。
    world.last_touch = Some(defender);
    world.check_backcourt();
    let restarted = world
        .possession
        .backcourt_started_game_clock
        .expect("八秒计时必须仍然在走");
    // 比赛钟是倒数的：重新起算的读数比原来更小。
    assert!(
        restarted < started,
        "防守人踢球之后八秒计时必须重新开始"
    );
    // 重新开始之后，八秒计时从头走：刚踢完球之后的那几秒不得判违例。
    let limit = f64::from(world.rules.backcourt_seconds);
    let mut violation = false;
    // 只走一半的时限：这段时间里不得判违例。
    for _ in 0..((limit / 2.0) / TICK_SECONDS as f64) as usize {
        world.step(TICK_SECONDS);
        if world.events.iter().any(|event| {
            matches!(
                &event.fact,
                Fact::Turnover {
                    kind: TurnoverKind::EightSecond,
                    ..
                }
            )
        }) {
            violation = true;
            break;
        }
    }
    assert!(
        !violation,
        "踢球之后计时重新开始，刚踢完球的那几秒不得判违例"
    );
    // 继续走满剩下的时间：这次必须判违例。
    for _ in 0..((limit / TICK_SECONDS as f64) as usize + 40) {
        world.step(TICK_SECONDS);
        if world.events.iter().any(|event| {
            matches!(
                &event.fact,
                Fact::Turnover {
                    kind: TurnoverKind::EightSecond,
                    ..
                }
            )
        }) {
            violation = true;
            break;
        }
    }
    assert!(
        violation,
        "踢球之后重新走满八秒必须判违例"
    );
    let _ = holder;
}

#[test]
fn crossing_midcourt_stops_the_eight_second_count() {
    let (mut world, holder, _) = backcourt_fixture(1503);
    world.step(TICK_SECONDS);
    assert!(
        world.possession.backcourt_started_game_clock.is_some(),
        "八秒计时必须已经开始"
    );
    // 把球送过中线：计时停。
    world.ball.pos = Vec2::new(60.0, 25.0);
    world.check_backcourt();
    assert!(
        world.possession.backcourt_started_game_clock.is_none(),
        "过中线之后八秒计时必须停"
    );
    let _ = holder;
}

#[test]
fn a_pass_across_midcourt_also_stops_the_eight_second_count() {
    // 八秒与回场都看球的位置：传球越过中线同样算推进。
    let (mut world, holder, _) = backcourt_fixture(1504);
    world.step(TICK_SECONDS);
    assert!(world.possession.backcourt_started_game_clock.is_some());
    // 球在中线另一侧，但持球人还在后场。
    world.ball.pos = Vec2::new(60.0, 25.0);
    world.players[holder].pos = Vec2::new(30.0, 25.0);
    world.check_backcourt();
    assert!(
        world.possession.backcourt_started_game_clock.is_none(),
        "球越过中线之后八秒计时必须停"
    );
}
