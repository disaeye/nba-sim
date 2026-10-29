//! 每帧的物理过程：读球历史、势头衰减、移动积分。
//!
//! 这些过程与规则状态机、决策视图分开，免得 `world.rs` 变成什么都往里塞
//! 的容器。

use glam::Vec2;

use crate::world::World;

/// 延迟读球位置：返回 `delay` 秒之前的球的位置。
///
/// 协防轮转取决于「防守人看到球在哪里」，而不是球此刻在哪里；
/// 延迟为 0 时等于当前球位，历史不够长时退到最早的一条。
pub fn delayed_ball_position(world: &World, delay: f64) -> Vec2 {
    let live = world
        .holder()
        .map(|handle| world.players[handle].pos)
        .unwrap_or_else(|| world.hoop_for(world.possession.team));
    if delay <= f64::default() {
        return live;
    }
    let target = world.now() - delay;
    let idx = world.ball_history.partition_point(|(t, _)| *t < target);
    if idx > 0 && idx < world.ball_history.len() {
        let (start, from) = world.ball_history[idx - 1];
        let (end, to) = world.ball_history[idx];
        let span = end - start;
        if span > f64::MIN_POSITIVE {
            return from.lerp(to, ((target - start) / span) as f32);
        }
        return from;
    }
    world
        .ball_history
        .first()
        .map(|(_, pos)| *pos)
        .unwrap_or(live)
}

/// 新回合开始：读球历史从头开始，上一个回合的球位不是这次协防的依据。
pub fn reset_ball_history(world: &mut World) {
    world.ball_history.clear();
}

fn record_ball_history(world: &mut World) {
    let Some(handle) = world.holder() else {
        return;
    };
    let now = world.now();
    let pos = world.players[handle].pos;
    world.ball_history.push((now, pos));
    let keep = world.params.policy.g("ball_history_seconds") as f64;
    let expired = world
        .ball_history
        .iter()
        .take_while(|(at, _)| now - *at > keep)
        .count();
    if expired >= 16 {
        world.ball_history.drain(0..expired);
    }
}

/// 势头按秒衰减：它是一条连续曲线，不是一次性标记。
pub fn decay_momentum(world: &mut World, dt: f64) {
    if world.momentum[0] <= 0.0 && world.momentum[1] <= 0.0 {
        return;
    }
    let decay = (-world.params.policy.g("momentum_decay_per_second") as f64 * dt).exp() as f32;
    world.momentum[0] *= decay;
    world.momentum[1] *= decay;
}

/// 每帧推进：记录读球、衰减势头、把所有在场球员按目标点移动一步。
pub fn advance_movement(world: &mut World, dt: f64) {
    record_ball_history(world);
    decay_momentum(world, dt);
    crate::policy::move_toward_targets(world);
    crate::body::integrate(world, dt);
}
