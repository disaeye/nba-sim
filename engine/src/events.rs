use serde::{Deserialize, Serialize};
use crate::action_window::{ActionPhase, ActionType};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PhysicsEvent {
    /// Soft physical contact between two player cylinders within [3.6 ft .. 4.2 ft]
    Contact {
        player_a: String,
        player_b: String,
        impact_speed: f32,
        contact_normal: (f32, f32),
        is_screen: bool,
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
    /// Shot release event
    ShotRelease {
        shooter_id: String,
        pos: (f32, f32),
        is_three: bool,
        contest_level: f32,
    },
    /// Rebound contest trigger
    ReboundContest {
        rebounder_id: String,
        landing_pos: (f32, f32),
        is_offensive: bool,
    },
}

impl PhysicsEvent {
    pub fn event_type_str(&self) -> &'static str {
        match self {
            PhysicsEvent::Contact { is_screen, .. } => {
                if *is_screen { "SCREEN_CONTACT" } else { "CONTACT_BUMP" }
            }
            PhysicsEvent::PathIntersection { .. } => "PASS_INTERCEPT_OPPORTUNITY",
            PhysicsEvent::HoopArrival { is_made, .. } => {
                if *is_made { "SCORE" } else { "SHOT_MISS" }
            }
            PhysicsEvent::BoundaryCross { .. } => "OUT_OF_BOUNDS",
            PhysicsEvent::WindowTransition { .. } => "ACTION_WINDOW_SHIFT",
            PhysicsEvent::ShotRelease { .. } => "SHOT_RELEASE",
            PhysicsEvent::ReboundContest { .. } => "REBOUND",
        }
    }
}
