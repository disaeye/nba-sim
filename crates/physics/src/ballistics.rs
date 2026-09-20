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
    /// Returns the arc coefficient `A` such that the sampled curve
    /// `z(p) = chest + (rim - chest)·p + A·p·(1-p)` actually reaches
    /// `peak_z` at its true maximum.
    ///
    /// 为什么不能直接线性缩放：`p·(1-p)` 的最大值在 `p=0.5`，但
    /// 叠加了线性项 `chest + (rim-chest)·p` 后，真实极值点
    /// `p* = (A + rim - chest) / (2A)`，位于 `p > 0.5`。此前直接用
    /// `multiplier × (peak_z - mid)` 作为 `A`，使实际采样峰值高于请求值
    /// （本轮实测：请求 35.0 ft，采样到 35.08 ft，违反 BALL_HEIGHT_BOUNDS）。
    ///
    /// 本函数用二分反解 `A`，保证 `max z(p) == peak_z`（在数值精度内），
    /// 从而让「请求峰值」成为真正的上界。
    fn shot_arc_amplitude(peak_z: f32, rules: &GameRules) -> f32 {
        let chest = rules.chest_height_ft;
        let rim = rules.rim_height_ft;
        let half = f32::from(2u8);
        let base = rim;
        // 峰值不可能低于线性项终点（否则无解，取最小弧）。
        if peak_z <= base || !peak_z.is_finite() {
            return f32::from(0u8);
        }
        // 在固定迭代次数内二分求解，避免运行时长依赖数据。
        let mut lo = f32::from(0u8);
        let mut hi = (peak_z - chest).abs().max(f32::from(1u8))
            * rules.ball_arc_multiplier.max(f32::from(1u8))
            + f32::from(1u8);
        for _ in 0..rules.shot_arc_solve_iterations {
            let mid = (lo + hi) * f32::from(2u8).recip();
            if mid <= f32::EPSILON {
                lo = mid;
                continue;
            }
            let p = ((mid + (rim - chest)) / (half * mid)).clamp(f32::from(0u8), f32::from(1u8));
            let z = chest + (rim - chest) * p + mid * p * (f32::from(1u8) - p);
            if z < peak_z {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (lo + hi) * f32::from(2u8).recip()
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
    ///
    /// 保留供需固定时长的场景/测试使用；**生产路径应用
    /// [`Self::solve_pass_landing`]**（用真实飞行时长求解）。
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

    /// 求解传球接球点与飞行时长（不动点）。
    ///
    /// ## 为什么不能用一个固定领传时长
    ///
    /// 旧实现用 `pass_lead_time_seconds = 0.65`（固定）估计接球人 T 秒后的位置，
    /// 但真实飞行时长 `T = pass_duration(|L − p|)` 随距离变化：
    ///
    /// ```text
    /// 距离    真实飞行    按 0.65s 领传    误差
    ///   8 ft   0.45s      0.65s           +0.20s（领过头）
    ///  39 ft   1.22s      0.65s           −0.57s（领不足）
    ///  50 ft   1.40s      0.65s           −0.75s（领不足）
    /// ```
    ///
    /// 短传与长传**双向都错**——这是用一个常数近似一个函数的必然结果。
    ///
    /// ## 不动点方程
    ///
    /// ```text
    /// T = pass_duration(|L − passer|)      # 飞多少时间
    /// L = receiver_pos + v̂ · brake_reach(v0, T)   # T 秒内能跑到哪
    /// ```
    ///
    /// `brake_reach` 必须与 `engine::receive_approach` **同源**（同一个制动模型），
    /// 否则两处对"能否到达"的判断不一致：
    ///
    /// ```text
    /// t_brake = v0 / a_max
    /// T <= t_brake:  v0·T − ½·a_max·T²
    /// 否则:          v0² / (2·a_max)      # 已停住
    /// ```
    ///
    /// ## 迭代与收敛
    ///
    /// 从"以当前位置估计飞行时长"开始，交替更新 T 与 L。实测 2–4 次迭代即
    /// 收敛到 `|ΔT| < 1e-4`（见单测）；上限 30 次作为安全阀。
    ///
    /// 返回 `(接球点, 飞行时长)`：两者必须一起冻结，否则又回到"三个位置并存"
    /// （gap.md §9.5 禁止）。
    pub fn solve_pass_landing(
        passer_pos: Vec2,
        receiver: &PlayerPhysicsState,
        rules: &GameRules,
    ) -> (Vec2, f32) {
        let v0 = receiver.vel_ft.length();
        let direct = (receiver.pos_ft - passer_pos).length();

        // 接球人静止：接球点即自身位置，时长由距离直接给出。
        if v0 <= f32::EPSILON {
            let t = rules.pass_duration(direct, false);
            return (
                rules
                    .court
                    .clamp_playable(receiver.pos_ft, rules.player_radius_ft),
                t,
            );
        }

        let dir = receiver.vel_ft / v0;
        let accel = rules.max_player_accel_ftps2.max(f32::EPSILON);
        let cap = rules.tactics.pass_lead_max_ft.max(0.0);
        let gain = rules.tactics.pass_lead_gain.clamp(0.0, 1.0);

        // 制动模型：T 秒内接球人可前进的距离（与 receive_approach 同源）。
        let brake_reach = |t: f32| -> f32 {
            let t_brake = v0 / accel;
            if t <= t_brake {
                v0 * t - 0.5 * accel * t * t
            } else {
                v0 * t_brake - 0.5 * accel * t_brake * t_brake
            }
        };

        let mut t = rules.pass_duration(direct, false);
        let mut landing = receiver.pos_ft;
        for _ in 0..30 {
            // 领传量：制动可达距离 × 增益，且不超过上限。
            let lead = (brake_reach(t) * gain).clamp(0.0, cap);
            landing = rules
                .court
                .clamp_playable(receiver.pos_ft + dir * lead, rules.player_radius_ft);
            let t_next = rules.pass_duration((landing - passer_pos).length(), false);
            if (t_next - t).abs() < 1e-4 {
                t = t_next;
                break;
            }
            t = t_next;
        }
        (landing, t)
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
                    let speed = carrier.vel_ft.length();
                    let forward = if speed > 0.5 {
                        carrier.vel_ft / speed
                    } else {
                        Vec2::X
                    };
                    let lateral = Vec2::new(-forward.y, forward.x);
                    let freq = if speed > 0.5 {
                        rules.ball_bounce_frequency_hz * 1.2
                    } else {
                        rules.ball_bounce_frequency_hz
                    };
                    let cycle = (current_time * freq * std::f32::consts::TAU).sin();
                    let side_offset = lateral * (cycle * rules.ball_holder_offset_ft * 0.45);
                    let forward_offset = forward * rules.ball_holder_offset_ft;
                    let bounce_progress =
                        ((current_time * freq * std::f32::consts::PI).sin()).abs();
                    let min_z = rules.ball_holder_height_ft * 0.70;
                    let max_z = rules.ball_holder_height_ft;
                    let bounce_z = min_z + bounce_progress * (max_z - min_z);
                    let requested = carrier.pos_ft + forward_offset + side_offset;
                    let offset = requested - carrier.pos_ft;
                    let bounded = if offset.length() > rules.invariant_holder_leash_ft {
                        offset.normalize() * rules.invariant_holder_leash_ft
                    } else {
                        offset
                    };
                    (carrier.pos_ft + bounded, bounce_z)
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
                target_pos,
                target_z,
                start_time,
                duration,
                ..
            } => {
                // Freeze both endpoints at transition creation. Sampling against the
                // moving carrier makes the trajectory non-deterministic and can make
                // the endpoint move faster than the configured ball envelope.
                let target = (*target_pos, *target_z);
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
            BallTrajectoryKind::InboundReady {
                baseline_pos,
                inbounder_id,
            } => {
                if let Some(inbounder) = players.get(inbounder_id) {
                    (inbounder.pos_ft, rules.chest_height_ft)
                } else {
                    (*baseline_pos, rules.chest_height_ft)
                }
            }
            BallTrajectoryKind::Drive {
                driver_id,
                move_kind,
                ..
            } => {
                if let Some(driver) = players.get(driver_id) {
                    let speed = driver.vel_ft.length();
                    let forward = if speed > 0.5 {
                        driver.vel_ft / speed
                    } else {
                        Vec2::X
                    };
                    let lateral = Vec2::new(-forward.y, forward.x);
                    let freq = rules.ball_bounce_frequency_hz
                        * (1.0 + (speed / rules.max_player_speed_ftps.max(f32::EPSILON)).min(0.6));
                    let cycle = (current_time * freq * std::f32::consts::TAU).sin();
                    let lateral_mult = match move_kind {
                        Some(nba_domain::action_window::DribbleMoveKind::Crossover) => 1.2,
                        Some(nba_domain::action_window::DribbleMoveKind::BehindTheBack) => 0.8,
                        Some(nba_domain::action_window::DribbleMoveKind::SpinMove) => 1.5,
                        _ => 0.6,
                    };
                    let side_offset =
                        lateral * (cycle * rules.ball_holder_offset_ft * lateral_mult);
                    let forward_offset = forward * rules.ball_holder_offset_ft;
                    let bounce_progress =
                        ((current_time * freq * std::f32::consts::PI).sin()).abs();
                    let min_z = rules.ball_holder_height_ft * 0.65;
                    let max_z = rules.ball_holder_height_ft;
                    let bounce_z = min_z + bounce_progress * (max_z - min_z);
                    let requested = driver.pos_ft + forward_offset + side_offset;
                    let offset = requested - driver.pos_ft;
                    let bounded = if offset.length() > rules.invariant_holder_leash_ft {
                        offset.normalize() * rules.invariant_holder_leash_ft
                    } else {
                        offset
                    };
                    (driver.pos_ft + bounded, bounce_z)
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
                start_time,
                duration,
                peak_z,
                ..
            } => {
                let progress =
                    ((current_time - start_time) / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
                let xy = from_pos.lerp(*to_pos, progress);
                let linear_z = *peak_z + (rules.chest_height_ft - *peak_z) * progress;
                let requested_arc =
                    rules.ball_arc_multiplier * (*peak_z - rules.chest_height_ft).max(0.0);
                let distance = (*to_pos - *from_pos).length();
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

#[cfg(test)]
mod landing_tests {
    use super::*;
    use nba_domain::GameRules;

    fn receiver(pos: Vec2, vel: Vec2) -> PlayerPhysicsState {
        PlayerPhysicsState {
            id: "T_1".to_string(),
            jersey: "1".to_string(),
            team: "home".to_string(),
            pos_ft: pos,
            vel_ft: vel,
            accel_ft: Vec2::ZERO,
            target_pos_ft: pos,
            target_speed_ftps: 0.0,
            max_speed_ftps: 20.0,
            max_accel_ftps2: 35.0,
            has_ball: false,
            on_court: true,
            action: "Run".to_string(),
            slot: "S".to_string(),
            morale: "Normal".to_string(),
            stamina: 100.0,
            max_stamina: 100.0,
            foul_count: 0,
            locomotion: crate::movement::LocomotionState::Idle,
            facing_dir: Vec2::X,
            turn_decel_timer: 0.0,
            is_locked_kinematics: false,
            out_of_bounds_placement: false,
            is_receiving_pass: false,
            is_driving_to_rim: false,
            boundary_cross_latched: false,
            attributes: nba_domain::PlayerAttributes::default(),
            tendencies: nba_domains_tendencies(),
        }
    }

    fn nba_domains_tendencies() -> nba_domain::PlayerTendencies {
        nba_domain::PlayerTendencies::default()
    }

    /// 不动点必须自洽：接球点所处距离对应的飞行时长 == 解出的飞行时长。
    #[test]
    fn solve_landing_is_a_fixed_point() {
        let rules = GameRules::default();
        for (pos, vel) in [
            (Vec2::new(50.0, 25.0), Vec2::new(-13.0, 0.1)),
            (Vec2::new(64.0, 40.0), Vec2::new(-8.0, 6.0)),
            (Vec2::new(30.0, 20.0), Vec2::new(6.0, -4.0)),
        ] {
            let passer = Vec2::new(97.0, 25.0);
            let r = receiver(pos, vel);
            let (landing, t) = BallisticsEngine::solve_pass_landing(passer, &r, &rules);
            let t_from_landing = rules.pass_duration((landing - passer).length(), false);
            assert!(
                (t_from_landing - t).abs() < 1e-3,
                "fixed point must hold: t={t} vs duration(|L-p|)={t_from_landing}"
            );
        }
    }

    /// 领传量不得超过接球人 T 秒内的制动可达距离（否则接球点不可达）。
    #[test]
    fn lead_never_exceeds_braking_reach() {
        let rules = GameRules::default();
        let v0 = 18.0f32;
        let r = receiver(Vec2::new(60.0, 25.0), Vec2::new(-v0, 0.0));
        let passer = Vec2::new(97.0, 25.0);
        let (landing, t) = BallisticsEngine::solve_pass_landing(passer, &r, &rules);
        let led = (landing - r.pos_ft).length();
        let accel = rules.max_player_accel_ftps2;
        let t_brake = v0 / accel;
        let reachable = if t <= t_brake {
            v0 * t - 0.5 * accel * t * t
        } else {
            v0 * t_brake - 0.5 * accel * t_brake * t_brake
        };
        assert!(
            led <= reachable + 1e-3,
            "lead {led:.3} must not exceed braking reach {reachable:.3}"
        );
        // 且必须为正（有提前量），否则接球人永远追不上球。
        assert!(led > 0.0, "a moving receiver must get a positive lead");
    }

    /// 静止接球人不得被领（接球点 = 自身位置）。
    #[test]
    fn stationary_receiver_gets_no_lead() {
        let rules = GameRules::default();
        let r = receiver(Vec2::new(60.0, 25.0), Vec2::ZERO);
        let passer = Vec2::new(97.0, 25.0);
        let (landing, t) = BallisticsEngine::solve_pass_landing(passer, &r, &rules);
        assert!((landing - r.pos_ft).length() < 1e-4);
        assert!((t - rules.pass_duration((r.pos_ft - passer).length(), false)).abs() < 1e-4);
    }

    /// 旧固定时长（0.65s）与真实飞行时长的误差必须被本函数消除。
    #[test]
    fn lead_time_tracks_distance_not_a_constant() {
        let rules = GameRules::default();
        let passer = Vec2::new(10.0, 25.0);
        let mut times = Vec::new();
        for d in [10.0f32, 25.0, 40.0, 55.0] {
            let r = receiver(Vec2::new(10.0 + d, 25.0), Vec2::new(-14.0, 0.0));
            let (_landing, t) = BallisticsEngine::solve_pass_landing(passer, &r, &rules);
            times.push(t);
        }
        // 距离越远，解出的飞行时长必须非递减（真实关系），而不是恒定 0.65。
        assert!(
            times.windows(2).all(|w| w[1] >= w[0] - 1e-4),
            "flight time must grow with distance: {times:?}"
        );
        assert!(
            times.last().unwrap() - times.first().unwrap() > 0.3,
            "flight time must vary materially with distance (not a constant): {times:?}"
        );
    }
}
