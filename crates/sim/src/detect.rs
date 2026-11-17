//! 原始事件检测：每种类别一个判定函数。
//!
//! 判定函数只读权威状态与账本，产出 [`RawIncident`]。它们不改比赛状态：
//! 后果由裁决层按档案给出，再经统一提交入口落下。
//!
//! 判定函数分三类：持续计时类读权威状态上的计时，程序时限类读程序自带
//! 的开始时刻，接触与行为类在发生当场成立。每个函数都由 [`World::step`]
//! 主循环调用，产出的事件来自完整比赛，不依赖测试注入。

use nba_domain::identity::ProgramId;

use crate::action::TeamSide;
use crate::incident::{IncidentKind, RawIncident};
use crate::world::World;

/// 检测 tick 末尾成立的抽样事件：它们都在活球里按当场接触或行为判定。
pub fn detect_sampled(world: &mut World) -> Vec<RawIncident> {
    // 抽样式接触事实只在活球里成立：死球程序、发球、罚球与节间没有
    // 对抗动作，读上一回合的球权或球员速度会把不属于这一段的判罚插进
    // 正在等待的程序。
    if !world.live_play() {
        return Vec::new();
    }
    // 投篮在途时不抽样式犯规：球在空中，犯规的判罚与投篮的结算会同时
    // 落在这一 tick 上，两者互相覆盖会让球态出现非法跃迁。
    if world.pending_shot.is_some() || world.ball_flight.is_some() {
        return Vec::new();
    }
    // 球的权威状态是唯一事实：它在飞行中时同样不抽样式犯规。
    if !matches!(world.ball.state(), crate::state::BallState::Held { .. }) {
        return Vec::new();
    }
    let mut incidents = Vec::new();
    incidents.extend(detect_transition_take(world));
    incidents.extend(detect_clear_path(world));
    incidents.extend(detect_flagrant(world));
    incidents.extend(detect_injury(world));
    incidents.extend(detect_team_technical(world));
    incidents.extend(detect_double_foul_sampled(world));
    incidents.extend(detect_player_technical(world));
    incidents
}

/// 背身五秒：进攻人在前场罚球线延长线以下背身超过时限。
///
/// 时限为零表示该联赛没有这条规则：FIBA 的档案里它是零。
pub fn detect_back_to_basket(world: &mut World) -> Option<RawIncident> {
    let limit = world.rules.back_to_basket_seconds();
    if limit <= f32::default() || !world.live_play() {
        return None;
    }
    if !world.possession.holder_back_to_basket {
        return None;
    }
    let elapsed = world.possession.back_to_basket_seconds;
    if elapsed + f32::EPSILON < limit {
        return None;
    }
    world.possession.back_to_basket_seconds = f32::default();
    let holder = world.holder();
    Some(
        RawIncident::new(IncidentKind::BackToBasket, world.now())
            .with_parties(holder, None)
            .with_team(world.possession.team)
            .with_measurement("limit_seconds", limit)
            .with_measurement("elapsed_seconds", elapsed)
            .with_program(ProgramId::from_raw(world.current_program_id)),
    )
}

/// 回场违例：已经建立前场控制又把球带回后场。
///
/// 检测层只认事实与责任方：后果（失误、发球地点、球权）由裁决层给出。
pub fn detect_backcourt_violation(world: &mut World) -> Option<RawIncident> {
    let holder = world.pending_backcourt_violation.take()?;
    Some(
        RawIncident::new(IncidentKind::Backcourt, world.now())
            .with_parties(Some(holder), None)
            .with_team(world.possession.team)
            .with_program(ProgramId::from_raw(world.current_program_id)),
    )
}

/// 八秒违例：控球方在后场停留超过时限。
pub fn detect_eight_second_violation(world: &mut World) -> Option<RawIncident> {
    let holder = world.pending_eight_second_violation.take()?;
    Some(
        RawIncident::new(IncidentKind::EightSecond, world.now())
            .with_parties(Some(holder), None)
            .with_team(world.possession.team)
            .with_measurement("limit_seconds", world.rules.backcourt_seconds())
            .with_program(ProgramId::from_raw(world.current_program_id)),
    )
}

/// 进攻钟走完：进攻方在二十四秒内没有完成一次出手。
///
/// 球在空中时计时归零不是违例：那一球会自己解出进与不进，违例只在计时
/// 归零时球还在某个球员手里成立。
pub fn detect_shot_clock(world: &mut World) -> Option<RawIncident> {
    if !world.live_play() || world.clock.shot_clock() > f64::default() {
        return None;
    }
    if world.pending_rebound.is_some() || world.ball_flight.is_some() {
        return None;
    }
    // 投篮动作在途时计时归零：那一球会自己解出结果。
    let shooting = world.any_interaction(|intent| {
        matches!(&intent.kind, crate::action::ActionKind::Shoot(_))
    });
    if shooting {
        return None;
    }
    let team = world.possession.team;
    Some(
        RawIncident::new(IncidentKind::ShotClock, world.now())
            .with_parties(world.holder(), None)
            .with_team(team)
            .with_measurement("shot_clock_seconds", world.clock.shot_clock() as f32)
            .with_program(ProgramId::from_raw(world.current_program_id)),
    )
}

/// 贴身五秒：持球人被紧逼盯防超过时限。
///
/// 时限为零表示该联赛没有这条规则。
pub fn detect_closely_guarded(world: &mut World) -> Option<RawIncident> {
    let limit = world.rules.league.clocks.closely_guarded_seconds;
    if limit <= f32::default() || !world.live_play() {
        return None;
    }
    let holder = world.holder()?;
    let elapsed = world.possession.closely_guarded_seconds;
    if elapsed + f32::EPSILON < limit {
        return None;
    }
    // 计时归零：这条违规已经成立，计时不再继续。
    world.possession.closely_guarded_seconds = f32::default();
    Some(
        RawIncident::new(IncidentKind::CloselyGuarded, world.now())
            .with_parties(Some(holder), None)
            .with_team(world.possession.team)
            .with_measurement("limit_seconds", limit)
            .with_measurement("elapsed_seconds", elapsed)
            .with_program(ProgramId::from_raw(world.current_program_id)),
    )
}

/// 发球五秒：发球队员超过五秒没有把球传入场内。
///
/// 计时从发球程序开始那一刻起算，到球被合法传入场内为止（NBA Rule 8
/// §III）。因此判据读的是发球程序的开始时刻，而不是球员的走位。
pub fn detect_inbound_timeout(world: &mut World) -> Option<RawIncident> {
    let started = world.inbound_started_at?;
    let limit = f64::from(world.rules.league.inbound.inbound_seconds);
    let now = world.now();
    if now - started <= limit {
        return None;
    }
    world.inbound_started_at = None;
    let team = world.possession.team;
    Some(
        RawIncident::new(IncidentKind::InboundTimeout, now)
            .with_team(team)
            .with_measurement("limit_seconds", limit as f32)
            .with_measurement("elapsed_seconds", (now - started) as f32)
            .with_program(ProgramId::from_raw(world.current_program_id)),
    )
}

/// 罚球出手时限：罚球人超过时限没有出手。
pub fn detect_free_throw_timeout(world: &mut World) -> Option<RawIncident> {
    // 球在空中属于飞行结算，不属于持球等待出手；旧时间戳此时不能判超时。
    if world.pending_shot.is_some() || world.ball_flight.is_some() {
        return None;
    }
    let started = world.free_throw_started?;
    let limit = f64::from(world.rules.league.free_throws.free_throw_time_limit_seconds);
    let now = world.now();
    if now - started <= limit {
        return None;
    }
    world.free_throw_started = None;
    let team = world.possession.team;
    Some(
        RawIncident::new(IncidentKind::FreeThrowTimeout, now)
            .with_team(team)
            .with_measurement("limit_seconds", limit as f32)
            .with_measurement("elapsed_seconds", (now - started) as f32)
            .with_program(ProgramId::from_raw(world.current_program_id)),
    )
}

/// 罚球禁区：罚球离手之前有球员提前进入禁区内侧。
///
/// 进攻方提前进区判失误，防守方提前进区重罚一次（NBA Rule 8 §VI）。
pub fn detect_free_throw_lane_violation(world: &mut World) -> Option<RawIncident> {
    let lane = world.free_throw_lane.clone()?;
    if !lane.offense_early && !lane.defense_early {
        return None;
    }
    Some(
        RawIncident::new(IncidentKind::FreeThrowLaneViolation, world.now())
            .with_team(world.possession.team)
            .with_lane_early(lane.offense_early, lane.defense_early)
            .with_program(ProgramId::from_raw(world.current_program_id)),
    )
}

/// 进攻三秒：进攻人在限制区连续停留超过时限。
pub fn detect_offensive_three_second(world: &mut World) -> Option<RawIncident> {
    let limit = world.rules.three_second_lane_seconds();
    if limit <= f32::default() {
        return None;
    }
    let actor = crate::lane::offense_timer_holder(world)?;
    let elapsed = world.players[actor].lane_seconds;
    if elapsed <= limit {
        return None;
    }
    // 计时归零：这条违规已经成立，计时不再继续。
    world.players[actor].lane_seconds = f32::default();
    Some(
        RawIncident::new(IncidentKind::OffensiveThreeSecond, world.now())
            .with_parties(Some(actor), None)
            .with_team(world.players[actor].team)
            .with_measurement("limit_seconds", limit)
            .with_measurement("elapsed_seconds", elapsed)
            .with_program(ProgramId::from_raw(world.current_program_id)),
    )
}

/// 防守三秒：防守人在限制区连续停留超过时限。
///
/// FIBA 没有守方三秒规则：档案里的零值让这条判定不成立。
pub fn detect_defensive_three_second(world: &mut World) -> Option<RawIncident> {
    let limit = world.rules.defensive_three_second_seconds();
    if limit <= f32::default() {
        return None;
    }
    let actor = crate::lane::defense_timer_holder(world)?;
    let elapsed = world.players[actor].lane_seconds;
    if elapsed <= limit {
        return None;
    }
    // 计时归零：这条违规已经成立，计时不再继续。
    world.players[actor].lane_seconds = f32::default();
    Some(
        RawIncident::new(IncidentKind::DefensiveThreeSecond, world.now())
            .with_parties(Some(actor), None)
            .with_team(world.players[actor].team)
            .with_measurement("limit_seconds", limit)
            .with_measurement("elapsed_seconds", elapsed)
            .with_program(ProgramId::from_raw(world.current_program_id)),
    )
}

/// 球出界：球越过边界，最后触球方的对手在出界点发球。
///
/// 出界一律是一次判罚：球权给最后触球方的对手。是否记失误则看最后触球的
/// 是谁：进攻方碰出界记失误，防守方碰出界则进攻方保留球权。
pub fn detect_out_of_bounds(world: &mut World, spot: glam::Vec2) -> RawIncident {
    let touched = world.control.last_touch;
    let offense = world.possession.team;
    let touched_team = touched.map(|handle| world.players[handle].team);
    let recipient = match touched_team {
        Some(team) => team.other(),
        None => offense.other(),
    };
    // 最后触球的是进攻方就是进攻方违例；防守方碰出界则进攻方保留球权。
    let violation = touched_team.is_none_or(|team| team == offense);
    // 投篮出手后球无人控制地飞出界是投篮不中，不记失误。
    let shot_in_air = world.pending_shot.is_some();
    RawIncident::new(IncidentKind::OutOfBounds, world.now())
        .with_parties(touched, None)
        .with_team(recipient)
        .with_measurement("spot_x", spot.x)
        .with_measurement("spot_y", spot.y)
        .with_out_of_bounds(violation, shot_in_air)
        .with_program(ProgramId::from_raw(world.current_program_id))
}

/// 走步：持球人移动了轴心脚或脚步用尽。
pub fn detect_travel(world: &mut World, actor: usize) -> RawIncident {
    let team = world.players[actor].team;
    RawIncident::new(IncidentKind::Travel, world.now())
        .with_parties(Some(actor), None)
        .with_team(team)
        .with_program(ProgramId::from_raw(world.current_program_id))
}

/// 二次运球：收球确立轴心脚之后又放下球。
pub fn detect_double_dribble(world: &mut World, actor: usize) -> RawIncident {
    let team = world.players[actor].team;
    RawIncident::new(IncidentKind::DoubleDribble, world.now())
        .with_parties(Some(actor), None)
        .with_team(team)
        .with_program(ProgramId::from_raw(world.current_program_id))
}

/// 非法掩护：掩护人没有落位就撞上防守人。
pub fn detect_illegal_screen(world: &mut World, screener: usize, defender: usize) -> RawIncident {
    let team = world.players[screener].team;
    RawIncident::new(IncidentKind::IllegalScreen, world.now())
        .with_parties(Some(screener), Some(defender))
        .with_team(team)
        .with_program(ProgramId::from_raw(world.current_program_id))
}

/// 普通个人犯规：防守人用手或身体阻止对方。
pub fn detect_personal_foul(world: &mut World, fouler: usize, fouled: usize) -> RawIncident {
    let team = world.players[fouler].team;
    RawIncident::new(IncidentKind::PersonalFoul, world.now())
        .with_parties(Some(fouler), Some(fouled))
        .with_team(team)
        .with_program(ProgramId::from_raw(world.current_program_id))
}

/// 投篮犯规：防守人在投篮过程中碰到投篮人。
pub fn detect_shooting_foul(
    world: &mut World,
    fouler: usize,
    shooter: usize,
    free_throws: u8,
    made: bool,
) -> RawIncident {
    let team = world.players[fouler].team;
    RawIncident::new(IncidentKind::ShootingFoul, world.now())
        .with_parties(Some(fouler), Some(shooter))
        .with_team(team)
        .with_free_throws(free_throws, made)
        .with_program(ProgramId::from_raw(world.current_program_id))
}

/// 进攻犯规：进攻人撞倒防守人。
pub fn detect_offensive_foul(world: &mut World, fouler: usize, fouled: usize) -> RawIncident {
    let team = world.players[fouler].team;
    RawIncident::new(IncidentKind::OffensiveFoul, world.now())
        .with_parties(Some(fouler), Some(fouled))
        .with_team(team)
        .with_program(ProgramId::from_raw(world.current_program_id))
}

/// 双方犯规：双方各记一次个人犯规，球权留在当时控球的一方。
pub fn detect_double_foul(world: &mut World, fouler: usize, fouled: usize) -> RawIncident {
    let team = world.players[fouler].team;
    RawIncident::new(IncidentKind::DoubleFoul, world.now())
        .with_parties(Some(fouler), Some(fouled))
        .with_team(team)
        .with_program(ProgramId::from_raw(world.current_program_id))
}

/// 技术犯规：责任主体是球队或教练，不计入任何场上球员的个人犯规。
pub fn detect_technical_foul(world: &mut World, actor: usize) -> RawIncident {
    let team = world.players[actor].team;
    RawIncident::new(IncidentKind::TechnicalFoul, world.now())
        .with_parties(Some(actor), None)
        .with_team(team)
        .with_program(ProgramId::from_raw(world.current_program_id))
}

/// 双方犯规的检出：两名对方球员在贴身对抗里同时向对方发力。
///
/// 判据是权威状态里的位置与速度：双方都在场上、分属两队、距离不超过
/// 身体包络，且相对速度达到接触门槛。双方同时发力时相对速度不会特别大，
/// 因此这条的上界低于球员技术犯规与恶意犯规的门槛；三档接触按不同阈值
/// 与不同抽样地址分别判定，同一 tick 只产出其中一类。
pub fn detect_double_foul_sampled(world: &mut World) -> Option<RawIncident> {
    let radius = world.rules.player_radius_ft
        * world.params.policy.w("double_foul_contact_radius_multiple");
    let threshold = world.params.policy.w("double_foul_closing_speed_ftps");
    let ceiling = world.params.policy.w("player_technical_closing_speed_ftps");
    let pair = simultaneous_contact_pair(world, radius, threshold, ceiling)?;
    let (first, second, closing) = pair;
    let rate = world.params.policy.w("double_foul_rate");
    if world.draw(
        crate::draw::Stream::Foul,
        first,
        crate::draw::purpose::DOUBLE_FOUL,
        0,
    ) >= rate
    {
        return None;
    }
    Some(
        detect_double_foul(world, first, second)
            .with_measurement("closing_speed_ftps", closing)
            .with_measurement("threshold_ftps", threshold),
    )
}

/// 球员技术犯规的检出：高速贴身接触之后的不当行为。
///
/// 判据同样是权威状态里的接触与相对速度，门槛高于双方犯规；责任主体是
/// 当时防守的一方，因为这类接触发生在防守人对持球人的对抗之后。
pub fn detect_player_technical(world: &mut World) -> Option<RawIncident> {
    let radius = world.rules.player_radius_ft
        * world.params.policy.w("player_technical_contact_radius_multiple");
    let threshold = world.params.policy.w("player_technical_closing_speed_ftps");
    let ceiling = world.params.policy.w("flagrant_closing_speed_ftps");
    let pair = simultaneous_contact_pair(world, radius, threshold, ceiling)?;
    let (first, second, closing) = pair;
    let defending = world.possession.team.other();
    let actor = if world.players[first].team == defending {
        first
    } else {
        second
    };
    let rate = world.params.policy.w("player_technical_rate");
    if world.draw(
        crate::draw::Stream::Foul,
        actor,
        crate::draw::purpose::PLAYER_TECHNICAL,
        0,
    ) >= rate
    {
        return None;
    }
    Some(
        detect_technical_foul(world, actor)
            .with_measurement("closing_speed_ftps", closing)
            .with_measurement("threshold_ftps", threshold),
    )
}

/// 找出一对符合接触强度区间的对方球员。
///
/// 遍历顺序按球员句柄升序固定，同一 tick 的多个候选取第一对；抽样地址
/// 由这一对的实体身份决定，因此调换事件容器顺序不会改变结果。
fn simultaneous_contact_pair(
    world: &World,
    radius: f32,
    threshold: f32,
    ceiling: f32,
) -> Option<(usize, usize, f32)> {
    for a in 0..world.players.len() {
        let left = &world.players[a];
        if !left.on_court {
            continue;
        }
        for b in (a + 1)..world.players.len() {
            let right = &world.players[b];
            if !right.on_court || left.team == right.team {
                continue;
            }
            if (left.pos - right.pos).length() > radius {
                continue;
            }
            let closing = (left.vel - right.vel).length();
            if closing >= threshold && closing < ceiling {
                return Some((a, b, closing));
            }
        }
    }
    None
}

/// 恶意犯规：防守人在已经失去防守位置的情况下用大力撞人。
///
/// NBA Rule 12A §X 与 FIBA 2026 Article 36：恶意犯规与破坏性犯规的判罚
/// 不同，因此这一层只产出「这是恶意犯规」这个事实，罚则由档案给出。
pub fn detect_flagrant(world: &mut World) -> Option<RawIncident> {
    detect_flagrant_with(world, false)
}

/// 恶意犯规，并指明它是二级（被逐）。
///
/// 二级恶意犯规的判据是犯规的严重程度已经到「取消比赛资格」：它由
/// 档案的复核范围给出，不由抽样决定。因此这个入口让调用方直接给出
/// 等级，判罚后果仍然由裁决层按档案给出。
pub fn detect_flagrant_with(world: &mut World, excessive: bool) -> Option<RawIncident> {
    if !world.live_play() {
        return None;
    }
    let holder = world.holder()?;
    let attacking = world.possession.team;
    let defending = attacking.other();
    let radius = world.params.policy.w("flagrant_contact_radius_multiple");
    let fouler = nearest_defender(world, holder, defending, radius)?;
    let to_holder = world.players[holder].pos - world.players[fouler].pos;
    let gap = to_holder.length().max(f32::EPSILON);
    let relative = world.players[holder].vel - world.players[fouler].vel;
    let closing = relative.dot(to_holder / gap);
    let threshold = world.params.policy.w("flagrant_closing_speed_ftps");
    if closing < threshold {
        return None;
    }
    let rate = world.params.policy.w("flagrant_foul_rate");
    if world.draw(crate::draw::Stream::Foul, fouler, crate::draw::purpose::REACH_IN_FOUL, 0)
        >= rate
    {
        return None;
    }
    Some(
        RawIncident::new(IncidentKind::FlagrantFoul, world.now())
            .with_parties(Some(fouler), Some(holder))
            .with_team(defending)
            .with_measurement("closing_speed_ftps", closing)
            .with_measurement("threshold_ftps", threshold)
            .with_excessive(excessive)
            .with_program(ProgramId::from_raw(world.current_program_id)),
    )
}

/// 清晰路径犯规：防守人在快攻中从背后撞倒快攻球员。
///
/// NBA Rule 12A §XIII 与 FIBA 2026 Article 34.1.2：这类犯规判给进攻方
/// 罚球并保留球权。判据是快攻中防守人从无球状态追上并撞倒持球人。
pub fn detect_clear_path(world: &mut World) -> Option<RawIncident> {
    detect_clear_path_with(world, false)
}

/// 清晰路径犯规，并指明是否由档案升格。
///
/// 归类由档案给出（NBA 普通个人犯规、FIBA 破坏性犯规），因此这一层只
/// 产出「这是清晰路径犯规」这个事实。
pub fn detect_clear_path_with(world: &mut World, upgraded: bool) -> Option<RawIncident> {
    if !world.live_play() {
        return None;
    }
    let holder = world.holder()?;
    let attacking = world.possession.team;
    let defending = attacking.other();
    let transition_window = world.params.policy.w("transition_take_window_seconds");
    if world.now() - world.possession.started_at > f64::from(transition_window) {
        return None;
    }
    let radius = world.params.policy.w("clear_path_contact_radius_multiple");
    let fouler = nearest_defender(world, holder, defending, radius)?;
    // 从背后追上：防守人在持球人身后，且双方相对速度足够大。
    let to_holder = world.players[holder].pos - world.players[fouler].pos;
    let gap = to_holder.length().max(f32::EPSILON);
    let behind = to_holder / gap;
    let relative = world.players[holder].vel - world.players[fouler].vel;
    let closing = relative.dot(behind);
    if closing < world.params.policy.w("clear_path_closing_speed_ftps") {
        return None;
    }
    let rate = world.params.policy.w("clear_path_foul_rate");
    if world.draw(
        crate::draw::Stream::Foul,
        fouler,
        crate::draw::purpose::OFF_BALL_HOLD,
        0,
    ) >= rate
    {
        return None;
    }
    Some(
        RawIncident::new(IncidentKind::ClearPathFoul, world.now())
            .with_parties(Some(fouler), Some(holder))
            .with_team(defending)
            .with_measurement("closing_speed_ftps", closing)
            .with_measurement("transition_window_seconds", transition_window)
            .with_excessive(upgraded)
            .with_program(ProgramId::from_raw(world.current_program_id)),
    )
}

/// 清晰路径犯规的事实：直接给责任方与受影响方。
///
/// 判定在别的层已经成立时（例如复核确认了这次犯规），这一条给出事实。
pub fn clear_path_fact(
    world: &World,
    fouler: usize,
    fouled: usize,
    upgraded: bool,
) -> RawIncident {
    RawIncident::new(IncidentKind::ClearPathFoul, world.now())
        .with_parties(Some(fouler), Some(fouled))
        .with_team(world.players[fouler].team)
        .with_excessive(upgraded)
        .with_program(ProgramId::from_raw(world.current_program_id))
}

/// 恶意犯规的事实：直接给责任方与受影响方以及等级。
pub fn flagrant_fact(
    world: &World,
    fouler: usize,
    fouled: usize,
    excessive: bool,
) -> RawIncident {
    RawIncident::new(IncidentKind::FlagrantFoul, world.now())
        .with_parties(Some(fouler), Some(fouled))
        .with_team(world.players[fouler].team)
        .with_excessive(excessive)
        .with_program(ProgramId::from_raw(world.current_program_id))
}

/// 转换进攻中的故意犯规：落后方在快攻中抢断不成而用犯规阻止对手。
///
/// NBA Rule 12A §XIII 与 FIBA 2026 Article 34 对这类犯规给同样的判罚：
/// 判给进攻方罚球并保留球权。
pub fn detect_transition_take(world: &mut World) -> Option<RawIncident> {
    if !world.live_play() {
        return None;
    }
    let holder = world.holder()?;
    let attacking = world.possession.team;
    let defending = attacking.other();
    // 快攻的判据：本方球权刚建立、进攻方还没有形成落位。
    let transition_window = world.params.policy.w("transition_take_window_seconds");
    if world.now() - world.possession.started_at > f64::from(transition_window) {
        return None;
    }
    // 落后方在快攻中抢断：防守方对持球人犯规，且落后方追分落后。
    let margin = (world.score[World::team_index(attacking)] as i32)
        - (world.score[World::team_index(defending)] as i32);
    if margin >= 0 {
        return None;
    }
    let radius = world.params.policy.w("clear_path_contact_radius_multiple");
    let fouler = nearest_defender(world, holder, defending, radius)?;
    let rate = world.params.policy.w("transition_take_foul_rate");
    if world.draw(crate::draw::Stream::Foul, fouler, crate::draw::purpose::REACH_IN_FOUL, 0)
        >= rate
    {
        return None;
    }
    Some(
        RawIncident::new(IncidentKind::TransitionTakeFoul, world.now())
            .with_parties(Some(fouler), Some(holder))
            .with_team(defending)
            .with_measurement("deficit_points", (-margin) as f32)
            .with_measurement("transition_window_seconds", transition_window)
            .with_program(ProgramId::from_raw(world.current_program_id)),
    )
}

/// 伤病：对抗冲量超过球员身体能承受的程度。
///
/// 判据有两个：碰撞的相对速度够大，并且被撞的一方已经疲劳。疲劳的
/// 球员更可能在对抗里受伤，因此同一场对抗在高强度时把人撞伤、在低
/// 强度时只是吹一次犯规。
pub fn detect_injury(world: &mut World) -> Option<RawIncident> {
    if !world.live_play() {
        return None;
    }
    // 只有场上球员会受伤：替补席上的对抗不进入比赛。
    let mut injured: Option<(usize, usize, f32)> = None;
    for (a, left) in world.players.iter().enumerate() {
        if !left.on_court {
            continue;
        }
        for (b, right) in world.players.iter().enumerate() {
            if a >= b || !right.on_court || left.team == right.team {
                continue;
            }
            let gap = (left.pos - right.pos).length();
            let contact_multiple = world.params.policy.w("injury_contact_radius_multiple");
            if gap > world.rules.min_player_separation_ft * contact_multiple {
                continue;
            }
            let relative = left.vel - right.vel;
            let closing = relative.length();
            // 疲劳的球员更脆弱：同样的冲量对疲劳者更危险。
            let vulnerability = f32::from(1u8) + (f32::from(1u8) - left.stamina);
            let threshold = world.params.policy.w("injury_closing_speed_ftps");
            if closing * vulnerability < threshold {
                continue;
            }
            let rate = world.params.policy.w("injury_rate");
            if world.draw(
                crate::draw::Stream::Foul,
                a,
                crate::draw::purpose::INJURY_CONTACT,
                0,
            ) >= rate
            {
                continue;
            }
            injured = Some((a, b, closing));
            break;
        }
        if injured.is_some() {
            break;
        }
    }
    let (struck, striker, closing) = injured?;
    Some(
        RawIncident::new(IncidentKind::Injury, world.now())
            .with_parties(Some(struck), Some(striker))
            .with_team(world.players[struck].team)
            .with_measurement("closing_speed_ftps", closing)
            .with_measurement("stamina", world.players[struck].stamina)
            .with_program(ProgramId::from_raw(world.current_program_id)),
    )
}

/// 伤病的事实：直接给被撞伤的球员。
///
/// 判定在别的层已经成立时（例如对抗冲量已经被检出），这一条给出事实。
pub fn injury_fact(world: &World, struck: usize) -> RawIncident {
    RawIncident::new(IncidentKind::Injury, world.now())
        .with_parties(Some(struck), None)
        .with_team(world.players[struck].team)
        .with_program(ProgramId::from_raw(world.current_program_id))
}

/// 球队技术犯规：教练或替补席抗议判罚。
///
/// NBA Rule 12A §II 与 FIBA 2026 Article 38：教练、替补席与球队可以成为
/// 规则责任主体，不需要把犯规归给场上球员。技术犯规不计入个人犯规
/// 累计，也不计入球队单节犯规。
///
/// 判据是连续判罚之后教练的抗议：每次不利的判罚给抗议热度加一，达到
/// 阈值时吹一次技术犯规，抗议热度随之归零。
pub fn detect_team_technical(world: &mut World) -> Option<RawIncident> {
    if !world.live_play() {
        return None;
    }
    let threshold = world.params.policy.w("team_technical_protest_threshold");
    if threshold <= f32::default() || world.team_technical_protest <= threshold {
        return None;
    }
    world.team_technical_protest = f32::default();
    Some(
        RawIncident::new(IncidentKind::TeamTechnicalFoul, world.now())
            .with_team(world.possession.team)
            .with_measurement("protest_threshold", threshold)
            .with_program(ProgramId::from_raw(world.current_program_id)),
    )
}

/// 离持球人最近的防守人。
///
/// 距离上限按球员碰撞包络的倍数给出：超过这个距离就没有接触。
fn nearest_defender(
    world: &World,
    holder: usize,
    defending: TeamSide,
    radius_multiple: f32,
) -> Option<usize> {
    world
        .players
        .iter()
        .enumerate()
        .filter(|(index, player)| {
            player.on_court
                && player.team == defending
                && *index != holder
                && (player.pos - world.players[holder].pos).length()
                    <= world.rules.player_radius_ft * radius_multiple
        })
        .min_by(|(left, left_player), (right, right_player)| {
            let left_distance = (left_player.pos - world.players[holder].pos).length_squared();
            let right_distance = (right_player.pos - world.players[holder].pos).length_squared();
            left_distance
                .partial_cmp(&right_distance)
                .expect("finite distances")
                .then(left.cmp(right))
        })
        .map(|(index, _)| index)
}
