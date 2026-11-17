//! 动作接口：决策层的输出、结算层的输出、结算带来的状态变化。
//!
//! 动作是因果链的单元。结算产出的是**状态变化**，不是结果：掩护产出分离与
//! 防守人路线，突破产出是否过掉首要防守人，传球产出接球质量。下一次决策读
//! 这些字段，出手位置与命中率都是动作链的后果。

use glam::Vec2;

use crate::clock::ClockCommit;
use crate::officiating::{ControlHandover, ControlState, ControlTransition};
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
    /// 跳步收球：收球后用跳步把步数用完。
    ProHop,
    /// 欧洲步：收球步往侧面绕开挡路防守人。
    EuroStep,
    /// 挑篮：单手挑球上篮。
    FingerRoll,
    /// 拉杆上篮：篮下二次打板。
    DoubleClutchLayup,
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
            Self::ProHop => "pro_hop",
            Self::EuroStep => "euro_step",
            Self::FingerRoll => "finger_roll",
            Self::DoubleClutchLayup => "double_clutch_layup",
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
            Self::ProHop | Self::EuroStep | Self::FingerRoll | Self::DoubleClutchLayup => "drive",
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
                | Self::ProHop
                | Self::EuroStep
                | Self::FingerRoll
                | Self::DoubleClutchLayup
        )
    }
}

/// 转换动作的技术变体。
///
/// 转换是攻防两端同时发生的一段：进攻方有人推进、有人跟进，防守方有人
/// 退防、有人落位形成阵地防守。四个变体因此分属两方。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    /// 快攻推进：带球的人往前场推进。
    Push,
    /// 快攻跟进：不带球的跑者跟上第二波。
    Trail,
    /// 退防：防守人回防。
    Retreat,
    /// 快速形成阵地防守：防守人先落位。
    Wall,
}

impl TransitionKind {
    /// 变体的档案键。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Push => "transition_push",
            Self::Trail => "transition_trail",
            Self::Retreat => "transition_retreat",
            Self::Wall => "transition_wall",
        }
    }
}

/// 篮板动作的技术变体。
///
/// 篮板不是一件事：起跳争抢、控制篮板、点球、点传与补篮各有各的
/// 后果。变体决定这次篮板动作的结束方式与后续。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReboundKind {
    /// 起跳争抢：跳起来够到球。
    JumpRebound,
    /// 控制篮板：把球控制在手里。
    SecureRebound,
    /// 点球：碰一下球，球还没被控制。
    Tap,
    /// 点传：把球拨给队友。
    TipPass,
    /// 补篮：点球之后直接进攻。
    TipIn,
    /// 地板球争抢：两人都趴下抢。
    LooseBallDive,
    /// 双方同时控制：裁判抛球重新开始。
    HeldBall,
}

impl ReboundKind {
    /// 变体的档案键。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::JumpRebound => "jump_rebound",
            Self::SecureRebound => "secure_rebound",
            Self::Tap => "tap",
            Self::TipPass => "tip_pass",
            Self::TipIn => "tip_in",
            Self::LooseBallDive => "loose_ball_dive",
            Self::HeldBall => "held_ball",
        }
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
    /// 闪切：短距离变向切入，在防守人换防之前到位。
    Flash,
    /// 拉开站位：向外线移动，把防守人带出禁区。
    SpotUp,
    /// 底角转移：沿底线到对侧底角。
    CornerRelocate,
    /// 手递手接球：切入人贴着持球人接球，两人手递手交接。
    Handoff,
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
            Self::Flash => "flash",
            Self::SpotUp => "spot_up",
            Self::CornerRelocate => "corner_relocate",
            Self::Handoff => "handoff",
        }
    }
}

/// 背身步法：低位进攻的技术变体。
///
/// 步法决定进攻人怎么移动与防守人被推到哪一侧。档案里的八个步法与
/// 生产路径里的三种判定分开：步法名是事实里的技术细分，判定层按它
/// 选空间效果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostStepKind {
    /// 背身要位：先占住低位。
    PostUp,
    /// 背身挤打：朝防守人反方向挤开。
    Backdown,
    /// 撤步转身：往底线一侧绕过去。
    DropStep,
    /// 前转身：以轴心脚为轴向底线转身。
    FrontTurn,
    /// 后转身：以轴心脚为轴向中路转身。
    ReverseTurn,
    /// 上下步：往底线方向顶，防守人被挤到身后。
    UpAndUnder,
    /// 勾手：背身勾手。
    Hook,
    /// 背身后仰：转身后仰跳投。
    TurnaroundFadeaway,
}

impl PostStepKind {
    /// 步法的档案键。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PostUp => "post_up",
            Self::Backdown => "backdown",
            Self::DropStep => "drop_step",
            Self::FrontTurn => "front_turn",
            Self::ReverseTurn => "reverse_turn",
            Self::UpAndUnder => "up_and_under",
            Self::Hook => "hook",
            Self::TurnaroundFadeaway => "turnaround_fadeaway",
        }
    }

    /// 从档案键读回步法。
    pub fn from_str(key: &str) -> Option<Self> {
        Some(match key {
            "post_up" => Self::PostUp,
            "backdown" => Self::Backdown,
            "drop_step" => Self::DropStep,
            "front_turn" => Self::FrontTurn,
            "reverse_turn" => Self::ReverseTurn,
            "up_and_under" => Self::UpAndUnder,
            "hook" => Self::Hook,
            "turnaround_fadeaway" => Self::TurnaroundFadeaway,
            _ => return None,
        })
    }
}

/// 收球终结的技术变体。
///
/// 收球之后的那一步有多种做法：普通上篮、反手上篮、拉杆上篮、挑篮、
/// 抛投、扣篮与补篮。收球步的动作本身（跳步、欧洲步）也是这一类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishKind {
    /// 普通上篮。
    Layup,
    /// 反手上篮。
    ReverseLayup,
    /// 拉杆上篮：篮下二次打板。
    DoubleClutchLayup,
    /// 挑篮：单手挑球上篮。
    FingerRoll,
    /// 抛投。
    Floater,
    /// 扣篮。
    Dunk,
    /// 补篮。
    Putback,
    /// 跳步收球。
    ProHop,
    /// 欧洲步。
    EuroStep,
}

impl FinishKind {
    /// 变体的档案键。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Layup => "layup",
            Self::ReverseLayup => "reverse_layup",
            Self::DoubleClutchLayup => "double_clutch_layup",
            Self::FingerRoll => "finger_roll",
            Self::Floater => "floater",
            Self::Dunk => "dunk",
            Self::Putback => "putback",
            Self::ProHop => "pro_hop",
            Self::EuroStep => "euro_step",
        }
    }

    /// 从出手类型读出收球终结的变体。
    pub fn from_shot(kind: ShotKind) -> Option<Self> {
        Some(match kind {
            ShotKind::Layup
            | ShotKind::DrivingLayup
            | ShotKind::CuttingLayup
            | ShotKind::RunningLayup => Self::Layup,
            ShotKind::ReverseLayup => Self::ReverseLayup,
            ShotKind::DoubleClutchLayup => Self::DoubleClutchLayup,
            ShotKind::FingerRoll => Self::FingerRoll,
            ShotKind::Floater => Self::Floater,
            ShotKind::Dunk
            | ShotKind::DrivingDunk
            | ShotKind::CuttingDunk
            | ShotKind::RunningDunk => Self::Dunk,
            ShotKind::Putback => Self::Putback,
            ShotKind::ProHop => Self::ProHop,
            ShotKind::EuroStep => Self::EuroStep,
            _ => return None,
        })
    }
}

/// 跑位的技术变体。
///
/// 跑位不是一件事：接球点调整、外线定点、无球绕行与错位后要位各有各的
/// 落点与防守反应。变体决定这一次跑位去哪里。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelocateKind {
    /// 接球点调整：按编队空位移动。
    Adjust,
    /// 外线定点：留在三分线外的定点位置。
    SpotUp,
    /// 无球绕行：绕到球的另一侧。
    Flare,
    /// 错位后要位：错位时占住对手的防守位。
    MismatchSeal,
}

impl RelocateKind {
    /// 变体的档案键。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Adjust => "adjust",
            Self::SpotUp => "spot_up",
            Self::Flare => "flare",
            Self::MismatchSeal => "mismatch_seal",
        }
    }
}

/// 传球技术细分：权威定义在领域层。
pub use nba_domain::PassKind;

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
    /// 顺下：掩护之后向篮下滚动。
    Roll,
    /// 外弹：掩护之后向外线弹开。
    Pop,
    /// 提前滑出：掩护者在接触之前先离开。
    Slip,
}

impl ScreenKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OnBall => "on_ball",
            Self::OffBall => "off_ball",
            Self::HandOff => "handoff",
            Self::Roll => "roll",
            Self::Pop => "pop",
            Self::Slip => "slip",
        }
    }
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
    /// 被包夹护球：两名防守人夹击时护住球。
    Trap,
}

impl ProtectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FaceUp => "face_up_protect",
            Self::Post => "post_protect",
            Self::Spin => "spin_protect",
            Self::Trap => "trap_protect",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ActionKind {
    Dribble(nba_domain::action_window::DribbleMoveKind),
    Drive(nba_domain::action_window::DriveKind),
    Finish(FinishKind),
    Pass(PassKind),
    Screen(ScreenKind),
    Cut(CutKind),
    PostUp(PostStepKind),
    Shoot(ShotKind),
    /// 罚球：罚球程序里的出手。它不是一种统计上的出手类型，而是另一套
    /// 程序里的动作，因此它在动作族里占一个自己的位置。
    FreeThrow,
    Relocate(RelocateKind),
    Fake(FakeKind),
    Protect(ProtectKind),
    Rebound(ReboundKind),
}

impl ActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dribble(_) => "Dribble",
            Self::Drive(_) => "Drive",
            Self::Finish(_) => "Finish",
            Self::Pass(_) => "Pass",
            Self::Screen(_) => "Screen",
            Self::Cut(_) => "Cut",
            Self::PostUp(_) => "PostUp",
            Self::Shoot(_) => "Shoot",
            Self::FreeThrow => "FreeThrow",
            Self::Relocate(_) => "Relocate",
            Self::Fake(_) => "Fake",
            Self::Protect(_) => "Protect",
            Self::Rebound(_) => "Rebound",
        }
    }

    /// 动作族标签，供决策审计与协议投影使用。
    pub fn label(self) -> &'static str {
        match self {
            Self::Shoot(_) => "shoot",
            Self::FreeThrow => "free_throw",
            Self::Drive(_) => "drive",
            Self::Pass(_) => "pass",
            Self::Screen(_) => "screen",
            Self::Cut(_) => "cut",
            Self::PostUp(_) => "post_up",
            Self::Dribble(_) => "hold",
            Self::Finish(_) => "finish",
            Self::Relocate(_) => "relocate",
            Self::Fake(_) => "fake",
            Self::Protect(_) => "protect",
            Self::Rebound(_) => "rebound",
        }
    }

    /// 动作族编号，供决策观测与过程判据使用。
    pub fn family(self) -> u8 {
        match self {
            Self::Shoot(_) => 0,
            Self::Drive(_) => 1,
            Self::Pass(_) => 2,
            Self::Screen(_) => 3,
            Self::Cut(_) => 4,
            Self::PostUp(_) => 5,
            Self::Dribble(_) => 6,
            Self::Finish(_) => 7,
            Self::Relocate(_) => 8,
            Self::Fake(_) => 9,
            Self::Protect(_) => 10,
            Self::Rebound(_) => 11,
            Self::FreeThrow => 12,
        }
    }
}

/// 动作族总数：族编号直接用作按族索引的数组长度。
pub const FAMILY_COUNT: usize = 13;

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
    /// 掏球：贴着球去摸，不赌抢断那么狠。
    Poke,
    /// 追防：持球人甩开之后追上去。
    Chase,
    /// 垂直起跳封盖：执行窗口内摸高增加，跳起的人无法卡位抢篮板。
    BlockJump,
    /// 垂直起跳扑防：防守人已经站到投篮人面前，他垂直跳起伸手而不
    /// 是向前扑。
    Verticality,
    /// 追身封盖：从投篮人身后扑上去盖。
    ChaseDownBlock,
    /// 滑步跟防：横向移动保持一个身位的间隔，不越过持球人的合法
    /// 防守位置（NBA Rule 12B §II.5）。
    Slide,
    /// 贴身施压：把间隔压到伸手的距离上，代价是赌博失败后失位。
    Contain,
    /// 站位吃犯规：面对突破时站定占住位置，逼进攻方绕开或撞人
    /// （NBA Rule 12B §II.1）。
    ChargeTake,
}

/// 对位防守人自己选的站位动作。
///
/// 这三种与「伸手切球」「扑防」「起跳封盖」不同：它们不是赌博，而是
/// 防守人面对持球人时自己选的站位方式，因此单独成一个枚举，选取与起手
/// 两处都不必处理其余防守动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnBallPositioning {
    /// 滑步跟防：横向移动保持一个身位的间隔。
    Slide,
    /// 贴身施压：把间隔压到伸手的距离上。
    Contain,
    /// 站位吃犯规：面对突破时站定占住位置。
    ChargeTake,
    /// 追防：被持球人甩开之后追上去。
    Chase,
}

impl OnBallPositioning {
    /// 转成防守动作的种类：站位动作与防守动作共用同一套运行时。
    pub fn defensive_kind(self) -> DefensiveActionKind {
        match self {
            Self::Slide => DefensiveActionKind::Slide,
            Self::Contain => DefensiveActionKind::Contain,
            Self::ChargeTake => DefensiveActionKind::ChargeTake,
            Self::Chase => DefensiveActionKind::Chase,
        }
    }
}

/// 防守人正在跑的站位动作。
///
/// 返回防守人当前正在跑的那一种站位动作；没有在跑时返回 `None`。
pub fn on_ball_positioning(world: &crate::World, defender: usize) -> Option<OnBallPositioning> {
    world.players.get(defender)?.on_ball_positioning
}

/// 站位动作对持球人的影响：贴身施压压低他的出手与出球质量。
///
/// 站位动作不改防守人站哪：那是职责层的事。站位动作改的是结算层读到
/// 的那个防守状态，因此它不会干扰对位的连续性。
pub fn on_ball_pressure(world: &crate::World, defender: usize) -> Option<f32> {
    match on_ball_positioning(world, defender)? {
        // 滑步跟防：守住位置，没有额外压力。
        OnBallPositioning::Slide => None,
        // 贴身施压：持球人被贴住，出手与出球都更难。
        OnBallPositioning::Contain => Some(world.params.policy.g("contain_pressure_multiplier")),
        // 站位吃犯规：防守人站住逼他绕开，进攻方要付出绕路的时间。
        OnBallPositioning::ChargeTake => {
            Some(world.params.policy.g("charge_take_pressure_multiplier"))
        }
        // 追防：追回来的人重新建立压力，压力与滑步跟防一样：追的过程
        // 里他没有压迫持球人。
        OnBallPositioning::Chase => None,
    }
}

impl DefensiveActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CloseoutContest => "closeout_contest",
            Self::StealAttempt => "steal_attempt",
            Self::Poke => "poke",
            Self::Chase => "chase",
            Self::BlockJump => "block_jump",
            Self::Verticality => "verticality",
            Self::ChaseDownBlock => "chase_down_block",
            Self::Slide => "slide",
            Self::Contain => "contain",
            Self::ChargeTake => "charge_take",
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
    /// 进攻三秒违例：进攻人在限制区里连续停留超过三秒。
    LaneViolation,
    /// 后场八秒未过中线。
    EightSecond,
    /// 已过中线又把球带回后场。
    Backcourt,
    /// 背身五秒违例（NBA Rule 10 §XVI）。
    BackToBasket,
    /// 罚球失手时球未碰筐：由篮板争抢处理，不记作违例。
    FreeThrowMissNoRim,
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
            Self::Backcourt => "backcourt",
            Self::BackToBasket => "back_to_basket",
            Self::LaneViolation => "lane_violation",
            Self::FreeThrowMissNoRim => "free_throw_miss_no_rim",
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
    /// 持续姿态开始。
    ///
    /// 持续姿态不是带窗口的动作：它没有准备、执行与收势三段，因此它
    /// 不借用动作生命周期的事实。它自己登记开始时刻与身份，事件流因此
    /// 能重建姿态的起止，而渲染器能从姿态身份认出「同一个姿态」。
    PostureStarted {
        action_id: u64,
        player: usize,
        /// 姿态槽位名：无球职责、持球站位、转换与扑防应答。
        slot: &'static str,
        /// 技术变体：职责名、站位名、转换变体与应答名。
        variant: &'static str,
    },
    /// 持续姿态结束：它与开始事实配对。
    PostureEnded {
        action_id: u64,
        player: usize,
        slot: &'static str,
        variant: &'static str,
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
        /// 本次突破的技术变体：直接、变向还是转身。
        kind: &'static str,
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
    /// 罚球禁区违例：`side` 是提前进区的一方（`offense` 或 `defense`）。
    FreeThrowLaneViolation {
        side: &'static str,
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
    /// 防守职责变了。
    ///
    /// 这一条事实既是职责变化，也是无球姿态的生命周期：职责就是姿态，
    /// 因此它的身份与时间边界在这一条里一起给出。渲染器从职责读出当前
    /// 姿态，事件流从这一条重建姿态的起止。
    DefenseResponsibilityChanged {
        player: usize,
        from: &'static str,
        to: &'static str,
        /// 这一段职责的动作身份：姿态与动作共用同一套身份空间。
        action_id: u64,
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
    /// 对位防守人的站位姿态变了：滑步、贴身施压、站位吃犯规。
    ///
    /// 姿态不是带窗口的动作，因此它只发布一次变化事实，不发布阶段变化
    /// 与完成事实：那三类事实是给「有准备、执行、收势三段」的动作用的。
    /// 对位防守人的站位姿态变了。
    ///
    /// 这一条事实既是站位变化，也是持球站位姿态的生命周期：站位就是
    /// 姿态，因此它的身份与时间边界在这一条里一起给出。
    OnBallPositioningChanged {
        defender: usize,
        kind: &'static str,
        /// 这一段站位的动作身份：姿态与动作共用同一套身份空间。
        action_id: u64,
    },
    /// 球员被逐：犯规升级到恶意犯规时使用（NBA Rule 12B §IV）。
    Ejected {
        team: TeamSide,
        player: usize,
        reason: &'static str,
    },
    /// 暂停被批准：比分与两个边界的暂停余额。
    Timeout {
        team: TeamSide,
        remaining: [u32; 2],
        /// 强制暂停（联赛规则在固定时点自动停表）还是主动申请的暂停。
        mandatory: bool,
    },
    /// 跳球程序在中圈建立；与后续拍球归属事实分开。
    JumpBallStarted {
        kind: JumpBallKind,
        home_jumper: usize,
        away_jumper: usize,
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
    /// 节末结算完成：全部待决事项关闭，本节比分与球权就此确定。
    PeriodSettlementCompleted {
        period: u32,
        home_score: u32,
        away_score: u32,
        /// 本节关闭的待决出手数。
        pending_shots_resolved: u32,
        /// 本节执行的罚球次数。
        free_throws_resolved: u32,
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
    /// 干扰球：球在下落的筐柱内被防守人碰到，出手直接记中。
    Goaltending {
        defender: usize,
        shooter: usize,
        shot_id: u64,
        time: f64,
    },
    /// 复核结论：教练挑战的结果与原判是否保留。
    Review {
        team: TeamSide,
        call: &'static str,
        overturned: bool,
        cost_timeout: bool,
        /// 改判带来的分值变化：正数补分，负数扣分。
        points_delta: i32,
        /// 改判涉及的球员：统计归属靠它。
        player: Option<usize>,
        time: f64,
    },
    PlacementApplied {
        player: usize,
        reason: &'static str,
    },
    /// 伤病事件：对抗负荷或碰撞引发的球员受伤。
    Injury {
        team: TeamSide,
        player: usize,
        severity: &'static str,
    },
    /// 一个提交批次内的全部时钟写入。指令、检查点与固定步推进按提交顺序记录。
    ClockDirectivesCommitted {
        commits: Vec<ClockCommit>,
    },
    /// 权威控制状态发生变化：验证器用它独立重建控制序列。
    ControlStateCommitted {
        transition: ControlTransition,
        state: ControlState,
    },
    /// 一次控制权交接：上一段控制序列结束，新的一段开始。
    ///
    /// 交接是原子的：它同时包含上一段的结束与新一段的开始，因此重建时
    /// 建立与结束永远成对。
    ControlHandoverCommitted {
        handover: ControlHandover,
        state: ControlState,
    },
    GameEnd,
}

impl Fact {
    pub fn name(&self) -> &'static str {
        match self {
            Self::ActionStarted { .. } => "ActionStarted",
            Self::PostureStarted { .. } => "PostureStarted",
            Self::PostureEnded { .. } => "PostureEnded",
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
            Self::FreeThrowLaneViolation { .. } => "FreeThrowLaneViolation",
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
            Self::OnBallPositioningChanged { .. } => "ON_BALL_POSITIONING_CHANGED",
            Self::Ejected { .. } => "EJECTED",
            Self::Timeout { .. } => "Timeout",
            Self::JumpBallStarted { .. } => "JumpBallStarted",
            Self::JumpBall { .. } => "JumpBall",
            Self::PeriodEnd { .. } => "PeriodEnd",
            Self::PeriodSettlementCompleted { .. } => "PeriodSettlementCompleted",
            Self::PlayActivated { .. } => "PlayActivated",
            Self::PlayWindowEnded { .. } => "PlayWindowEnded",
            Self::BallContact { kind, .. } => kind,
            Self::Goaltending { .. } => "Goaltending",
            Self::Review { .. } => "Review",
            Self::PlacementApplied { .. } => "PlacementApplied",
            Self::Injury { .. } => "Injury",
            Self::ClockDirectivesCommitted { .. } => "ClockDirectivesCommitted",
            Self::ControlStateCommitted { .. } => "ControlStateCommitted",
            Self::ControlHandoverCommitted { .. } => "ControlHandoverCommitted",
            Self::GameEnd => "GameEnd",
        }
    }

    /// 这条事实是否与这名球员有关。
    ///
    /// 原始事件用它指回自己读到的事实：一条判罚要能说清它依据的是哪几次
    /// 接触、哪几次动作。
    pub fn mentions_player(&self, player: usize) -> bool {
        match self {
            Self::ActionStarted { player: p, .. }
            | Self::ActionIntentRejected { player: p, .. }
            | Self::ActionPhaseChanged { player: p, .. }
            | Self::ActionCompleted { player: p, .. }
            | Self::ActionFailed { player: p, .. }
            | Self::ActionCancelled { player: p, .. }
            | Self::Pass { from: p, .. }
            | Self::Screen { screener: p, .. }
            | Self::Drive { driver: p, .. }
            | Self::PostUp { player: p, .. }
            | Self::Shot { shooter: p, .. }
            | Self::Foul { fouler: p, .. }
            | Self::FreeThrow { shooter: p, .. }
            | Self::Rebound { player: p, .. }
            | Self::LooseBallRecovered { player: p, .. }
            | Self::SecondChanceStarted { rebounder: p, .. }
            | Self::Cut { cutter: p, .. } => *p == player,
            | Self::DribbleMove { player: p, .. }
            | Self::FakeAction { player: p, .. }
            | Self::BallProtection { player: p, .. }
            | Self::PokePrevented { player: p, .. }
            | Self::BoxOut { player: p, .. }
            | Self::Turnover { player: p, .. }
            | Self::Steal { player: p, .. }
            | Self::Substitution { incoming: p, .. }
            | Self::FoulOutRetention { player: p, .. }
            | Self::OnBallPositioningChanged { defender: p, .. }
            | Self::PlacementApplied { player: p, .. }
            | Self::Injury { player: p, .. } => *p == player,
            Self::JumpBallStarted {
                home_jumper,
                away_jumper,
                ..
            } => *home_jumper == player || *away_jumper == player,
            Self::BallContact { entity: Some(p), .. } => *p == player,
            Self::DefensiveActionStarted { defender: p, .. }
            | Self::DefensiveActionPhaseChanged { defender: p, .. }
            | Self::DefensiveActionCompleted { defender: p, .. }
            | Self::DefensiveActionCancelled { defender: p, .. } => *p == player,
            Self::Goaltending { defender: p, .. } => *p == player,
            _ => false,
        }
    }
}
