//! 比赛生命周期词汇：阶段投影、程序内阶段与运球控制状态。
//!
//! 这些类型只描述比赛世界的事实，不包含决策、物理或裁判实现。
//!
//! 权威的比赛阶段与比赛程序在 `match_model` 里；本模块的 [`GameFlowState`]
//! 是它们的只读投影，由 [`GameFlowState::from_parts`] 派生，自身不维护
//! 独立的转换表。

use crate::action_window::Foot;
use crate::match_model::{MatchProgram, MatchStage};
use serde::{Deserialize, Serialize};

/// 比赛生命周期的兼容投影。
///
/// 它回答「比赛现在处于哪种宏观局面」，由权威的 [`MatchStage`] 与
/// [`MatchProgram`] 派生。投影只读：任何阶段变化都由权威状态发布。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameFlowState {
    Pregame,
    TipOff,
    LiveBall,
    DeadBall,
    Timeout,
    FreeThrow,
    QuarterEnd,
    Halftime,
    Overtime,
    GameEnd,
}

impl GameFlowState {
    /// 由权威阶段与程序派生兼容投影。
    pub fn from_parts(stage: MatchStage, program: MatchProgram) -> Self {
        match (stage, program) {
            (MatchStage::Pregame, _) => Self::Pregame,
            (MatchStage::Finished, _) => Self::GameEnd,
            (MatchStage::Intermission, _) => Self::Halftime,
            (MatchStage::Overtime, _) => Self::Overtime,
            (MatchStage::Regulation, MatchProgram::JumpBall) => Self::TipOff,
            (MatchStage::Regulation, MatchProgram::FreeThrows) => Self::FreeThrow,
            (MatchStage::Regulation, MatchProgram::Timeout) => Self::Timeout,
            (MatchStage::Regulation, MatchProgram::PeriodSettlement) => Self::QuarterEnd,
            (MatchStage::Regulation, MatchProgram::OpenPlay) => Self::LiveBall,
            (MatchStage::Regulation, _) => Self::DeadBall,
            (MatchStage::Finalizing, _) => Self::QuarterEnd,
        }
    }

    /// 活球动作是否允许发生。
    pub fn allows_live_ball_actions(self) -> bool {
        matches!(self, Self::LiveBall | Self::Overtime)
    }

    /// 比赛时钟只在实际活球期间推进；罚球和死球均停表。
    pub fn advances_game_clock(self) -> bool {
        matches!(self, Self::LiveBall | Self::Overtime)
    }

    /// 进攻时钟只在活球期间推进。
    pub fn advances_shot_clock(self) -> bool {
        matches!(self, Self::LiveBall | Self::Overtime)
    }

    pub fn is_terminal(self) -> bool {
        self == Self::GameEnd
    }

    /// 没有任何活球动作可以发生的局面。
    pub fn is_dead_ball(self) -> bool {
        !self.allows_live_ball_actions()
    }

    /// 允许进行死球准备的局面。
    pub fn allows_dead_ball_setup(self) -> bool {
        matches!(
            self,
            Self::DeadBall | Self::Timeout | Self::QuarterEnd | Self::Halftime
        )
    }

    /// 合法宏观迁移：由权威阶段迁移表派生，自身不另存一份表。
    pub fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        match self {
            Self::Pregame => matches!(next, Self::TipOff | Self::GameEnd),
            Self::TipOff => matches!(
                next,
                Self::LiveBall | Self::DeadBall | Self::FreeThrow | Self::GameEnd
            ),
            Self::LiveBall => matches!(
                next,
                Self::DeadBall
                    | Self::Timeout
                    | Self::FreeThrow
                    | Self::QuarterEnd
                    | Self::Halftime
                    | Self::Overtime
                    | Self::GameEnd
            ),
            Self::DeadBall => matches!(
                next,
                Self::LiveBall
                    | Self::Timeout
                    | Self::FreeThrow
                    | Self::QuarterEnd
                    | Self::Halftime
                    | Self::Overtime
                    | Self::GameEnd
            ),
            Self::Timeout => matches!(next, Self::DeadBall | Self::LiveBall | Self::GameEnd),
            Self::FreeThrow => matches!(
                next,
                Self::LiveBall
                    | Self::DeadBall
                    | Self::QuarterEnd
                    | Self::Halftime
                    | Self::GameEnd
            ),
            Self::QuarterEnd => matches!(
                next,
                Self::Halftime | Self::LiveBall | Self::Overtime | Self::DeadBall | Self::GameEnd
            ),
            Self::Halftime => matches!(next, Self::DeadBall | Self::LiveBall | Self::GameEnd),
            Self::Overtime => matches!(
                next,
                Self::DeadBall | Self::Timeout | Self::QuarterEnd | Self::GameEnd
            ),
            Self::GameEnd => false,
        }
    }
}

/// 程序内阶段。阶段是约束激活和决策目标的边界。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhaseType {
    TipOff,
    Inbound,
    Transition,
    SetPlay,
    Resolution,
    Rebound,
    FreeThrow,
    Timeout,
    DeadBallReset,
}

impl PhaseType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TipOff => "TIP_OFF",
            Self::Inbound => "INBOUND",
            Self::Transition => "TRANSITION",
            Self::SetPlay => "SET_PLAY",
            Self::Resolution => "RESOLUTION",
            Self::Rebound => "REBOUND",
            Self::FreeThrow => "FREE_THROW",
            Self::Timeout => "TIMEOUT",
            Self::DeadBallReset => "DEAD_BALL_RESET",
        }
    }
}

/// 持球人运球与收球的规则状态。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DribbleControlState {
    /// 当前持球段仍在运球，可连续改变方向并再次启动突破。
    LiveDribble {
        dribbled: bool,
        pivot_foot: Option<Foot>,
    },
    /// 球员已收球；接下来最多可完成两步动作，并且不得移动已确立的轴心脚。
    Gathered {
        pivot_foot: Option<Foot>,
        pivot_position: glam::Vec2,
        steps_after_gather: u8,
    },
}

impl Default for DribbleControlState {
    fn default() -> Self {
        Self::LiveDribble {
            dribbled: false,
            pivot_foot: None,
        }
    }
}

impl DribbleControlState {
    pub fn is_gathered(self) -> bool {
        matches!(self, Self::Gathered { .. })
    }
}
