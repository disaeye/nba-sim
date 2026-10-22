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
    if !stale {
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
    for (_, slot, rank) in pairs.iter().take(count) {
        if taken_defender[*slot] || taken_attacker[*rank] {
            continue;
        }
        taken_defender[*slot] = true;
        taken_attacker[*rank] = true;
        world.possession.matchup[*slot] = attackers.as_slice()[*rank];
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
    let read_ball =
        world.delayed_ball_position(policy.g("defense_reaction_delay_seconds") as f64);
    let to_hoop = (hoop - ball).normalize_or_zero();
    let carrier_distance = (hoop - read_ball).length();
    // 球的篮筐威胁：越靠近篮筐越接近 1。
    let threat = (f32::from(1u8) - carrier_distance / potential.threat_radius_ft)
        .clamp(f32::default(), f32::from(1u8));
    let rim_help = hoop + (read_ball - hoop).normalize_or_zero() * potential.rim_buffer_ft;
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
        if (man_pos.y - read_ball.y).abs() <= potential.weak_side_lateral_ft - blend {
            continue;
        }
        nearest_any = nearest_any.min((hoop - man_pos).length());
    }
    for (slot, defender) in defenders.as_slice().iter().enumerate() {
        let man = world.possession.matchup[slot];
        let player = &world.players[*defender];
        let stamina = player.stamina.clamp(f32::default(), f32::from(1u8));
        let stamina_factor = potential.stamina_floor
            + (f32::from(1u8) - potential.stamina_floor) * stamina.powf(potential.stamina_gain);
        if Some(man) == carrier {
            // 领防间隔：基础间隔按方案倍率缩放，并且不超过持球人距篮距离的固定比例。
            // 领防人看的是此刻的球，延迟只影响协防轮转。
            let live_distance = (hoop - ball).length();
            let gap = (potential.on_ball_gap_min_ft * scheme.on_ball_gap_multiplier)
                .min(potential.on_ball_gap_hoop_ratio * live_distance);
            targets[slot] = ball + to_hoop * gap;
            continue;
        }
        if man == UNASSIGNED || !world.players[man].on_court {
            targets[slot] = player.home_spot;
            continue;
        }
        let man_pos = world.players[man].pos;
        let man_to_hoop = (hoop - man_pos).length();
        let sag = if man_to_hoop > potential.perimeter_attribution_ft {
            potential.sag_distance_perimeter_ft
        } else {
            potential.sag_distance_interior_ft
        } * scheme.sag_multiplier;
        let sagged = man_pos + (hoop - man_pos).normalize_or_zero() * sag;
        let lateral = (man_pos.y - read_ball.y).abs();
        // 弱侧从「门限减一个混合带」开始渐变、到门限时算完全弱侧：
        // 门限仍是弱侧的定义，只是分类边界不再是一道阶跃。
        let weak = ((lateral - (potential.weak_side_lateral_ft - blend)) / blend)
            .clamp(f32::default(), f32::from(1u8));
        let low_closeness = if nearest_any.is_finite() {
            (f32::from(1u8) - (man_to_hoop - nearest_any) / blend)
                .clamp(f32::default(), f32::from(1u8))
        } else {
            f32::default()
        };
        let weak_gain = potential.high_man_threat_gain
            + (potential.low_man_threat_gain - potential.high_man_threat_gain) * low_closeness;
        let gain = potential.default_threat_gain
            + (weak_gain - potential.default_threat_gain) * weak;
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
    for (slot, handle) in defenders.as_slice().iter().enumerate() {
        world.players[*handle].target = targets[slot];
    }
}
