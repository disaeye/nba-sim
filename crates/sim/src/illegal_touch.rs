//! 非法触球：防守人对持球人伸手阻挡。
//!
//! NBA Rule 12B §I.1：「A player shall not hold, push, charge into, impede
//! the progress of an opponent by extending a hand, arm, leg or knee or by
//! bending the body into a position that is not normal.」
//!
//! §I.2 把防守人对持球人的接触单列出来：这种接触不合法，包括前臂、
//! 手与身体的阻挡。例外是 §I.5.e 的「不影响速度、节奏与平衡的偶然
//! 接触」——贴防时蹭到一下不吹，但伸手把人按住就是犯规。
//!
//! 判定分三层：
//!
//! 1. 距离：防守人在一臂之内（`hand_check_contact_ft`）。
//! 2. 动作：防守人的速度里有一块朝着持球人的分量，说明他不是在平行
//!    贴防，而是在伸手挡路（`hand_check_speed_reference` 量的是这一块
//!    相对速度的量级）。
//! 3. 位置：防守人没有处在合法防守位置——他的身体没有正对持球人
//!    （`facing` 与两人连线的点积小于 `hand_check_facing_dot`）。
//!
//! 三层都成立才算一次非法接触。犯规是在接触发生的那一刻判的，不是
//! 在接触持续期间反复抽样：因此上一次接触的防守人记在世界上，同一个
//! 防守人持续挡路只判一次，他一离开再回来才算新的接触。

use crate::draw::{purpose, Stream};
use crate::world::World;

/// 这次接触的三层条件是否都成立。
///
/// 返回 `Some(defender)` 表示防守人正在非法接触持球人。
pub fn illegal_contact(world: &World) -> Option<usize> {
    if !world.live_play() {
        return None;
    }
    let holder = world.holder()?;
    let contact_ft = world.params.policy.w("hand_check_contact_ft");
    let facing_limit = world.params.policy.g("hand_check_facing_dot");
    let holder_team = world.players[holder].team;
    let holder_pos = world.players[holder].pos;

    // 持球人的对位防守人：离他最近的那个。远处的防守人够不到他。
    let defender = (0..world.players.len())
        .filter(|index| {
            let player = &world.players[*index];
            player.on_court && player.team != holder_team
        })
        .min_by(|a, b| {
            world.players[*a]
                .pos
                .distance(holder_pos)
                .total_cmp(&world.players[*b].pos.distance(holder_pos))
        })?;

    let guard = &world.players[defender];
    let to_holder = holder_pos - guard.pos;
    let gap = to_holder.length();
    if gap > contact_ft || gap <= f32::EPSILON {
        return None;
    }
    let direction = to_holder / gap;

    // 防守人朝持球人的速度分量：平行贴防时这一块接近零，伸手挡路时
    // 明显为正。
    let closing = (guard.vel - world.players[holder].vel).dot(direction);
    if closing <= 0.0 {
        return None;
    }

    // 合法防守位置：防守人的身体正对着持球人。身体压过去或者背对着
    // 持球人伸手，都不是合法位置（Rule 12B §I.2）。
    if guard.facing.normalize_or_zero().dot(direction) >= facing_limit {
        return None;
    }
    Some(defender)
}

/// 判定一次新的非法接触并吹哨。
///
/// 返回被吹哨的防守人；没有成立时返回 `None`。
pub fn judge(world: &mut World, dt: f64) -> Option<usize> {
    let Some(defender) = illegal_contact(world) else {
        world.hand_check_active = None;
        return None;
    };
    // 同一个防守人持续挡路只判一次。
    if world.hand_check_active == Some(defender) {
        return None;
    }
    world.hand_check_active = Some(defender);

    let policy = &world.params.policy;
    let reference = policy.w("hand_check_speed_reference").max(0.001);
    let rate_base = policy.w("hand_check_rate");
    let physicality_base = policy.w("hand_check_physicality_base");
    let penalty_scale = policy.w("hand_check_penalty_scale");
    let holder = world.holder()?;

    let guard_pos = world.players[defender].pos;
    let holder_pos = world.players[holder].pos;
    let to_holder = holder_pos - guard_pos;
    let gap = to_holder.length().max(f32::EPSILON);
    let direction = to_holder / gap;
    let closing = (world.players[defender].vel - world.players[holder].vel).dot(direction);
    let exposure = (closing / reference).clamp(0.0, 1.5);

    let physicality = world.players[defender].data.tendencies.physicality;
    // 到 bonus 之后防守人收手：同样的对位，伸手阻挡明显变少。
    let discipline = if crate::resolve::foul::bonus_free_throws(world, world.players[defender].team)
        > 0
    {
        penalty_scale
    } else {
        1.0
    };
    let rate = f64::from(rate_base * (physicality_base + physicality) * exposure * discipline);
    let chance = 1.0 - (-rate * dt).exp();
    if f64::from(world.draw(Stream::Foul, defender, purpose::HAND_CHECK, 0)) < chance {
        return Some(defender);
    }
    None
}

/// 一臂之内的距离：防守人伸手能够到持球人的范围。
pub fn contact_distance(world: &World) -> f32 {
    world.params.policy.w("hand_check_contact_ft")
}

/// 防守人朝持球人的速度分量，供诊断使用。
pub fn closing_speed(world: &World, defender: usize, holder: usize) -> f32 {
    let guard = &world.players[defender];
    let holder = &world.players[holder];
    let to_holder = holder.pos - guard.pos;
    let gap = to_holder.length();
    if gap <= f32::EPSILON {
        return 0.0;
    }
    (guard.vel - holder.vel).dot(to_holder / gap)
}

/// 防守人的身体是否正对持球人。合法防守位置要求它成立。
pub fn in_legal_guarding_position(world: &World, defender: usize, holder: usize) -> bool {
    let guard = &world.players[defender];
    let holder = &world.players[holder];
    let to_holder = holder.pos - guard.pos;
    let gap = to_holder.length();
    if gap <= f32::EPSILON {
        return false;
    }
    let limit = world.params.policy.g("hand_check_facing_dot");
    guard.facing.normalize_or_zero().dot(to_holder / gap) >= limit
}
