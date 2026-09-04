use crate::movement::PlayerPhysicsState;
use glam::Vec2;
use nba_domain::GameRules;
use rand::Rng;
use std::collections::HashMap;

// 球的弹道/归属状态类型 = 领域层 BallState（M2 收敛：归属语义
// 不再住在 physics，physics 只按闭式参数采样位置）。
pub use nba_domain::BallState as BallTrajectoryKind;

pub struct ReboundLandingSpot {
    pub landing_pos: Vec2,
    pub flight_duration: f32,
    pub rebounder_id: Option<String>,
}

pub struct BallisticsEngine;
impl BallisticsEngine {
    /// Returns the requested arc coefficient used by the normalized shot curve.
    fn shot_arc_amplitude(peak_z: f32, rules: &GameRules) -> f32 {
        rules.ball_arc_multiplier.max(0.0)
            * (peak_z - ((rules.chest_height_ft + rules.rim_height_ft) / 2.0)).max(1.0)
    }

    /// Computes a shot flight duration that fits the configured ball-speed envelope.
    ///
    /// The shot curve has constant horizontal velocity and a linear vertical
    /// derivative plus the parabolic arc derivative. The sum of the absolute
    /// endpoint derivatives is a conservative bound for its vertical travel,
    /// so this duration prevents a high-arcing long shot from exceeding the
    /// same speed limit used by the stream audit.
    pub fn shot_duration(distance_ft: f32, peak_z: f32, rules: &GameRules) -> f32 {
        let distance = distance_ft.max(0.0);
        let nominal = (distance / rules.shot_speed_ftps.max(f32::EPSILON)).clamp(
            rules.min_shot_duration_seconds,
            rules.max_shot_duration_seconds,
        );
        let arc = Self::shot_arc_amplitude(peak_z, rules);
        let vertical_travel_bound = (rules.rim_height_ft - rules.chest_height_ft).abs() + arc;
        let path_bound = distance.hypot(vertical_travel_bound);
        nominal.max(path_bound / rules.ball_max_speed_ftps.max(f32::EPSILON))
    }

    /// Reduces only the requested arc when a caller supplies a duration whose
    /// horizontal component is valid but whose full arc would exceed the
    /// configured three-dimensional speed envelope.
    fn shot_arc_for_duration(
        distance_ft: f32,
        duration: f32,
        requested_arc: f32,
        rules: &GameRules,
    ) -> f32 {
        let distance = distance_ft.max(0.0);
        let duration = duration.max(f32::EPSILON);
        let max_travel = rules.ball_max_speed_ftps.max(0.0) * duration;
        let horizontal_travel = distance;
        if max_travel <= horizontal_travel {
            return 0.0;
        }
        let vertical_travel_budget =
            (max_travel * max_travel - horizontal_travel * horizontal_travel).sqrt();
        (vertical_travel_budget - (rules.rim_height_ft - rules.chest_height_ft).abs())
            .max(0.0)
            .min(requested_arc)
    }

    /// Extrapolates a receiver within the configured playable court.
    pub fn extrapolate_receiver_pos(
        receiver: &PlayerPhysicsState,
        lead_time_sec: f32,
        rules: &GameRules,
    ) -> Vec2 {
        let predicted = receiver.pos_ft + receiver.vel_ft * lead_time_sec;
        rules
            .court
            .clamp_playable(predicted, rules.player_radius_ft)
    }

    pub fn extrapolate_receiver_pos_default(
        receiver: &PlayerPhysicsState,
        lead_time_sec: f32,
    ) -> Vec2 {
        Self::extrapolate_receiver_pos(receiver, lead_time_sec, &GameRules::default())
    }

    pub fn sample_ball_position(
        state: &BallTrajectoryKind,
        current_time: f32,
        players: &HashMap<String, PlayerPhysicsState>,
        rules: &GameRules,
    ) -> (Vec2, f32) {
        match state {
            BallTrajectoryKind::Held { carrier_id } => {
                if let Some(carrier) = players.get(carrier_id) {
                    let offset = if carrier.vel_ft.length() > 0.5 {
                        carrier.vel_ft.normalize() * rules.ball_holder_offset_ft
                    } else {
                        Vec2::new(rules.ball_holder_offset_ft, 0.0)
                    };
                    (carrier.pos_ft + offset, rules.ball_holder_height_ft)
                } else {
                    (
                        Vec2::new(rules.court.width_ft / 2.0, rules.court.height_ft / 2.0),
                        rules.ball_holder_height_ft,
                    )
                }
            }
            BallTrajectoryKind::ControlTransfer {
                from_pos,
                from_z,
                carrier_id,
                start_time,
                duration,
            } => {
                let target = if let Some(carrier) = players.get(carrier_id) {
                    let offset = if carrier.vel_ft.length() > 0.5 {
                        carrier.vel_ft.normalize() * rules.ball_holder_offset_ft
                    } else {
                        Vec2::new(rules.ball_holder_offset_ft, 0.0)
                    };
                    (carrier.pos_ft + offset, rules.ball_holder_height_ft)
                } else {
                    (*from_pos, *from_z)
                };
                let tau = if *duration > 0.0 {
                    ((current_time - start_time) / duration).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                (
                    from_pos.lerp(target.0, tau),
                    from_z + (target.1 - from_z) * tau,
                )
            }
            BallTrajectoryKind::InboundTransfer {
                from_pos,
                from_z,
                baseline_pos,
                start_time,
                duration,
                ..
            } => {
                let progress =
                    ((current_time - start_time) / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
                let xy = from_pos.lerp(*baseline_pos, progress);
                let z = from_z + (rules.chest_height_ft - *from_z) * progress;
                (xy, z)
            }
            BallTrajectoryKind::InboundReady { baseline_pos, inbounder_id } => {
                if let Some(inbounder) = players.get(inbounder_id) {
                    (inbounder.pos_ft, rules.chest_height_ft)
                } else {
                    (*baseline_pos, rules.chest_height_ft)
                }
            }
            BallTrajectoryKind::Drive { driver_id, .. } => {
                if let Some(driver) = players.get(driver_id) {
                    let offset = if driver.vel_ft.length() > 0.5 {
                        driver.vel_ft.normalize() * rules.ball_holder_offset_ft
                    } else {
                        Vec2::new(rules.ball_holder_offset_ft * 0.5, 0.0)
                    };
                    let bounce = rules.ball_bounce_base_ft
                        + rules.ball_bounce_amplitude_ft
                            * (current_time
                                * rules.ball_bounce_frequency_hz
                                * std::f32::consts::TAU)
                                .sin();
                    (driver.pos_ft + offset, bounce)
                } else {
                    (
                        Vec2::new(rules.court.width_ft / 2.0, rules.court.height_ft / 2.0),
                        rules.ball_holder_height_ft,
                    )
                }
            }
            BallTrajectoryKind::Pass {
                from_pos,
                to_pos,
                target_id,
                start_time,
                duration,
                peak_z,
                ..
            } => {
                let progress =
                    ((current_time - start_time) / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
                let catch_pos = players
                    .get(target_id)
                    .map(|player| player.pos_ft)
                    .unwrap_or(*to_pos);
                let xy = from_pos.lerp(catch_pos, progress);
                let linear_z = *peak_z + (rules.chest_height_ft - *peak_z) * progress;
                let requested_arc =
                    rules.ball_arc_multiplier * (*peak_z - rules.chest_height_ft).max(0.0);
                let distance = (catch_pos - *from_pos).length();
                let arc_amplitude =
                    Self::shot_arc_for_duration(distance, *duration, requested_arc, rules);
                let arc = arc_amplitude * progress * (1.0 - progress);
                (xy, (linear_z + arc).max(0.0))
            }
            BallTrajectoryKind::Shot {
                from_pos,
                hoop_pos,
                start_time,
                duration,
                peak_z,
                ..
            } => {
                let progress =
                    ((current_time - start_time) / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
                let xy = from_pos.lerp(*hoop_pos, progress);
                let linear_z = rules.chest_height_ft
                    + (rules.rim_height_ft - rules.chest_height_ft) * progress;
                let requested_arc = Self::shot_arc_amplitude(*peak_z, rules);
                let distance = (*hoop_pos - *from_pos).length();
                let arc_amplitude =
                    Self::shot_arc_for_duration(distance, *duration, requested_arc, rules);
                let arc = arc_amplitude * progress * (1.0 - progress);
                (xy, (linear_z + arc).max(0.0))
            }
            BallTrajectoryKind::LooseBall { pos, z, .. } => {
                (*pos, (*z).clamp(0.0, rules.ball_max_speed_ftps))
            }
            BallTrajectoryKind::RimRebound {
                from_pos,
                from_z,
                target_landing,
                start_time,
                duration,
                peak_z,
                ..
            } => {
                let progress =
                    ((current_time - start_time) / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
                // The explicit contact point keeps the trajectory continuous;
                // hoop_pos remains metadata for semantic consumers.
                let xy = from_pos.lerp(*target_landing, progress);
                let linear_z = *from_z + (rules.chest_height_ft - *from_z) * progress;
                let requested_arc = rules.ball_arc_multiplier
                    * (*peak_z - (*from_z).max(rules.rim_height_ft)).max(rules.rebound_min_arc_ft);
                let distance = (*target_landing - *from_pos).length();
                let arc_amplitude =
                    Self::shot_arc_for_duration(distance, *duration, requested_arc, rules);
                let arc = arc_amplitude * progress * (1.0 - progress);
                (xy, (linear_z + arc).max(0.0))
            }
            BallTrajectoryKind::Dead { pos, z, .. } => (*pos, *z),
        }
    }

    /// Computes rebound landing spot from the configured shot geometry policy.
    pub fn compute_rebound_landing(
        shot_origin: Vec2,
        hoop_pos: Vec2,
        rng: &mut impl Rng,
        rules: &GameRules,
    ) -> ReboundLandingSpot {
        let shot_vector = hoop_pos - shot_origin;
        let shot_dist = shot_vector.length();
        let shot_dir = shot_vector.normalize_or_zero();
        let (min_dist, max_dist) = if shot_dist > rules.league.three_point_distance_ft {
            (rules.rebound_long_min_ft, rules.rebound_long_max_ft)
        } else {
            (rules.rebound_short_min_ft, rules.rebound_short_max_ft)
        };
        let bounce_dist = rng.gen_range(min_dist..max_dist);
        let angle_offset =
            rng.gen_range(-rules.rebound_angle_range_radians..rules.rebound_angle_range_radians);
        let perp = Vec2::new(-shot_dir.y, shot_dir.x);
        let rebound_dir = (-shot_dir * 0.7 + perp * angle_offset).normalize_or_zero();
        let raw_landing = hoop_pos + rebound_dir * bounce_dist;
        let margin = rules.player_radius_ft.max(0.0);
        let landing_pos = rules.court.clamp_playable(raw_landing, margin);
        let flight_duration = rules.rebound_flight_base_seconds
            + (bounce_dist / rules.rebound_distance_scale_ft.max(f32::EPSILON))
                * rules.rebound_flight_distance_factor;
        ReboundLandingSpot {
            landing_pos,
            flight_duration,
            rebounder_id: None,
        }
    }

    pub fn compute_rebound_landing_default(
        shot_origin: Vec2,
        hoop_pos: Vec2,
        rng: &mut impl Rng,
    ) -> ReboundLandingSpot {
        Self::compute_rebound_landing(shot_origin, hoop_pos, rng, &GameRules::default())
    }
}
