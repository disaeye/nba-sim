//! 权威比赛模型词汇：比赛阶段、比赛程序与球的宏观归属状态。
//!
//! 这三个维度互相正交：阶段回答「比赛进行到哪一段」，程序回答「当前在跑
//! 哪套程序」，球态回答「球现在归谁」。任何程序都不替换球态，任何球态
//! 都不替换阶段。
//!
//! 球态的合法迁移表在本模块给出，写入口（`crates/sim/src/control`）按这张
//! 表检查，非法边就地失败。

use crate::action_window::DribbleMoveKind;
use glam::Vec2;
use serde::{Deserialize, Serialize};

/// 比赛阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchStage {
    Pregame,
    Regulation,
    Intermission,
    Overtime,
    Finalizing,
    Finished,
}

impl MatchStage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pregame => "pregame",
            Self::Regulation => "regulation",
            Self::Intermission => "intermission",
            Self::Overtime => "overtime",
            Self::Finalizing => "finalizing",
            Self::Finished => "finished",
        }
    }

    /// 阶段之间的合法迁移。
    ///
    /// 阶段只按比赛本身的顺序推进：赛前进入常规赛，时钟归零后进入结算，
    /// 结算完成后进入休息、下一节、加时或比赛结束。
    ///
    /// 结算期间比赛钟可能被复核恢复到出手那一刻的读数，本节因此继续：
    /// 结算退回活球阶段是合法迁移。
    pub fn can_transition_to(self, next: MatchStage) -> bool {
        if self == next {
            return true;
        }
        match self {
            Self::Pregame => matches!(next, Self::Regulation),
            Self::Regulation => matches!(next, Self::Finalizing),
            Self::Intermission => matches!(next, Self::Regulation | Self::Overtime),
            Self::Overtime => matches!(next, Self::Finalizing),
            Self::Finalizing => matches!(
                next,
                Self::Regulation | Self::Overtime | Self::Intermission | Self::Finished
            ),
            Self::Finished => false,
        }
    }
}

/// 比赛程序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

impl MatchProgram {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenPlay => "open_play",
            Self::JumpBall => "jump_ball",
            Self::Inbound => "inbound",
            Self::FreeThrows => "free_throws",
            Self::Timeout => "timeout",
            Self::Review => "review",
            Self::PeriodSettlement => "period_settlement",
            Self::PenaltyAdministration => "penalty_administration",
            Self::Intermission => "intermission",
            Self::Pregame => "pregame",
            Self::Complete => "complete",
        }
    }
}

/// 投篮语义：进攻投篮与罚球。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShotContext {
    FieldGoal,
    FreeThrow,
}

impl ShotContext {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FieldGoal => "field_goal",
            Self::FreeThrow => "free_throw",
        }
    }
}

/// 传球技术细分。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// 空中接力：球送到篮筐上空，接球人在空中完成。
    AlleyOop,
}

impl PassKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Chest => "chest",
            Self::Bounce => "bounce",
            Self::Lob => "lob",
            Self::Outlet => "outlet",
            Self::Overhead => "overhead",
            Self::OneHand => "one_hand",
            Self::BehindBack => "behind_back",
            Self::Handoff => "handoff",
            Self::AlleyOop => "alley_oop",
        }
    }
}

/// 球的宏观归属状态。
///
/// 七种状态覆盖持球、运球飞行、传球飞行、投篮飞行、松球、裁判控球与比赛
/// 用球离场。持球人用进程内句柄表达：句柄用于快速访问，事件侧用稳定身份
/// （见 `docs/protocol.md`）。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum BallState {
    /// 持球。持球人身份由这里携带，不再有第二个「是否持球」标志。
    Held {
        holder: usize,
    },
    /// 运球飞行：球在持球人控制下往复触地。
    DribbleFlight {
        holder: usize,
        from_pos: Vec2,
        to_pos: Vec2,
        start: f64,
        duration: f64,
        move_kind: DribbleMoveKind,
    },
    /// 传球飞行。
    PassFlight {
        pass_id: u64,
        from: usize,
        to: usize,
        kind: PassKind,
        from_pos: Vec2,
        to_pos: Vec2,
        start: f64,
        duration: f64,
        inbound: bool,
    },
    /// 投篮飞行。罚球飞行期间比赛程序仍是 `FreeThrows`。
    ShotFlight {
        shot_id: u64,
        shooter: usize,
        context: ShotContext,
        from_pos: Vec2,
        target: Vec2,
        start: f64,
        /// 出手时算出的有效筐口。
        aperture_ft: f32,
        /// 碰到筐沿之后仍能补进的概率。
        rim_in_probability: f32,
        /// 球心在筐平面上的落点偏移（瞄准误差）。
        aim_offset: Vec2,
    },
    /// 松球：球没有归属，最后触球方由控制状态给出。
    Loose {
        pos: Vec2,
        vel: Vec2,
    },
    /// 裁判控球：死球期间球在裁判手里。
    ControlledByOfficial {
        holder: usize,
        inbound_spot: Vec2,
        authorized_at: f64,
    },
    /// 比赛用球离场：球已出界或出界待发球。
    OutOfPlay {
        pos: Vec2,
        #[serde(deserialize_with = "out_of_play_reason")]
        reason: OutOfPlayLabel,
    },
}

type OutOfPlayLabel = &'static str;

fn out_of_play_reason<'de, D: serde::Deserializer<'de>>(decoder: D) -> Result<OutOfPlayLabel, D::Error> {
    match String::deserialize(decoder)?.as_str() {
        "out_of_bounds" => Ok("out_of_bounds"),
        "out_of_play" => Ok("out_of_play"),
        "inbound" => Ok("inbound"),
        reason => Err(serde::de::Error::custom(format!("unknown out-of-play reason `{reason}`"))),
    }
}

/// 球的归属种类：用于迁移表与派生视图，不承载载荷。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BallStateKind {
    Held,
    DribbleFlight,
    PassFlight,
    ShotFlight,
    Loose,
    ControlledByOfficial,
    OutOfPlay,
}

impl BallStateKind {
    pub const ALL: [BallStateKind; 7] = [
        BallStateKind::Held,
        BallStateKind::DribbleFlight,
        BallStateKind::PassFlight,
        BallStateKind::ShotFlight,
        BallStateKind::Loose,
        BallStateKind::ControlledByOfficial,
        BallStateKind::OutOfPlay,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Held => "held",
            Self::DribbleFlight => "dribble_flight",
            Self::PassFlight => "pass_flight",
            Self::ShotFlight => "shot_flight",
            Self::Loose => "loose",
            Self::ControlledByOfficial => "controlled_by_official",
            Self::OutOfPlay => "out_of_play",
        }
    }
}

impl BallState {
    pub fn kind(&self) -> BallStateKind {
        match self {
            Self::Held { .. } => BallStateKind::Held,
            Self::DribbleFlight { .. } => BallStateKind::DribbleFlight,
            Self::PassFlight { .. } => BallStateKind::PassFlight,
            Self::ShotFlight { .. } => BallStateKind::ShotFlight,
            Self::Loose { .. } => BallStateKind::Loose,
            Self::ControlledByOfficial { .. } => BallStateKind::ControlledByOfficial,
            Self::OutOfPlay { .. } => BallStateKind::OutOfPlay,
        }
    }

    pub fn name(&self) -> &'static str {
        self.kind().as_str()
    }

    /// 球是否在空中。
    pub fn is_in_flight(&self) -> bool {
        matches!(
            self.kind(),
            BallStateKind::DribbleFlight | BallStateKind::PassFlight | BallStateKind::ShotFlight
        )
    }

    /// 当前持球人。传球与投篮飞行期间没有持球人。
    pub fn holder(&self) -> Option<usize> {
        match self {
            Self::Held { holder }
            | Self::DribbleFlight { holder, .. }
            | Self::ControlledByOfficial { holder, .. } => Some(*holder),
            _ => None,
        }
    }
}

/// 非法球态迁移的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IllegalBallTransition {
    pub from: BallStateKind,
    pub to: BallStateKind,
}

/// 球态迁移是否合法。
///
/// 合法边按篮球本身的归属变化给出：
///
/// - 持球可以开始运球、传球、投篮、失去控制（松球）、交给裁判或出界；
/// - 运球与持球之间可以来回（收球与再起运球）；
/// - 传球飞行可以被接住（回到持球）、被打断（松球）、或出界；
/// - 投篮飞行可以弹出（松球）、被盖（松球）、或出界；
/// - 松球可以被任一方控制（持球）、继续滚动（松球）、交给裁判或出界；
/// - 裁判控球可以交给发球人（持球）、直接发球（传球飞行）或出界；
/// - 比赛用球离场回到裁判控球或直接被任一方控制。
pub fn ball_state_transition_allowed(from: BallStateKind, to: BallStateKind) -> bool {
    use BallStateKind::*;
    if from == to {
        return true;
    }
    match from {
        Held => matches!(
            to,
            DribbleFlight | PassFlight | ShotFlight | Loose | ControlledByOfficial | OutOfPlay
        ),
        DribbleFlight => matches!(to, Held | PassFlight | ShotFlight | Loose | OutOfPlay),
        PassFlight => matches!(to, Held | Loose | OutOfPlay),
        ShotFlight => matches!(to, Loose | OutOfPlay),
        Loose => matches!(to, Held | ControlledByOfficial | OutOfPlay),
        ControlledByOfficial => matches!(to, Held | PassFlight | OutOfPlay),
        OutOfPlay => matches!(to, ControlledByOfficial | Held),
    }
}

/// 受检查的球态写入口：合法迁移返回新状态，非法迁移就地失败。
pub fn transition_ball_state(
    current: &BallState,
    next: BallState,
) -> Result<BallState, IllegalBallTransition> {
    let from = current.kind();
    let to = next.kind();
    if ball_state_transition_allowed(from, to) {
        Ok(next)
    } else {
        Err(IllegalBallTransition { from, to })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_name() {
        for kind in BallStateKind::ALL {
            assert!(!kind.as_str().is_empty());
        }
    }

    #[test]
    fn the_transition_table_rejects_every_illegal_edge() {
        for from in BallStateKind::ALL {
            for to in BallStateKind::ALL {
                let allowed = ball_state_transition_allowed(from, to);
                if from == to {
                    assert!(allowed, "{from:?} -> {to:?} must be allowed");
                    continue;
                }
                // 飞行中的球不能凭空变成另一种飞行：传球与投篮之间没有直接边。
                if matches!(from, BallStateKind::PassFlight | BallStateKind::ShotFlight)
                    && matches!(to, BallStateKind::PassFlight | BallStateKind::ShotFlight)
                {
                    assert!(!allowed, "{from:?} -> {to:?} must be rejected");
                }
            }
        }
    }

    #[test]
    fn the_stage_machine_only_moves_forward() {
        assert!(MatchStage::Pregame.can_transition_to(MatchStage::Regulation));
        assert!(!MatchStage::Regulation.can_transition_to(MatchStage::Pregame));
        assert!(!MatchStage::Finished.can_transition_to(MatchStage::Regulation));
        assert!(MatchStage::Regulation.can_transition_to(MatchStage::Finalizing));
        assert!(MatchStage::Overtime.can_transition_to(MatchStage::Finalizing));
        assert!(MatchStage::Finalizing.can_transition_to(MatchStage::Intermission));
        assert!(MatchStage::Finalizing.can_transition_to(MatchStage::Overtime));
        assert!(MatchStage::Finalizing.can_transition_to(MatchStage::Finished));
        assert!(MatchStage::Finalizing.can_transition_to(MatchStage::Regulation));
        assert!(!MatchStage::Finalizing.can_transition_to(MatchStage::Pregame));
        assert!(!MatchStage::Regulation.can_transition_to(MatchStage::Intermission));
        assert!(!MatchStage::Overtime.can_transition_to(MatchStage::Finished));
    }
}
