use glam::Vec2;
use rand::Rng;
use std::collections::HashMap;

use nba_domain::event::PhysicsEvent;
use nba_physics::movement::PlayerPhysicsState;

#[derive(Debug, Clone)]
pub enum ResolutionOutcome {
    NoChange,
    Score { points: u32, shooter_id: String },
    Miss { rebound_spot: Vec2 },
    PassIntercepted { defender_id: String },
    PassTipped { defender_id: String },
    PassReceived { receiver_id: String },
    Foul { fouled_player_id: String, fouler_id: String, is_shooting: bool },
    ReboundSecured { rebounder_id: String, is_offensive: bool },
}

pub struct ResolutionLayer;

impl ResolutionLayer {
    /// Adjudicates a pass intercept opportunity event based on defender rating and clearance
    pub fn resolve_pass_intersection(
        event: &PhysicsEvent,
        _players: &HashMap<String, PlayerPhysicsState>,
        rng: &mut impl Rng,
    ) -> ResolutionOutcome {
        if let PhysicsEvent::PathIntersection { defender_id, clearance_dist, .. } = event {
            // If clearance < 2.0 ft, chance of tip or steal
            let base_contest = (1.0 - (clearance_dist / 2.5)).clamp(0.0, 1.0);
            let steal_prob = (base_contest * 0.25).clamp(0.05, 0.40);
            let tip_prob = (base_contest * 0.40).clamp(0.10, 0.60);

            let roll = rng.gen::<f32>();
            if roll < steal_prob {
                return ResolutionOutcome::PassIntercepted { defender_id: defender_id.clone() };
            } else if roll < steal_prob + tip_prob {
                return ResolutionOutcome::PassTipped { defender_id: defender_id.clone() };
            }
        }
        ResolutionOutcome::NoChange
    }

    /// Adjudicates shot outcome at hoop arrival
    pub fn resolve_shot_arrival(
        is_made: bool,
        is_three: bool,
        shooter_id: &str,
    ) -> ResolutionOutcome {
        if is_made {
            let points = if is_three { 3 } else { 2 };
            ResolutionOutcome::Score {
                points,
                shooter_id: shooter_id.to_string(),
            }
        } else {
            ResolutionOutcome::Miss { rebound_spot: Vec2::ZERO }
        }
    }

    /// Adjudicates soft contact / screen collisions for fouls or momentum reduction
    pub fn resolve_contact(
        player_a_id: &str,
        player_b_id: &str,
        impact_speed: f32,
        is_screen: bool,
        _players: &HashMap<String, PlayerPhysicsState>,
        rng: &mut impl Rng,
    ) -> ResolutionOutcome {
        if impact_speed > 16.0 {
            // High impact collision - small probability of offensive charge or defensive blocking foul
            let foul_roll = rng.gen_range(0.0..1.0);
            if foul_roll < 0.08 {
                let (fouled, fouler) = if is_screen {
                    (player_a_id.to_string(), player_b_id.to_string())
                } else {
                    (player_b_id.to_string(), player_a_id.to_string())
                };
                return ResolutionOutcome::Foul {
                    fouled_player_id: fouled,
                    fouler_id: fouler,
                    is_shooting: false,
                };
            }
        }
        ResolutionOutcome::NoChange
    }
}
