use crate::state::{PossessionKind, TeamSide};
use crate::world::World;

/// 剩余比赛时间（秒）：本节剩余加上后续各节整节时长。
pub fn remaining_seconds(world: &World) -> f64 {
    let periods = world.rules.league.regulation_periods;
    let left = periods.saturating_sub(world.clock.period);
    world.clock.game_clock + left as f64 * world.clock.period_seconds
}

/// 是否处于末节决胜时段。
pub fn is_clutch(world: &World) -> bool {
    world.clock.period >= world.rules.league.regulation_periods
}

/// 解析胜率：`Φ((分差 + 球权价值) / (波动 × √剩余回合数))`。
///
/// 用 logistic 逼近正态分布函数，系数全部读 `policy.json`；
/// 球权价值按持球方符号计入，所以攻守两侧用的是同一个式子。
pub fn win_probability(world: &World, team: TeamSide, has_ball: bool) -> f64 {
    let diff = world.score[World::team_index(team)] as f64
        - world.score[World::team_index(team.other())] as f64;
    let remaining = remaining_seconds(world).max(f64::MIN_POSITIVE);
    let possessions = (remaining / world.params.policy.w("seconds_per_possession") as f64)
        .max(world.params.policy.w("min_remaining_possessions") as f64);
    let swing = world.params.policy.w("possession_value") as f64;
    let signed = if has_ball { diff + swing } else { diff - swing };
    let spread = world.params.policy.w("wp_sigma") as f64 * possessions.sqrt();
    let z = signed / spread.max(f64::MIN_POSITIVE);
    let scale = world.params.policy.w("wp_logistic_scale") as f64;
    f64::from(1u8) / (f64::from(1u8) + (-scale * z).exp())
}

/// 末节出手约束：领先时压时间，落后三分时必须找三分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndgameShot {
    /// 不额外约束。
    Free,
    /// 只允许三分出手。
    ThreeOnly,
    /// 压时间：进攻时钟没走到阈值就不出手。
    Hold,
}

pub fn shot_constraint(world: &World) -> EndgameShot {
    if !is_clutch(world) || world.possession.kind != PossessionKind::Live {
        return EndgameShot::Free;
    }
    let team = world.possession.team;
    let remaining = remaining_seconds(world);
    let diff = world.score[World::team_index(team)] as i32
        - world.score[World::team_index(team.other())] as i32;
    let shot_clock = world.clock.shot_clock;
    // 落后三分且在最后时段：两分球救不了比赛，只投三分。
    if diff <= -3
        && remaining < world.params.policy.w("endgame_three_required_seconds") as f64
    {
        return EndgameShot::ThreeOnly;
    }
    // 领先且时间还够：把进攻时钟耗到阈值以下再出手。
    if diff > 0
        && diff < world.params.policy.w("endgame_hold_margin_max") as i32
        && remaining < world.params.policy.w("endgame_hold_seconds") as f64
        && shot_clock > world.params.policy.w("endgame_hold_shot_clock") as f64
    {
        return EndgameShot::Hold;
    }
    EndgameShot::Free
}

/// 防守方是否该故意犯规，返回要犯规的防守人。
///
/// 三种情形：落后三分时的抢断式犯规、领先时的「犯规送罚球换球权」、
/// 以及针对罚球极差的持球人的砍人战术。最后两分钟的无球故意犯规会
/// 被判一罚一掷，收益为负，所以那时只允许对持球人犯规。
pub fn intentional_foul(world: &World) -> Option<usize> {
    if !is_clutch(world) || world.possession.kind != PossessionKind::Live {
        return None;
    }
    let offense = world.possession.team;
    let defense = offense.other();
    let holder = world.holder()?;
    let remaining = remaining_seconds(world);
    let defense_diff = world.score[World::team_index(defense)] as i32
        - world.score[World::team_index(offense)] as i32;
    let free_throw = world.players[holder].data.attributes.free_throw;
    let hack_target = free_throw < world.params.policy.w("hack_a_shaq_free_throw") as f32
        && remaining < world.params.policy.w("hack_a_shaq_seconds") as f64;
    let behind = defense_diff < 0
        && defense_diff >= -(world.params.policy.w("endgame_foul_margin") as i32)
        && remaining < world.params.policy.w("endgame_foul_seconds") as f64;
    let should_foul = hack_target || behind;
    if !should_foul {
        return None;
    }
    // 最后两分钟的无球故意犯规是一罚一掷，所以只对持球人下手。
    let off_ball_allowed = remaining > world.params.policy.w("off_ball_foul_cutoff_seconds") as f64;
    if !off_ball_allowed && world.holder() != Some(holder) {
        return None;
    }
    let foul_before = win_probability(world, defense, false);
    let after = win_probability(world, defense, true) * world.params.policy.w("foul_retain_possession") as f64;
    if after <= foul_before {
        return None;
    }
    let defenders = world.on_court(defense);
    defenders
        .as_slice()
        .iter()
        .copied()
        .max_by(|left, right| {
            let distance = |handle: usize| (world.players[handle].pos - world.players[holder].pos).length();
            distance(*right).total_cmp(&distance(*left))
        })
}
