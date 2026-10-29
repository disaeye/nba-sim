//! 动作接口：决策层的输出、结算层的输出、结算带来的状态变化。
//!
//! 动作是因果链的单元。结算产出的是**状态变化**，不是结果：掩护产出分离与
//! 防守人路线，突破产出是否过掉首要防守人，传球产出接球质量。下一次决策读
//! 这些字段，出手位置与命中率都是动作链的后果。

use glam::Vec2;

pub use nba_domain::event::FoulKind;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

    /// 出手来源：drive / cut / post / pull_up / putback / transition / set。
    /// 出手在进攻分类里的归属。
    ///
    /// 分类与档案的 `kind_by_creation` 一致：接球投与站着完成的扣篮、上篮
    /// 属于 set；突破后的上篮、抛投属于 drive；低位与转身属于 post；空切
    /// 属于 cut；快攻属于 transition；补篮属于 putback。
    pub fn creation(self) -> &'static str {
        match self {
            Self::CatchShoot | Self::Dunk | Self::Layup => "set",
            Self::PullUp | Self::StepBack => "pull_up",
            Self::Turnaround | Self::Hook => "post",
            Self::DrivingLayup
            | Self::DrivingDunk
            | Self::ReverseLayup
            | Self::Floater => "drive",
            Self::CuttingLayup | Self::CuttingDunk => "cut",
            Self::RunningLayup | Self::RunningDunk | Self::RunningJump | Self::AlleyOop => {
                "transition"
            }
            Self::Putback => "putback",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PassKind {
    Chest,
    Bounce,
    Lob,
    Outlet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ScreenKind {
    OnBall,
    OffBall,
    HandOff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ActionKind {
    Dribble,
    Drive,
    Finish,
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
            Self::Finish => "Finish",
            Self::Pass(_) => "Pass",
            Self::Screen(_) => "Screen",
            Self::Cut => "Cut",
            Self::PostUp => "PostUp",
            Self::Shoot(_) => "Shoot",
            Self::Relocate => "Relocate",
        }
    }

    /// 动作族标签，供决策审计与协议投影使用。
    pub fn label(self) -> &'static str {
        match self {
            Self::Shoot(_) => "shoot",
            Self::Drive => "drive",
            Self::Pass(_) => "pass",
            Self::Screen(_) => "screen",
            Self::Cut => "cut",
            Self::PostUp => "post_up",
            Self::Dribble => "hold",
            Self::Finish => "finish",
            Self::Relocate => "relocate",
        }
    }

    /// 动作族编号，供决策观测与过程判据使用。
    pub fn family(self) -> u8 {
        match self {
            Self::Shoot(_) => 0,
            Self::Drive => 1,
            Self::Pass(_) => 2,
            Self::Screen(_) => 3,
            Self::Cut => 4,
            Self::PostUp => 5,
            Self::Dribble => 6,
            Self::Finish => 7,
            Self::Relocate => 8,
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
        team: TeamSide,
        on_ball: bool,
        contact: bool,
        route: DefenderRoute,
        separation_ft: f32,
        /// 掩护人与防守人接触时，掩护人的力量值。
        strength: f32,
        /// 掩护接触使防守人改变的位置距离（英尺）。
        displacement_ft: f32,
    },
    Drive {
        driver: usize,
        reached: bool,
        defender_beaten: bool,
        distance_ft: f32,
        separation_ft: f32,
    },
    /// 背身单打：持球人进入低位开始背打（本身不产出结果，结果由后续出手给出）。
    PostUp {
        player: usize,
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
        kind: FoulKind,
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
    /// 投失后产生的真实篮板机会：场上人数与满足控球半径的人数。
    ReboundOpportunity {
        players: u8,
        contestable: u8,
    },
    /// 二次进攻回合的边界：前场篮板被控制后开启。
    SecondChanceStarted {
        rebounder: usize,
        possession: u64,
    },
    Cut {
        cutter: usize,
        arrived: bool,
        defender_distance_ft: f32,
    },
    BoxOut {
        player: usize,
        distance_ft: f32,
    },
    DefenseResponsibilityChanged {
        player: usize,
        from: &'static str,
        to: &'static str,
    },
    Turnover {
        player: usize,
        kind: TurnoverKind,
    },
    Steal {
        player: usize,
        victim: usize,
    },
    Substitution {
        team: TeamSide,
        incoming: usize,
        outgoing: usize,
    },
    /// 暂停被批准：比分与两个边界的暂停余额。
    Timeout {
        team: TeamSide,
        remaining: [u32; 2],
    },
    PeriodEnd {
        period: u32,
    },
    /// 选板激活：阵地回合进入某份剧本的行为窗口。
    PlayActivated {
        play: String,
        possession: u64,
    },
    /// 选板窗口结束。
    PlayWindowEnded {
        play: String,
        possession: u64,
    },
    /// 球与地板、篮筐、篮板、球员或手部的接触。
    BallContact {
        kind: &'static str,
        entity: Option<usize>,
        time: f64,
        x: f32,
        y: f32,
        z: f32,
        sub_tick: u16,
    },
    PlacementApplied {
        player: usize,
        reason: &'static str,
    },
    GameEnd,
}

impl Fact {
    pub fn name(&self) -> &'static str {
        match self {
            Self::PossessionStarted { .. } => "PossessionStarted",
            Self::Pass { .. } => "Pass",
            Self::Screen { .. } => "Screen",
            Self::Drive { .. } => "Drive",
            Self::PostUp { .. } => "PostUp",
            Self::Shot { .. } => "Shot",
            Self::Foul { .. } => "Foul",
            Self::FreeThrow { .. } => "FreeThrow",
            Self::Rebound { .. } => "Rebound",
            Self::ReboundOpportunity { .. } => "ReboundOpportunity",
            Self::SecondChanceStarted { .. } => "SecondChanceStarted",
            Self::Cut { .. } => "Cut",
            Self::BoxOut { .. } => "BoxOut",
            Self::DefenseResponsibilityChanged { .. } => "DefenseResponsibilityChanged",
            Self::Turnover { .. } => "Turnover",
            Self::Steal { .. } => "Steal",
            Self::Substitution { .. } => "Substitution",
            Self::Timeout { .. } => "Timeout",
            Self::PeriodEnd { .. } => "PeriodEnd",
            Self::PlayActivated { .. } => "PlayActivated",
            Self::PlayWindowEnded { .. } => "PlayWindowEnded",
            Self::BallContact { kind, .. } => kind,
            Self::PlacementApplied { .. } => "PlacementApplied",
            Self::GameEnd => "GameEnd",
        }
    }
}
