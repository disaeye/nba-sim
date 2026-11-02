use crate::state::MAX_ON_COURT;
use crate::world::World;
use glam::Vec2;

/// 对位表未分配时的占位值。
pub const UNASSIGNED: usize = usize::MAX;

const PAIRS: usize = MAX_ON_COURT * MAX_ON_COURT;

/// 建立或修复对位表。
///
/// 只在空表或表里出现不在场的球员时重算，其余时间原样保留——对位身份不是
/// 每 tick 从距离重新推导的量，否则一次传球就会让两名防守人互换目标。
pub fn refresh_matchup(world: &mut World) {
    let offense = world.possession.team;
    let defense = offense.other();
    let defenders = world.on_court(defense);
    let attackers = world.on_court(offense);
    if defenders.as_slice().len() != attackers.as_slice().len() {
        return;
    }
    let stale = world
        .possession
        .matchup
        .iter()
        .any(|handle| *handle == UNASSIGNED || !world.players[*handle].on_court);
    let pairing_changed_at = world.possession.pairing_changed_at;
    let new_possession = pairing_changed_at < world.possession.started_at;
    // 传球本身不换对位：只有突破得手（防守人被过）才重算最近匹配；
    // 传球后重新配对会让对位在无人要求的情况下自己变。
    let possession_reassigned = world
        .possession
        .last_drive_at
        .is_some_and(|at| at > pairing_changed_at);
    if !stale && !new_possession && !possession_reassigned {
        return;
    }
    // 贪心最近匹配：25 个候选对全部落在栈上，排序不产生堆分配。
    let mut pairs = [(0.0_f32, 0_usize, 0_usize); PAIRS];
    let mut count = 0;
    for (slot, defender) in defenders.as_slice().iter().enumerate() {
        for (rank, attacker) in attackers.as_slice().iter().enumerate() {
            let distance =
                (world.players[*defender].pos - world.players[*attacker].pos).length_squared();
            pairs[count] = (distance, slot, rank);
            count += 1;
        }
    }
    for index in 1..count {
        let mut position = index;
        while position > 0 && pairs[position - 1].0 > pairs[position].0 {
            pairs.swap(position - 1, position);
            position -= 1;
        }
    }
    let mut taken_defender = [false; MAX_ON_COURT];
    let mut taken_attacker = [false; MAX_ON_COURT];
    let mut changed = false;
    for (_, slot, rank) in pairs.iter().take(count) {
        if taken_defender[*slot] || taken_attacker[*rank] {
            continue;
        }
        taken_defender[*slot] = true;
        taken_attacker[*rank] = true;
        let attacker = attackers.as_slice()[*rank];
        if world.possession.matchup[*slot] != attacker {
            changed = true;
        }
        world.possession.matchup[*slot] = attacker;
    }
    if changed {
        // 对位换了：职责层必须立刻改人，否则会出现好几个人同时是对位防守人。
        world.possession.pairing_changed_at = world.now();
    }
}

/// 换防：把两名防守人的对位对象互换。
pub fn switch_matchup(world: &mut World, first: usize, second: usize) {
    let defense = world.possession.team.other();
    let defenders = world.on_court(defense);
    let mut first_slot = None;
    let mut second_slot = None;
    for (slot, handle) in defenders.as_slice().iter().enumerate() {
        if *handle == first {
            first_slot = Some(slot);
        }
        if *handle == second {
            second_slot = Some(slot);
        }
    }
    if let (Some(left), Some(right)) = (first_slot, second_slot) {
        world.possession.matchup.swap(left, right);
        world.possession.pairing_changed_at = world.now();
    }
}

/// 求解五名防守人的目标点。
///
/// 势能由三项叠加：对位羁绊把人拉向自己的对位人，下沉把人朝篮筐方向收缩，
/// 协防引力在球威胁篮筐时把弱侧人拉向护筐点。强侧无球防守人不参与协防，
/// 因此持球人突破时他们的目标点不变。全部计算在栈上完成，不产生堆分配。
pub fn solve_defense_targets(world: &World) -> [Vec2; MAX_ON_COURT] {
    let offense = world.possession.team;
    let defense = offense.other();
    let defenders = world.on_court(defense);
    let hoop = world.hoop_for(offense);
    let carrier = world.holder();
    let mut targets = [hoop; MAX_ON_COURT];
    let ball = carrier
        .map(|handle| world.players[handle].pos)
        .unwrap_or(hoop);
    let potential = &world.rules.tactics.defense.potential_field;
    let scheme = &world.rules.tactics.defense;
    let policy = &world.params.policy;
    // 协防看的是延迟读球的位置：防守人反应要时间，轮转因此总慢半拍。
    let read_ball = world.delayed_ball_position(policy.g("defense_reaction_delay_seconds") as f64);
    let ball_delta = hoop - ball;
    let live_distance = ball_delta.length();
    let to_hoop = if live_distance > f32::MIN_POSITIVE {
        ball_delta / live_distance
    } else {
        Vec2::ZERO
    };
    let carrier_distance = (hoop - read_ball).length();
    // 球的篮筐威胁：越靠近篮筐越接近 1。
    let threat = (f32::from(1u8) - carrier_distance / potential.threat_radius_ft)
        .clamp(f32::default(), f32::from(1u8));
    let rim_dir = if carrier_distance > f32::MIN_POSITIVE {
        (read_ball - hoop) / carrier_distance
    } else {
        Vec2::ZERO
    };
    let rim_help = hoop + rim_dir * potential.rim_buffer_ft;
    // 落后方的防守赌得更凶（夹击、轮转早），领先方收得更紧；
    // γ 连续变化，所以“第几节”不参与判断。
    let defense_margin = world.score[World::team_index(defense)] as i32
        - world.score[World::team_index(offense)] as i32;
    let leverage = crate::endgame::leverage(
        world,
        defense_margin,
        crate::endgame::remaining_seconds(world),
    );
    let gamble =
        (f32::from(1u8) + policy.w("defense_leverage_gamble") * leverage.variance_appetite).clamp(
            policy.w("defense_leverage_gamble_floor"),
            policy.w("defense_leverage_gamble_ceiling"),
        );
    // 低位/高位与弱侧都按距离连续插值：阶跃分类会让球位动一点就整条
    // 协防路线瞬移。最近的人拿满低位增益，离得越远越靠近高位增益。
    let blend = policy.g("help_gain_blend_ft").max(f32::EPSILON);
    let mut nearest_any = f32::MAX;
    for (slot, _) in defenders.as_slice().iter().enumerate() {
        let man = world.possession.matchup[slot];
        if man == UNASSIGNED || Some(man) == carrier || !world.players[man].on_court {
            continue;
        }
        let man_pos = world.players[man].pos;
        // 只有进入弱侧混合带的人才参与「低位人」竞争：带边缘的人的
        // 弱侧权重本来就是 0，所以这个集合变化不影响连续性。
        let same_side = (man_pos.y - 25.0) * (ball.y - 25.0) > 0.0;
        if same_side || (man_pos.y - read_ball.y).abs() <= potential.weak_side_lateral_ft - blend {
            continue;
        }
        nearest_any = nearest_any.min((hoop - man_pos).length());
    }
    for (slot, defender) in defenders.as_slice().iter().enumerate() {
        let man = world.possession.matchup[slot];
        let player = &world.players[*defender];
        let stamina = player.stamina.clamp(f32::default(), f32::from(1u8));
        let stamina_pow = if (potential.stamina_gain - 1.5).abs() < 1e-4 {
            stamina * stamina.sqrt()
        } else {
            stamina.powf(potential.stamina_gain)
        };
        let stamina_factor =
            potential.stamina_floor + (f32::from(1u8) - potential.stamina_floor) * stamina_pow;
        if Some(man) == carrier {
            // 领防间隔：基础间隔按方案倍率缩放，并且不超过持球人距篮距离的固定比例。
            // 领防人看的是此刻的球，延迟只影响协防轮转。
            let tactical_gap = (potential.on_ball_gap_min_ft * scheme.on_ball_gap_multiplier)
                .min(potential.on_ball_gap_hoop_ratio * live_distance);
            // 贴身防守允许进到贴身间隔：目标点不能小于贴身间隔，
            // 否则会落在非法区域。
            let pressure = crate::assignment::pressure_separation(world);
            let gap = tactical_gap.max(pressure);
            // 持球人已经进入身体间隔以内时，目标不能继续放在他与篮筐之间，
            // 否则目标会越过篮筐并随方向翻转。此时站到持球人背向篮筐的一侧。
            let target = if live_distance > gap {
                ball + to_hoop * gap
            } else {
                let away = (ball - hoop).normalize_or_zero();
                let direction = if away == Vec2::ZERO { Vec2::X } else { away };
                ball + direction * gap
            };
            targets[slot] = target;
            continue;
        }
        if man == UNASSIGNED || !world.players[man].on_court {
            targets[slot] = player.home_spot;
            continue;
        }
        let man_pos = world.players[man].pos;
        let man_to_hoop = (hoop - man_pos).length();
        let lateral = (man_pos.y - read_ball.y).abs();
        let same_side = (man_pos.y - 25.0) * (ball.y - 25.0) > 0.0;
        // 弱侧从「门限减一个混合带」开始渐变、到门限时算完全弱侧：
        // 门限仍是弱侧的定义，只是分类边界不再是一道阶跃。
        let weak = if same_side {
            f32::default()
        } else {
            ((lateral - (potential.weak_side_lateral_ft - blend)) / blend)
                .clamp(f32::default(), f32::from(1u8))
        };
        // 下沉量在同侧与弱侧之间连续过渡：弱侧外线人先站到禁区边缘，
        // 突破发生时他要跑的距离才在一两步之内。
        let sag = if man_to_hoop > potential.perimeter_attribution_ft {
            potential.sag_distance_perimeter_ft
                + (potential.sag_distance_weak_side_ft - potential.sag_distance_perimeter_ft)
                    * weak
        } else {
            potential.sag_distance_interior_ft
        } * scheme.sag_multiplier;
        let sagged = man_pos + (hoop - man_pos).normalize_or_zero() * sag;
        let low_closeness = if nearest_any.is_finite() {
            (f32::from(1u8) - (man_to_hoop - nearest_any) / blend)
                .clamp(f32::default(), f32::from(1u8))
        } else {
            f32::default()
        };
        let weak_gain = potential.high_man_threat_gain
            + (potential.low_man_threat_gain - potential.high_man_threat_gain) * low_closeness;
        let gain = if same_side {
            f32::default()
        } else {
            potential.default_threat_gain + (weak_gain - potential.default_threat_gain) * weak
        };
        let drive_boost = if world.possession.last_drive_at.is_some() {
            potential.drive_help_threat_gain
        } else {
            f32::from(1u8)
        };
        let anchor = (potential.help_hoop_weight_base
            + potential.help_priority_tilt_gain * scheme.help_priority)
            .clamp(
                potential.help_hoop_weight_min,
                potential.help_hoop_weight_max,
            );
        let tendency = potential.help_tendency_floor
            + player.data.tendencies.help_aggressiveness * potential.help_tendency_span;
        // 协防涌现门限：威胁占比达不到阈值就不轮转，强侧无球人因此不动。
        let ratio = threat * gain * drive_boost;
        let weight = if ratio < potential.rim_help_threat_ratio {
            f32::default()
        } else {
            ((ratio - potential.rim_help_threat_ratio) * tendency * stamina_factor * gamble)
                .min(potential.help_hoop_weight_max)
        };
        let help = rim_help * anchor + sagged * (f32::from(1u8) - anchor);
        targets[slot] = sagged.lerp(help, weight);
    }
    targets
}

/// 把求解结果写进球员目标点。
pub fn apply_defense_targets(world: &mut World) {
    let defense = world.possession.team.other();
    let defenders = world.on_court(defense);
    let targets = solve_defense_targets(world);
    crate::assignment::assign_defense_responsibilities(world, &targets);
    let carrier = world.holder();
    let pressure = crate::assignment::pressure_separation(world);
    let hoop = world.hoop_for(world.possession.team);
    for (slot, handle) in defenders.as_slice().iter().enumerate() {
        let man = world.possession.matchup[slot];
        let responsibility = world.players[*handle].defense_responsibility;
        let (kind, partner, target) = match responsibility {
            crate::state::DefenseResponsibility::OnBall => {
                // 对位防守人的目标点：站在持球人与篮筐之间，并与持球人保持
                // 一个身位的间隔。已经贴得更近时目标点就是当前位置——把目标
                // 强行拉到间隔之外会落在非法区域，职责层会把它算成一次无效
                // 提交。
                let target = if let Some(carrier) = carrier {
                    let direction = (hoop - world.players[carrier].pos).normalize_or_zero();
                    let distance =
                        (world.players[carrier].pos - world.players[*handle].pos).length();
                    if distance <= pressure {
                        world.players[*handle].pos
                    } else {
                        world.players[carrier].pos + direction * pressure
                    }
                } else {
                    targets[slot]
                };
                (
                    crate::state::TargetKind::Pressure,
                    carrier.unwrap_or(usize::MAX),
                    target,
                )
            }
            crate::state::DefenseResponsibility::Deny => {
                let target = if man != usize::MAX && world.players[man].on_court {
                    let ball = carrier
                        .map(|carrier| world.players[carrier].pos)
                        .unwrap_or(world.players[*handle].pos);
                    let direction = (ball - world.players[man].pos).normalize_or_zero();
                    // 已经比间隔更近时保持在当前位置，目标是合法且可达的。
                    let distance = (world.players[man].pos - world.players[*handle].pos).length();
                    if distance <= pressure {
                        world.players[*handle].pos
                    } else {
                        world.players[man].pos + direction * pressure
                    }
                } else {
                    targets[slot]
                };
                (crate::state::TargetKind::Pressure, man, target)
            }
            crate::state::DefenseResponsibility::BoxOut => {
                let target = if man != usize::MAX && world.players[man].on_court {
                    let direction = (hoop - world.players[man].pos).normalize_or_zero();
                    world.players[man].pos + direction * pressure
                } else {
                    targets[slot]
                };
                (crate::state::TargetKind::Pressure, man, target)
            }
            crate::state::DefenseResponsibility::Help
            | crate::state::DefenseResponsibility::Sink => {
                (crate::state::TargetKind::Station, usize::MAX, targets[slot])
            }
            // 从掩护上方挤过：继续站在持球人与篮筐之间，贴着人不放。
            crate::state::DefenseResponsibility::GoOver => {
                let target = carrier
                    .map(|carrier| {
                        let direction = (hoop - world.players[carrier].pos).normalize_or_zero();
                        world.players[carrier].pos + direction * pressure
                    })
                    .unwrap_or(targets[slot]);
                (
                    crate::state::TargetKind::Pressure,
                    carrier.unwrap_or(usize::MAX),
                    target,
                )
            }
            // 从掩护下方绕过：先沉到掩护人与篮筐之间，让持球人只能往外投。
            crate::state::DefenseResponsibility::GoUnder => {
                let screener = world.possession.last_screen.map(|read| read.screener);
                let target = screener
                    .filter(|screener| world.players[*screener].on_court)
                    .map(|screener| {
                        let spot = world.players[screener].pos;
                        let direction = (hoop - spot).normalize_or_zero();
                        spot + direction * (pressure + 1.5)
                    })
                    .unwrap_or(targets[slot]);
                (
                    crate::state::TargetKind::Pressure,
                    screener.unwrap_or(usize::MAX),
                    target,
                )
            }
            // 换防：接管新的对位人，站位按他的切入路线取。
            crate::state::DefenseResponsibility::Switch => {
                let target = if man != usize::MAX && world.players[man].on_court {
                    let ball = carrier
                        .map(|carrier| world.players[carrier].pos)
                        .unwrap_or(world.players[*handle].pos);
                    let direction = (ball - world.players[man].pos).normalize_or_zero();
                    world.players[man].pos + direction * pressure
                } else {
                    targets[slot]
                };
                (crate::state::TargetKind::Pressure, man, target)
            }
            // 回位：直接回到自己的对位人与球之间。
            crate::state::DefenseResponsibility::Recover => {
                let target = if man != usize::MAX && world.players[man].on_court {
                    let ball = carrier
                        .map(|carrier| world.players[carrier].pos)
                        .unwrap_or(world.players[*handle].pos);
                    let direction = (ball - world.players[man].pos).normalize_or_zero();
                    world.players[man].pos + direction * pressure
                } else {
                    targets[slot]
                };
                (crate::state::TargetKind::Pressure, man, target)
            }
        };
        let used_closeout = responsibility == crate::state::DefenseResponsibility::OnBall
            && world.apply_defensive_closeout_target(*handle);
        if !used_closeout {
            crate::assignment::commit_target_as(world, *handle, target, kind, partner);
        }
        crate::assignment::reselect_yielding(world, *handle, world.players[*handle].target);
    }
}
