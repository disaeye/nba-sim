//! 动作接口：决策层的输出、结算层的输出、结算带来的状态变化。
//!
//! 动作是因果链的单元。结算产出的是**状态变化**，不是结果：掩护产出分离与
//! 防守人路线，突破产出是否过掉首要防守人，传球产出接球质量。下一次决策读
//! 这些字段，出手位置与命中率都是动作链的后果。

use glam::Vec2;

use crate::state::PossessionKind;
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
            Self::DrivingLayup | Self::DrivingDunk | Self::ReverseLayup | Self::Floater => "drive",
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
/// 无球切入的路线类型。
///
/// 每一种路线都有自己的起点、终点的取法：背切从防守人背后绕，
/// 反跑先向球再折回篮下，V 字跑位先下沉再上提，绕掩护从掩护人身侧绕出，
/// 上提接球向持球人靠，空切直接跑向篮下。
pub enum CutKind {
    Basket,
    Backdoor,
    GiveAndGo,
    VCut,
    Curl,
    PopUp,
}

impl CutKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Basket => "basket",
            Self::Backdoor => "backdoor",
            Self::GiveAndGo => "give_and_go",
            Self::VCut => "v_cut",
            Self::Curl => "curl",
            Self::PopUp => "pop_up",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PassKind {
    Chest,
    Bounce,
    Lob,
    Outlet,
    /// 头顶传球：出手点高，从防守人的手上方过去。
    Overhead,
    /// 单手推传：长距离推进，速度快但路线直。
    OneHand,
    /// 背后传球：出手隐蔽但路线难以控制。
    BehindBack,
    /// 手递手：两人贴近交接，几乎没有飞行距离。
    Handoff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum JumpBallKind {
    Opening,
    Overtime,
    HeldBall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ScreenKind {
    OnBall,
    OffBall,
    HandOff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FakeKind {
    /// 投篮假动作：骗起防守人的重心，随后的出手不再被他干扰。
    Shot,
    /// 传球假动作：把防守人的重心引向假目标，真实传球线路变宽。
    Pass,
    /// 试探步：向篮筐方向的脚步试探，换回半步空间。
    JabStep,
    /// 迟疑步：顿一下再起，冻结防守人的伸手时机。
    Hesitation,
}

impl FakeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shot => "shot_fake",
            Self::Pass => "pass_fake",
            Self::JabStep => "jab_step",
            Self::Hesitation => "hesitation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ProtectKind {
    /// 面框护球：把球挡在防守人与身体之间。
    FaceUp,
    /// 背身护球：用身体宽度隔开防守人。
    Post,
    /// 转身护球：转体把球带到远离防守的一侧。
    Spin,
}

impl ProtectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FaceUp => "face_up_protect",
            Self::Post => "post_protect",
            Self::Spin => "spin_protect",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ActionKind {
    Dribble(nba_domain::action_window::DribbleMoveKind),
    Drive,
    Finish,
    Pass(PassKind),
    Screen(ScreenKind),
    Cut(CutKind),
    PostUp,
    Shoot(ShotKind),
    Relocate,
    Fake(FakeKind),
    Protect(ProtectKind),
}

impl ActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dribble(_) => "Dribble",
            Self::Drive => "Drive",
            Self::Finish => "Finish",
            Self::Pass(_) => "Pass",
            Self::Screen(_) => "Screen",
            Self::Cut(_) => "Cut",
            Self::PostUp => "PostUp",
            Self::Shoot(_) => "Shoot",
            Self::Relocate => "Relocate",
            Self::Fake(_) => "Fake",
            Self::Protect(_) => "Protect",
        }
    }

    /// 动作族标签，供决策审计与协议投影使用。
    pub fn label(self) -> &'static str {
        match self {
            Self::Shoot(_) => "shoot",
            Self::Drive => "drive",
            Self::Pass(_) => "pass",
            Self::Screen(_) => "screen",
            Self::Cut(_) => "cut",
            Self::PostUp => "post_up",
            Self::Dribble(_) => "hold",
            Self::Finish => "finish",
            Self::Relocate => "relocate",
            Self::Fake(_) => "fake",
            Self::Protect(_) => "protect",
        }
    }

    /// 动作族编号，供决策观测与过程判据使用。
    pub fn family(self) -> u8 {
        match self {
            Self::Shoot(_) => 0,
            Self::Drive => 1,
            Self::Pass(_) => 2,
            Self::Screen(_) => 3,
            Self::Cut(_) => 4,
            Self::PostUp => 5,
            Self::Dribble(_) => 6,
            Self::Finish => 7,
            Self::Relocate => 8,
            Self::Fake(_) => 9,
            Self::Protect(_) => 10,
        }
    }
}

/// 动作族总数：族编号直接用作按族索引的数组长度。
pub const FAMILY_COUNT: usize = 11;

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
pub enum ActionIntentRole {
    Primary,
    ScreenPartner,
    CutPartner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionTurn {
    Begin,
    Advance,
    Resolve,
}

#[derive(Debug, Clone)]
/// 一次多人交互：同一份意图由若干球员的运行时共同执行。
///
/// 交互是结算的调度单位。主参与者的运行时到期时，交互整体结算一次；
/// 同一时刻可以存在多个交互，每个交互只依赖自己的运行时推进。
pub struct Interaction {
    pub id: u64,
    pub action_id: u64,
    pub intent: ActionIntent,
    /// 掩护的决策时刻防守人：随交互保存，避免并发动作互相清掉这份状态。
    pub screen_defender: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct ActionRuntime {
    pub action_id: u64,
    pub interaction_id: u64,
    pub player: usize,
    pub role: ActionIntentRole,
    pub intent: ActionIntent,
    pub phase: nba_domain::action_window::ActionPhase,
    pub preparation_ends_at: f64,
    pub execution_ends_at: f64,
    pub ends_at: f64,
}

impl ActionRuntime {
    pub fn new(
        action_id: u64,
        interaction_id: u64,
        intent: ActionIntent,
        player: usize,
        role: ActionIntentRole,
        phase: nba_domain::action_window::ActionPhase,
        preparation_seconds: f64,
        execution_seconds: f64,
        follow_through_seconds: f64,
    ) -> Self {
        assert!(intent.duration.is_finite() && intent.duration > 0.0);
        assert!(preparation_seconds.is_finite() && preparation_seconds >= 0.0);
        assert!(execution_seconds.is_finite() && execution_seconds >= 0.0);
        assert!(follow_through_seconds.is_finite() && follow_through_seconds >= 0.0);
        let window_total = preparation_seconds + execution_seconds + follow_through_seconds;
        assert!(
            window_total > 0.0,
            "action window must have positive duration"
        );
        let scale = intent.duration / window_total;
        let preparation_ends_at = intent.start_time + preparation_seconds * scale;
        let execution_ends_at = preparation_ends_at + execution_seconds * scale;
        let ends_at = intent.end_time();
        Self {
            action_id,
            interaction_id,
            player,
            role,
            intent,
            phase,
            preparation_ends_at,
            execution_ends_at,
            ends_at,
        }
    }

    pub fn phase_at(&self, now: f64) -> Option<nba_domain::action_window::ActionPhase> {
        let phase = if now < self.preparation_ends_at {
            nba_domain::action_window::ActionPhase::Preparation
        } else if self.preparation_ends_at == self.execution_ends_at && now < self.ends_at {
            nba_domain::action_window::ActionPhase::FollowThrough
        } else if now < self.execution_ends_at {
            nba_domain::action_window::ActionPhase::Execution
        } else if now < self.ends_at {
            nba_domain::action_window::ActionPhase::FollowThrough
        } else {
            return None;
        };
        if phase == self.phase {
            None
        } else {
            Some(phase)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefensiveActionKind {
    CloseoutContest,
    /// 伸手切球：主动赌一次抄截，执行窗口内成功率提高，收势窗口内失去干扰。
    StealAttempt,
    /// 垂直起跳封盖：执行窗口内摸高增加，跳起的人无法卡位抢篮板。
    BlockJump,
}

impl DefensiveActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CloseoutContest => "closeout_contest",
            Self::StealAttempt => "steal_attempt",
            Self::BlockJump => "block_jump",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DefensiveActionRuntime {
    pub action_id: u64,
    pub interaction_id: u64,
    pub kind: DefensiveActionKind,
    pub defender: usize,
    pub shooter: usize,
    pub phase: nba_domain::action_window::ActionPhase,
    pub preparation_ends_at: f64,
    pub execution_ends_at: f64,
    pub ends_at: f64,
    pub start_position: Vec2,
    pub target_position: Vec2,
    pub completed: bool,
}

impl DefensiveActionRuntime {
    pub fn phase_at(&self, now: f64) -> Option<nba_domain::action_window::ActionPhase> {
        let phase = if now < self.preparation_ends_at {
            nba_domain::action_window::ActionPhase::Preparation
        } else if now < self.execution_ends_at {
            nba_domain::action_window::ActionPhase::Execution
        } else if now < self.ends_at {
            nba_domain::action_window::ActionPhase::FollowThrough
        } else {
            return None;
        };
        (phase != self.phase).then_some(phase)
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
    CloselyGuarded,
    /// 后场八秒未过中线。
    EightSecond,
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
            Self::CloselyGuarded => "closely_guarded",
            Self::EightSecond => "eight_second",
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
    ActionStarted {
        action_id: u64,
        interaction_id: u64,
        player: usize,
        kind: &'static str,
        target: Option<usize>,
    },
    ActionIntentRejected {
        player: usize,
        kind: &'static str,
        reason: &'static str,
    },
    ActionPhaseChanged {
        action_id: u64,
        player: usize,
        from: nba_domain::action_window::ActionPhase,
        to: nba_domain::action_window::ActionPhase,
    },
    ActionCompleted {
        action_id: u64,
        player: usize,
        kind: &'static str,
    },
    ActionFailed {
        action_id: u64,
        player: usize,
        kind: &'static str,
    },
    ActionCancelled {
        action_id: u64,
        player: usize,
        kind: &'static str,
        reason: &'static str,
    },
    DefensiveActionStarted {
        action_id: u64,
        interaction_id: u64,
        defender: usize,
        shooter: usize,
        kind: &'static str,
        start_position: Vec2,
    },
    DefensiveActionPhaseChanged {
        action_id: u64,
        defender: usize,
        from: nba_domain::action_window::ActionPhase,
        to: nba_domain::action_window::ActionPhase,
    },
    DefensiveActionCompleted {
        action_id: u64,
        defender: usize,
        shooter: usize,
        kind: &'static str,
        displacement_ft: f32,
    },
    DefensiveActionCancelled {
        action_id: u64,
        defender: usize,
        shooter: usize,
        kind: &'static str,
        reason: &'static str,
    },
    PossessionStarted {
        team: TeamSide,
        live: bool,
        possession_id: u64,
    },
    PossessionEnded {
        team: TeamSide,
        possession_id: u64,
        points: u32,
        end_reason: nba_domain::event::PossessionEndCause,
        termination_detail: &'static str,
        start_game_clock: f64,
        end_game_clock: f64,
        start_shot_clock: f64,
        end_shot_clock: f64,
        duration: f64,
        passes: u32,
        screens: u32,
        drives: u32,
        possession_kind: PossessionKind,
        had_shot: bool,
        had_offensive_rebound: bool,
        first_action_seconds: Option<f64>,
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
        /// 本次背身选用的步法名称：撤步转身、上下步、匀手、后仰、挤打。
        kind: &'static str,
    },
    Shot {
        shot_id: u64,
        shooter: usize,
        kind: ShotKind,
        zone: &'static str,
        distance_ft: f32,
        contest: f32,
        is_three: bool,
        blocked_by: Option<usize>,
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
        foul_id: u64,
    },
    FreeThrow {
        shooter: usize,
        made: bool,
        index: u8,
        total: u8,
        foul_id: u64,
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
    /// 活球被球员拾起（失误松球），不算篮板。
    LooseBallRecovered {
        player: usize,
        kind: &'static str,
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
        /// 本次切入的路线类型。
        kind: &'static str,
    },
    /// 运球技术细分：动作本身就是一次球与身体的移动，效果按技术区分。
    DribbleMove {
        player: usize,
        kind: nba_domain::action_window::DribbleMoveKind,
        /// 这次技术动作换到的分离距离（英尺）。
        separation_ft: f32,
        /// 这次技术动作期间球暴露给防守人的程度（0 = 贴身护住，1 = 完全暴露）。
        exposure: f32,
    },
    /// 假动作：持球人做出投篮、传球或试探步的假动作，防守人的反应可观察。
    FakeAction {
        player: usize,
        defender: Option<usize>,
        kind: FakeKind,
        /// 防守人是否被骗到；未骗到时没有任何后续效果。
        bitten: bool,
        /// 防守人被骗后实际移动的距离（英尺）。
        displacement_ft: f32,
    },
    /// 护球姿态：持球人进入护球，切球成功率随之折减。
    BallProtection {
        player: usize,
        kind: ProtectKind,
        seconds: f32,
    },
    /// 护球姿态实际挡下一次切球：防守人伸手，球没有丢。
    PokePrevented {
        player: usize,
        defender: usize,
        kind: ProtectKind,
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
    BenchDepleted {
        team: TeamSide,
        player: usize,
    },
    FoulOutRetention {
        team: TeamSide,
        player: usize,
        foul_id: u64,
    },
    TeamTechnicalFoul {
        team: TeamSide,
        reason: &'static str,
        foul_id: u64,
    },
    /// 暂停被批准：比分与两个边界的暂停余额。
    Timeout {
        team: TeamSide,
        remaining: [u32; 2],
    },
    /// 跳球中，一方触球后实际确定控制球权的结果。
    JumpBall {
        kind: JumpBallKind,
        home_jumper: usize,
        away_jumper: usize,
        tapper: usize,
        tapped_team: TeamSide,
    },
    PeriodEnd {
        period: u32,
    },
    /// 选板激活：阵地回合进入某份剧本的行为窗口。
    PlayActivated {
        play: usize,
        possession: u64,
    },
    /// 选板窗口结束。
    PlayWindowEnded {
        play: usize,
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
            Self::ActionStarted { .. } => "ActionStarted",
            Self::ActionIntentRejected { .. } => "ActionIntentRejected",
            Self::ActionPhaseChanged { .. } => "ActionPhaseChanged",
            Self::ActionCompleted { .. } => "ActionCompleted",
            Self::ActionFailed { .. } => "ActionFailed",
            Self::ActionCancelled { .. } => "ActionCancelled",
            Self::DefensiveActionStarted { .. } => "DefensiveActionStarted",
            Self::DefensiveActionPhaseChanged { .. } => "DefensiveActionPhaseChanged",
            Self::DefensiveActionCompleted { .. } => "DefensiveActionCompleted",
            Self::DefensiveActionCancelled { .. } => "DefensiveActionCancelled",
            Self::PossessionStarted { .. } => "PossessionStarted",
            Self::PossessionEnded { .. } => "PossessionEnded",
            Self::Pass { .. } => "Pass",
            Self::Screen { .. } => "Screen",
            Self::Drive { .. } => "Drive",
            Self::PostUp { .. } => "PostUp",
            Self::Shot { .. } => "Shot",
            Self::Foul { .. } => "Foul",
            Self::FreeThrow { .. } => "FreeThrow",
            Self::Rebound { .. } => "Rebound",
            Self::ReboundOpportunity { .. } => "ReboundOpportunity",
            Self::LooseBallRecovered { .. } => "LooseBallRecovered",
            Self::SecondChanceStarted { .. } => "SecondChanceStarted",
            Self::Cut { .. } => "Cut",
            Self::DribbleMove { .. } => "DribbleMove",
            Self::FakeAction { .. } => "FakeAction",
            Self::BallProtection { .. } => "BallProtection",
            Self::PokePrevented { .. } => "PokePrevented",
            Self::BoxOut { .. } => "BoxOut",
            Self::DefenseResponsibilityChanged { .. } => "DefenseResponsibilityChanged",
            Self::Turnover { .. } => "Turnover",
            Self::Steal { .. } => "Steal",
            Self::Substitution { .. } => "Substitution",
            Self::BenchDepleted { .. } => "BenchDepleted",
            Self::FoulOutRetention { .. } => "FoulOutRetention",
            Self::TeamTechnicalFoul { .. } => "TeamTechnicalFoul",
            Self::Timeout { .. } => "Timeout",
            Self::JumpBall { .. } => "JumpBall",
            Self::PeriodEnd { .. } => "PeriodEnd",
            Self::PlayActivated { .. } => "PlayActivated",
            Self::PlayWindowEnded { .. } => "PlayWindowEnded",
            Self::BallContact { kind, .. } => kind,
            Self::PlacementApplied { .. } => "PlacementApplied",
            Self::GameEnd => "GameEnd",
        }
    }
}
