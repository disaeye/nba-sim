//! 停表裁决：停表原因、时钟调整与恢复方式收拢到单一提交入口。
//!
//! 依据 `docs/dev/current/adjudication_design.md`：各结算分支只负责发布事实
//! 并构造一次 [`Adjudication`]，比分之外的状态写入统一经 [`World::adjudicate`]。

use glam::Vec2;
use nba_domain::event::PossessionEndCause;

use crate::state::{Phase, PossessionKind};
use crate::world::World;
use crate::TeamSide;

/// 停表原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoppageCause {
    MadeBasket,
    /// 罚球程序的结算（不论命中与否）。
    FreeThrow,
    Violation(ViolationKind),
    Foul,
    Timeout,
    PeriodEnd,
    GameEnd,
    /// 球权在活球中直接转移（篮板、抢断、松球回收），没有停表。
    PossessionChange,
}

/// 违例细分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationKind {
    OutOfBounds,
    EightSecond,
    Backcourt,
    CloselyGuarded,
    ShotClock,
    Travel,
    DoubleDribble,
}

impl ViolationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OutOfBounds => "out_of_bounds",
            Self::EightSecond => "eight_second",
            Self::Backcourt => "backcourt",
            Self::CloselyGuarded => "closely_guarded",
            Self::ShotClock => "shot_clock",
            Self::Travel => "travel",
            Self::DoubleDribble => "double_dribble",
        }
    }
}

/// 时钟调整：只记录与默认推进不同的部分。
#[derive(Debug, Clone, Copy, Default)]
pub struct ClockAdjustment {
    /// 进攻时钟要设定的值；`None` 表示不改。
    pub shot_clock: Option<f64>,
}

/// 恢复方式。
#[derive(Debug, Clone, Copy)]
pub enum RestartPlan {
    /// 在给定地点（`None` 表示本方底线）发球；`ready_after` 是停表时长。
    Inbound {
        team: TeamSide,
        spot: Option<Vec2>,
        ready_after: Option<f64>,
    },
    /// 直接开始活球回合。
    Live { team: TeamSide },
    /// 停表但不结束回合，到点后按当前状态恢复（换人、延迟发球）。
    Stoppage { team: TeamSide, ready_after: f64 },
    /// 节次间歇：到点后按下个阶段继续。
    PeriodBreak {
        resume_at: f64,
        next_stage: crate::state::MatchStage,
    },
    /// 进入罚球程序：球交给主罚人，阶段切到 FreeThrows。
    FreeThrows {
        shooter: usize,
        total: u8,
        remaining: u8,
        made: u8,
        and_one: bool,
        retain_possession: bool,
        foul_id: u64,
        technical_retained_team: Option<TeamSide>,
        after_technical: Option<crate::resolve::freethrow::FreeThrowFollowUp>,
    },
    /// 比赛结束。
    GameOver,
}

/// 一次裁决。
#[derive(Debug, Clone, Copy)]
pub struct Adjudication {
    pub cause: StoppageCause,
    /// 需要结束当前统计回合时给出原因与明细。
    pub possession_end: Option<(PossessionEndCause, &'static str)>,
    /// 本次得分（命中或罚球结算），已结束的回合带上它。
    pub points: u32,
    pub clock: ClockAdjustment,
    pub restart: RestartPlan,
}

impl Adjudication {
    /// 基础形状：只有停表原因与恢复方式，不结束回合、不得分、不改时钟。
    pub fn new(cause: StoppageCause, restart: RestartPlan) -> Self {
        Self {
            cause,
            possession_end: None,
            points: 0,
            clock: ClockAdjustment::default(),
            restart,
        }
    }

    /// 结束当前统计回合。
    pub fn ending(mut self, cause: PossessionEndCause, detail: &'static str) -> Self {
        self.possession_end = Some((cause, detail));
        self
    }

    /// 本次得分。
    pub fn scoring(mut self, points: u32) -> Self {
        self.points = points;
        self
    }

    /// 时钟调整。
    pub fn clock(mut self, clock: ClockAdjustment) -> Self {
        self.clock = clock;
        self
    }

    /// 违例裁决的常用形状：结束回合并由对方在给定地点发球。
    pub fn violation(kind: ViolationKind, restart: RestartPlan) -> Self {
        Self::new(StoppageCause::Violation(kind), restart)
            .ending(PossessionEndCause::TurnoverViolation, kind.as_str())
    }
}

impl World {
    /// 唯一写入入口：比分之外的停表后果都在这里落。
    pub fn adjudicate(&mut self, adjudication: Adjudication) {
        if let Some((cause, detail)) = adjudication.possession_end {
            self.end_possession(cause, detail, adjudication.points);
        }
        if let Some(shot_clock) = adjudication.clock.shot_clock {
            self.clock.set_shot_clock(shot_clock);
        }
        match adjudication.restart {
            RestartPlan::Inbound {
                team,
                spot,
                ready_after,
            } => {
                self.inbound_spot = spot;
                // 取消动作的原因就是停表原因：事实里保留违例名，便于回放与审计。
                let reason = match adjudication.cause {
                    StoppageCause::Violation(kind) => kind.as_str(),
                    StoppageCause::MadeBasket => "made_basket",
                    StoppageCause::FreeThrow => "free_throw",
                    StoppageCause::Foul => "foul",
                    StoppageCause::Timeout => "timeout",
                    StoppageCause::PeriodEnd => "period_end",
                    StoppageCause::GameEnd => "game_end",
                    StoppageCause::PossessionChange => "possession_change",
                };
                self.cancel_actions(reason);
                let wait = ready_after.unwrap_or(f64::from(self.rules.inbound_setup_seconds));
                self.set_phase(Phase::DeadBall {
                    team,
                    ready_at: self.now() + wait,
                });
            }
            RestartPlan::Live { team } => {
                self.start_possession(team, PossessionKind::Live);
            }
            RestartPlan::Stoppage { team, ready_after } => {
                self.set_phase(Phase::DeadBall {
                    team,
                    ready_at: self.now() + ready_after,
                });
            }
            RestartPlan::PeriodBreak {
                resume_at,
                next_stage,
            } => {
                self.cancel_actions("period_end");
                self.set_period_phase(Phase::PeriodBreak {
                    resume_at,
                    next_stage,
                });
            }
            RestartPlan::FreeThrows {
                shooter,
                total,
                remaining,
                made,
                and_one,
                retain_possession,
                foul_id,
                technical_retained_team,
                after_technical,
            } => {
                self.set_phase(Phase::FreeThrows {
                    shooter,
                    total,
                    remaining,
                    made,
                    next_at: self.now() + f64::from(self.rules.free_throw_interval_seconds),
                    and_one,
                    retain_possession,
                    foul_id,
                    technical_retained_team,
                    after_technical,
                });
                self.set_ball_held(shooter);
            }
            RestartPlan::GameOver => {
                self.cancel_actions("game_end");
                self.set_phase(Phase::Finished);
            }
        }
    }
}
