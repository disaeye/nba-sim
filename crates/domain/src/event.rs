use crate::action_window::{ActionPhase, ActionType};
use crate::flow::PhaseType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimedGameEvent {
    pub time: f32,
    pub sequence: u64,
    pub event: GameEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GameEvent {
    Contact {
        player_a: String,
        player_b: String,
        impact_speed: f32,
        contact_normal: (f32, f32),
        is_screen: bool,
        semantic_kind: String,
        semantic_severity: String,
        possessor_id: Option<String>,
        legal_position: bool,
    },
    /// Ball trajectory intersects defender contest/reach cylinder during flight
    PathIntersection {
        ball_pos: (f32, f32, f32),
        defender_id: String,
        clearance_dist: f32,
        flight_time: f32,
    },
    /// Ball arrives at hoop cylinder
    HoopArrival {
        shooter_id: String,
        shot_origin: (f32, f32),
        is_made: bool,
        is_three: bool,
        contest_intensity: f32,
    },
    /// Player crosses boundary line
    BoundaryCross {
        player_id: String,
        pos: (f32, f32),
        boundary_name: String,
    },
    /// Action time-window phase transition
    WindowTransition {
        player_id: String,
        action_type: ActionType,
        new_phase: ActionPhase,
    },
    /// Pass release and arrival are explicit facts, not direct state mutation.
    PassRelease {
        passer_id: String,
        receiver_id: String,
        from_pos: (f32, f32),
        to_pos: (f32, f32),
    },
    PassReceived {
        receiver_id: String,
        /// Frozen physical position where the receiver secured the pass.
        position: (f32, f32),
    },
    /// Pass arrives without being secured by the receiver.
    PassDropped {
        passer_id: String,
        receiver_id: String,
        position: (f32, f32),
    },
    /// A defender touched the pass without securing it.
    PassTipped {
        passer_id: String,
        receiver_id: String,
        defender_id: String,
        position: (f32, f32),
    },
    /// A defender secured the ball during a pass flight.
    PassIntercepted {
        passer_id: String,
        receiver_id: String,
        defender_id: String,
        position: (f32, f32),
    },
    /// A ball handler starts a semantically resolved drive toward a target.
    DriveInitiated {
        driver_id: String,
        from_pos: (f32, f32),
        target_pos: (f32, f32),
    },
    /// A drive reaches its resolution boundary and records the outcome.
    DriveOutcome {
        driver_id: String,
        successful: bool,
        finish_made: bool,
    },
    /// A loose ball was secured after a failed pass or tip.
    LooseBallSecured {
        player_id: String,
        position: (f32, f32),
    },
    /// A free-throw attempt is resolved by the rules layer.
    FreeThrowAttempt {
        shooter_id: String,
        attempt: u8,
        made: bool,
    },
    /// Shot release fact; the semantic result is resolved before flight.
    ShotRelease {
        shooter_id: String,
        pos: (f32, f32),
        is_three: bool,
        contest_level: f32,
        make_probability: f32,
    },
    /// Rebound contest trigger
    ReboundContest {
        rebounder_id: String,
        landing_pos: (f32, f32),
        is_offensive: bool,
    },
    /// Macro phase transition observed by the application event loop.
    PhaseTransition { from: PhaseType, to: PhaseType },
    /// Officiating result derived from a physical contact fact.
    Foul {
        fouled_player_id: String,
        fouler_id: String,
        is_shooting: bool,
    },
    /// A semantic rule was violated after constraint evaluation.
    RuleViolation {
        constraint_id: String,
        reason: String,
    },
    /// A normalized enforcement intent has been applied by the application layer.
    EnforcementApplied {
        constraint_id: String,
        action: String,
    },
    /// 回合结束时的完整语义与因果图总结（用于 L2 叙事一致性校验）。
    PossessionSummary(PossessionSummary),
    /// 犯满离场触发的强制换人（tactics 换人调度规范）。
    Substitution {
        team: String,
        out_player: String,
        in_player: String,
    },
    /// 双方倒地争抢地板球导致争球（Held Ball / Jump Ball）。
    JumpBallTriggered {
        player_a_id: String,
        player_b_id: String,
    },
    /// 离散位置重置（gap.md 第四节第三小节）：发球人从界外 placement 到场内合法位置。
    /// 这是显式生命周期事实，不属于普通运动，检查器据此豁免瞬移判定。
    PlacementApplied {
        player_id: String,
        from: (f32, f32),
        to: (f32, f32),
        reason: String,
        phase: String,
    },
}

/// 回合终结的显式归因（dev 方案 §3.2 D0.1）。
///
/// 每个回合结束必须携带一个终结原因；**没有兜底变体**——历史
/// `UNATTRIBUTED_END` 被物理删除，任何到达回合边界却拿不出原因的
/// 路径在编译期就写不出这条总结。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PossessionEndCause {
    /// 得分（投篮命中或罚球终结）。
    Score,
    /// 防守篮板终结回合（含篮板后的控制转移）。
    DefensiveRebound,
    /// 传球/运球被直接抢断。
    TurnoverSteal,
    /// 传球被点掉后由对方控制松球。
    TurnoverPassTipped,
    /// 传球掉球（未被点掉）后由对方控制。
    TurnoverPassDropped,
    /// 松球易主（其他无法细分到上述三类的松球转换）。
    TurnoverLooseBall,
    /// 违例（24 秒/8 秒/回场/出界等）。
    TurnoverViolation,
    /// 节末/终场导致的回合终结。
    PeriodEnd,
}

impl PossessionEndCause {
    pub fn as_str(&self) -> &'static str {
        match self {
            PossessionEndCause::Score => "SCORE",
            PossessionEndCause::DefensiveRebound => "DEFENSIVE_REBOUND",
            PossessionEndCause::TurnoverSteal => "TURNOVER_STEAL",
            PossessionEndCause::TurnoverPassTipped => "TURNOVER_PASS_TIPPED",
            PossessionEndCause::TurnoverPassDropped => "TURNOVER_PASS_DROPPED",
            PossessionEndCause::TurnoverLooseBall => "TURNOVER_LOOSE_BALL",
            PossessionEndCause::TurnoverViolation => "TURNOVER_VIOLATION",
            PossessionEndCause::PeriodEnd => "PERIOD_END",
        }
    }

    pub fn is_turnover(&self) -> bool {
        self.as_str().starts_with("TURNOVER")
    }
}

impl std::fmt::Display for PossessionEndCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 回合完整因果语义总结。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PossessionSummary {
    pub possession_index: u64,
    pub offense_team: String,
    pub start_clock: f32,
    pub end_clock: f32,
    pub duration_seconds: f32,
    pub passes_count: u32,
    /// 终结原因（向后兼容：旧流中的字符串经 serde 映射到本枚举；
    /// 历史 `UNATTRIBUTED_END` 不再是合法值，解析旧流遇到它将报错——
    /// 严格解析是设计特性，见 dev 方案 §4 D1.3）。
    pub terminal_event: PossessionEndCause,
    #[serde(default)]
    pub shooter_id: Option<String>,
    #[serde(default)]
    pub rebounder_id: Option<String>,
    #[serde(default)]
    pub turnover_player_id: Option<String>,
    #[serde(default)]
    pub shot_contest_intensity: Option<f32>,
    #[serde(default)]
    pub rebound_distance_ft: Option<f32>,
}

impl GameEvent {
    pub fn event_type_str(&self) -> &'static str {
        match self {
            GameEvent::Contact { is_screen, .. } => {
                if *is_screen {
                    "SCREEN_CONTACT"
                } else {
                    "CONTACT_BUMP"
                }
            }
            GameEvent::PathIntersection { .. } => "PASS_INTERCEPT_OPPORTUNITY",
            GameEvent::HoopArrival { is_made, .. } => {
                if *is_made {
                    "SCORE"
                } else {
                    "SHOT_MISS"
                }
            }
            GameEvent::BoundaryCross { .. } => "OUT_OF_BOUNDS",
            GameEvent::WindowTransition { .. } => "ACTION_WINDOW_SHIFT",
            GameEvent::PassRelease { .. } => "PASS",
            GameEvent::PassReceived { .. } => "PASS_RECEIVED",
            GameEvent::PassDropped { .. } => "PASS_DROPPED",
            GameEvent::PassTipped { .. } => "PASS_TIPPED",
            GameEvent::PassIntercepted { .. } => "STEAL",
            GameEvent::DriveInitiated { .. } => "DRIVE_INITIATED",
            GameEvent::DriveOutcome {
                successful,
                finish_made,
                ..
            } => {
                if *finish_made {
                    "DRIVE_SCORE"
                } else if *successful {
                    "DRIVE_MISS"
                } else {
                    "DRIVE_STOPPED"
                }
            }
            GameEvent::LooseBallSecured { .. } => "LOOSE_BALL_SECURED",
            GameEvent::FreeThrowAttempt { .. } => "FREE_THROW",
            GameEvent::ShotRelease { .. } => "SHOT_RELEASE",
            GameEvent::ReboundContest { .. } => "REBOUND",
            GameEvent::PhaseTransition { .. } => "PHASE_TRANSITION",
            GameEvent::Foul { .. } => "FOUL",
            GameEvent::RuleViolation { .. } => "VIOLATION",
            GameEvent::EnforcementApplied { .. } => "ENFORCEMENT_APPLIED",
            GameEvent::PossessionSummary(_) => "POSSESSION_SUMMARY",
            GameEvent::Substitution { .. } => "SUBSTITUTION",
            GameEvent::JumpBallTriggered { .. } => "JUMP_BALL_TRIGGERED",
            GameEvent::PlacementApplied { .. } => "PLACEMENT_APPLIED",
        }
    }
}
