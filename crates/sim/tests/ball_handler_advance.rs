//! 持球人的推进落点不受无球绕行里程上限改写。
//!
//! 后场推进时持球人的目标是前场弧顶。编队层的绕行恢复上限只适用于无球
//! 球员：如果它把持球人目标改成当前位置附近，持球人会在中线附近停住，
//! 后场八秒计数继续走到违例。

mod common;

use glam::Vec2;
use nba_sim_core::formation::move_toward_targets;
use nba_sim_core::state::TargetKind;
use nba_sim_core::{MatchSetup, Params, TeamSide};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::world::World;
use nba_domain::rules::GameRules;

fn world_with_advancing_holder(seed: u64) -> (World, usize, Vec2) {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, seed);
    let holder = world.on_court(TeamSide::Home).as_slice()[0];
    world.possession.team = TeamSide::Home;
    world.set_ball_held(holder);
    world.ball.pos = Vec2::new(20.0, 25.0);
    world.players[holder].pos = world.ball.pos;
    let advance = nba_sim_core::formation::top_of_key(&world, TeamSide::Home);
    world.players[holder].target = advance;
    world.players[holder].target_kind = TargetKind::Station;
    world.players[holder].home_spot = advance;
    world.players[holder].schedule.next_redecide_at = world.now() + 1_000.0;
    (world, holder, advance)
}

#[test]
fn the_recovery_path_cap_does_not_rewrite_the_ball_handlers_advance_target() {
    let (mut world, holder, advance) = world_with_advancing_holder(9101);
    let min_separation = world.rules.min_player_separation_ft;
    let target_distance = (advance - world.players[holder].pos).length();
    let cap = target_distance + 2.0 * std::f32::consts::PI * min_separation;
    // 已经进入恢复段，而且绕行里程超过上限：这正是会触发目标改写的状态。
    world.players[holder].recovery_since = world.now() - 1.0;
    world.players[holder].recovery_path_ft = cap + 100.0;

    move_toward_targets(&mut world);

    assert_eq!(
        world.players[holder].target, advance,
        "持球人的推进落点必须保持在前场弧顶"
    );
    assert_eq!(
        world.players[holder].target_kind,
        TargetKind::Station,
        "持球人的推进目标不能被改成原地窗口目标"
    );
    assert!(
        world.players[holder].recovery_path_ft > cap,
        "持球人不进入无球绕行恢复段，里程不应被清零"
    );
}

#[test]
fn the_ball_handler_advances_across_midcourt_within_the_eight_second_limit() {
    let (mut world, holder, _) = world_with_advancing_holder(9102);
    let mid = world.rules.court.width_ft * 0.5;
    let limit = f64::from(world.rules.backcourt_seconds());
    for _ in 0..((limit / nba_sim_core::world::TICK_SECONDS) as usize) {
        world.step(nba_sim_core::world::TICK_SECONDS);
        if world.ball.pos.x > mid {
            break;
        }
    }
    assert!(
        world.ball.pos.x > mid,
        "持球人必须在八秒内把球带过中线，实际球位 x={:.2}",
        world.ball.pos.x
    );
    let _ = holder;
}
