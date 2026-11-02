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
    /// 在给定地点（`None` 表示本方底线）发球。
    Inbound {
        team: TeamSide,
        spot: Option<Vec2>,
    },
    /// 直接开始活球回合。
    Live { team: TeamSide },
    /// 比赛结束。
    GameOver,
}

/// 一次裁决。
#[derive(Debug, Clone, Copy)]
pub struct Adjudication {
    pub cause: StoppageCause,
    /// 需要结束当前统计回合时给出原因与明细。
    pub possession_end: Option<(PossessionEndCause, &'static str)>,
    pub clock: ClockAdjustment,
    pub restart: RestartPlan,
}

impl Adjudication {
    /// 违例裁决的常用形状：结束回合并由对方在给定地点发球。
    pub fn violation(kind: ViolationKind, restart: RestartPlan) -> Self {
        Self {
            cause: StoppageCause::Violation(kind),
            possession_end: Some((
                PossessionEndCause::TurnoverViolation,
                kind.as_str(),
            )),
            clock: ClockAdjustment::default(),
            restart,
        }
    }
}

impl World {
    /// 唯一写入入口：比分之外的停表后果都在这里落。
    pub fn adjudicate(&mut self, adjudication: Adjudication) {
        if let Some((cause, detail)) = adjudication.possession_end {
            self.end_possession(cause, detail, 0);
        }
        if let Some(shot_clock) = adjudication.clock.shot_clock {
            self.clock.set_shot_clock(shot_clock);
        }
        match adjudication.restart {
            RestartPlan::Inbound { team, spot } => {
                self.inbound_spot = spot;
                // 取消动作的原因就是停表原因：事实里保留违例名，便于回放与审计。
                let reason = match adjudication.cause {
                    StoppageCause::Violation(kind) => kind.as_str(),
                    StoppageCause::MadeBasket => "made_basket",
                    StoppageCause::Foul => "foul",
                    StoppageCause::Timeout => "timeout",
                    StoppageCause::PeriodEnd => "period_end",
                    StoppageCause::GameEnd => "game_end",
                    StoppageCause::PossessionChange => "possession_change",
                };
                self.cancel_actions(reason);
                self.set_phase(Phase::DeadBall {
                    team,
                    ready_at: self.now() + f64::from(self.rules.inbound_setup_seconds),
                });
            }
            RestartPlan::Live { team } => {
                self.start_possession(team, PossessionKind::Live);
            }
            RestartPlan::GameOver => {
                self.set_phase(Phase::Finished);
            }
        }
    }
}
