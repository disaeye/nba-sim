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
    pub terminal_event: String,
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
        }
    }
}
