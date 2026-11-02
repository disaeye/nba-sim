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

#[test]
fn a_defensive_foul_restarts_the_eight_second_count() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, 617);
    let holder = world.on_court(TeamSide::Home).as_slice()[0];
    let fouler = world.on_court(TeamSide::Away).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == holder || index == fouler;
        player.vel = Vec2::ZERO;
    }
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(holder);
    let hoop = world.hoop_for(TeamSide::Home);
    let away_hoop = world.hoop_for(TeamSide::Away);
    let midcourt = (hoop.x + away_hoop.x) * 0.5;
    let backcourt_x = midcourt + if hoop.x > midcourt { -25.0 } else { 25.0 };
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
    // 防守犯规：被犯规方重新从后场发球，八秒从新时刻算起。
    world.players[fouler].pos = world.players[holder].pos + Vec2::new(0.8, 0.0);
    nba_sim_core::resolve::foul::apply_non_shooting_foul(&mut world, fouler, holder);
    assert!(
        world.possession.backcourt_started_game_clock.is_none(),
        "防守犯规之后八秒计时必须重新开始"
    );
}

#[test]
fn team_period_fouls_reset_at_the_next_period() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(short_rules()), params, 619);
    world.fouls[0].period = 4;
    world.fouls[0].total = 9;
    let period = world.clock.period();
    let mut guard = 0;
    while world.clock.period() == period && guard < 400_000 {
        world.step(TICK_SECONDS);
        guard += 1;
    }
    assert!(world.clock.period() > period, "必须进入下一节");
    assert_eq!(
        world.fouls[0].period, 0,
        "下一节球队犯规计数必须清零"
    );
    assert!(
        world.fouls[0].total >= 9,
        "全场累计犯规不得被清零，实际 {}",
        world.fouls[0].total
    );
}

#[test]
fn a_free_throw_shooter_who_never_releases_is_called_for_delay() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, 631);
    let shooter = world.on_court(TeamSide::Home).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == shooter;
        player.vel = Vec2::ZERO;
    }
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.ball.set_held(shooter);
    // 已经超过联赛时限，而且下一次出手排在很久以后。
    world.free_throw_started = Some(world.now() - 20.0);
    world.set_phase(Phase::FreeThrows {
        shooter,
        total: 2,
        remaining: 2,
        made: 0,
        next_at: world.now() + 100.0,
        and_one: false,
        retain_possession: false,
        foul_id: 7,
        technical_retained_team: None,
        after_technical: None,
    });
    world.step(TICK_SECONDS);
    let team = match world.phase() {
        Phase::DeadBall { team, .. } | Phase::Inbound { team, .. } => team,
        other => panic!("罚球超时必须停表换边，实际 {other:?}"),
    };
    assert_eq!(team, TeamSide::Away, "罚球超时由对方获得球权");
}



#[test]
fn a_pass_nobody_controls_becomes_loose() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, 653);
    let passer = world.on_court(TeamSide::Home).as_slice()[0];
    let receiver = world.on_court(TeamSide::Home).as_slice()[1];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == passer || index == receiver;
        player.vel = Vec2::ZERO;
    }
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(passer);
    let spot = Vec2::new(60.0, 25.0);
    world.players[passer].pos = spot;
    world.players[passer].target = spot;
    world.players[receiver].pos = Vec2::new(66.0, 27.0);
    world.players[receiver].target = world.players[receiver].pos;
    world.ball.pos = spot;
    world.ball.z = world.rules.ball_radius_ft;
    let intent = nba_sim_core::action::ActionIntent {
        actor: passer,
        kind: nba_sim_core::action::ActionKind::Pass(nba_sim_core::action::PassKind::Chest),
        target: Some(receiver),
        from_ft: spot,
        to_ft: world.players[receiver].pos,
        start_time: world.now(),
        duration: 0.3,
    };
    nba_sim_core::resolve::pass::resolve(
        &mut world,
        &intent,
        nba_sim_core::action::PassKind::Chest,
    );
    // 接球人没拿住：这次传球的结果记为未完成。
    world.pass_result = Some(nba_sim_core::ball::PassResult::Incomplete);
    let mut loose = false;
    for _ in 0..180 {
        world.step(TICK_SECONDS);
        if matches!(
            world.ball.state(),
            nba_sim_core::state::BallState::Loose { .. }
        ) {
            loose = true;
            break;
        }
    }
    assert!(loose, "无人接住的传球必须成为松球");
}
