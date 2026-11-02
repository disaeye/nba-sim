//! 传球技术细分：每种技术都有自己的释放几何与防守可及半径。

use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_sim_core::action::PassKind;
use nba_sim_core::params::{workspace_calibration_dir, Params};
use nba_sim_core::state::PossessionKind;
use nba_sim_core::world::World;
use nba_sim_core::{MatchSetup, TeamSide};

mod common;

/// 摆出一次传球：投球人、接球人、以及站在传球路线里的防守人。
fn fixture(seed: u64, gap_ft: f32, along_ft: f32, lateral_ft: f32) -> (World, usize, usize) {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, seed);
    let passer = world.on_court(TeamSide::Home).as_slice()[0];
    let receiver = world.on_court(TeamSide::Home).as_slice()[1];
    let defender = world.on_court(TeamSide::Away).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == passer || index == receiver || index == defender;
        player.vel = Vec2::ZERO;
    }
    let from = Vec2::new(55.0, 25.0);
    world.players[passer].pos = from;
    world.players[passer].target = from;
    world.players[passer].facing = Vec2::X;
    world.players[receiver].pos = from + Vec2::new(gap_ft, 0.0);
    world.players[receiver].target = world.players[receiver].pos;
    // 防守人站在传球路线里：沿传球方向 along_ft，垂直方向偏 lateral_ft。
    world.players[defender].pos = from + Vec2::new(along_ft, lateral_ft);
    world.players[defender].target = world.players[defender].pos;
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(nba_sim_core::state::Phase::Live);
    world.ball.set_held(passer);
    world.ball.pos = from;
    world.ball.z = world.rules.ball_radius_ft;
    (world, passer, receiver)
}

#[test]
fn each_pass_technique_changes_the_completion_probability() {
    // 近距离交接：防守人贴在两人之间。
    let (close, passer, receiver) = fixture(301, 7.0, 3.5, 1.9);
    let handoff = nba_sim_core::value::pass_completion_probability(&close, passer, receiver, PassKind::Handoff);
    let chest_close = nba_sim_core::value::pass_completion_probability(&close, passer, receiver, PassKind::Chest);
    assert!(
        handoff > chest_close,
        "手递手必须比胸前传球更安全：{handoff:.3} vs {chest_close:.3}"
    );

    // 长距离推进：防守人堵在线路上。
    let (far, passer, receiver) = fixture(307, 22.0, 11.0, 2.6);
    let overhead = nba_sim_core::value::pass_completion_probability(&far, passer, receiver, PassKind::Overhead);
    let chest = nba_sim_core::value::pass_completion_probability(&far, passer, receiver, PassKind::Chest);
    let one_hand = nba_sim_core::value::pass_completion_probability(&far, passer, receiver, PassKind::OneHand);
    let behind_back = nba_sim_core::value::pass_completion_probability(&far, passer, receiver, PassKind::BehindBack);
    assert!(
        overhead > chest,
        "头顶传球必须从手上方过去：{overhead:.3} vs {chest:.3}"
    );
    assert!(
        chest > one_hand && one_hand > behind_back,
        "单手推传与背后传球的完成率必须更低：{chest:.3} > {one_hand:.3} > {behind_back:.3}"
    );
}

#[test]
fn real_games_use_several_pass_techniques() {
    let mut counts: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for seed in 1..=4u64 {
        let report = common::short_world(seed).run();
        for event in &report.events {
            if let nba_sim_core::action::Fact::Pass { kind, .. } = &event.fact {
                *counts.entry(format!("{kind:?}")).or_default() += 1;
            }
        }
    }
    assert!(
        counts.len() >= 3,
        "真实比赛必须用到至少三种传球技术，实际 {counts:?}"
    );
    let chest = counts.get("Chest").copied().unwrap_or(0);
    assert!(chest > 0, "胸前传球仍是主要技术");
    eprintln!("pass kinds: {counts:?}");
}
