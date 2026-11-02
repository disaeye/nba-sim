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
    /// 比赛钟要设定的值；`None` 表示不改。
    pub game_clock: Option<f64>,
}

impl ClockAdjustment {
    /// 展开成时钟要求：裁决只描述要什么，改钟由 `ClockController` 执行。
    pub fn directives(&self) -> Vec<crate::clock::ClockDirective> {
        let mut directives = Vec::new();
        if let Some(seconds) = self.shot_clock {
            directives.push(crate::clock::ClockDirective::ShotClock(seconds));
        }
        if let Some(seconds) = self.game_clock {
            directives.push(crate::clock::ClockDirective::GameClock(seconds));
        }
        directives
    }
}

/// 球权的控制状态：谁拿着球、球是否活着。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlState {
    /// 某名球员持球。
    Held(usize),
    /// 活球无人控制（地滚球、拍飞的球）。
    Loose,
    /// 球在空中（传球或投篮飞行中）。
    InFlight,
    /// 死球程序期间。
    DeadBall,
}

/// 发球地点：底线发球与本场地点发球分开记。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InboundLocation {
    /// 本方底线（得分后、节开局）。
    OwnBaseline,
    /// 指定的场地坐标（边线发球、违例的干扰点）。
    Spot(Vec2),
}

/// 罚则计划：一次犯规判什么，与它怎么恢复分开描述。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PenaltyPlan {
    pub free_throws: u8,
    pub and_one: bool,
    pub retain_possession: bool,
    pub possession_after: Option<TeamSide>,
    pub technical: bool,
}

impl PenaltyPlan {
    pub fn none() -> Self {
        Self {
            free_throws: 0,
            and_one: false,
            retain_possession: false,
            possession_after: None,
            technical: false,
        }
    }

    pub fn turnover(team: TeamSide) -> Self {
        Self {
            possession_after: Some(team),
            ..Self::none()
        }
    }

    pub fn free_throws(count: u8) -> Self {
        Self {
            free_throws: count,
            ..Self::none()
        }
    }
}

/// 恢复上下文：一次停表之后在哪个阶段、什么时间、哪队发球。
#[derive(Debug, Clone, Copy)]
pub struct ResumeContext {
    pub team: TeamSide,
    pub inbound: InboundLocation,
    pub ready_at: f64,
}

/// 恢复方式。
#[derive(Debug, Clone, Copy)]
pub enum RestartPlan {
    /// 在给定地点（`OwnBaseline` 表示本方底线）发球；`ready_after` 是停表时长。
    Inbound {
        team: TeamSide,
        location: InboundLocation,
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
        if !adjudication.clock.directives().is_empty() {
            let mut controller = crate::clock::ClockController::new(&mut self.clock);
            for directive in adjudication.clock.directives() {
                controller.apply(directive);
            }
        }
        match adjudication.restart {
            RestartPlan::Inbound {
                team,
                location,
                ready_after,
            } => {
                self.inbound_spot = match location {
                    InboundLocation::OwnBaseline => None,
                    InboundLocation::Spot(spot) => Some(spot),
                };
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
                // 恢复上下文把「哪队、在哪发、什么时候能发」打成一包：
                // 阶段赋值、发球地点与计时都读同一份描述，不各算一遍。
                let resume = ResumeContext {
                    team,
                    inbound: location,
                    ready_at: self.now()
                        + ready_after.unwrap_or(f64::from(self.rules.inbound_setup_seconds)),
                };
                self.cancel_actions(reason);
                self.set_phase(Phase::DeadBall {
                    team: resume.team,
                    ready_at: resume.ready_at,
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
