//! 发球接应位（inbound safety，ui 合并特性）。
//!
//! 真实篮球发球必有接应人回到发球员身边：人选按能力通道
//! （handler_score 次高者，发球员本身是最高者），落位点在发球点
//! 沿场地内侧方向 `inbound_safety_distance_ft`。无状态推导（确定性、
//! 幂等，charter C4）；`inbound_safety_distance_ft = 0` 关闭机制。

use glam::Vec2;

use super::MatchEngine;
use nba_domain::Possession;

/// 发球接应推导：返回（接应人 id，接应落位点）。
/// `inbounder_override` 为 `Some((发球员 id, 发球点))` 时才评估。
pub(crate) fn resolve_inbound_safety(
    engine: &MatchEngine,
    inbounder_override: Option<&(String, Vec2)>,
) -> Option<(String, Vec2)> {
    let (inb_id, inb_pos) = inbounder_override?;
    let dist = engine.config.rules.inbound_safety_distance_ft;
    if dist <= f32::EPSILON {
        return None;
    }
    let is_home_possession = engine.flow.possession == Possession::Home;
    // 场内侧方向：从界外发球点指向场内最近点的单位向量。
    // 发球点在界外（depth>0），clamped 与 inb_pos 不重合，
    // 归一化必然成功。
    let court = engine.config.rules.court;
    let clamped = court.clamp_playable(*inb_pos, 0.0);
    let inward = (clamped - *inb_pos)
        .try_normalize()
        .expect("inbounder release pos is out of bounds, direction to court is nonzero");
    let spot = clamped + inward * dist;
    // 接应人：进攻方除发球员外 handler_score 最高者。
    let attacking_team = if is_home_possession { "home" } else { "away" };
    let spec = if is_home_possession {
        &engine.config.home_offense_spec
    } else {
        &engine.config.away_offense_spec
    };
    let safety_id = engine
        .systems
        .physics
        .get_players()
        .values()
        .filter(|p| p.on_court && p.team == attacking_team && p.id != *inb_id)
        .max_by(|a, b| {
            let sa = nba_decision::tactics::TacticalPlanner::handler_score(spec, &a.attributes);
            let sb = nba_decision::tactics::TacticalPlanner::handler_score(spec, &b.attributes);
            sa.partial_cmp(&sb)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.id.cmp(&a.id))
        })
        .map(|p| p.id.clone())?;
    Some((safety_id, spot))
}
