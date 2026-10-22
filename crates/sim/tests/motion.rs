use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_sim_core::defense::{refresh_matchup, solve_defense_targets};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::resolve::move_player;
use nba_sim_core::state::{Phase, MAX_ON_COURT};
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, TeamSide, World};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

/// 推进到有持球人的活球回合，并让站位落定。
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

/// 惯性：速度方向受加速度上限约束，不会一帧之内掉头。
#[test]
fn acceleration_limits_direction_change() {
    let mut world = world(3);
    let handle = live_holder(&mut world);
    let policy = world.params.policy.clone();
    let dt = TICK_SECONDS as f32;
    let speed = world.players[handle].effective_speed(&policy);
    let accel = world.players[handle].effective_acceleration(&policy);
    world.players[handle].vel = Vec2::new(speed, 0.0);
    let before = world.players[handle].vel;
    // 目标放在正后方：允许瞬间转向的实现会让速度立刻变成 -x。
    let target = world.players[handle].pos + Vec2::new(-10.0, 0.0);
    move_player(&mut world.players[handle], target, dt, &policy);
    let after = world.players[handle].vel;
    let change = (after - before).length();
    assert!(
        change <= accel * dt + 1e-3,
        "一帧内的速度变化 {change} 超过加速度上限 {}",
        accel * dt
    );
    assert!(
        after.x > speed * 0.5,
        "速度方向在一帧内翻转：{before:?} -> {after:?}"
    );
}

/// 固定延迟下的协防目标。
fn targets_at(world: &mut World, delay: f32) -> [Vec2; MAX_ON_COURT] {
    world
        .params
        .policy
        .geometry
        .insert("defense_reaction_delay_seconds".to_string(), delay);
    solve_defense_targets(world)
}

/// 延迟从 0 扫到 0.4 秒时，相邻两步之间的最大协防点位移。
fn max_jump(world: &mut World, step: f32) -> f32 {
    let count = (0.4 / step).round() as u32;
    let mut previous = targets_at(world, 0.0);
    let mut worst = 0.0f32;
    for index in 1..=count {
        let current = targets_at(world, step * index as f32);
        for slot in 0..MAX_ON_COURT {
            worst = worst.max((current[slot] - previous[slot]).length());
        }
        previous = current;
    }
    worst
}

/// 防守读球延迟：协防位置取决于延迟之前看到的球位，且随延迟连续变化。
#[test]
fn defensive_read_delay_moves_the_help_target_continuously() {
    let mut world = world(5);
    let carrier = live_holder(&mut world);
    // 让持球人在读球历史里向篮下走：协防只在有篮筐威胁时才启动，
    // 站在外线时无论延迟多少，协防点都不依赖球位。
    let team = world.players[carrier].team;
    let hoop = world.hoop_for(team);
    let forward = if world.attacking_right(team) { 1.0 } else { -1.0 };
    let rim = hoop + Vec2::new(-6.0 * forward, 0.0);
    let start = world.players[carrier].pos;
    for step in 1..=20 {
        if world.holder() != Some(carrier) {
            break;
        }
        let t = step as f32 / 20.0;
        world.players[carrier].pos = start.lerp(rim, t);
        world.step(TICK_SECONDS);
    }
    assert_eq!(world.holder(), Some(carrier), "测试期间持球人发生了变更");
    // 把双方摆成一个会真的启动协防的局面：持球人已到篮下，弱侧
    // 对位人在另一侧篮下附近。协防只对弱侧人生效，站在弧顶的人
    // 无论延迟多少都不会动。
    let mut offense: Vec<usize> = world.on_court(team).as_slice().to_vec();
    offense.retain(|handle| *handle != carrier);
    let spots = [
        hoop + Vec2::new(-3.0 * forward, 14.0),
        hoop + Vec2::new(-22.0 * forward, -18.0),
        hoop + Vec2::new(-24.0 * forward, 0.0),
        hoop + Vec2::new(-22.0 * forward, 18.0),
    ];
    for (index, handle) in offense.iter().enumerate() {
        world.players[*handle].pos = spots[index];
    }
    world.players[carrier].pos = rim;
    // 直接求解防守目标时不会走游戏主循环的对位刷新，这里手动对齐一次。
    // 先清空对位表：刷新在表不旧的时候会直接返回，而旧回合的对位人
    // 可能还都在场上。
    world.possession.matchup = [nba_sim_core::defense::UNASSIGNED; MAX_ON_COURT];
    refresh_matchup(&mut world);
    // 连续性用「步长减半、最大跳变也减半」来判：延迟对协防点的影响
    // 如果是一道阶跃，缩小步长不会让跳变变小。
    let coarse = max_jump(&mut world, 0.1);
    let fine = max_jump(&mut world, 0.05);
    assert!(coarse > 0.0, "读球延迟没有改变任何协防点");
    assert!(
        fine < coarse * 0.75,
        "协防点随读球延迟不连续：步长减半后最大跳变 {fine} ft 没有明显变小（粗步长 {coarse} ft）"
    );
}

/// 势头：得分方累积、被得分方清零，并按秒衰减。
#[test]
fn momentum_builds_on_scoring_and_decays() {
    let mut world = world(7);
    assert_eq!(world.momentum, [0.0, 0.0]);
    world.add_points(TeamSide::Home, 3);
    let hot = world.momentum[0];
    assert!(hot > 0.0, "得分后势头没有累积");
    assert_eq!(world.momentum[1], 0.0, "被得分方势头没有清零");
    world.step(TICK_SECONDS);
    let cooler = world.momentum[0];
    assert!(
        cooler < hot && cooler > 0.0,
        "势头没有连续衰减：{hot} -> {cooler}"
    );
    world.add_points(TeamSide::Away, 2);
    assert_eq!(world.momentum[0], 0.0, "被得分后势头没有清零");
    assert!(world.momentum[1] > 0.0, "得分方势头没有累积");
}

/// 运动参数来自校准表，改表就改行为。
#[test]
fn motion_parameters_are_read_from_the_calibration_table() {
    let mut world = world(9);
    let handle = world.on_court(TeamSide::Home).as_slice()[0];
    let policy = world.params.policy.clone();
    let speed = world.players[handle].effective_speed(&policy);
    let accel = world.players[handle].effective_acceleration(&policy);
    let mut capped = policy.clone();
    capped
        .geometry
        .insert("motion_speed_cap_ftps".to_string(), speed - 1.0);
    assert!(
        world.players[handle].effective_speed(&capped) < speed,
        "速度上限没有从校准表读取"
    );
    let mut lowered = policy.clone();
    lowered
        .geometry
        .insert("motion_accel_cap_ftps2".to_string(), accel - 1.0);
    assert!(
        world.players[handle].effective_acceleration(&lowered) < accel,
        "加速度上限没有从校准表读取"
    );
    let mut softer = policy.clone();
    softer
        .weights
        .insert("motion_stamina_floor".to_string(), 0.2);
    softer
        .weights
        .insert("motion_stamina_span".to_string(), 0.0);
    assert!(
        world.players[handle].effective_speed(&softer) < speed,
        "体能系数没有从校准表读取"
    );
}
