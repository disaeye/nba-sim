//! 规则矩阵补充：加时、节次方向、八秒清除与 FIBA 档案差异。

use glam::Vec2;
use nba_domain::league::LeagueProfile;
use nba_domain::rules::GameRules;
use nba_sim_core::action::TeamSide;
use nba_sim_core::params::{workspace_calibration_dir, Params};
use nba_sim_core::state::{Phase, PossessionKind};
use nba_sim_core::timeouts::TimeoutBudget;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, World};

mod common;

fn short_rules() -> GameRules {
    common::nba_short()
}

#[test]
fn a_tied_regulation_end_enters_overtime() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(short_rules()), params, 601);
    // 打到第四节末尾：短节次把这段压到几秒钟。
    let mut guard = 0;
    while world.clock.period() < short_rules().league.regulation_periods && guard < 400_000 {
        world.step(TICK_SECONDS);
        guard += 1;
    }
    world.score = [77, 77];
    world.set_phase(Phase::DeadBall {
        team: TeamSide::Home,
        ready_at: world.now() + 2.0,
    });
    world.clock.set_game_clock(0.0);
    for _ in 0..60 {
        world.step(TICK_SECONDS);
        if world.clock.period() > short_rules().league.regulation_periods {
            break;
        }
    }
    assert!(
        world.clock.period() > short_rules().league.regulation_periods,
        "平局必须进入加时，实际节次 {}",
        world.clock.period()
    );
    assert!(
        world.clock.shot_clock() <= 24.0 && world.clock.shot_clock() > 0.0,
        "加时开局必须有进攻钟"
    );
}

#[test]
fn fiba_overtime_is_five_minutes() {
    let nba = LeagueProfile::nba().periods();
    let fiba = LeagueProfile::fiba().periods();
    assert!(
        (fiba.overtime_duration_seconds - 300.0).abs() < 0.01,
        "FIBA 加时必须是五分钟，实际 {}",
        fiba.overtime_duration_seconds
    );
    assert!(
        (nba.overtime_duration_seconds - 300.0).abs() < 0.01,
        "NBA 加时必须是五分钟，实际 {}",
        nba.overtime_duration_seconds
    );
    assert_eq!(fiba.regulation_periods, nba.regulation_periods);
}

#[test]
fn overtime_timeouts_follow_the_league_budget() {
    let rules = GameRules::default();
    let budget = TimeoutBudget::new(&rules);
    assert_eq!(
        budget.remaining,
        rules.league.timeouts().timeouts_per_game,
        "开局暂停额度必须等于档案额度"
    );
    assert!(
        rules.league.timeouts().overtime_timeouts > 0,
        "每个加时必须给出暂停额度"
    );
}

#[test]
fn crossing_midcourt_clears_the_eight_second_count() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, 607);
    let holder = world.on_court(TeamSide::Home).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == holder;
        player.vel = Vec2::ZERO;
    }
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(holder);
    let hoop = world.hoop_for(TeamSide::Home);
    let away_hoop = world.hoop_for(TeamSide::Away);
    let midcourt = (hoop.x + away_hoop.x) * 0.5;
    let attacking_right = hoop.x > midcourt;
    // 持球人先在自己的后场，计时开始。
    let backcourt_x = midcourt + if attacking_right { -25.0 } else { 25.0 };
    world.players[holder].pos = Vec2::new(backcourt_x, 25.0);
    world.players[holder].target = world.players[holder].pos;
    world.ball.pos = world.players[holder].pos;
    world.ball.z = world.rules.ball_radius_ft;
    world.clock.set_shot_clock(24.0);
    world.step(TICK_SECONDS);
    assert!(
        world.possession.backcourt_started_game_clock.is_some(),
        "后场持球必须开始八秒计时"
    );
    // 球过中线：计时清除。
    let frontcourt_x = midcourt + if attacking_right { 15.0 } else { -15.0 };
    world.players[holder].pos = Vec2::new(frontcourt_x, 25.0);
    world.players[holder].target = world.players[holder].pos;
    world.ball.pos = world.players[holder].pos;
    world.step(TICK_SECONDS);
    assert!(
        world.possession.backcourt_started_game_clock.is_none(),
        "球过中线之后八秒计时必须清除"
    );
}

#[test]
fn the_second_period_switches_basket_direction() {
    let rules = GameRules::default();
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(rules.clone()), params, 613);
    let first = world.hoop_for(TeamSide::Home);
    // 推到第二节：短节次而不是整场。
    let mut guard = 0;
    while world.clock.period() == 1 && guard < 400_000 {
        world.step(TICK_SECONDS);
        guard += 1;
    }
    let second = world.hoop_for(TeamSide::Home);
    assert!(world.clock.period() >= 2, "必须进入第二节");
    assert!(
        (first.x - second.x).abs() > 1.0,
        "第二节必须换边，主队进攻篮筐 {first:?} → {second:?}"
    );
}
