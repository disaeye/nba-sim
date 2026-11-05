//! 节次开始：发球权归属与交替拥有箭头。
//!
//! NBA Rule 8 §I（节次开始）、Rule 10 §I（球队控制）；FIBA Article 8
//! （比赛时间）与 Article 12.4（交替拥有箭头）。
//!
//! 判定落在 `World::advance_phase` 的节次结束分支与
//! `LeagueProfile::period_starts_with_jump_ball`。

mod common;

use nba_domain::league::LeagueProfile;
use nba_domain::rules::GameRules;
use nba_sim_core::action::Fact;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::Phase;
use nba_sim_core::world::{World, TICK_SECONDS};
use nba_sim_core::{MatchSetup, Params, TeamSide};

fn world_with_profile(seed: u64, league: LeagueProfile) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut rules = GameRules::default();
    rules.league = league;
    World::new(MatchSetup::builtin(rules), params, seed)
}

fn nba_world(seed: u64) -> World {
    world_with_profile(seed, LeagueProfile::nba())
}

fn fiba_world(seed: u64) -> World {
    world_with_profile(seed, LeagueProfile::fiba())
}

/// 把比赛推进到下一节开始之后。
fn advance_to_next_period(world: &mut World) {
    let period = world.clock.period();
    for _ in 0..4_000 {
        world.step(TICK_SECONDS);
        if world.clock.period() > period {
            return;
        }
    }
    panic!("the game never advanced to the next period");
}

#[test]
fn the_nba_opens_the_second_period_with_a_throw_in() {
    let mut world = nba_world(1401);
    // 第一节结束：比赛钟归零。
    world
        .clock
        .set_regulation_clock(1, 0.05, world.clock.shot_clock());
    advance_to_next_period(&mut world);
    assert_eq!(world.clock.period(), 2);
    // NBA 第二节是中线延长线发球，不是跳球。
    assert!(
        !world.rules.league.period_starts_with_jump_ball(2),
        "NBA 第二节不跳球"
    );
}

#[test]
fn the_fiba_opens_later_periods_with_the_alternate_possession_arrow() {
    let mut world = fiba_world(1402);
    assert!(
        world.rules.league.period_starts_with_jump_ball(1),
        "FIBA 第一节跳球"
    );
    for period in 2..=6u32 {
        assert!(
            !world.rules.league.period_starts_with_jump_ball(period),
            "FIBA 第 {period} 节按交替拥有箭头发球，不跳球"
        );
    }
    assert!(
        world.rules.league.use_alternate_possession_arrow,
        "FIBA 使用交替拥有箭头"
    );
}

#[test]
fn the_nba_overtime_periods_start_with_a_jump_ball() {
    let mut world = nba_world(1403);
    assert!(
        world.rules.league.period_starts_with_jump_ball(5),
        "NBA 加时节跳球"
    );
    assert!(
        !world.rules.league.period_starts_with_jump_ball(4),
        "NBA 第四节不跳球"
    );
}

#[test]
fn the_alternate_possession_arrow_advances_each_period() {
    let mut world = fiba_world(1404);
    // 走到第一节结束：箭头指向第一节开球球队的对方。
    world
        .clock
        .set_regulation_clock(1, 0.05, world.clock.shot_clock());
    let arrow_before = world.alternate_possession_next;
    advance_to_next_period(&mut world);
    assert_eq!(
        world.alternate_possession_next,
        arrow_before.other(),
        "每节开始时交替拥有箭头推进到对方"
    );
}

#[test]
fn the_period_start_throw_in_goes_to_the_team_the_arrow_points_at() {
    let mut world = fiba_world(1405);
    // 第一节由跳球决定球权，箭头指向它的对方。
    for _ in 0..2_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::Live) && world.holder().is_some() {
            break;
        }
    }
    let opening_team = world.first_possession.expect("开局跳球必须给出球权");
    assert_eq!(
        world.alternate_possession_next,
        opening_team.other(),
        "箭头指向第一节开球球队的对方"
    );
    world
        .clock
        .set_regulation_clock(1, 0.05, world.clock.shot_clock());
    advance_to_next_period(&mut world);
    // 第二节的发球权交给箭头指向的球队（FIBA Article 12.4）。
    assert_eq!(
        world.period_started_with,
        opening_team.other(),
        "节次开始的发球权必须给箭头指向的球队"
    );
}

#[test]
fn a_period_break_stops_the_game_clock() {
    let mut world = nba_world(1406);
    world
        .clock
        .set_regulation_clock(1, 0.05, world.clock.shot_clock());
    // 归零之后进入节间休息，两钟都停。
    let mut saw_break = false;
    for _ in 0..400 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::PeriodBreak { .. }) {
            saw_break = true;
            break;
        }
    }
    assert!(saw_break, "节末必须进入节间休息");
    let before = world.clock.game_clock();
    world.step(TICK_SECONDS);
    assert_eq!(
        world.clock.game_clock(),
        before,
        "节间休息期间比赛钟不得走动"
    );
}

#[test]
fn the_period_start_records_which_team_opened_the_game() {
    let mut world = nba_world(1407);
    // 第一节由跳球决定球权。
    for _ in 0..2_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::Live) && world.holder().is_some() {
            break;
        }
    }
    let jump = world
        .events
        .iter()
        .find_map(|event| match &event.fact {
            Fact::JumpBall { tapped_team, .. } => Some(*tapped_team),
            _ => None,
        })
        .expect("the opening jump-ball result must be published");
    assert_eq!(
        world.first_possession,
        Some(jump),
        "第一节的球权必须记在开局跳球的球队身上"
    );
}
