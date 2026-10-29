//! S8c 投篮模型不变量。
//!
//! 四条判据：
//! 1. 三分线外的命中概率连续衰减，并在超远距离趋近于零，不用统一下界
//!    顶住。
//! 2. 超远距离的三分命中概率就是零，不是 0.02 之类的下界。
//! 3. 价值模型按候选自己的出手类型与动作阶段估值，同一位置的不同候选
//!    得到不同的价值。
//! 4. 末段是否抢投三分由比分、剩余时间与暂停情况决定，不由出手距离
//!    决定。

use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_sim_core::action::ShotKind;
use nba_sim_core::params::{workspace_calibration_dir, Params};
use nba_sim_core::state::{BallPhase, BallState, Phase};
use nba_sim_core::{MatchSetup, TeamSide, World};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

fn shooter(world: &mut World) -> usize {
    let handle = world.on_court(TeamSide::Home).as_slice()[0];
    for player in world.players.iter_mut() {
        player.on_court = false;
        player.vel = Vec2::ZERO;
    }
    world.players[handle].on_court = true;
    world.players[handle].stamina = 1.0;
    world.possession.team = TeamSide::Home;
    world.phase = Phase::Live;
    world.ball.set_held(handle);
    handle
}

#[test]
fn three_distance_decay_is_continuous_and_reaches_zero() {
    let mut world = world(31);
    let handle = shooter(&mut world);
    let hoop = world.hoop_for(TeamSide::Home);
    let arc = world.rules.league.three_point_distance_ft;
    let at_arc = nba_sim_core::shot_quality::three_distance_decay(&world, arc);
    assert!(
        (at_arc - 1.0).abs() < 1.0e-6,
        "the decay must start at one on the arc, got {at_arc}"
    );
    // 连续：距离每增加一英尺，衰减单调不增。
    let mut previous = 1.0f32;
    let mut distance = arc;
    while distance <= arc + 40.0 {
        let decay = nba_sim_core::shot_quality::three_distance_decay(&world, distance);
        assert!(
            decay <= previous + 1.0e-6,
            "the decay must not increase with distance at {distance} ft"
        );
        previous = decay;
        distance += 1.0;
    }
    // 超远距离趋近于零。
    let far = nba_sim_core::shot_quality::three_distance_decay(&world, 90.0);
    assert!(
        far < 1.0e-6,
        "a 90 ft heave must decay to zero, got {far}"
    );
    let _ = handle;
    let _ = hoop;
}

#[test]
fn a_heave_has_no_make_probability() {
    let mut world = world(32);
    let handle = shooter(&mut world);
    let hoop = world.hoop_for(TeamSide::Home);
    let spot = hoop + Vec2::new(-80.0, 0.0);
    world.players[handle].pos = spot;
    world.possession.ball_phase = BallPhase::Dribble;
    let model = nba_sim_core::shot_quality::evaluate(
        &world,
        handle,
        ShotKind::PullUp,
        BallPhase::Dribble,
        spot,
    );
    assert!(
        model.p_make < 1.0e-4,
        "an 80 ft heave must not carry a make probability, got {}",
        model.p_make
    );
}

#[test]
fn shot_value_follows_the_candidate_kind_and_phase() {
    let mut world = world(33);
    let handle = shooter(&mut world);
    let hoop = world.hoop_for(TeamSide::Home);
    let spot = hoop + Vec2::new(-22.0, 0.0);
    world.players[handle].pos = spot;
    world.possession.ball_phase = BallPhase::Catch;
    let catch_shoot = nba_sim_core::value::shot_value(
        &world,
        handle,
        spot,
        ShotKind::CatchShoot,
        BallPhase::Catch,
    );
    let pull_up = nba_sim_core::value::shot_value(
        &world,
        handle,
        spot,
        ShotKind::PullUp,
        BallPhase::Dribble,
    );
    assert!(
        (catch_shoot - pull_up).abs() > 1.0e-4,
        "the value must follow the candidate kind and phase, got {catch_shoot} and {pull_up}"
    );
    // 同一个候选重复估值必须稳定。
    let again = nba_sim_core::value::shot_value(
        &world,
        handle,
        spot,
        ShotKind::CatchShoot,
        BallPhase::Catch,
    );
    assert!((catch_shoot - again).abs() < 1.0e-6);
}

#[test]
fn the_endgame_three_decision_follows_score_time_and_timeouts() {
    let mut world = world(34);
    let handle = shooter(&mut world);
    let hoop = world.hoop_for(TeamSide::Home);
    let spot = hoop + Vec2::new(-40.0, 0.0);
    world.players[handle].pos = spot;
    world.possession.ball_phase = BallPhase::Dribble;
    world.ball.release(BallState::Loose {
        pos: spot,
        vel: Vec2::ZERO,
    });
    world.ball.set_held(handle);
    // 时间充足、领先、还有暂停时不抢投。
    world.clock.period = 4;
    world.clock.game_clock = 300.0;
    world.score = [9, 3];
    world.timeouts[World::team_index(TeamSide::Home)].remaining = 2;
    let calm = shot_candidate_context(&world, handle);
    // 时间将尽、落后、没有暂停时，同一个远距离三分必须更值钱。
    world.clock.game_clock = 4.0;
    world.score = [3, 8];
    world.timeouts[World::team_index(TeamSide::Home)].remaining = 0;
    let desperate = shot_candidate_context(&world, handle);
    assert!(
        desperate > calm,
        "a trailing team with no timeouts must value the long three higher: {desperate} vs {calm}"
    );
}

/// 出手候选里的语境项：接球就投、补篮、掩护窗口与末段抢投都在这里。
fn shot_candidate_context(world: &World, handle: usize) -> f32 {
    let state = world.decision_state();
    debug_assert_eq!(state.actor, handle);
    nba_sim_core::policy::candidate_utilities(world, &state)
        .into_iter()
        .find(|(kind, _, _, _)| matches!(kind, nba_sim_core::action::ActionKind::Shoot(_)))
        .map(|(_, _, _, utility)| utility.epv.context)
        .expect("a shot candidate must exist")
}
