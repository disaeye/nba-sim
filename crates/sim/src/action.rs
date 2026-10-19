//! 动作接口：决策层的输出、结算层的输出、结算带来的状态变化。
//!
//! 动作是因果链的单元。结算产出的是**状态变化**，不是结果：掩护产出分离与
//! 防守人路线，突破产出是否过掉首要防守人，传球产出接球质量。下一次决策读
//! 这些字段，出手位置与命中率都是动作链的后果。

use glam::Vec2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TeamSide {
    Home,
    Away,
}

impl TeamSide {
    pub fn other(self) -> Self {
        match self {
            Self::Home => Self::Away,
            Self::Away => Self::Home,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Away => "away",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotKind {
    CatchShoot,
    PullUp,
    StepBack,
    Turnaround,
    Hook,
    Floater,
    Layup,
    DrivingLayup,
    CuttingLayup,
    RunningLayup,
    ReverseLayup,
    Dunk,
    DrivingDunk,
    CuttingDunk,
    RunningDunk,
    AlleyOop,
    Putback,
    RunningJump,
}

impl ShotKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CatchShoot => "catch_shoot",
            Self::PullUp => "pull_up",
            Self::StepBack => "step_back",
            Self::Turnaround => "turnaround",
            Self::Hook => "hook",
            Self::Floater => "floater",
            Self::Layup => "layup",
            Self::DrivingLayup => "driving_layup",
            Self::CuttingLayup => "cutting_layup",
            Self::RunningLayup => "running_layup",
            Self::ReverseLayup => "reverse_layup",
            Self::Dunk => "dunk",
            Self::DrivingDunk => "driving_dunk",
            Self::CuttingDunk => "cutting_dunk",
            Self::RunningDunk => "running_dunk",
            Self::AlleyOop => "alley_oop",
            Self::Putback => "putback",
            Self::RunningJump => "running_jump",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "catch_shoot" => Self::CatchShoot,
            "pull_up" => Self::PullUp,
            "step_back" => Self::StepBack,
            "turnaround" => Self::Turnaround,
            "hook" => Self::Hook,
            "floater" => Self::Floater,
            "layup" => Self::Layup,
            "driving_layup" => Self::DrivingLayup,
            "cutting_layup" => Self::CuttingLayup,
            "running_layup" => Self::RunningLayup,
            "reverse_layup" => Self::ReverseLayup,
            "dunk" => Self::Dunk,
            "driving_dunk" => Self::DrivingDunk,
            "cutting_dunk" => Self::CuttingDunk,
            "running_dunk" => Self::RunningDunk,
            "alley_oop" => Self::AlleyOop,
            "putback" => Self::Putback,
            "running_jump" => Self::RunningJump,
            other => panic!("unknown shot kind `{other}`"),
        }
    }

    /// 出手来源：drive / cut / post / pull_up / putback / transition / set。
    pub fn creation(self) -> &'static str {
        match self {
            Self::CatchShoot => "set",
            Self::PullUp | Self::StepBack | Self::Floater => "pull_up",
            Self::Turnaround | Self::Hook => "post",
            Self::Layup | Self::DrivingLayup | Self::DrivingDunk | Self::ReverseLayup => "drive",
            Self::CuttingLayup | Self::CuttingDunk => "cut",
            Self::RunningLayup | Self::RunningDunk | Self::RunningJump | Self::AlleyOop => {
                "transition"
            }
            Self::Putback => "putback",
            Self::Dunk => "cut",
        }
    }

    pub fn is_rim_finish(self) -> bool {
        matches!(
            self,
            Self::Layup
                | Self::DrivingLayup
                | Self::CuttingLayup
                | Self::RunningLayup
                | Self::ReverseLayup
                | Self::Dunk
                | Self::DrivingDunk
                | Self::CuttingDunk
                | Self::RunningDunk
                | Self::AlleyOop
                | Self::Putback
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassKind {
    Chest,
    Bounce,
    Lob,
    Outlet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenKind {
    OnBall,
    OffBall,
    HandOff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Dribble,
    Drive,
    Pass(PassKind),
    Screen(ScreenKind),
    Cut,
    PostUp,
    Shoot(ShotKind),
    Relocate,
}

impl ActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dribble => "Dribble",
            Self::Drive => "Drive",
            Self::Pass(_) => "Pass",
            Self::Screen(_) => "Screen",
            Self::Cut => "Cut",
            Self::PostUp => "PostUp",
            Self::Shoot(_) => "Shoot",
            Self::Relocate => "Relocate",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ActionIntent {
    pub actor: usize,
    pub kind: ActionKind,
    pub target: Option<usize>,
    pub from_ft: Vec2,
    pub to_ft: Vec2,
    pub start_time: f64,
    pub duration: f64,
}

impl ActionIntent {
    pub fn end_time(&self) -> f64 {
        self.start_time + self.duration
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    Completed,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefenderRoute {
    Over,
    Under,
    Switch,
    Trapped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnoverKind {
    BadPass,
    LostBall,
    OffensiveFoul,
    Travel,
    DoubleDribble,
    ShotClock,
    OutOfBounds,
}

impl TurnoverKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BadPass => "bad_pass",
            Self::LostBall => "lost_ball",
            Self::OffensiveFoul => "offensive_foul",
            Self::Travel => "travel",
            Self::DoubleDribble => "double_dribble",
            Self::ShotClock => "shot_clock",
            Self::OutOfBounds => "out_of_bounds",
        }
    }
}

/// 结算产出的状态变化。每个动作必须写出它能改变的全部字段，
/// 下一次决策只读这些字段，不读动作历史。
#[derive(Debug, Clone, Default)]
pub struct StateDelta {
    pub separation_ft: Option<f32>,
    pub defender_route: Option<DefenderRoute>,
    pub help_arrival_seconds: Option<f32>,
    pub balance: Option<f32>,
    pub control: Option<usize>,
    pub turnover: Option<TurnoverKind>,
    pub shot_clock_reset: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct ActionOutcome {
    pub actor: usize,
    pub kind: ActionKind,
    pub completion: Completion,
    pub delta: StateDelta,
    pub facts: Vec<Fact>,
}

#[derive(Debug, Clone)]
pub enum Fact {
    PossessionStarted {
        team: TeamSide,
        live: bool,
    },
    Pass {
        pass_id: u64,
        from: usize,
        to: usize,
        kind: PassKind,
        completed: bool,
    },
    Screen {
        screener: usize,
        on_ball: bool,
        contact: bool,
        route: DefenderRoute,
        separation_ft: f32,
    },
    Drive {
        driver: usize,
        reached: bool,
        defender_beaten: bool,
    },
    Shot {
        shot_id: u64,
        shooter: usize,
        kind: ShotKind,
        zone: &'static str,
        distance_ft: f32,
        contest: f32,
        made: bool,
        points: u8,
        assisted_by: Option<usize>,
    },
    Foul {
        fouler: usize,
        fouled: usize,
        shooting: bool,
        free_throws: u8,
        and_one: bool,
    },
    FreeThrow {
        shooter: usize,
        made: bool,
        index: u8,
        total: u8,
    },
    Rebound {
        player: usize,
        offensive: bool,
        distance_ft: f32,
    },
    Turnover {
        player: usize,
        kind: TurnoverKind,
    },
    Substitution {
        team: TeamSide,
        incoming: usize,
        outgoing: usize,
    },
    PeriodEnd {
        period: u32,
    },
    GameEnd,
}
