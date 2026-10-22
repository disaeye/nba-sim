use crate::state::Phase;
use crate::world::World;
use crate::TeamSide;
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
            | Phase::Timeout { .. }
    );
    for player in world.players.iter_mut() {
        if player.on_court {
            if stopped {
                player.stamina =
                    (player.stamina + dead_ball_recovery * dt as f32).min(f32::from(1u8));
            } else {
                let drain = drain_rate * (f32::from(1u8) - player.data.attributes.stamina);
                player.stamina =
                    (player.stamina - drain * dt as f32).clamp(f32::default(), f32::from(1u8));
            }
        } else {
            player.stamina = (player.stamina + bench_recovery * dt as f32).min(f32::from(1u8));
        }
    }
}

/// 阵容只能在比赛停表时变更：死球、发球、罚球、暂停、节间。
///
/// 活球期间教练只能登记意图，不能把人换上来。这条门控是
/// 「主力身背 4 犯却因为两分钟没有死球而下不了场」这类真实博弈的前提。
pub fn is_substitution_window(world: &World) -> bool {
    matches!(
        world.phase,
        Phase::DeadBall { .. }
            | Phase::Inbound { .. }
            | Phase::FreeThrows { .. }
            | Phase::PeriodBreak { .. }
            | Phase::Timeout { .. }
    )
}

/// 需要换下的紧迫度：数值越大越应该先下场，`None` 表示不需要休息。
///
/// 体能门槛随场上胶着程度下降：胜率接近五成时，主力的产出远重于
/// 多歇两分钟，于是能忍受到更低的体能值；一边倒时门槛回到基准。
fn rest_priority(world: &World, handle: usize) -> Option<f32> {
    let rules = &world.rules;
    let policy = &world.params.policy;
    let player = &world.players[handle];
    if player.disqualified || player.fouls >= rules.league.max_personal_fouls {
        return Some(f32::MAX);
    }
    let has_ball = world.possession.team == player.team;
    let wp = crate::endgame::win_probability(world, player.team, has_ball) as f32;
    // 刚换上场的人不能立刻被换下去：否则替补上去一个回合就被当成
    // 「需要休息」再换下来，窗口配额在来回拉锯中耗尽。开场就在场上的人
    // 不受这条限制，`entered_at` 为零就是「从开场就没下过」。
    let settle = policy.w("rotation_settle_seconds") as f64;
    if player.entered_at > f64::default() && world.now() - player.entered_at < settle {
        return None;
    }
    let one = f32::from(1u8);
    let closeness = (wp * (one - wp) * policy.w("closeness_scale")).clamp(f32::default(), one);
    let threshold = policy.w("stamina_rest_threshold")
        * (one - policy.w("clutch_stamina_retention") * closeness);
    let mut priority = f32::default();
    if player.stamina < threshold {
        priority = priority.max(policy.w("rest_priority_stamina") + (threshold - player.stamina));
    }
    if player.data.starter {
        // 犯规危机：一节两犯、两节三犯、三节四犯都要先坐下来。
        let foul_limit = rules.league.max_personal_fouls;
        let threshold = match world.clock.period {
            1 => policy.w("foul_trouble_first") as u8,
            2 => policy.w("foul_trouble_second") as u8,
            3 => policy.w("foul_trouble_third") as u8,
            _ => foul_limit,
        };
        if player.fouls >= threshold {
            priority = priority.max(policy.w("rest_priority_foul_trouble") + player.stamina);
        }
    }
    // 垃圾时间：胜率已经贴到 0 或 1。它不在「需要休息」里处理，因为
    // 换人的顺序不是「谁最累」，而是「谁打得最久」，在清场那一处统一处理。
    (priority > f32::default()).then_some(priority)
}

fn needs_rest(world: &World, handle: usize) -> bool {
    rest_priority(world, handle).is_some()
}

/// 胜负已定：胜负概率已经贴到 0 或 1，阵容强度不再影响结果。
fn garbage_decided(world: &World, team: TeamSide) -> bool {
    let has_ball = world.possession.team == team;
    let wp = crate::endgame::win_probability(world, team, has_ball) as f32;
    let band = world.params.policy.w("garbage_wp_band");
    let one = f32::from(1u8);

    wp <= one - band || wp >= band
}

/// 垃圾时间换人：一次死球窗口把替补席整套换上去。
///
/// 这是一次换阵，不是连续几次战术调整，所以不占 `max_substitutions_per_window`
/// 的配额：胜负已定时教练换上的是整套替补，现实里也是死球后一次换完。
/// 出场时间最长的首发先下，直到替补席用尽。
fn empty_the_bench(world: &mut World, team: TeamSide, index: usize) {
    let foul_limit = world.rules.league.max_personal_fouls;
    let roster = world.players.len();
    let mut swapped = 0usize;
    loop {
        let on_court = world.on_court(team);
        if on_court.as_slice().len() <= 1 {
            break;
        }
        let outgoing = on_court
            .as_slice()
            .iter()
            .copied()
            .filter(|handle| world.players[*handle].data.starter)
            .max_by(|left, right| {
                world
                    .line(*left)
                    .seconds
                    .total_cmp(&world.line(*right).seconds)
            });
        let Some(outgoing) = outgoing else {
            break;
        };
        let incoming = (0..world.players.len())
            .filter(|candidate| {
                let player = &world.players[*candidate];
                // 替补席随时可上；首发只有在出场时间比被换下者少时才可上。
                // 前者保证替补席能全部上桌，后者挡住「刚换下去的人又被
                // 换回来」的循环。
                player.team == team
                    && !player.on_court
                    && player.fouls < foul_limit
                    && !player.disqualified
                    && (!player.data.starter
                        || world.line(*candidate).seconds < world.line(outgoing).seconds)
            })
            .max_by(|left, right| {
                // 先在替补席里找体能最好的人，替补席用尽才用已经歇够的
                // 首发填位：名单只有八个人，死守「替补席」会让场上无人可换。
                let score = |candidate: usize| {
                    let player = &world.players[candidate];
                    let bench_first = if player.data.starter {
                        f32::default()
                    } else {
                        f32::from(1u8)
                    };
                    bench_first + player.stamina
                };
                score(*left).total_cmp(&score(*right))
            });
        let Some(incoming) = incoming else {
            break;
        };
        swap(world, team, outgoing, incoming);
        swapped += 1;
        assert!(
            swapped <= roster,
            "垃圾时间清场在 {team:?} 的同一个死球窗口里换了 {swapped} 人次，超过名单人数 {roster}"
        );
    }
    world.rotation.applied_in_window[index] += swapped as u32;
}

/// 球员的比赛价值：持球、投射与敏捷的综合。
///
/// 关键时段换人不能只看体能：追分或守分时要换上的是更好的人，
/// 不是更不累的人。这张分数只用于比较同一支球队内部的替补人选。
fn player_quality(world: &World, handle: usize) -> f32 {
    let attributes = &world.players[handle].data.attributes;
    attributes.ball_handling
        + attributes.shooting_three
        + attributes.shooting_mid
        + attributes.agility
}

fn best_bench(world: &World, team: TeamSide, handle: usize) -> Option<usize> {
    let foul_limit = world.rules.league.max_personal_fouls;
    let eligible: Vec<usize> = (0..world.players.len())
        .filter(|candidate| {
            let player = &world.players[*candidate];
            player.team == team
                && !player.on_court
                && player.fouls < foul_limit
                && !player.disqualified
                && player.resting_until <= world.now()
        })
        .collect();
    if eligible.is_empty() {
        return None;
    }
    // 优先用不需要休息的替补；若都不合格（例如垃圾时间全队都累了），
    // 仍然要能把人换上来，否则阵容会死在场上。
    let rested: Vec<usize> = eligible
        .iter()
        .copied()
        .filter(|candidate| !needs_rest(world, *candidate))
        .collect();
    let pool = if rested.is_empty() {
        &eligible
    } else {
        &rested
    };
    // 比赛越胶着，能力在换人选择里的分量越重：收官阵容是最好的人。
    let has_ball = world.possession.team == team;
    let wp = crate::endgame::win_probability(world, team, has_ball) as f32;
    let one = f32::from(1u8);
    let closeness =
        (wp * (one - wp) * world.params.policy.w("closeness_scale")).clamp(f32::default(), one);
    let quality_weight = world.params.policy.w("closing_quality_weight") * closeness;
    pool.iter().copied().max_by(|left, right| {
        let slot_penalty = |candidate: usize| {
            if world.players[candidate].slot == world.players[handle].slot {
                f32::default()
            } else {
                f32::from(1u8)
            }
        };
        let score = |candidate: usize| {
            world.players[candidate].stamina - slot_penalty(candidate)
                + quality_weight * player_quality(world, candidate)
        };
        score(*left)
            .partial_cmp(&score(*right))
            .expect("finite stamina")
    })
}

/// 执行换人：替补继承被换下者的位置、目标与槽位。
fn swap(world: &mut World, team: TeamSide, outgoing: usize, incoming: usize) {
    let position = world.players[outgoing].pos;
    let target = world.players[outgoing].target;
    let slot = world.players[outgoing].slot;
    world.players[incoming].on_court = true;
    world.players[incoming].entered_at = world.now();
    // 下场的人要坐够最短休息时间才能回来：没有这条，垃圾时间会在
    // 「主力下去、替补上来、替补又被当成该休息、主力回来」里无限拉锯。
    world.players[outgoing].resting_until =
        world.now() + world.rules.rotation.min_rest_seconds as f64;
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

/// 场上不能留一个已经犯满的球员：一旦出现，比赛立即停表。
///
/// 停表后由 [`apply_dead_ball_window`] 把人换下；球权不变，所以这不是
/// 球权转换，只是把比赛停下来一次。
fn enforce_disqualification(world: &mut World) {
    let live = matches!(
        world.phase,
        Phase::Live | Phase::PassFlight { .. } | Phase::ShotFlight { .. }
    );
    if !live {
        return;
    }
    if !world.players.iter().any(|p| p.on_court && p.disqualified) {
        return;
    }
    let team = world.possession.team;
    world.phase = Phase::DeadBall {
        team,
        ready_at: world.now() + world.rules.inbound_setup_seconds as f64,
    };
}

pub fn note_intents(world: &mut World) {
    enforce_disqualification(world);
    if is_substitution_window(world) {
        return;
    }
    world.rotation.applied_in_window = [0, 0];
    let mut pending = world.rotation.pending;
    for (index, team) in [TeamSide::Home, TeamSide::Away].into_iter().enumerate() {
        if pending[index] {
            continue;
        }
        let on_court = world.on_court(team);
        pending[index] = on_court
            .as_slice()
            .iter()
            .any(|handle| needs_rest(world, *handle));
    }
    world.rotation.pending = pending;
}

/// 死球窗口执行已登记的换人意图。
///
/// 活球时什么都不做——阵容一动，比赛就在活球状态下换人了。配额用尽、
/// 无人需要休息、或没有合格替补时提前结束。
pub fn apply_dead_ball_window(world: &mut World) {
    if !is_substitution_window(world) {
        return;
    }
    let budget_limit = world.rules.rotation.max_substitutions_per_window;
    for (index, team) in [TeamSide::Home, TeamSide::Away].into_iter().enumerate() {
        // 犯满离场的替换不是教练的选择：它不占窗口配额，也不能被
        // 配额耗尽挡下来，否则场上会留下一个已经犯满的球员。
        loop {
            let on_court = world.on_court(team);
            let Some(outgoing) = on_court
                .as_slice()
                .iter()
                .copied()
                .find(|handle| world.players[*handle].disqualified)
            else {
                break;
            };
            let Some(incoming) = best_bench(world, team, outgoing) else {
                break;
            };
            swap(world, team, outgoing, incoming);
        }
        if !world.rotation.pending[index] && !garbage_decided(world, team) {
            continue;
        }
        // 胜负已定时换人是一次性清场：按出场时间从长到短撤下，
        // 换上的人本场不再回来。这条路径不占用名单状态，只占窗口配额。
        if garbage_decided(world, team) {
            empty_the_bench(world, team, index);
            world.rotation.pending[index] = false;
            continue;
        }
        let mut budget = budget_limit.saturating_sub(world.rotation.applied_in_window[index]);
        while budget > 0 {
            let on_court = world.on_court(team);
            // 拉最高紧迫度的人下场：垃圾时间先撤主力，体能不足先撤最累的。
            let resting = on_court
                .as_slice()
                .iter()
                .copied()
                .filter_map(|handle| {
                    rest_priority(world, handle).map(|priority| (handle, priority))
                })
                .max_by(|(_, left), (_, right)| left.total_cmp(right))
                .map(|(handle, _)| handle);
            let Some(outgoing) = resting else {
                break;
            };
            let Some(incoming) = best_bench(world, team, outgoing) else {
                break;
            };
            swap(world, team, outgoing, incoming);
            budget -= 1;
            world.rotation.applied_in_window[index] += 1;
        }
        world.rotation.pending[index] = false;
    }
}

/// 记一次犯规并登记犯满离场。
///
/// 返回本次犯规是否触发了犯满离场，供调用方立即进入停表程序——
/// 场上不能留一个已经犯满的球员继续打。
pub fn record_foul(world: &mut World, handle: usize) -> bool {
    let limit = world.rules.league.max_personal_fouls;
    let player = &mut world.players[handle];
    player.fouls = player.fouls.saturating_add(1);
    if player.fouls >= limit {
        player.disqualified = true;
        return true;
    }
    false
}
