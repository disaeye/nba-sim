//! 世界状态与决策视图。
//!
//! 决策层只读 [`DecisionState`]；它由世界状态派生，不含动作历史。这样
//! 「掩护是否造成错位」「突破是否过掉人」必须以状态字段的形式存在，否则
//! 下一次决策看不到它们。

use glam::Vec2;
use nba_domain::PlayerData;

pub use crate::action::TeamSide;
use crate::action::{ActionRuntime, DefenderRoute};
use crate::params::PolicyTable;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffenseJob {
    Handler,
    SpotUp,
    Cut,
    Screen,
    Roll,
    PostUp,
    Rebound,
}

impl OffenseJob {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Handler => "handler",
            Self::SpotUp => "spot_up",
            Self::Cut => "cut",
            Self::Screen => "screen",
            Self::Roll => "roll",
            Self::PostUp => "post_up",
            Self::Rebound => "rebound",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Station,
    Pressure,
    Contact,
    Window,
}

impl TargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Station => "station",
            Self::Pressure => "pressure",
            Self::Contact => "contact",
            Self::Window => "window",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Station => 0,
            Self::Pressure => 1,
            Self::Contact => 2,
            Self::Window => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefenseResponsibility {
    OnBall,
    Deny,
    Sink,
    Help,
    BoxOut,
    /// 从掩护上方挤过去，继续贴着持球人。
    GoOver,
    /// 从掩护下方绕过，先沉到掩护人与篮筐之间。
    GoUnder,
    /// 被掩护后直接换防。
    Switch,
    /// 失去位置后回位：先回到自己的对位人身上。
    Recover,
}

impl DefenseResponsibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OnBall => "on_ball",
            Self::Deny => "deny",
            Self::Sink => "sink",
            Self::Help => "help",
            Self::BoxOut => "box_out",
            Self::GoOver => "go_over",
            Self::GoUnder => "go_under",
            Self::Switch => "switch",
            Self::Recover => "recover",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Handler,
    Wing,
    Corner,
    Top,
    Post,
    DunkerSpot,
}

impl Slot {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Handler => "handler",
            Self::Wing => "wing",
            Self::Corner => "corner",
            Self::Top => "top",
            Self::Post => "post",
            Self::DunkerSpot => "dunker",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionTrigger {
    PossessionStart,
    ActionComplete,
    BallHolderChanged,
    PassReceived,
    ScreenContact,
    HelpCommitted,
    ShotClockBelow(u8),
    Periodic,
}

/// 单个球员的重决策调度。
///
/// 决策由事件触发；`next_redecide_at` 是无事件时的回退时刻。
/// `pending` 记录尚未消费的触发来源，供审计与协议投影。
#[derive(Debug, Clone, Copy)]
pub struct DecisionSchedule {
    pub next_redecide_at: f64,
    pub pending: Option<DecisionTrigger>,
}

/// 被假动作骗到之后的失衡窗口。
///
/// 三种假动作造成同一个状态，但消费者不同：投篮假动作降低干扰，
/// 传球假动作压缩抄截半径，试探步两者都轻微受影响。窗口到期即恢复。
#[derive(Debug, Clone, Copy)]
pub struct Bait {
    pub kind: crate::action::FakeKind,
    pub until: f64,
    /// 被骗后实际移动的距离（英尺）。
    pub displacement_ft: f32,
}

/// 持球人的护球姿态：在窗口内降低切球成功率。
#[derive(Debug, Clone, Copy)]
pub struct BallProtection {
    pub holder: usize,
    pub kind: crate::action::ProtectKind,
    pub until: f64,
}

impl Default for DecisionSchedule {
    fn default() -> Self {
        Self {
            next_redecide_at: 0.0,
            pending: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlayerState {
    pub id: String,
    pub name: String,
    pub jersey: String,
    pub team: TeamSide,
    pub data: PlayerData,
    pub pos: Vec2,
    pub vel: Vec2,
    pub facing: Vec2,
    pub on_court: bool,
    pub stamina: f32,
    pub fouls: u8,
    /// 犯满离场：置位后不再上场，除非 NBA 名单耗尽规则要求留在场上。
    pub disqualified: bool,
    /// NBA 替补全部犯满后，规则要求该球员留在场上。
    pub retained_after_foul_limit: bool,
    pub slot: Slot,
    pub offense_job: OffenseJob,
    pub defense_responsibility: DefenseResponsibility,
    pub target: Vec2,
    pub target_kind: TargetKind,
    /// Pressure/Contact/Window 的关联对象；Station 为 usize::MAX。
    pub target_partner: usize,
    /// 提交时被拒绝的次数，供职责段测量。
    pub invalid_commits: u32,
    pub invalid_commits_by_kind: [u32; 4],
    /// 重选恢复段的起点与起点到目标的距离；0 表示不在恢复段。
    pub recovery_since: f64,
    pub recovery_d0_ft: f32,
    pub recovery_path_ft: f32,
    /// 目标点最近一次变更的时刻：职责变更与目标变化阈值都读它。
    pub target_changed_at: f64,
    /// 当前职责开始生效的时刻。
    pub assignment_since: f64,
    /// 本次避让选定的绕行侧：-1 左，1 右，0 未避让。
    pub avoidance_side: i8,
    /// 当前阻挡通路的球员句柄；`usize::MAX` 表示没有阻挡。
    pub avoidance_blocker: usize,
    /// 通道被挡住且没有净接近的起始时刻；0 表示没有在等待。
    pub yielding_since: f64,
    /// 对当前目标连续没有净接近的秒数。
    pub nonprogress_seconds: f64,
    /// 对当前目标达到过的最小距离。
    pub progress_best_ft: f32,
    /// 本 tick 运动积分产生的位移，供职责段测量。
    pub last_motion_ft: f32,
    /// 这个球员在限制区里已经连续停留的秒数。
    ///
    /// 进攻三秒与防守三秒都按「某个球员连续停留」计时（NBA Rule 10
    /// §VI.1「An offensive player shall not remain for more than three
    /// seconds」），因此计时记在球员身上而不是回合上。
    pub lane_seconds: f32,
    /// 本 tick 碰撞分离产生的位移，供职责段测量。
    pub last_separation_ft: f32,
    pub home_spot: Vec2,
    /// 本次上场的时刻：两次换人之间必须隔够 `min_rest_seconds`。
    pub entered_at: f64,
    /// 下场后到什么时候才能再上：休息不满的人不能换回来。
    pub resting_until: f64,
    /// 重决策调度：事件触发加最小间隔。
    pub schedule: DecisionSchedule,
    /// 本球员当前执行的身体动作；多人配合使用相同的 interaction_id。
    pub action: Option<ActionRuntime>,
    /// 被假动作骗到后的失衡窗口：干扰、抄截半径与切球成功率都读它。
    pub bait: Option<Bait>,
    /// 最近一次完成掩护的时刻：掩护结束后掩护人要顺下或外弹。
    pub screened_at: f64,
}

impl PlayerState {
    pub fn new(data: PlayerData, team: TeamSide, slot: Slot, pos: Vec2) -> Self {
        Self {
            id: data.id.clone(),
            name: data.name.clone(),
            jersey: data.jersey.clone(),
            team,
            data,
            pos,
            vel: Vec2::ZERO,
            facing: Vec2::X,
            on_court: false,
            stamina: 1.0,
            fouls: 0,
            disqualified: false,
            retained_after_foul_limit: false,
            slot,
            offense_job: OffenseJob::Handler,
            defense_responsibility: DefenseResponsibility::OnBall,
            target: pos,
            target_kind: TargetKind::Station,
            target_partner: usize::MAX,
            invalid_commits: 0,
            invalid_commits_by_kind: [0; 4],
            recovery_since: 0.0,
            recovery_d0_ft: 0.0,
            recovery_path_ft: 0.0,
            target_changed_at: f64::default(),
            assignment_since: f64::default(),
            avoidance_side: 0,
            avoidance_blocker: usize::MAX,
            yielding_since: 0.0,
            nonprogress_seconds: 0.0,
            progress_best_ft: f32::MAX,
            last_motion_ft: 0.0,
            lane_seconds: 0.0,
            last_separation_ft: 0.0,
            home_spot: pos,
            entered_at: f64::default(),
            resting_until: f64::default(),
            screened_at: f64::NEG_INFINITY,
            schedule: DecisionSchedule::default(),
            action: None,
            bait: None,
        }
    }

    pub fn attribute(&self, name: &str) -> f32 {
        let a = &self.data.attributes;
        match name {
            "speed" => a.speed,
            "acceleration" => a.acceleration,
            "agility" => a.agility,
            "strength" => a.strength,
            "vertical" => a.vertical,
            "stamina" => a.stamina,
            "ball_handling" => a.ball_handling,
            "passing" => a.passing,
            "shooting_close" => a.shooting_close,
            "shooting_near" => a.shooting_near,
            "shooting_mid" => a.shooting_mid,
            "shooting_three" => a.shooting_three,
            "free_throw" => a.free_throw,
            "finishing" => a.finishing,
            "defense_perimeter" => a.defense_perimeter,
            "defense_interior" => a.defense_interior,
            "steal" => a.steal,
            "block" => a.block,
            "offensive_rebound" => a.offensive_rebound,
            "defensive_rebound" => a.defensive_rebound,
            "decision_iq" => a.decision_iq,
            "off_ball_sense" => a.off_ball_sense,
            other => panic!("unknown attribute `{other}`"),
        }
    }

    pub fn shooting_attribute(&self, zone: &str) -> f32 {
        match zone {
            "rim" => self.data.attributes.shooting_close,
            "near" => self.data.attributes.shooting_near,
            "mid" => self.data.attributes.shooting_mid,
            "three" => self.data.attributes.shooting_three,
            other => panic!("unknown zone `{other}`"),
        }
    }

    /// 移动速度：属性给基础值，体能按数据表里的地板与跨度缩放。
    pub fn effective_speed(&self, policy: &PolicyTable) -> f32 {
        crate::physical::Physical::of(self, policy).speed_ftps
    }

    /// 加速度能力决定多快到达目标速度，直接影响推进与回合时长。
    pub fn effective_acceleration(&self, policy: &PolicyTable) -> f32 {
        crate::physical::Physical::of(self, policy).accel_ftps2
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockAdvance {
    pub game_clock: bool,
    pub shot_clock: bool,
    pub elapsed: bool,
    pub player_time: bool,
}

impl ClockAdvance {
    pub fn validate(self) {
        assert!(
            !self.player_time || self.game_clock,
            "player time requires a running game clock"
        );
        assert!(
            !self.shot_clock || self.game_clock,
            "shot clock cannot run while game clock is stopped"
        );
        assert!(
            !self.elapsed || self.game_clock,
            "match elapsed time requires a running game clock"
        );
    }

    const STOPPED: Self = Self {
        game_clock: false,
        shot_clock: false,
        elapsed: false,
        player_time: false,
    };
}

#[derive(Debug, Clone)]
pub struct Clock {
    period: u32,
    game_clock: f64,
    shot_clock: f64,
    elapsed: f64,
    simulation_time: f64,
    tick: u64,
    period_seconds: f64,
}

impl Clock {
    pub fn new(period_seconds: f64, shot_clock: f64) -> Self {
        Self {
            period: 1,
            game_clock: period_seconds,
            shot_clock,
            elapsed: 0.0,
            simulation_time: 0.0,
            tick: 0,
            period_seconds,
        }
    }

    pub(crate) fn advance(&mut self, seconds: f64, clocks: ClockAdvance) {
        assert!(
            seconds.is_finite() && seconds >= 0.0,
            "clock interval must be finite and non-negative"
        );
        clocks.validate();
        self.advance_simulation_time(seconds);
        if clocks.game_clock {
            self.game_clock = (self.game_clock - seconds).max(0.0);
        }
        if clocks.shot_clock {
            self.shot_clock = (self.shot_clock - seconds).max(0.0);
        }
        if clocks.elapsed {
            self.elapsed += seconds;
        }
    }

    pub fn shot_clock(&self) -> f64 {
        self.shot_clock
    }

    /// 时钟快照：账本与验证只读这三个数，不碰计时器的其余内部状态。
    pub fn state(&self) -> crate::clock::ClockState {
        crate::clock::ClockState {
            period: self.period,
            game_clock: self.game_clock,
            shot_clock: self.shot_clock,
        }
    }

    pub fn set_shot_clock(&mut self, seconds: f64) {
        assert!(
            seconds.is_finite() && seconds >= 0.0,
            "shot clock must be finite and non-negative"
        );
        self.shot_clock = seconds;
    }

    pub fn game_clock(&self) -> f64 {
        self.game_clock
    }

    pub fn set_game_clock(&mut self, seconds: f64) {
        assert!(
            seconds.is_finite() && seconds >= 0.0,
            "game clock must be finite and non-negative"
        );
        self.game_clock = seconds;
    }

    pub fn period(&self) -> u32 {
        self.period
    }

    pub fn set_regulation_clock(&mut self, period: u32, seconds: f64, shot_clock_seconds: f64) {
        assert!(period > 0, "period number must be positive");
        assert!(
            seconds.is_finite() && seconds >= 0.0,
            "game clock must be finite and non-negative"
        );
        assert!(
            shot_clock_seconds.is_finite() && shot_clock_seconds >= 0.0,
            "shot clock must be finite and non-negative"
        );
        self.period = period;
        self.period_seconds = seconds;
        self.game_clock = seconds;
        self.shot_clock = shot_clock_seconds;
    }

    pub fn period_seconds(&self) -> f64 {
        self.period_seconds
    }

    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }

    pub fn simulation_time(&self) -> f64 {
        self.simulation_time
    }

    pub fn advance_simulation_time(&mut self, seconds: f64) {
        assert!(
            seconds.is_finite() && seconds >= 0.0,
            "simulation interval must be finite and non-negative"
        );
        self.simulation_time += seconds;
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn advance_tick(&mut self) {
        self.tick += 1;
    }

    pub(crate) fn set_shot_clock_at_least(&mut self, seconds: f64) {
        assert!(
            seconds.is_finite() && seconds >= 0.0,
            "shot-clock reset must be finite and non-negative"
        );
        self.shot_clock = self.shot_clock.max(seconds);
    }

    pub(crate) fn set_period_duration(&mut self, seconds: f64) {
        assert!(
            seconds.is_finite() && seconds > 0.0,
            "period duration must be finite and positive"
        );
        self.period_seconds = seconds;
    }

    pub(crate) fn start_next_period(&mut self, seconds: f64, shot_clock_seconds: f64) {
        assert!(
            seconds.is_finite() && seconds > 0.0,
            "period duration must be finite and positive"
        );
        assert!(
            shot_clock_seconds.is_finite() && shot_clock_seconds >= 0.0,
            "shot clock must be finite and non-negative"
        );
        self.period += 1;
        self.period_seconds = seconds;
        self.game_clock = seconds;
        self.shot_clock = shot_clock_seconds;
    }
}

/// 球的飞行状态：球在空中时才有值，与比赛阶段分开存在。
///
/// 传球、投篮、罚球与跳球撮球的飞行不替换比赛程序：程序仍是活球，
/// 飞行只是球的第二个维度。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BallFlight {
    pub kind: FlightKind,
    /// 飞行结束、该结算的时刻。
    pub resolve_at: f64,
}

/// 四种飞行：传球、投篮（含上篮）、罚球、跳球撮球。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlightKind {
    Pass,
    Shot,
    FreeThrow,
    JumpBall,
}

impl FlightKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Shot => "shot",
            Self::FreeThrow => "free_throw",
            Self::JumpBall => "jump_ball",
        }
    }

    /// 飞行期间哪些时钟继续走：传球两个钟都走；投篮与跳球只走比赛钟；
    /// 罚球飞行全停。
    pub fn clocks(self) -> ClockAdvance {
        match self {
            Self::Pass => ClockAdvance {
                game_clock: true,
                shot_clock: true,
                elapsed: true,
                player_time: true,
            },
            Self::Shot | Self::JumpBall => ClockAdvance {
                game_clock: true,
                shot_clock: false,
                elapsed: true,
                player_time: true,
            },
            Self::FreeThrow => ClockAdvance::STOPPED,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum BallState {
    Held {
        holder: usize,
    },
    Pass {
        pass_id: u64,
        from: usize,
        to: usize,
        kind: crate::action::PassKind,
        from_pos: Vec2,
        to_pos: Vec2,
        start: f64,
        duration: f64,
    },
    Shot {
        shot_id: u64,
        shooter: usize,
        from_pos: Vec2,
        target: Vec2,
        start: f64,
        /// 出手时算出的有效筐口：价值层与执行层共用同一个值。
        aperture_ft: f32,
        /// 碰到筐沿之后仍能补进的概率。
        rim_in_probability: f32,
        /// 球心在筐平面上的落点偏移（瞄准误差）。
        aim_offset: Vec2,
    },
    Loose {
        pos: Vec2,
        vel: Vec2,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Inbound {
        team: TeamSide,
        ready_at: f64,
    },
    JumpBall {
        home_jumper: usize,
        away_jumper: usize,
        resolve_at: f64,
        kind: crate::action::JumpBallKind,
    },
    Live,
    FreeThrows {
        shooter: usize,
        total: u8,
        remaining: u8,
        made: u8,
        next_at: f64,
        and_one: bool,
        retain_possession: bool,
        foul_id: u64,
        technical_retained_team: Option<TeamSide>,
        after_technical: Option<crate::resolve::freethrow::FreeThrowFollowUp>,
    },
    DeadBall {
        team: TeamSide,
        ready_at: f64,
    },
    /// 暂停：比赛钟与进攻钟都停，暂停结束时叫暂停方继续持有球权。
    Timeout {
        team: TeamSide,
        started_at: f64,
        resume_at: f64,
        resume_phase: ResumePhase,
    },
    PeriodBreak {
        resume_at: f64,
        next_stage: MatchStage,
    },
    Pregame,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResumePhase {
    Inbound {
        team: TeamSide,
        ready_at: f64,
    },
    JumpBall {
        home_jumper: usize,
        away_jumper: usize,
        resolve_at: f64,
        kind: crate::action::JumpBallKind,
    },
    Live,
    FreeThrows {
        shooter: usize,
        total: u8,
        remaining: u8,
        made: u8,
        next_at: f64,
        and_one: bool,
        retain_possession: bool,
        foul_id: u64,
        technical_retained_team: Option<TeamSide>,
        after_technical: Option<crate::resolve::freethrow::FreeThrowFollowUp>,
    },
    DeadBall {
        team: TeamSide,
        ready_at: f64,
    },
    PeriodBreak {
        resume_at: f64,
        next_stage: MatchStage,
    },
    Pregame,
    Finished,
}

impl ResumePhase {
    pub fn capture(phase: Phase) -> Self {
        match phase {
            Phase::Inbound { team, ready_at } => Self::Inbound { team, ready_at },
            Phase::JumpBall {
                home_jumper,
                away_jumper,
                resolve_at,
                kind,
            } => Self::JumpBall {
                home_jumper,
                away_jumper,
                resolve_at,
                kind,
            },
            Phase::Live => Self::Live,
            Phase::FreeThrows {
                shooter,
                total,
                remaining,
                made,
                next_at,
                and_one,
                retain_possession,
                foul_id,
                technical_retained_team,
                after_technical,
            } => Self::FreeThrows {
                shooter,
                total,
                remaining,
                made,
                next_at,
                and_one,
                retain_possession,
                foul_id,
                technical_retained_team,
                after_technical,
            },
            Phase::DeadBall { team, ready_at } => Self::DeadBall { team, ready_at },
            Phase::PeriodBreak {
                resume_at,
                next_stage,
            } => Self::PeriodBreak {
                resume_at,
                next_stage,
            },
            Phase::Pregame => Self::Pregame,
            Phase::Finished => Self::Finished,
            Phase::Timeout { .. } => unreachable!("timeout phase must be unwrapped before capture"),
        }
    }

    pub const fn phase(self) -> Phase {
        match self {
            Self::Inbound { team, ready_at } => Phase::Inbound { team, ready_at },
            Self::JumpBall {
                home_jumper,
                away_jumper,
                resolve_at,
                kind,
            } => Phase::JumpBall {
                home_jumper,
                away_jumper,
                resolve_at,
                kind,
            },
            Self::Live => Phase::Live,
            Self::FreeThrows {
                shooter,
                total,
                remaining,
                made,
                next_at,
                and_one,
                retain_possession,
                foul_id,
                technical_retained_team,
                after_technical,
            } => Phase::FreeThrows {
                shooter,
                total,
                remaining,
                made,
                next_at,
                and_one,
                retain_possession,
                foul_id,
                technical_retained_team,
                after_technical,
            },
            Self::DeadBall { team, ready_at } => Phase::DeadBall { team, ready_at },
            Self::PeriodBreak {
                resume_at,
                next_stage,
            } => Phase::PeriodBreak {
                resume_at,
                next_stage,
            },
            Self::Pregame => Phase::Pregame,
            Self::Finished => Phase::Finished,
        }
    }

    pub const fn is_live_gameplay(self) -> bool {
        matches!(self, Self::Live)
    }
}

impl Phase {
    pub fn shift_deadlines(self, seconds: f64) -> Self {
        assert!(
            seconds.is_finite() && seconds >= 0.0,
            "phase shift must be finite and non-negative"
        );
        match self {
            Self::Inbound { team, ready_at } => Self::Inbound {
                team,
                ready_at: ready_at + seconds,
            },
            Self::JumpBall {
                home_jumper,
                away_jumper,
                resolve_at,
                kind,
            } => Self::JumpBall {
                home_jumper,
                away_jumper,
                resolve_at: resolve_at + seconds,
                kind,
            },
            Self::Live => Self::Live,
            Self::FreeThrows {
                shooter,
                total,
                remaining,
                made,
                next_at,
                and_one,
                retain_possession,
                foul_id,
                technical_retained_team,
                after_technical,
            } => Self::FreeThrows {
                shooter,
                total,
                remaining,
                made,
                next_at: next_at + seconds,
                and_one,
                retain_possession,
                foul_id,
                technical_retained_team,
                after_technical,
            },
            Self::DeadBall { team, ready_at } => Self::DeadBall {
                team,
                ready_at: ready_at + seconds,
            },
            Self::Timeout { .. } => panic!("cannot shift an active timeout"),
            Self::PeriodBreak {
                resume_at,
                next_stage,
            } => Self::PeriodBreak {
                resume_at: resume_at + seconds,
                next_stage,
            },
            Self::Pregame => Self::Pregame,
            Self::Finished => Self::Finished,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchStage {
    Pregame,
    Regulation,
    Intermission,
    Overtime,
    Finalizing,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchProgram {
    OpenPlay,
    JumpBall,
    Inbound,
    FreeThrows,
    Timeout,
    Review,
    PeriodSettlement,
    PenaltyAdministration,
    Intermission,
    Pregame,
    Complete,
}

impl Phase {
    pub const fn clock_advance(self) -> ClockAdvance {
        match self {
            Self::Live => ClockAdvance {
                game_clock: true,
                shot_clock: true,
                elapsed: true,
                player_time: true,
            },
            Self::Pregame
            | Self::FreeThrows { .. }
            | Self::Inbound { .. }
            | Self::JumpBall { .. }
            | Self::DeadBall { .. }
            | Self::Timeout { .. }
            | Self::PeriodBreak { .. }
            | Self::Finished => ClockAdvance::STOPPED,
        }
    }

    pub const fn is_live_gameplay(self) -> bool {
        matches!(self, Self::Live)
    }

    pub const fn stage(self, period: u32, regulation_periods: u32) -> MatchStage {
        if matches!(self, Self::Finished) {
            MatchStage::Finished
        } else if matches!(self, Self::Pregame) {
            MatchStage::Pregame
        } else if let Self::PeriodBreak { next_stage, .. } = self {
            next_stage
        } else if let Self::Timeout { resume_phase, .. } = self {
            resume_phase.phase().stage(period, regulation_periods)
        } else if period > regulation_periods {
            MatchStage::Overtime
        } else {
            MatchStage::Regulation
        }
    }

    pub const fn clocks(self) -> ClockAdvance {
        self.clock_advance()
    }

    pub const fn program(self) -> MatchProgram {
        match self {
            Self::Live => MatchProgram::OpenPlay,
            Self::Inbound { .. } => MatchProgram::Inbound,
            Self::JumpBall { .. } => MatchProgram::JumpBall,
            Self::FreeThrows { .. } => MatchProgram::FreeThrows,
            Self::Timeout { .. } => MatchProgram::Timeout,
            Self::DeadBall { .. } => MatchProgram::PenaltyAdministration,
            Self::PeriodBreak { .. } => MatchProgram::PeriodSettlement,
            Self::Pregame => MatchProgram::Pregame,
            Self::Finished => MatchProgram::Complete,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PossessionKind {
    Dead,
    Live,
}

impl PossessionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dead => "dead",
            Self::Live => "live",
        }
    }
}

/// 持球状态：运球中或已收球。收球后只允许终结（投篮/传球）或两步
/// 内的调整；再运球是二次运球，收球后超步幅位移是走步。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BallPhase {
    #[default]
    Dribble,
    Catch,
    Drive,
    Post,
    Cut,
    Gather,
    Putback,
}

impl BallPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dribble => "dribble",
            Self::Catch => "catch",
            Self::Drive => "drive",
            Self::Post => "post",
            Self::Cut => "cut",
            Self::Gather => "gather",
            Self::Putback => "putback",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DribbleState {
    #[default]
    Dribbling,
    Gathered,
}

#[derive(Debug, Clone)]
pub struct PossessionState {
    pub team: TeamSide,
    pub id: u64,
    pub kind: PossessionKind,
    pub started_at: f64,
    pub passes: u32,
    pub screens: u32,
    pub drives: u32,
    pub had_shot: bool,
    pub had_offensive_rebound: bool,
    pub last_pass_at: Option<f64>,
    pub last_pass_from: Option<usize>,
    pub assist_candidate: Option<(usize, f64)>,
    pub shot_clock_start: f64,
    /// 回合开始时的比赛钟读数。
    pub start_game_clock: f64,
    pub last_cut_at: Option<f64>,
    pub last_offensive_rebound_at: Option<f64>,
    pub last_offensive_rebound_actor: Option<usize>,
    pub last_drive_at: Option<f64>,
    pub last_post_at: Option<f64>,
    pub last_drive_actor: Option<usize>,
    pub last_post_actor: Option<usize>,
    pub last_cut_actor: Option<usize>,
    /// 回合开始到首个动作起手的秒数。
    pub first_action_seconds: Option<f64>,
    /// 本回合已经发生的无球自主动作数：无球球员每个回合只另起一次动作。
    pub off_ball_actions: u8,
    /// 当前运球技术带给球的暴露程度（0 = 护住，1 = 完全暴露）。
    pub dribble_exposure: f32,
    /// 暴露程度的到期时刻。
    pub dribble_exposure_until: f64,
    /// 对位表最后一次被重算的时刻：职责层据此立刻换人。
    pub pairing_changed_at: f64,
    /// 持球状态与收球位置。
    pub dribble_state: DribbleState,
    pub gather_pos: Option<Vec2>,
    /// 收球后确立的轴心与脚步状态。
    ///
    /// 收球那一刻确定轴心脚，此后不再改变；每一步的接触都记在这里，
    /// 走步判定按「轴心脚被移动」与「步数用尽」两条规则走，不用收球后
    /// 的平面位移一次性判定。
    pub pivot: Option<nba_domain::footwork::PivotState>,
    /// 当前持球人：脚步与轴心都跟着这个人走。
    pub pivot_holder: Option<usize>,
    /// 支撑脚状态：哪只脚在地上、落在哪里。
    ///
    /// 脚步接触在左右之间交替，落点记在接触位置。收球时确立的轴心脚
    /// 从这里取：支撑脚就是收球瞬间地上的那只脚。
    pub support_foot: nba_domain::footwork::SupportFoot,
    /// 当前进攻阶段：由实际动作结算写入，出手类型只读这里。
    pub ball_phase: BallPhase,
    /// 最近一次掩护结算出的防守人路线，供下一次决策读取。
    pub last_screen: Option<ScreenRead>,
    /// 对位表：`matchup[防守人在场序号] = 所对位的进攻人 handle`，未分配时为 `usize::MAX`。
    /// 只在掩护换防或突破得手时重算，其余时间保持不变。
    pub matchup: [usize; MAX_ON_COURT],
    /// 当前生效的选板窗口，未激活时为 `None`。
    pub play: Option<crate::tactics::PlayWindow>,
    /// 本回合是否已经叫过战术：每个回合只叫一次，之后进自由进攻。
    pub play_called: bool,
    /// 快攻跑位改过的球员及其原站位：窗口结束时按这里还回去。
    pub break_return: Vec<(usize, Vec2)>,
    /// 持球人被紧逼盯防的累计秒数：一旦不再紧逼就清零。
    pub closely_guarded_seconds: f32,
    /// 进攻人在限制区里已经连续停留的秒数：进攻三秒的计时。
    ///
    /// 攻方三秒只对进攻方的前场球员计时，而且只在他**持球或准备接球**
    /// 时算——无球球员在限制区里站位是允许的（NBA Rule 10 §VI）。
    pub offense_lane_seconds: f32,
    /// 防守人在限制区里已经连续停留的秒数：防守三秒的计时。
    pub defense_lane_seconds: f32,
    /// 持球人背身运球的累计秒数：NBA Rule 10 §XVI 的五秒违例计时。
    pub back_to_basket_seconds: f32,
    /// 上一 tick 持球人是否背对篮筐：计时在朝向翻转的那一刻终止。
    pub holder_back_to_basket: bool,
    /// 后场八秒的起算点（比赛时钟）：过中线即清空。
    pub backcourt_started_game_clock: Option<f64>,
    /// 八秒计时开始时球最后被谁碰过：防守人之后碰球就是「踢球」，
    /// 计时重新开始（NBA Rule 10 §VIII.1）。
    pub backcourt_started_last_touch: Option<usize>,
    /// 本回合是否已经在前场建立控制：已建立后不能再把球带回后场。
    pub frontcourt_established: bool,
    /// 本回合球是否已经触过筐。
    pub rim_contact: bool,
}

impl PossessionState {
    pub fn new(
        team: TeamSide,
        id: u64,
        kind: PossessionKind,
        now: f64,
        shot_clock: f64,
        game_clock: f64,
    ) -> Self {
        Self {
            team,
            id,
            kind,
            started_at: now,
            passes: 0,
            screens: 0,
            drives: 0,
            had_shot: false,
            had_offensive_rebound: false,
            last_pass_at: None,
            last_pass_from: None,
            assist_candidate: None,
            shot_clock_start: shot_clock,
            start_game_clock: game_clock,
            last_cut_at: None,
            last_offensive_rebound_at: None,
            last_offensive_rebound_actor: None,
            last_drive_at: None,
            last_post_at: None,
            last_drive_actor: None,
            last_post_actor: None,
            last_cut_actor: None,
            first_action_seconds: None,
            off_ball_actions: 0,
            dribble_exposure: 1.0,
            dribble_exposure_until: 0.0,
            pairing_changed_at: 0.0,
            dribble_state: DribbleState::Dribbling,
            gather_pos: None,
            pivot: None,
            pivot_holder: None,
            support_foot: nba_domain::footwork::SupportFoot::new(
                nba_domain::action_window::Foot::Left,
                Vec2::ZERO,
            ),
            ball_phase: BallPhase::Dribble,
            last_screen: None,
            matchup: [usize::MAX; MAX_ON_COURT],
            play: None,
            play_called: false,
            break_return: Vec::new(),
            closely_guarded_seconds: 0.0,
            offense_lane_seconds: 0.0,
            defense_lane_seconds: 0.0,
            back_to_basket_seconds: 0.0,
            holder_back_to_basket: false,
            backcourt_started_game_clock: None,
            backcourt_started_last_touch: None,
            frontcourt_established: false,
            rim_contact: false,
        }
    }
}

/// 掩护结算出的防守人路线及其有效期。
///
/// 决策层看不到这个结构就直接是战术盲人：防守人钻掩护下沿还是换防，
/// 决定了持球人该干拔还是突破。
#[derive(Debug, Clone, Copy)]
pub struct ScreenRead {
    pub route: DefenderRoute,
    pub at: f64,
    pub screener: usize,
    pub separation_ft: f32,
}

/// 逐次出手的过程观测。
#[derive(Debug, Clone, Copy)]
pub struct ShotSample {
    pub shot_clock: f64,
    /// 出手时的比赛时间与节次：分层诊断需要按末段窗口切样本。
    pub game_clock: f64,
    pub period: u32,
    pub contest: f32,
    /// 出手瞬间的移动程度：球员速度除以最大速度。
    pub movement: f32,
    /// 出手人的区域投篮属性。
    pub skill: f32,
    /// 出手瞬间最近防守人的距离（英尺）。
    pub defender_ft: f32,
    pub zone: u8,
    /// 出手时由动作结算写入的持球阶段。
    pub phase: &'static str,
    /// 出手点到篮筐的平面距离。
    pub distance_ft: f32,
    /// 本次出手抽样得到的瞄准半径。
    pub target_radius_ft: f32,
    /// 出手时的有效筐口：模型预测与执行层判定用的是同一个值。
    pub aperture_ft: f32,
    /// 出手时的瞄准误差标准差。
    pub sigma_ft: f32,
    /// 模型对这次出手的命中概率（含封盖与犯规的边际）。
    pub p_make: f32,
    /// 这次出手是否被吹犯规、是否被封盖（用于核对边际守恒）。
    pub fouled: bool,
    pub blocked: bool,
    /// 抽样时实际使用的三个概率：犯规、封盖、条件化后的命中率。
    pub p_foul: f32,
    pub p_block: f32,
    pub p_make_cond: f32,
    /// 球体结算给出的结果类别：made / missed / blocked。
    pub result: &'static str,
    pub made: bool,
    pub points: u8,
}

/// 逐次传球的观测，用于乒乓传球与传导质量判定。
#[derive(Debug, Clone, Copy)]
pub struct PassSample {
    pub possession: u64,
    pub from: usize,
    pub to: usize,
    pub from_pos: Vec2,
    pub to_pos: Vec2,
    pub at: f64,
}

/// 单个候选动作的审计记录。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CandidateTrace {
    pub kind: crate::action::ActionKind,
    pub target: Option<usize>,
    pub to_ft: Vec2,
    pub utility: crate::utility::Utility,
    /// 带 γ 的总效用。
    pub total: f32,
    /// 玻尔兹曼概率（全部候选合计为 1）。
    pub probability: f32,
}

/// 一次决策的因果快照：候选集、各自的效用分解与概率。
/// 一次实际决策的审计记录。
///
/// `observation_digest` 是观察集合的稳定摘要；反事实与信息干预用它
/// 判断「决策输入是否改变」。`revalidation` 记录执行前重校验结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DecisionRecord {
    pub decision_id: u64,
    pub tick: u64,
    pub actor: usize,
    pub observation_digest: u64,
    pub candidates: Vec<CandidateTrace>,
    pub chosen: usize,
    pub revalidation: Revalidation,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Revalidation {
    Accepted,
    Cancelled(String),
}

#[derive(Debug, Clone)]
pub struct DecisionTraceSample {
    pub actor: usize,
    pub beta: f32,
    pub gamma: f32,
    /// 按概率降序排列的全部候选。
    pub candidates: Vec<CandidateTrace>,
    /// 实际被抽中的候选下标。
    pub chosen: usize,
}

impl DecisionTraceSample {
    /// Top-N 候选（N 大于候选数时返回全部）。
    pub fn top(&self, count: usize) -> &[CandidateTrace] {
        &self.candidates[..count.min(self.candidates.len())]
    }
}

/// 逐次决策的观测：掩护路线、所选动作族与效用的完整分解。
#[derive(Debug, Clone, Copy)]
pub struct DecisionSample {
    pub route: Option<DefenderRoute>,
    pub family: u8,
    pub shot_clock: f64,
    /// 距最近一次掩护结算的秒数；大于一个决策步就不是「下一决策步」。
    pub since_screen: f64,
    /// 持球人的三分能力，用于限定路线响应的适用人群。
    pub shooter_three: f32,
    /// 选中动作的效用分解：期望得分的七个成因、方差、时钟成本、犯规成本、战术偏置。
    pub utility: crate::utility::Utility,
    /// 当时的逆温度 β。
    pub beta: f32,
    /// 当时的方差偏好 γ。
    pub gamma: f32,
    /// 当时的候选数量。
    pub candidates: u8,
    /// 这次决策是否由持球人做出：无球球员的决策不计入持球决策的统计口径。
    pub on_ball: bool,
    /// 指向 `World::decision_traces` 的审计快照下标。
    pub trace: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct DefenderView {
    pub index: usize,
    pub distance_ft: f32,
    pub closing_speed_ftps: f32,
    pub between_ball_and_hoop: bool,
    pub beaten: bool,
    pub route: Option<DefenderRoute>,
    /// 摸高超出基准的部分（英尺），与结算层同一个映射。
    pub reach_excess_ft: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct HelpView {
    pub distance_ft: f32,
    pub arrival_seconds: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct TeammateView {
    pub index: usize,
    pub pos: Vec2,
    pub openness: f32,
    pub defender_distance_ft: f32,
    pub lane_clearance: f32,
    pub distance_to_hoop: f32,
}

/// 比赛处境：杠杆、时间稀缺与方差偏好。
///
/// 三个量都是连续特征，没有「第四节」或「剩 N 秒」这种阶跃开关：
/// 第三节末落后 15 分与第四节剩 2 分钟落后 6 分在数学上是同一件事，
/// 只是 τ 与 ω 不同。
#[derive(Debug, Clone, Copy, Default)]
pub struct Leverage {
    /// τ = 分差 / 剩余回合数（持球方视角，落后为负）。
    pub score_pressure: f32,
    /// ω = exp(−剩余时间 / 稀缺尺度)：终场前接近 1，垃圾时间接近 0。
    pub urgency: f32,
    /// γ：落后为正（偏好高方差），领先为负（压低方差）。
    pub variance_appetite: f32,
}

#[derive(Debug, Clone)]
pub struct DecisionState {
    pub actor: usize,
    pub actor_pos: Vec2,
    pub hoop: Vec2,
    pub attacking_right: bool,
    pub clock: f64,
    pub shot_clock: f64,
    pub margin: i32,
    pub period: u32,
    pub primary_defender: Option<DefenderView>,
    pub help: Option<HelpView>,
    pub teammates: Vec<TeammateView>,
    pub lane_clearance: f32,
    /// 持球人与首要防守人的突破技能差（未截断）。
    pub skill_edge: f32,
    pub mismatch: f32,
    pub stamina: f32,
    pub possession_kind: PossessionKind,
    pub elapsed_in_possession: f64,
    /// 本次决策时的比赛处境。
    pub leverage: Leverage,
    /// 本方当前的势头（0 = 冷却，越大越热）。
    pub momentum: f32,
}

impl DecisionState {
    pub fn distance_to_hoop(&self) -> f32 {
        (self.actor_pos - self.hoop).length()
    }

    pub fn openness(&self) -> f32 {
        match self.primary_defender {
            Some(defender) => (1.0
                - contest_from_defender(
                    defender.distance_ft,
                    defender.between_ball_and_hoop,
                    defender.reach_excess_ft,
                ))
            .clamp(0.0, 1.0),
            None => 1.0,
        }
    }

    pub fn shot_clock_pressure(&self) -> f32 {
        (((12.0 - self.shot_clock) / 12.0) as f32).clamp(0.0, 1.0)
    }
}

/// 干扰强度：防守人越近、越处在球与篮筐之间、摸得越高，干扰越高。
///
/// `reach_excess_ft` 来自 [`crate::physical::Physical`]：摸高超出基准的
/// 部分直接抵掉距离，所以臂展/弹跳是通过物理量而不是属性直接进公式的。
pub fn contest_from_defender(distance_ft: f32, between: bool, reach_excess_ft: f32) -> f32 {
    let gap = distance_ft - reach_excess_ft;
    let base = ((6.0 - gap) / 5.0).clamp(0.0, 1.0);
    if between {
        base
    } else {
        base * 0.55
    }
}

pub fn nearest_opponent(players: &[PlayerState], actor: usize) -> Option<&PlayerState> {
    let team = players[actor].team;
    let pos = players[actor].pos;
    players
        .iter()
        .filter(|p| p.on_court && p.team != team)
        .min_by(|a, b| {
            let da = (a.pos - pos).length_squared();
            let db = (b.pos - pos).length_squared();
            da.partial_cmp(&db).expect("distances are finite")
        })
}

pub fn distance_between(players: &[PlayerState], a: usize, b: usize) -> f32 {
    (players[a].pos - players[b].pos).length()
}

pub fn is_between(point: Vec2, a: Vec2, b: Vec2) -> bool {
    let ab = b - a;
    let length = ab.length();
    if length < 1e-4 {
        return false;
    }
    let dir = ab / length;
    let ap = point - a;
    let along = ap.dot(dir);
    if along <= 0.0 || along >= length {
        return false;
    }
    let lateral = (ap - dir * along).length();
    lateral < 2.2
}

/// 传球线路净空：线路上防守人离传球直线的最小距离。
/// 走廊无阻挡时的返回值（英尺）：远大于球场宽度。
const NO_OBSTRUCTION_FT: f32 = 99.0;
/// 走廊长度的退化阈值（英尺）：两端重合时方向向量无定义。
const MIN_LANE_LENGTH_FT: f32 = 1e-4;

pub fn lane_clearance(players: &[PlayerState], from: Vec2, to: Vec2, offense: TeamSide) -> f32 {
    let segment = to - from;
    let length = segment.length();
    if length < MIN_LANE_LENGTH_FT {
        return NO_OBSTRUCTION_FT;
    }
    let dir = segment / length;
    let mut minimum = NO_OBSTRUCTION_FT;
    for player in players.iter().filter(|p| p.on_court && p.team != offense) {
        let relative = player.pos - from;
        let along = relative.dot(dir).clamp(0.0, length);
        let lateral = (relative - dir * along).length();
        minimum = minimum.min(lateral);
    }
    minimum
}

/// 传球走廊的空隙：只算两端之间的防守人。
///
/// 站在传球人身后或接球人身后的防守人不在走廊里，拦不到球；
/// 按纵向投影区间筛掉它们，而不是把它们按两端点投影后当成堵住走廊。
pub fn pass_lane_clearance(
    players: &[PlayerState],
    from: Vec2,
    to: Vec2,
    offense: TeamSide,
) -> f32 {
    let segment = to - from;
    let length = segment.length();
    if length < MIN_LANE_LENGTH_FT {
        return NO_OBSTRUCTION_FT;
    }
    let dir = segment / length;
    let mut minimum = NO_OBSTRUCTION_FT;
    for player in players.iter().filter(|p| p.on_court && p.team != offense) {
        let relative = player.pos - from;
        let along = relative.dot(dir) / length;
        if along <= f32::default() {
            continue;
        }
        let lateral = (relative - dir * (along * length)).length();
        minimum = minimum.min(lateral);
    }
    minimum
}

/// 同时在场人数上限：名册八人，场上五人。
pub const MAX_ON_COURT: usize = 5;

/// 在场球员句柄的定长集合。每 tick 的防守目标求解与箱体计时都读它，
/// 用定长数组避免逐 tick 堆分配。
#[derive(Debug, Clone, Copy)]
pub struct OnCourt {
    handles: [usize; MAX_ON_COURT],
    len: usize,
}

impl OnCourt {
    pub fn new() -> Self {
        Self {
            handles: [0; MAX_ON_COURT],
            len: 0,
        }
    }

    pub fn push(&mut self, handle: usize) {
        assert!(
            self.len < MAX_ON_COURT,
            "on-court set exceeds {MAX_ON_COURT} players"
        );
        self.handles[self.len] = handle;
        self.len += 1;
    }

    pub fn as_slice(&self) -> &[usize] {
        &self.handles[..self.len]
    }

    pub fn as_mut_slice(&mut self) -> &mut [usize] {
        &mut self.handles[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for OnCourt {
    fn default() -> Self {
        Self::new()
    }
}

impl IntoIterator for OnCourt {
    type Item = usize;
    type IntoIter = std::iter::Take<std::array::IntoIter<usize, MAX_ON_COURT>>;

    fn into_iter(self) -> Self::IntoIter {
        self.handles.into_iter().take(self.len)
    }
}
