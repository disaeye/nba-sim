//! 无球拉拽犯规：防守人贴着手放在无球球员身上，被拉住的球员一动，哨子就响。
//!
//! 真实联盟每队每场约 20 次个人犯规，投篮犯规占不到一半，其余多数发生在
//! 球之外：贴防时拉拽、抢位时推人、追无球切入时拽住髋部。投篮犯规、突破
//! 伸手犯规、掩护犯规各有自己的裁定路径，本模块补上「球在别处、两个人贴着
//! 身子互相较劲」这一类。持球人不在此列，他的对位由 `drive` 的伸手犯规处理。

use crate::draw::{purpose, Stream};
use crate::state::Phase;
use crate::world::World;

/// 每个 tick 遍历防守人与其附近无球球员的配对，按相对移动的速度抽样。
pub fn step(world: &mut World, dt: f64) {
    if !matches!(world.phase(), Phase::Live) {
        return;
    }
    let Some(holder) = world.holder() else {
        return;
    };
    let policy = &world.params.policy;
    let contact_ft = policy.w("off_ball_hold_contact_ft");
    let reference = policy.w("off_ball_hold_speed_reference").max(0.001);
    let physicality_base = policy.w("off_ball_hold_physicality_base");
    let rate_base = policy.w("off_ball_hold_rate");
    let penalty_scale = policy.w("off_ball_hold_penalty_scale");
    let offense = world.possession.team;
    let players = world.players.len();
    let mut called = None;
    'outer: for defender in 0..players {
        let guard = &world.players[defender];
        if !guard.on_court || guard.team == offense {
            continue;
        }
        for offender in 0..players {
            let cutter = &world.players[offender];
            if !cutter.on_court || cutter.team != offense || offender == holder {
                continue;
            }
            let gap = guard.pos.distance(cutter.pos);
            if gap > contact_ft {
                continue;
            }
            // 两个人都在往同一方向跑时不构成拉拽；拉扯发生在相对移动里。
            let relative = (cutter.vel - guard.vel).length();
            let exposure = (relative / reference).clamp(0.0, 1.5);
            let physicality = guard.data.tendencies.physicality;
            // 到 bonus 之后防守人收手：同样的对位，拉拽明显变少。
            let discipline = if crate::resolve::foul::bonus_free_throws(world, guard.team) > 0 {
                penalty_scale
            } else {
                1.0
            };
            let rate = f64::from(
                rate_base * (physicality_base + physicality) * exposure * discipline,
            );
            let chance = 1.0 - (-rate * dt).exp();
            if f64::from(world.draw(Stream::Foul, defender, purpose::OFF_BALL_HOLD, 0)) < chance {
                called = Some((defender, offender));
                break 'outer;
            }
        }
    }
    if let Some((fouler, fouled)) = called {
        crate::resolve::foul::apply_non_shooting_foul(world, fouler, fouled);
    }
}
