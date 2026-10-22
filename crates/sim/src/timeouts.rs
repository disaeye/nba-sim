//! 暂停：配额账本与申请门控。
//!
//! 暂停不是「随时可用的资源」：只有持球方（或死球状态下的任意一方）可以
//! 申请，且同时受整场配额、第四节配额、最后时段配额三层约束。把这三层
//! 分开记账，暂停才不会在第四节被无限制地花光。

use nba_domain::rules::GameRules;

use crate::action::Fact;
use crate::state::{Phase, TeamSide};
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

impl TimeoutBudget {
    pub fn new(rules: &GameRules) -> Self {
        Self {
            remaining: rules.league.timeouts_per_game,
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
    let dead = !matches!(world.phase, Phase::Live | Phase::PassFlight { .. } | Phase::ShotFlight { .. });
    let holder_team = world.holder().map(|handle| world.players[handle].team);
    if !dead && holder_team != Some(team) {
        return Err(TimeoutRejection::NoPossession);
    }
    let rules = &world.rules;
    let ledger = &world.timeouts[World::team_index(team)];
    if ledger.remaining == 0 {
        return Err(TimeoutRejection::NoRemaining);
    }
    if world.clock.period == rules.league.regulation_periods
        && ledger.used_in_fourth >= rules.league.timeouts_fourth_quarter_max
    {
        return Err(TimeoutRejection::FourthQuarterExhausted);
    }
    if world.clock.period >= rules.league.bonus_final_period
        && world.clock.game_clock <= rules.league.timeouts_final_seconds as f64
        && ledger.used_in_final_window >= rules.league.timeouts_final_max
    {
        return Err(TimeoutRejection::FinalWindowExhausted);
    }
    Ok(())
}

/// 申请并执行暂停。返回是否成功。
pub fn request(world: &mut World, team: TeamSide) -> bool {
    if can_request(world, team).is_err() {
        return false;
    }
    let duration = world.rules.tactics.timeout_duration_seconds as f64;
    let index = World::team_index(team);
    let period = world.clock.period;
    let ledger = &mut world.timeouts[index];
    if ledger.period != period {
        ledger.period = period;
        ledger.used_in_final_window = 0;
    }
    ledger.remaining -= 1;
    if period == world.rules.league.regulation_periods {
        ledger.used_in_fourth += 1;
    }
    if period >= world.rules.league.bonus_final_period
        && world.clock.game_clock <= world.rules.league.timeouts_final_seconds as f64
    {
        ledger.used_in_final_window += 1;
    }
    world.phase = Phase::Timeout {
        team,
        resume_at: world.now() + duration,
    };
    world.emit(Fact::Timeout {
        team,
        remaining: [world.timeouts[0].remaining, world.timeouts[1].remaining],
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
    // 暂停已经回答了这一波得分，同一波不该在下一个回合再触发一次。
    world.scoring_run = 0;
    true
}
