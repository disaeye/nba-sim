//! 防守技术动作：伸手切球的执行窗口与收势窗口。
//!
//! 这两个窗口分别改变切球成功率与干扰强度，都由其他结算直接读取；
//! 测试比较的是执行时真正用到的那两个数，而不是事实里是否出现名字。

use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_sim_core::action::{DefensiveActionKind, DefensiveActionRuntime, Fact};
use nba_sim_core::params::{workspace_calibration_dir, Params};
use nba_sim_core::state::{DefenseResponsibility, DribbleState, Phase};
use nba_sim_core::{MatchSetup, TeamSide, World};

mod common;

fn fixture(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

/// 摆出一次持球面对贴防：只有持球人与一名防守人在场上。
fn setup(seed: u64, gap_ft: f32) -> (World, usize, usize) {
    let mut world = fixture(seed);
    let holder = world.on_court(TeamSide::Home).as_slice()[0];
    let defender = world.on_court(TeamSide::Away).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == holder || index == defender;
        player.vel = Vec2::ZERO;
    }
    let hoop = world.hoop_for(TeamSide::Home);
    let spot = hoop + Vec2::new(-24.0, 0.0);
    world.players[holder].pos = spot;
    world.players[holder].target = spot;
    world.players[holder].facing = (hoop - spot).normalize_or_zero();
    let towards_hoop = (hoop - spot).normalize_or_zero();
    let defender_spot = spot + towards_hoop * gap_ft;
    world.players[defender].pos = defender_spot;
    world.players[defender].target = defender_spot;
    world.players[defender].facing = (spot - defender_spot).normalize_or_zero();
    world.players[defender].defense_responsibility = DefenseResponsibility::OnBall;
    world.possession.team = TeamSide::Home;
    world.possession.dribble_state = DribbleState::Dribbling;
    world.possession.ball_phase = nba_sim_core::state::BallPhase::Dribble;
    world.ball.set_held(holder);
    world.ball.pos = spot;
    world.ball.z = world.rules.ball_radius_ft;
    world.set_phase(Phase::Live);
    world.pending_shot = None;
    (world, holder, defender)
}

/// 装一个指定阶段与种类的防守动作运行时。
fn install(
    world: &mut World,
    defender: usize,
    holder: usize,
    kind: DefensiveActionKind,
    phase: nba_domain::action_window::ActionPhase,
) {
    let now = world.now();
    world.defensive_actions[defender] = Some(DefensiveActionRuntime {
        action_id: 1,
        interaction_id: 1,
        kind,
        defender,
        shooter: holder,
        phase,
        preparation_ends_at: now + 0.1,
        execution_ends_at: now + 0.2,
        ends_at: now + 0.5,
        start_position: world.players[defender].pos,
        target_position: world.ball.pos,
        completed: false,
    });
}

#[test]
fn steal_attempt_boosts_poke_success_only_in_its_execution_window() {
    let (mut world, holder, defender) = setup(41, 2.0);
    let nearest = 2.0;
    let now = world.now();
    let base = nba_sim_core::resolve::poke::poke_success(&world, defender, holder, nearest, now);
    assert!(base > 0.0, "场景无效：基础成功率必须为正");

    install(
        &mut world,
        defender,
        holder,
        DefensiveActionKind::StealAttempt,
        nba_domain::action_window::ActionPhase::Execution,
    );
    let executing = nba_sim_core::resolve::poke::poke_success(&world, defender, holder, nearest, now);
    let bonus = world.rules.resolve.technique.steal_attempt_exec_bonus;
    assert!(
        (executing - base * (1.0 + bonus)).abs() < 1e-6,
        "执行窗口的成功率必须按策略加成：{base:.4} -> {executing:.4}"
    );

    install(
        &mut world,
        defender,
        holder,
        DefensiveActionKind::StealAttempt,
        nba_domain::action_window::ActionPhase::Preparation,
    );
    let preparing = nba_sim_core::resolve::poke::poke_success(&world, defender, holder, nearest, now);
    assert!(
        (preparing - base).abs() < 1e-6,
        "准备窗口不该有加成：{base:.4} -> {preparing:.4}"
    );
}

#[test]
fn steal_attempt_follow_through_reduces_the_defender_contest() {
    let (mut world, holder, defender) = setup(43, 2.0);
    let pos = world.players[holder].pos;
    let base = nba_sim_core::resolve::contest_for(&world, holder, pos);
    assert!(base > 0.0, "场景无效：贴防下的干扰必须为正");

    install(
        &mut world,
        defender,
        holder,
        DefensiveActionKind::StealAttempt,
        nba_domain::action_window::ActionPhase::FollowThrough,
    );
    let after = nba_sim_core::resolve::contest_for(&world, holder, pos);
    let penalty = world.rules.resolve.technique.steal_attempt_follow_penalty;
    assert!(
        (after - base * (1.0 - penalty)).abs() < 1e-6,
        "收势窗口必须折减干扰：{base:.4} -> {after:.4}"
    );

    install(
        &mut world,
        defender,
        holder,
        DefensiveActionKind::CloseoutContest,
        nba_domain::action_window::ActionPhase::FollowThrough,
    );
    let closeout = nba_sim_core::resolve::contest_for(&world, holder, pos);
    assert!(
        (closeout - base).abs() < 1e-6,
        "封盖收势不该套用切球惩罚：{base:.4} -> {closeout:.4}"
    );
}

#[test]
fn real_games_run_steal_attempts_through_their_phases() {
    let mut started = 0u32;
    let mut phases = 0u32;
    let mut completed = 0u32;
    for seed in 1..=3u64 {
        let report = common::short_world(seed).run();
        for event in &report.events {
            match &event.fact {
                Fact::DefensiveActionStarted { kind, .. } if *kind == "steal_attempt" => {
                    started += 1;
                }
                Fact::DefensiveActionPhaseChanged { .. } => phases += 1,
                Fact::DefensiveActionCompleted { kind, .. } if *kind == "steal_attempt" => {
                    completed += 1;
                }
                _ => {}
            }
        }
    }
    assert!(started > 0, "真实比赛里防守人必须主动伸手切球");
    assert!(phases > 0, "切球动作必须经过阶段变化");
    assert!(completed > 0, "切球动作必须走完生命周期");
}
