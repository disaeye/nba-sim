use nba_domain::rules::GameRules;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::physical::Physical;
use nba_sim_core::state::Phase;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, TeamSide, World};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

fn live_holder(world: &mut World) -> usize {
    for _ in 0..40_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase, Phase::Live) && world.holder().is_some() {
            let possession = world.possession.id;
            for _ in 0..24 {
                world.step(TICK_SECONDS);
                if world.possession.id != possession {
                    break;
                }
            }
            if let Some(handle) = world.holder() {
                return handle;
            }
        }
    }
    panic!("no live possession with a holder");
}

/// 映射的每一个物理量都由校准表驱动：改表就改量。
#[test]
fn mapping_parameters_come_from_the_calibration_table() {
    let mut world = world(3);
    let handle = live_holder(&mut world);
    let policy = world.params.policy.clone();
    let player = &world.players[handle];
    let before = Physical::of(player, &policy);

    let mut taller = policy.clone();
    taller
        .geometry
        .insert("reach_per_cm_ft".to_string(), policy.g("reach_per_cm_ft") * 2.0);
    let reach = Physical::of(player, &taller).standing_reach_ft;
    assert!(
        reach > before.standing_reach_ft,
        "摸高没有随身高换算系数变化：{} -> {reach}",
        before.standing_reach_ft
    );

    let mut jumpier = policy.clone();
    jumpier
        .geometry
        .insert("vertical_gain_ft".to_string(), policy.g("vertical_gain_ft") * 2.0);
    assert!(
        Physical::of(player, &jumpier).vertical_ft > before.vertical_ft,
        "起跳高度没有随弹跳系数变化"
    );

    let mut slower = policy.clone();
    slower
        .geometry
        .insert("motion_speed_cap_ftps".to_string(), before.speed_ftps - 1.0);
    assert!(
        Physical::of(player, &slower).speed_ftps < before.speed_ftps,
        "速度没有随上限变化"
    );

    let mut baseline = policy.clone();
    baseline.geometry.insert(
        "contest_reach_baseline_ft".to_string(),
        before.contest_reach_ft(&policy) - 0.5,
    );
    assert!(
        Physical::of(player, &baseline).reach_excess_ft(&baseline) > f32::default(),
        "摸高超出基准的部分没有按基准换算"
    );
}

/// 决策层的干扰与结算层的干扰用的是同一个摸高。
#[test]
fn decision_and_resolution_share_the_same_reach() {
    let mut world = world(5);
    let _ = live_holder(&mut world);
    let before_state = world.decision_state();
    let defender = before_state
        .primary_defender
        .expect("a primary defender exists");
    let policy = world.params.policy.clone();
    let physical = Physical::of(&world.players[defender.index], &policy);
    assert!(
        (physical.reach_excess_ft(&policy) - defender.reach_excess_ft).abs() < 1e-6,
        "决策视图里的摸高与物理映射不一致：{} vs {}",
        physical.reach_excess_ft(&policy),
        defender.reach_excess_ft
    );

    let shooter = before_state.actor;
    let pos = before_state.actor_pos;
    let before_resolution = nba_sim_core::resolve::contest_for(&world, shooter, pos);
    let before_openness = before_state.openness();
    world.params.policy.geometry.insert(
        "reach_per_cm_ft".to_string(),
        policy.g("reach_per_cm_ft") * 1.5,
    );
    let after_state = world.decision_state();
    let after_resolution = nba_sim_core::resolve::contest_for(&world, shooter, pos);
    let after_openness = after_state.openness();
    assert!(
        after_openness > before_openness,
        "决策层的干扰没有随摸高上升：{before_openness} -> {after_openness}"
    );
    assert!(
        after_resolution > before_resolution,
        "结算层的干扰没有随摸高上升：{before_resolution} -> {after_resolution}"
    );
}

/// 同样的距离，摸得更高的人干扰更强。
#[test]
fn taller_defender_contests_from_farther() {
    let mut world = world(9);
    let _ = live_holder(&mut world);
    let policy = world.params.policy.clone();
    let handles: Vec<usize> = world
        .on_court(TeamSide::Away)
        .as_slice()
        .iter()
        .copied()
        .collect();
    let mut shortest = handles[0];
    let mut tallest = handles[0];
    for handle in &handles {
        if world.players[*handle].data.height_cm < world.players[shortest].data.height_cm {
            shortest = *handle;
        }
        if world.players[*handle].data.height_cm > world.players[tallest].data.height_cm {
            tallest = *handle;
        }
    }
    assert_ne!(shortest, tallest, "名单里没有身高差，测试无从比较");
    let distance = 4.0_f32;
    let short = nba_sim_core::state::contest_from_defender(
        distance,
        true,
        Physical::of(&world.players[shortest], &policy).reach_excess_ft(&policy),
    );
    let tall = nba_sim_core::state::contest_from_defender(
        distance,
        true,
        Physical::of(&world.players[tallest], &policy).reach_excess_ft(&policy),
    );
    assert!(
        tall > short,
        "更高的防守人在同样距离上干扰没有更强：{short} vs {tall}"
    );
}
