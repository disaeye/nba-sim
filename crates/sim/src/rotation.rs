use nba_domain::rules::GameRules;

use crate::state::{Phase, TeamSide};
use crate::world::World;

/// 体能的消耗与恢复。
///
/// 在场上消耗，坐在场下与停表期间恢复；两者都用 `policy.json` 的系数，
/// 这样轮换才有真实的时间尺度：一场 48 分钟的比赛里，首发会掉到替换阈值，
/// 替补休息够久会回到满体能。
pub fn update_stamina(world: &mut World, dt: f64) {
    let drain_rate = world.params.policy.w("stamina_drain_rate");
    let bench_recovery = world.params.policy.w("stamina_bench_recovery");
    let dead_ball_recovery = world.params.policy.w("stamina_dead_ball_recovery");
    let stopped = matches!(
        world.phase,
        Phase::FreeThrows { .. }
            | Phase::DeadBall { .. }
            | Phase::PeriodBreak { .. }
            | Phase::Inbound { .. }
    );
    for player in world.players.iter_mut() {
        if player.on_court {
            if stopped {
                player.stamina = (player.stamina + dead_ball_recovery * dt as f32).min(f32::from(1u8));
            } else {
                let drain = drain_rate * (f32::from(1u8) - player.data.attributes.stamina);
                player.stamina = (player.stamina - drain * dt as f32).clamp(f32::default(), f32::from(1u8));
            }
        } else {
            player.stamina = (player.stamina + bench_recovery * dt as f32).min(f32::from(1u8));
        }
    }
}

/// 剩余比赛时间（秒）：本节的剩余时间加上后续各节的整节时间。
fn remaining_seconds(world: &World, rules: &GameRules) -> f64 {
    let periods = rules.league.regulation_periods;
    let left = periods.saturating_sub(world.clock.period);
    world.clock.game_clock + left as f64 * world.clock.period_seconds
}

/// 需要换下的判定：犯满、体能、犯规危机、垃圾时间。
fn needs_rest(world: &World, handle: usize, rules: &GameRules) -> bool {
    let player = &world.players[handle];
    if player.disqualified {
        return true;
    }
    if player.stamina < world.params.policy.w("stamina_rest_threshold") {
        return true;
    }
    let foul_limit = rules.league.max_personal_fouls;
    if player.fouls >= foul_limit {
        return true;
    }
    // 犯规危机只针对首发：一节两犯、两节三犯、三节四犯都要先坐下来。
    if player.data.starter {
        let threshold = match world.clock.period {
            1 => world.params.policy.w("foul_trouble_first") as u8,
            2 => world.params.policy.w("foul_trouble_second") as u8,
            3 => world.params.policy.w("foul_trouble_third") as u8,
            _ => foul_limit,
        };
        if player.fouls >= threshold {
            return true;
        }
    }
    let remaining = remaining_seconds(world, rules);
    // 垃圾时间只让首发下来：它是全队语境，不是每名球员都需要休息。
    if player.data.starter && remaining < world.params.policy.w("garbage_time_seconds") as f64 {
        let margin = (world.score[0] as f64 - world.score[1] as f64).abs();
        let threshold = world.params.policy.w("garbage_margin_scale") as f64 * remaining.sqrt()
            + world.params.policy.w("garbage_margin_base") as f64;
        if margin > threshold {
            return true;
        }
    }
    false
}

fn best_bench(world: &World, team: TeamSide, handle: usize, rules: &GameRules) -> Option<usize> {
    let foul_limit = rules.league.max_personal_fouls;
    (0..world.players.len())
        .filter(|candidate| {
            let player = &world.players[*candidate];
            player.team == team
                && !player.on_court
                && player.fouls < foul_limit
                && !player.disqualified
                // 替补本身也需要休息时不能顶上去，否则两人会来回互换。
                && !needs_rest(world, *candidate, rules)
        })
        .max_by(|left, right| {
            let slot_penalty = |candidate: usize| {
                if world.players[candidate].slot == world.players[handle].slot {
                    f32::default()
                } else {
                    f32::from(1u8)
                }
            };
            let score = |candidate: usize| world.players[candidate].stamina - slot_penalty(candidate);
            score(*left).partial_cmp(&score(*right)).expect("finite stamina")
        })
}

/// 执行换人：替补继承被换下者的位置、目标与槽位。
fn swap(world: &mut World, team: TeamSide, outgoing: usize, incoming: usize) {
    let position = world.players[outgoing].pos;
    let target = world.players[outgoing].target;
    let slot = world.players[outgoing].slot;
    world.players[incoming].on_court = true;
    world.players[incoming].pos = position;
    world.players[incoming].target = target;
    world.players[incoming].slot = slot;
    world.players[outgoing].on_court = false;
    if world.holder() == Some(outgoing) {
        world.ball = crate::state::BallState::Held(incoming);
    }
    world.emit(crate::action::Fact::Substitution {
        team,
        incoming,
        outgoing,
    });
}

/// 轮换：逐个把需要休息的在场球员换成替补。
///
/// 每次调用最多换一轮在场人数，避免换上来的人又立即满足休息条件时
/// 在同一个调用里反复交换。
pub fn substitute_if_needed(world: &mut World, rules: &GameRules) {
    for team in [TeamSide::Home, TeamSide::Away] {
        let mut budget = crate::state::MAX_ON_COURT;
        while budget > 0 {
            budget -= 1;
            let on_court = world.on_court(team);
            let mut resting = None;
            for handle in on_court.as_slice() {
                if needs_rest(world, *handle, rules) {
                    resting = Some(*handle);
                    break;
                }
            }
            let Some(outgoing) = resting else {
                break;
            };
            let Some(incoming) = best_bench(world, team, outgoing, rules) else {
                break;
            };
            swap(world, team, outgoing, incoming);
        }
    }
}

/// 记一次犯规并登记犯满离场。
pub fn record_foul(world: &mut World, handle: usize) {
    let limit = world.rules.league.max_personal_fouls;
    let player = &mut world.players[handle];
    player.fouls = player.fouls.saturating_add(1);
    if player.fouls >= limit {
        player.disqualified = true;
    }
}
