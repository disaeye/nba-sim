//! 暂停：配额账本与申请门控。
//!
//! 暂停不是「随时可用的资源」：只有持球方（或死球状态下的任意一方）可以
//! 申请，且同时受整场配额、第四节配额、最后时段配额三层约束。把这三层
//! 分开记账，暂停才不会在第四节被无限制地花光。

use nba_domain::rules::GameRules;

use crate::action::Fact;
use crate::state::{Phase, ResumePhase, TeamSide};
use crate::world::World;

/// 一支球队的暂停账本。
#[derive(Debug, Clone, Copy, Default)]
pub struct TimeoutBudget {
    /// 本场剩余暂停数。
    pub remaining: u32,
    /// 第四节已用次数。
    pub used_in_fourth: u32,
    /// 当前节的最后时段已用次数。
    pub used_in_final_window: u32,
    /// 当前节次，用来在换节时重置最后时段计数。
    pub period: u32,
}

/// 暂停来源：决定这次停表占用联赛档案里的哪一档时长。
///
/// 完整暂停、短暂停与加时暂停是三种不同的程序，时长不同。请求入口按
/// 上下文推断来源，[`request_with_origin`] 让调用方显式指定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutOrigin {
    /// 常规的完整暂停。
    Full,
    /// 末段死球里的短暂停；档案没有短暂停时回落到完整档。
    Short,
    /// 加时节的暂停。
    Overtime,
}

/// 按来源与节次读取联赛档案里的暂停时长。
///
/// FIBA 2026 Article 18 只给完整暂停（`short_seconds == 0`），短暂停来源
/// 因此自动回落到完整档。`overtime_seconds` 只在加时节生效。
pub fn duration_for(rules: &GameRules, period: u32, origin: TimeoutOrigin) -> f64 {
    let durations = &rules.league.timeouts.timeout_durations;
    let overtime = period > rules.league.periods.regulation_periods;
    let seconds = if overtime {
        durations.overtime_seconds
    } else {
        match origin {
            TimeoutOrigin::Full | TimeoutOrigin::Overtime => durations.full_seconds,
            TimeoutOrigin::Short if durations.short_seconds > 0.0 => durations.short_seconds,
            TimeoutOrigin::Short => durations.full_seconds,
        }
    };
    assert!(
        seconds > 0.0 && seconds.is_finite(),
        "联赛档案必须给出正的有限暂停时长"
    );
    f64::from(seconds)
}

/// 从当前上下文推断这次请求的暂停来源。
fn infer_origin(world: &World) -> TimeoutOrigin {
    let period = world.clock.period();
    if period > world.rules.league.periods.regulation_periods {
        return TimeoutOrigin::Overtime;
    }
    let dead = !world.phase().is_live_gameplay() || world.holder().is_none();
    let last_period = period >= world.rules.league.periods.regulation_periods;
    let in_final_window =
        world.clock.game_clock() <= f64::from(world.rules.league.timeouts.timeouts_final_seconds);
    if dead && last_period && in_final_window {
        TimeoutOrigin::Short
    } else {
        TimeoutOrigin::Full
    }
}

impl TimeoutBudget {
    pub fn new(rules: &GameRules) -> Self {
        Self {
            remaining: rules.league.timeouts.timeouts_per_game,
            used_in_fourth: 0,
            used_in_final_window: 0,
            period: 0,
        }
    }
}

/// 暂停的拒给原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutRejection {
    /// 活球且本方没有球权。
    NoPossession,
    /// 整场配额耗尽。
    NoRemaining,
    /// 第四节配额耗尽。
    FourthQuarterExhausted,
    /// 最后时段配额耗尽。
    FinalWindowExhausted,
    /// 比赛已经结束。
    GameOver,
}

/// 是否可以申请暂停。
///
/// 死球状态下两队都可以叫暂停（真实比赛里教练在哨响后立刻叫停）；
/// 活球状态下只有持球方可以，因为球在对方手上时叫暂停等于换球权。
pub fn can_request(world: &World, team: TeamSide) -> Result<(), TimeoutRejection> {
    if world.finished {
        return Err(TimeoutRejection::GameOver);
    }
    let dead = !world.phase().is_live_gameplay() || world.holder().is_none();
    let holder_team = world.holder().map(|handle| world.players[handle].team);
    if !dead && holder_team != Some(team) {
        return Err(TimeoutRejection::NoPossession);
    }
    let rules = &world.rules;
    let ledger = &world.timeouts[World::team_index(team)];
    if ledger.remaining == 0 {
        return Err(TimeoutRejection::NoRemaining);
    }
    if world.clock.period() == rules.league.periods.regulation_periods
        && ledger.used_in_fourth >= rules.league.timeouts.timeouts_fourth_quarter_max
    {
        return Err(TimeoutRejection::FourthQuarterExhausted);
    }
    if world.clock.period() >= rules.league.fouls.bonus_final_period
        && world.clock.game_clock() <= rules.league.timeouts.timeouts_final_seconds as f64
        && ledger.used_in_final_window >= rules.league.timeouts.timeouts_final_max
    {
        return Err(TimeoutRejection::FinalWindowExhausted);
    }
    Ok(())
}

/// 申请并执行暂停。返回是否成功。
///
/// 时长按 [`infer_origin`] 推断的来源从联赛档案读取；`request_with_origin`
/// 让调用方显式指定来源。
pub fn request(world: &mut World, team: TeamSide) -> bool {
    let origin = infer_origin(world);
    request_with_origin(world, team, origin)
}

/// 指定暂停来源申请并执行暂停。返回是否成功。
pub fn request_with_origin(
    world: &mut World,
    team: TeamSide,
    origin: TimeoutOrigin,
) -> bool {
    if can_request(world, team).is_err() {
        return false;
    }
    let period = world.clock.period();
    let duration = duration_for(&world.rules, period, origin);
    let index = World::team_index(team);
    let previous_phase = world.phase();
    assert!(
        !matches!(previous_phase, Phase::Timeout { .. }),
        "timeout cannot be requested during an active timeout"
    );
    let ledger = &mut world.timeouts[index];
    if ledger.period != period {
        ledger.period = period;
        ledger.used_in_final_window = 0;
    }
    ledger.remaining -= 1;
    if period == world.rules.league.periods.regulation_periods {
        ledger.used_in_fourth += 1;
    }
    if period >= world.rules.league.fouls.bonus_final_period
        && world.clock.game_clock() <= world.rules.league.timeouts.timeouts_final_seconds as f64
    {
        ledger.used_in_final_window += 1;
    }
    let now = world.now();
    let resume_phase = ResumePhase::capture(previous_phase);
    let resume_at = now + duration;
    world.set_timeout_phase(team, resume_at, resume_phase, now, previous_phase);
    // 末段暂停可以选择前场发球：把发球点移到前场边线的标记处。
    if world
        .rules
        .league
        .in_final_window(period, world.clock.game_clock())
    {
        let mid = world.rules.court.width_ft * 0.5;
        let attacking_right = world.attacking_right(team);
        let offset = world.params.policy.g("frontcourt_inbound_offset_ft");
        let radius = world.rules.player_radius_ft;
        let x = if attacking_right {
            mid + offset
        } else {
            mid - offset
        };
        world.inbound_spot = Some(glam::Vec2::new(
            x.clamp(radius, world.rules.court.width_ft - radius),
            radius,
        ));
    }
    world.emit(Fact::Timeout {
        team,
        remaining: [world.timeouts[0].remaining, world.timeouts[1].remaining],
        mandatory: false,
    });
    true
}

/// 强制暂停：联赛规则在固定时点自动停表，不靠某一方主动申请。
///
/// NBA 2025-26 Rule 5 §VI 要求第二节走到 6:59 以下、第四节走到 2:59 以下
/// 后的第一次死球停表，并给每队记上对应的暂停数。这个函数只在死球时
/// 触发：活球中停表会把一次正在进行的进攻直接掐断。
///
/// 返回这次触发的强制暂停是否停表。
pub fn enforce_mandatory(world: &mut World) -> bool {
    if world.finished {
        return false;
    }
    let period = world.clock.period();
    let remaining = world.clock.game_clock();
    let triggers = &world.rules.league.timeouts.mandatory_timeouts;
    let mut due: Option<(usize, nba_domain::league::MandatoryTimeout)> = None;
    for (index, trigger) in triggers.iter().enumerate() {
        if world.mandatory_timeouts_fired & (1 << index) != 0 {
            continue;
        }
        if trigger.period == period && remaining <= f64::from(trigger.remaining_seconds) {
            due = Some((index, *trigger));
            break;
        }
    }
    let Some((index, trigger)) = due else {
        return false;
    };
    // 只允许在死球时停表：活球期间进攻方正在推进，哨子和停表都不成立。
    let dead = !world.phase().is_live_gameplay() || world.holder().is_none();
    if !dead {
        return false;
    }
    // 节次交接期间的时钟还没重置，而缩短节时长的比赛里本节一开始就已经
    // 低于阈值。开节发球属于上一节留下的程序：第一个回合还没开始，
    // 这时停表会把开节发球的方向换掉。零剩余时间同样不成立。
    if world.period_start_inbound_pending || remaining <= 0.0 {
        return false;
    }
    // 罚球、跳球与已经开始的暂停都是进行中的死球程序，强制暂停必须等它们结束。
    if world.free_throw_pending()
        || matches!(
            world.phase(),
            Phase::JumpBall { .. } | Phase::Timeout { .. }
        )
    {
        return false;
    }
    world.mandatory_timeouts_fired |= 1 << index;
    let charge = trigger.charge;
    for ledger in world.timeouts.iter_mut() {
        ledger.remaining = ledger.remaining.saturating_sub(charge);
    }
    let previous_phase = world.phase();
    let resume_phase = ResumePhase::capture(previous_phase);
    let now = world.now();
    let resume_at = now + f64::from(trigger.duration_seconds);
    world.set_timeout_phase(
        world.possession.team,
        resume_at,
        resume_phase,
        now,
        previous_phase,
    );
    world.emit(Fact::Timeout {
        team: world.possession.team,
        remaining: [world.timeouts[0].remaining, world.timeouts[1].remaining],
        mandatory: true,
    });
    true
}

/// 教练在对手打出一波连续得分后叫暂停止血。
///
/// 阈值来自 `RotationRules::timeout_run_threshold`：连续被对手得到的分数
/// 达到该值就停表。是否真的批准由 [`can_request`] 决定，因此这个函数
/// 在没有球权时会静默地什么都不做。
pub fn coach_stop_run(world: &mut World) -> bool {
    let Some(index) = world.run_against() else {
        return false;
    };
    let team = if index == 0 {
        TeamSide::Home
    } else {
        TeamSide::Away
    };
    if !request(world, team) {
        return false;
    }
    world.scoring_run = 0;
    true
}
