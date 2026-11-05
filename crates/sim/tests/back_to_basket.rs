//! 背身五秒违例（NBA Rule 10 §XVI）。
//!
//! 规则原文：「An offensive player in his frontcourt below the free throw
//! line extended shall not be permitted to dribble with his back or side to
//! the basket for more than five seconds. The count ends when (1) the
//! player picks up the ball, (2) dribbles above the free throw line
//! extended or (3) a defensive player deflects the ball away.」
//!
//! 罚则：球权交给对方，在靠近违例点的罚球线延长线边线发球。
//!
//! 这些测试直接推进 `World::check_back_to_basket`：规则本身是计时判定，
//! 让整场比赛自己凑出背身持球五分钟不现实，计时从哪一刻起算、什么条件
//! 下终止，由这里逐项验证。

mod common;

use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_sim_core::action::{Fact, TurnoverKind};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::{DribbleState, Phase, PossessionKind};
use nba_sim_core::world::{World, TICK_SECONDS};
use nba_sim_core::{MatchSetup, Params, TeamSide};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

/// 持球人站在罚球线延长线以下，防守人在他前面。
///
/// Home 在第一节向右进攻：篮筐在 x=88.75，罚球线延长线在 x=75，罚球线
/// 延长线以下就是 x >= 75 的那一侧。
fn post_fixture(seed: u64) -> (World, usize, usize) {
    let mut world = world(seed);
    let holder = world.on_court(TeamSide::Home).as_slice()[0];
    let defender = world.on_court(TeamSide::Away).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == holder || index == defender;
        player.vel = Vec2::ZERO;
    }
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(holder);
    world.possession.dribble_state = DribbleState::Dribbling;
    let spot = Vec2::new(80.0, 25.0);
    world.players[holder].pos = spot;
    world.players[holder].target = spot;
    world.players[defender].pos = Vec2::new(78.0, 25.0);
    world.players[defender].target = world.players[defender].pos;
    world.ball.pos = spot;
    world.ball.z = world.rules.ball_radius_ft;
    (world, holder, defender)
}

/// 持球人是否被判了背身五秒违例。
fn violated(world: &World) -> bool {
    world.events.iter().any(|event| {
        matches!(
            &event.fact,
            Fact::Turnover {
                kind: TurnoverKind::BackToBasket,
                ..
            }
        )
    })
}

#[test]
fn a_handler_dribbling_with_his_back_to_the_basket_for_five_seconds_is_a_violation() {
    let (mut world, holder, _) = post_fixture(501);
    // 背对篮筐：面朝 -X。
    world.players[holder].facing = Vec2::NEG_X;
    let limit = world.rules.back_to_basket_seconds;
    let mut violation = false;
    for _ in 0..(limit / TICK_SECONDS as f32) as usize + 20 {
        world.check_back_to_basket(TICK_SECONDS);
        if violated(&world) {
            violation = true;
            break;
        }
    }
    assert!(violation, "背身运球满五秒必须判违例");
    // 罚则：停表，对方在罚球线延长线的边线发球。
    match world.phase() {
        Phase::DeadBall { team, .. } => assert_eq!(team, TeamSide::Away, "背身违例后由对方发球"),
        other => panic!("背身违例后必须停表换边，实际 {other:?}"),
    }
    let spot = world
        .inbound_spot
        .expect("背身违例之后必须在罚球线延长线的边线发球");
    let line = world
        .rules
        .court
        .free_throw_line_extended_x(world.attacking_right(TeamSide::Home));
    assert!(
        (spot.x - line).abs() < 0.01,
        "发球点必须在罚球线延长线上，实际 {spot:?}"
    );
}

#[test]
fn a_handler_turning_to_face_the_basket_is_not_a_violation() {
    let (mut world, holder, _) = post_fixture(502);
    // 面朝篮筐：计时不成立。
    world.players[holder].facing = Vec2::X;
    for _ in 0..600 {
        world.check_back_to_basket(TICK_SECONDS);
    }
    assert!(!violated(&world), "面朝篮筐的持球人不受背身五秒限制");
}

#[test]
fn a_handler_dribbling_above_the_free_throw_line_is_not_a_violation() {
    let (mut world, holder, _) = post_fixture(503);
    // 越过罚球线延长线：规则说计时在越过时终止。
    world.players[holder].facing = Vec2::NEG_X;
    world.players[holder].pos = Vec2::new(70.0, 25.0);
    world.players[holder].target = world.players[holder].pos;
    world.ball.pos = world.players[holder].pos;
    for _ in 0..600 {
        world.check_back_to_basket(TICK_SECONDS);
    }
    assert!(
        !violated(&world),
        "罚球线延长线以上的持球人不受背身五秒限制"
    );
}

#[test]
fn a_handler_who_picks_up_the_ball_ends_the_count() {
    let (mut world, holder, _) = post_fixture(504);
    world.players[holder].facing = Vec2::NEG_X;
    // 收球：规则说计时在收球时终止。
    world.possession.dribble_state = DribbleState::Gathered;
    for _ in 0..600 {
        world.check_back_to_basket(TICK_SECONDS);
    }
    assert!(!violated(&world), "收球之后背身五秒的计时终止");
}

#[test]
fn a_deflection_ends_the_count() {
    let (mut world, holder, defender) = post_fixture(505);
    world.players[holder].facing = Vec2::NEG_X;
    // 防守人碰了球：规则说球被碰开时计时终止。
    world.last_touch = Some(defender);
    for _ in 0..600 {
        world.check_back_to_basket(TICK_SECONDS);
    }
    assert!(!violated(&world), "防守人把球碰开之后背身五秒的计时终止");
}
