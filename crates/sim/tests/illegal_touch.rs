//! 非法触球（hand-check）：防守人对持球人伸手阻挡。
//!
//! NBA Rule 12B §I.1 与 §I.2：防守人不得用手臂阻挡持球人，防守人对
//! 持球人的接触不合法；§I.5.e 的例外是「不影响速度、节奏与平衡的
//! 偶然接触」。
//!
//! 判定在 `nba_sim_core::illegal_touch` 里，因此这些测试直接推进那一层：
//! 距离、速度方向与合法防守位置三层条件逐项验证，而不是靠整场比赛里
//! 偶发的一次吹哨来说明规则存在。

mod common;

use glam::Vec2;
use nba_sim_core::illegal_touch;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::{PossessionKind, Phase};
use nba_sim_core::world::{World, TICK_SECONDS};
use nba_sim_core::{MatchSetup, Params, TeamSide};
use nba_domain::rules::GameRules;

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

/// 一个持球人与他的对位防守人，两人都在前场。
///
/// 持球人在 x=70，防守人在 x=68，所以「从防守人指向持球人」的方向是
/// +X：防守人面向 +X 才是合法防守位置。
fn matchup_fixture(seed: u64) -> (World, usize, usize) {
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
    world.players[holder].pos = Vec2::new(70.0, 25.0);
    world.players[holder].target = world.players[holder].pos;
    world.players[holder].facing = Vec2::X;
    world.players[defender].pos = Vec2::new(68.0, 25.0);
    world.players[defender].target = world.players[defender].pos;
    world.players[defender].facing = Vec2::X;
    world.ball.pos = world.players[holder].pos;
    world.ball.z = world.rules.ball_holder_height_ft;
    (world, holder, defender)
}

#[test]
fn a_defender_who_reaches_across_the_handler_commits_the_illegal_touch() {
    let (world, holder, defender) = matchup_fixture(401);
    // 防守人贴得比合法间隔更近，而且速度朝着持球人：他在往持球人身上压。
    let mut world = world;
    world.players[defender].pos = world.players[holder].pos - Vec2::X * 0.8;
    world.players[defender].vel = Vec2::new(4.0, 0.0);
    world.players[holder].vel = Vec2::ZERO;
    assert!(
        illegal_touch::pressing_past_legal_separation(&world, defender, holder),
        "夹具必须让防守人守得比合法间隔更近"
    );
    assert!(
        illegal_touch::closing_toward_holder(&world, defender, holder) > 0.0,
        "夹具必须让防守人朝持球人移动"
    );
    assert_eq!(
        illegal_touch::illegal_contact(&world),
        Some(defender),
        "伸手阻挡的三层条件必须都成立"
    );
}

#[test]
fn a_defender_in_a_legal_guarding_position_commits_no_illegal_touch() {
    let (mut world, holder, defender) = matchup_fixture(402);
    // 平行贴防：守在职责层给的间隔上，速度只是横向的。
    world.players[defender].pos = world.players[holder].pos - Vec2::X * 2.5;
    world.players[defender].vel = Vec2::new(0.0, 3.0);
    world.players[holder].vel = Vec2::ZERO;
    assert!(
        !illegal_touch::pressing_past_legal_separation(&world, defender, holder),
        "夹具必须让防守人守在合法间隔上"
    );
    assert_eq!(
        illegal_touch::illegal_contact(&world),
        None,
        "合法间隔上的平行贴防不是非法触球"
    );
}

#[test]
fn a_defender_out_of_arm_reach_commits_no_illegal_touch() {
    let (mut world, holder, defender) = matchup_fixture(403);
    world.players[defender].vel = Vec2::new(4.0, 0.0);
    world.players[holder].vel = Vec2::ZERO;
    // 防守人退到一臂之外：伸手够不到，不构成阻挡。
    world.players[defender].pos = Vec2::new(
        70.0 - illegal_touch::contact_distance(&world) - 1.0,
        25.0,
    );
    assert_eq!(
        illegal_touch::illegal_contact(&world),
        None,
        "够不到的防守人不会被判非法触球"
    );
}

#[test]
fn a_defender_who_backs_off_stops_being_an_illegal_touch() {
    let (mut world, holder, defender) = matchup_fixture(404);
    world.players[defender].pos = world.players[holder].pos - Vec2::X * 0.8;
    world.players[defender].vel = Vec2::new(4.0, 0.0);
    world.players[holder].vel = Vec2::ZERO;
    assert_eq!(illegal_touch::illegal_contact(&world), Some(defender));
    // 防守人退开并且不再朝持球人移动：接触结束。
    world.players[defender].vel = Vec2::new(-2.0, 0.0);
    world.players[defender].pos = world.players[holder].pos - Vec2::X * 2.5;
    assert_eq!(
        illegal_touch::illegal_contact(&world),
        None,
        "防守人退开之后不再是非法接触"
    );
}

#[test]
fn the_hand_check_is_booked_as_a_personal_foul_in_a_live_game() {
    let mut world = common::short_world(405);
    // 先跑到活球有人持球的状态。
    let mut holder = None;
    for _ in 0..40_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::Live) {
            if let Some(handle) = world.holder() {
                holder = Some(handle);
                break;
            }
        }
    }
    let holder = holder.expect("the engine never reached a live ball state");
    let team = world.players[holder].team;
    let defender = (0..world.players.len())
        .filter(|index| {
            let player = &world.players[*index];
            player.on_court && player.team != team
        })
        .min_by(|a, b| {
            world.players[*a]
                .pos
                .distance(world.players[holder].pos)
                .total_cmp(&world.players[*b].pos.distance(world.players[holder].pos))
        })
        .expect("the defence must have an on-court player");
    // 防守人贴上来并且身体没有正对持球人。
    let toward = (world.players[holder].pos - world.players[defender].pos).normalize_or_zero();
    world.players[defender].pos = world.players[holder].pos - toward * 2.0;
    world.players[defender].facing = Vec2::NEG_Y;
    world.players[defender].vel = toward * 5.0;
    let fouls_before = world.fouls[World::team_index(team.other())].total;
    let mut booked = false;
    for _ in 0..600 {
        world.step(TICK_SECONDS);
        if world.fouls[World::team_index(team.other())].total > fouls_before {
            booked = true;
            break;
        }
    }
    assert!(booked, "非法触球必须记为防守方的个人犯规");
}
