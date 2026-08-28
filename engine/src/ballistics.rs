use glam::Vec2;
use rand::Rng;
use std::collections::HashMap;
use crate::court::{COURT_HEIGHT_FT, COURT_WIDTH_FT};
use crate::movement::PlayerPhysicsState;

pub const GRAVITY_FTPS2: f32 = 32.174;
pub const RIM_HEIGHT_FT: f32 = 10.0;
pub const CHEST_HEIGHT_FT: f32 = 4.0;
pub const MAX_BALL_SPEED_FTPS: f32 = 85.0;

#[derive(Debug, Clone)]
pub enum BallTrajectoryKind {
    Held { carrier_id: String },
    Pass {
        from_pos: Vec2,
        to_pos: Vec2,
        target_id: String,
        start_time: f32,
        duration: f32,
        peak_z: f32,
    },
    Shot {
        shooter_id: String,
        from_pos: Vec2,
        hoop_pos: Vec2,
        start_time: f32,
        duration: f32,
        is_made: bool,
        is_three: bool,
        peak_z: f32,
    },
    LooseBall {
        pos: Vec2,
        vel: Vec2,
        z: f32,
        vel_z: f32,
    },
    RimRebound {
        hoop_pos: Vec2,
        target_landing: Vec2,
        start_time: f32,
        duration: f32,
        peak_z: f32,
    },
}

#[derive(Debug, Clone)]
pub struct ReboundLandingSpot {
    pub landing_pos: Vec2,
    pub flight_duration: f32,
    pub rebounder_id: Option<String>,
}

pub struct BallisticsEngine;

impl BallisticsEngine {
    /// Extrapolates a receiver's future position using current velocity and target intent
    pub fn extrapolate_receiver_pos(
        receiver: &PlayerPhysicsState,
        lead_time_sec: f32,
    ) -> Vec2 {
        let predicted = receiver.pos_ft + receiver.vel_ft * lead_time_sec;
        Vec2::new(
            predicted.x.clamp(2.0, COURT_WIDTH_FT - 2.0),
            predicted.y.clamp(2.0, COURT_HEIGHT_FT - 2.0),
        )
    }

    /// Evaluates exact 3D ball position (x, y, z) at continuous time t
    pub fn sample_ball_position(
        state: &BallTrajectoryKind,
        current_time: f32,
        players: &HashMap<String, PlayerPhysicsState>,
    ) -> (Vec2, f32) {
        match state {
            BallTrajectoryKind::Held { carrier_id } => {
                if let Some(carrier) = players.get(carrier_id) {
                    let offset = if carrier.vel_ft.length() > 0.5 {
                        carrier.vel_ft.normalize() * 0.8
                    } else {
                        Vec2::new(0.5, 0.0)
                    };
                    (carrier.pos_ft + offset, CHEST_HEIGHT_FT)
                } else {
                    (Vec2::new(47.0, 25.0), CHEST_HEIGHT_FT)
                }
            }
            BallTrajectoryKind::Pass { from_pos, to_pos, start_time, duration, peak_z, target_id } => {
                let target_player = players.get(target_id);
                let target_pos = target_player.map(|p| p.pos_ft).unwrap_or(*to_pos);
                let progress = ((current_time - start_time) / duration).clamp(0.0, 1.0);
                let xy = from_pos.lerp(target_pos, progress);
                let start_z = *peak_z;
                let end_z = CHEST_HEIGHT_FT;
                let linear_z = start_z + (end_z - start_z) * progress;
                let arc = 0.0;
                let z = linear_z + arc;
                (xy, z)
            }
            BallTrajectoryKind::Shot { from_pos, hoop_pos, start_time, duration, peak_z, .. } => {
                let progress = ((current_time - start_time) / duration).clamp(0.0, 1.0);
                let xy = from_pos.lerp(*hoop_pos, progress);
                let start_z = CHEST_HEIGHT_FT;
                let end_z = RIM_HEIGHT_FT;
                let linear_z = start_z + (end_z - start_z) * progress;
                let arc = 4.0 * (peak_z - ((start_z + end_z) / 2.0)).max(1.0) * progress * (1.0 - progress);
                let z = linear_z + arc;
                (xy, z)
            }
            BallTrajectoryKind::LooseBall { pos, z, .. } => (*pos, *z),
            BallTrajectoryKind::RimRebound { hoop_pos, target_landing, start_time, duration, peak_z } => {
                let progress = ((current_time - start_time) / duration).clamp(0.0, 1.0);
                let xy = hoop_pos.lerp(*target_landing, progress);
                let start_z = RIM_HEIGHT_FT;
                let end_z = CHEST_HEIGHT_FT;
                let linear_z = start_z + (end_z - start_z) * progress;
                let arc = 4.0 * (peak_z - start_z).max(0.5) * progress * (1.0 - progress);
                let z = (linear_z + arc).max(1.0);
                (xy, z)
            }
        }
    }

    /// Computes rebound landing spot based on shot origin and hoop physics
    pub fn compute_rebound_landing(
        shot_origin: Vec2,
        hoop_pos: Vec2,
        rng: &mut impl Rng,
    ) -> ReboundLandingSpot {
        let shot_vector = hoop_pos - shot_origin;
        let shot_dist = shot_vector.length();
        let shot_dir = shot_vector.normalize_or_zero();

        // Rebound direction: Long shots bounce longer and opposite or angled
        let bounce_dist = if shot_dist > 22.0 {
            rng.gen_range(8.0..18.0) // Long rebound
        } else {
            rng.gen_range(3.0..9.0)  // Short rebound near paint
        };

        // Angular deflection around hoop
        let angle_offset = rng.gen_range(-1.0_f32..1.0_f32);
        let perp = Vec2::new(-shot_dir.y, shot_dir.x);
        let rebound_dir = (-shot_dir * 0.7 + perp * angle_offset).normalize_or_zero();

        let raw_landing = hoop_pos + rebound_dir * bounce_dist;
        let landing_pos = Vec2::new(
            raw_landing.x.clamp(4.0, COURT_WIDTH_FT - 4.0),
            raw_landing.y.clamp(4.0, COURT_HEIGHT_FT - 4.0),
        );

        let flight_duration = 0.8 + (bounce_dist / 15.0) * 0.4;

        ReboundLandingSpot {
            landing_pos,
            flight_duration,
            rebounder_id: None,
        }
    }
}
