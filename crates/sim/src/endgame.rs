//! 比赛处境：胜率、杠杆与由此产生的末段行为。
//!
//! 这个模块不再提供「第几节剩几秒就做什么」的枚举指令。它只回答两个
//! 问题：现在赢的概率是多少，以及一次动作（犯规、压时间、抢投）会让
//! 这个概率往哪个方向变。行为由效用函数里的连续特征涌现，而不是由
//! 阶跃分支直接下达。

use crate::state::{Leverage, TeamSide};
use crate::world::World;

/// 剩余比赛时间（秒）：本节剩余加上后续各节整节时长。
pub fn remaining_seconds(world: &World) -> f64 {
    let periods = world.rules.league.regulation_periods;
    let left = periods.saturating_sub(world.clock.period);
    world.clock.game_clock + left as f64 * world.clock.period_seconds
}

/// 一次决策时的比赛处境。
///
/// `margin` 是决策方视角的分差（领先为正）；`remaining` 是剩余秒数。
/// τ 与 ω 连续变化，因此第三节末与第四节末的同一种处境给出同一个 γ。
pub fn leverage(world: &World, margin: i32, remaining: f64) -> Leverage {
    let weights = &world.params.policy;
    let horizon = remaining.max(f64::MIN_POSITIVE);
    let possessions = (horizon / weights.w("seconds_per_possession") as f64)
        .max(weights.w("min_remaining_possessions") as f64);
    let tau = margin as f64 / possessions;
    let urgency = (-horizon / weights.g("variance_urgency_seconds") as f64).exp();
    let gamma = -(weights.w("variance_appetite_scale") as f64 * tau.tanh() * urgency) as f32;
    Leverage {
        score_pressure: tau as f32,
        urgency: urgency as f32,
        variance_appetite: gamma,
    }
}

/// 解析胜率：`Φ((分差 + 球权价值) / (波动 × √剩余回合数))`。
///
/// 用 logistic 逼近正态分布函数，系数全部读 `policy.json`；
/// 球权价值按持球方符号计入，所以攻守两侧用的是同一个式子。
pub fn win_probability(world: &World, team: TeamSide, has_ball: bool) -> f64 {
    win_probability_at(
        world,
        team,
        f64::default(),
        remaining_seconds(world),
        has_ball,
    )
}

/// 假想局面的胜率：允许注入分差增量与剩余时间。
///
/// 故意犯规、压时间、抢投三分的判断都在比较「做与不做之后的胜率」，
/// 所以胜率必须能在假想局面上求值，而不是只能读当前比分。
pub fn win_probability_at(
    world: &World,
    team: TeamSide,
    score_delta: f64,
    remaining: f64,
    has_ball: bool,
) -> f64 {
    let index = World::team_index(team);
    let diff = world.score[index] as f64 - world.score[World::team_index(team.other())] as f64
        + score_delta;
    let horizon = remaining.max(f64::MIN_POSITIVE);
    let possessions = (horizon / world.params.policy.w("seconds_per_possession") as f64)
        .max(world.params.policy.w("min_remaining_possessions") as f64);
    let swing = world.params.policy.w("possession_value") as f64;
    // 球权价值只能按「还能用完的几个回合」折算：剩余时间不足一个回合时，
    // 手里的球权换不来一次完整出手。不折算就会得出「时间已经耗尽、
    // 落后方的球权还值一个回合」这种不可能的结果。
    let usable = possessions.min(f64::from(1u8));
    let signed = if has_ball {
        diff + swing * usable
    } else {
        diff - swing * usable
    };
    let spread = world.params.policy.w("wp_sigma") as f64 * possessions.sqrt();
    let z = signed / spread.max(f64::MIN_POSITIVE);
    let scale = world.params.policy.w("wp_logistic_scale") as f64;
    f64::from(1u8) / (f64::from(1u8) + (-scale * z).exp())
}

/// 一次故意犯规的执行计划。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoulPlan {
    /// 执行犯规的防守人。
    pub fouler: usize,
    /// 被犯规的进攻球员（可以是无球人）。
    pub target: usize,
}

/// 本次犯规之后对手实际获得的罚球次数。
///
/// 这条账决定「砍人是否划算」：没进 bonus 的犯规不给罚球，只停表；
/// 最后时段的无球犯规给一罚且球权仍属对手，因此几乎总是不划算。
fn free_throw_award(world: &World, defense: TeamSide, off_ball: bool) -> (f64, bool) {
    let league = &world.rules.league;
    let final_window = league.in_final_window(world.clock.period, world.clock.game_clock);
    if off_ball && final_window {
        // 一罚一掷：罚球次数是 1，且球权不转。
        return (league.away_from_play_free_throws as f64, false);
    }
    let index = World::team_index(defense);
    let after = world.fouls[index].period as u32 + 1;
    let award = league.bonus_free_throws_for(after, world.clock.period, world.clock.game_clock);
    let retains = award == 0;
    (award as f64, retains)
}

/// 防守方是否该故意犯规，返回要执行的犯规计划。
///
/// 判据只有一条：犯与不犯之后的胜率哪个更高。
///
/// - 常规时段：送两次罚球（期望 `2·FT%` 分）比让对手把这次进攻打完
///   （期望一个回合的得分）更亏，于是不犯规。
/// - 最后时刻落后：不犯规意味着时钟被耗光、胜率归零，犯规换回的球权
///   与剩余时间让胜率上升，于是自然涌现。
/// - 砍人战术：对手罚球命中率足够低时，`2·FT%` 低于一个回合的期望得分，
///   对那个人的犯规在常规时段就已经划算。
/// - 无球犯规：最后时段是一罚一掷（球权不转），所以只剩持球人可犯。
pub fn intentional_foul(world: &World) -> Option<FoulPlan> {
    // 只有活球回合才能犯规。`possession.kind` 记录的是回合如何开始
    // （死球发球还是活球抢断），不能用来判断当前是否在打球。
    if !matches!(world.phase, crate::state::Phase::Live) {
        return None;
    }
    let offense = world.possession.team;
    let defense = offense.other();
    let holder = world.holder()?;
    let weights = &world.params.policy;
    let remaining = remaining_seconds(world);
    let procedure = weights.g("foul_procedure_seconds") as f64;
    let retain = weights.w("foul_retain_possession") as f64;
    let ev_offense = weights.w("possession_value") as f64;
    let shot_clock = world.clock.shot_clock;
    let final_window = world
        .rules
        .league
        .in_final_window(world.clock.period, world.clock.game_clock);
    let defenders = world.on_court(defense);

    let mut best: Option<(FoulPlan, f64)> = None;
    for target in world.on_court(offense).as_slice() {
        let off_ball = Some(*target) != Some(holder);
        // 最后时段对无球人犯规是一罚一掷，收益为负，直接不列入候选。
        if off_ball && final_window {
            continue;
        }
        let free_throw = world.players[*target].data.attributes.free_throw as f64;
        let (awards, retains) = free_throw_award(world, defense, off_ball);
        // 犯规之后：对手罚球，比赛过去一次罚球程序的时间。
        // `retains` 为真时球权仍在进攻方，为假时换回自己手上。
        let fouled = win_probability_at(
            world,
            defense,
            -(awards * free_throw),
            remaining - procedure,
            !retains,
        ) * retain;
        // 不犯规：对手把这次进攻打完，随后球权转到自己手上。对手能消耗的
        // 时间不会超过比赛剩余时间：时钟走完就没有球权可拿了。
        let consumed = shot_clock.min(remaining);
        let left = remaining - consumed;
        let played_out = if left <= f64::default() {
            f64::default()
        } else {
            win_probability_at(world, defense, -ev_offense, left, true)
        };
        let gain = fouled - played_out;
        // 故意犯规是一次显式承诺：犯规累计、轮换与判罚差异这些模型之外的
        // 代价会陪到比赛结束。因此门槛随时钟稀缺度升高而降低：终场前
        // 一个千分点的胜率增量就值得犯规，开局阶段同样的增量不值得。
        let defense_margin = world.score[World::team_index(defense)] as i32
            - world.score[World::team_index(offense)] as i32;
        let urgency = leverage(world, defense_margin, remaining).urgency as f64;
        let min_gain = weights.w("foul_min_gain") as f64 * (f64::from(1u8) - urgency);
        if gain <= min_gain {
            continue;
        }
        // 执行人：犯规额度充裕的防守人中离目标最近的一个。
        let limit = world.rules.league.max_personal_fouls as u32;
        let fouler = defenders
            .as_slice()
            .iter()
            .copied()
            .filter(|handle| world.players[*handle].fouls as u32 + 1 < limit)
            .min_by(|left, right| {
                let distance = |handle: usize| {
                    (world.players[handle].pos - world.players[*target].pos).length()
                };
                distance(*left).total_cmp(&distance(*right))
            });
        let Some(fouler) = fouler else {
            continue;
        };
        if best.as_ref().is_none_or(|(_, current)| gain > *current) {
            best = Some((
                FoulPlan {
                    fouler,
                    target: *target,
                },
                gain,
            ));
        }
    }
    best.map(|(plan, _)| plan)
}
