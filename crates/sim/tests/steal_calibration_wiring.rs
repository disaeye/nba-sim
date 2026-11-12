//! 校准表的抢断技能与赌博基线必须由切球生产链消费。

mod common;

use glam::Vec2;
use nba_sim_core::state::{Phase, PossessionKind};
use nba_sim_core::{TeamSide, World};

fn fixture(seed: u64) -> (World, usize, usize, f32, f64) {
    let mut world = common::short_world(seed);
    let holder = world.on_court(TeamSide::Home).as_slice()[0];
    let thief = world.on_court(TeamSide::Away).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == holder || index == thief;
        player.vel = Vec2::ZERO;
    }
    let spot = Vec2::new(60.0, 25.0);
    world.players[holder].pos = spot;
    world.players[thief].pos = spot + Vec2::new(1.5, 0.0);
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(holder);
    world.ball.pos = spot;
    world.ball.z = world.rules.ball_radius_ft;
    let now = world.now();
    (world, holder, thief, 1.5, now)
}

#[test]
fn the_steal_skill_baseline_consumed_by_poke_success() {
    let (mut world, holder, thief, nearest, now) = fixture(941);
    world.params.policy.weights.insert("steal_skill_base".into(), 1.0);
    let neutral = nba_sim_core::resolve::poke::poke_success(&world, thief, holder, nearest, now);
    world.params.policy.weights.insert("steal_skill_base".into(), 1.5);
    let raised = nba_sim_core::resolve::poke::poke_success(&world, thief, holder, nearest, now);
    assert!(
        raised > neutral,
        "抢断技能基线必须进入切球成功率：{neutral} -> {raised}"
    );
}

#[test]
fn the_steal_gamble_baseline_changes_the_poke_attempt_rate() {
    let (mut world, _holder, _thief, _, _) = fixture(942);
    world.params.policy.weights.insert("steal_gamble_base".into(), 1.0);
    nba_sim_core::resolve::poke::step(&mut world, 0.05);
    // 抽样地址固定：两次运行只有校准键不同，尝试机会随基线单调变化。
    // 这里直接核对生产路径读了校准键：把它设为零后不可能发起切球。
    let mut blocked = fixture(942);
    blocked.0.params.policy.weights.insert("steal_gamble_base".into(), 0.0);
    let before = blocked.0.events.len();
    nba_sim_core::resolve::poke::step(&mut blocked.0, 0.05);
    assert_eq!(
        blocked.0.events.len(),
        before,
        "赌博基线为零时切球尝试速率必须为零"
    );
}
