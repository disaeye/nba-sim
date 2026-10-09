// 本模块按球在飞行中的职责分为两段，边界是「发射端」与「接触端」：
//
// - 本文件：出手瞄准、投篮逐 tick 飞行与首次接触判定、传球落点求解、
//   球位置/速度采样，以及全部对外值类型；
// - `ballistics/contact.rs`：自由球-人接触与篮板反弹落点族。
//
// 二者共享同一批闭式抛体参数（`ProjectileArc` 与 `GameRules` 的时长/速度
// 上限），放在同一模块树下才不会各写一套而漂移。

mod contact;

use crate::movement::PlayerPhysicsState;
use glam::Vec2;
use nba_domain::GameRules;
use rand::Rng;
use std::collections::HashMap;

// 球的弹道/归属状态类型 = 领域层 BallState（M2 收敛：归属语义
// 不再住在 physics，physics 只按闭式参数采样位置）。
pub use nba_domain::BallState as BallTrajectoryKind;

pub struct ReboundLandingSpot {
    /// 筐环上的接触点（触筐反弹的起点）；打板路径为板面上的触点。
    pub contact_pos: Vec2,
    /// 接触点高度：近筐沿路径 = 筐高，打板路径 = 板面触点高度。
    pub contact_z: f32,
    pub landing_pos: Vec2,
    pub flight_duration: f32,
    /// 反弹抛体的自然弧顶（仅作载荷元数据，采样由 ProjectileArc 产生）。
    pub peak_z: f32,
    pub rebounder_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShotArrivalOutcome {
    Made,
    RimContact {
        ball_position: Vec2,
        contact_position: Vec2,
    },
    Miss,
}

/// A collision or terminal crossing found while advancing one shot interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShotFlightStep {
    InFlight,
    Made {
        time: f32,
        ball_position: glam::Vec3,
    },
    Contact(ShotFlightContact),
    Miss {
        time: f32,
        ball_position: glam::Vec3,
        incoming_velocity: glam::Vec3,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShotContactSurface {
    Rim,
    Backboard,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShotFlightContact {
    pub surface: ShotContactSurface,
    pub time: f32,
    pub ball_position: glam::Vec3,
    pub contact_position: glam::Vec3,
    pub incoming_velocity: glam::Vec3,
}

// 重力抛体纯数学住在 domain（projectile.rs），physics 直接复用同一实现：
// `GameRules::pass_duration` 与 `BallisticsEngine::shot_duration` 必须同源，
// 两处各写一份必然漂移。
pub use nba_domain::projectile::ProjectileArc;

pub struct BallisticsEngine;
impl BallisticsEngine {
    /// 用命中概率输入生成投篮释放方向，命中结果由到筐几何决定。
    pub fn sample_shot_aim(
        shot_origin: Vec2,
        hoop_pos: Vec2,
        make_probability: f32,
        rng: &mut impl Rng,
        rules: &GameRules,
    ) -> Vec2 {
        assert!(
            make_probability.is_finite()
                && !make_probability.is_sign_negative()
                && make_probability <= f32::from(1u8),
            "shot make probability must be finite and within [0, 1]"
        );
        let probability = make_probability;
        if probability >= f32::from(1u8) {
            return hoop_pos;
        }
        let approach = (hoop_pos - shot_origin).normalize_or_zero();
        let forward = if approach.length_squared() > f32::EPSILON {
            approach
        } else {
            Vec2::X
        };
        let lateral = Vec2::new(-forward.y, forward.x);
        let clear_radius = (rules.rim_radius_ft - rules.ball_radius_ft).max(f32::EPSILON);
        let aim_error = if probability <= f32::EPSILON {
            clear_radius + rules.ball_radius_ft + f32::from(1u8)
        } else {
            let two = f32::from(2u8);
            let one = f32::from(1u8);
            let denominator = (-two * (one - probability).ln()).sqrt();
            let sigma = clear_radius / denominator.max(f32::EPSILON);
            let radial_sample = rng.gen::<f32>().clamp(f32::EPSILON, one - f32::EPSILON);
            let radius = sigma * (-two * (one - radial_sample).ln()).sqrt();
            let sign = if rng.gen::<bool>() { one } else { -one };
            radius * sign
        };
        hoop_pos + lateral * aim_error
    }

    /// 按罚球概率输入或指定结果生成罚球瞄准点。
    pub fn sample_free_throw_aim(
        from_pos: Vec2,
        hoop_pos: Vec2,
        make_probability: f32,
        forced_result: Option<bool>,
        rng: &mut impl Rng,
        rules: &GameRules,
    ) -> Vec2 {
        assert!(
            make_probability.is_finite()
                && !make_probability.is_sign_negative()
                && make_probability <= f32::from(1u8),
            "free-throw probability must be finite and within [0, 1]"
        );
        match forced_result {
            Some(true) => hoop_pos,
            Some(false) => {
                let approach = (hoop_pos - from_pos).normalize_or_zero();
                let forward = if approach.length_squared() > f32::EPSILON {
                    approach
                } else {
                    Vec2::X
                };
                hoop_pos + Vec2::new(-forward.y, forward.x) * rules.rim_radius_ft
            }
            None => Self::sample_shot_aim(from_pos, hoop_pos, make_probability, rng, rules),
        }
    }

    /// 以球心通过篮筐平面的位置判定入筐、筐环接触或偏出。
    pub fn classify_shot_arrival(
        ball_position: Vec2,
        hoop_pos: Vec2,
        rules: &GameRules,
    ) -> ShotArrivalOutcome {
        let radial = ball_position - hoop_pos;
        let distance = radial.length();
        let clear_radius = (rules.rim_radius_ft - rules.ball_radius_ft).max(0.0);
        if distance < clear_radius {
            return ShotArrivalOutcome::Made;
        }
        if distance <= rules.rim_radius_ft + rules.ball_radius_ft {
            let direction = radial.normalize_or_zero();
            let direction = if direction.length_squared() > f32::EPSILON {
                direction
            } else {
                Vec2::X
            };
            return ShotArrivalOutcome::RimContact {
                ball_position,
                contact_position: hoop_pos + direction * rules.rim_radius_ft,
            };
        }
        ShotArrivalOutcome::Miss
    }

    /// Advances a released shot through one interval and returns its first physical result.
    pub fn shot_flight_step(
        state: &BallTrajectoryKind,
        from_time: f32,
        to_time: f32,
        rules: &GameRules,
    ) -> ShotFlightStep {
        assert!(
            from_time.is_finite() && to_time.is_finite() && from_time <= to_time,
            "shot collision interval must be finite and ordered"
        );
        let BallTrajectoryKind::Shot {
            from_pos,
            hoop_pos,
            aim_pos,
            start_time,
            duration,
            ..
        } = state
        else {
            panic!("shot collision detection requires a Shot ball state");
        };
        assert!(
            start_time.is_finite() && duration.is_finite() && *duration > f32::EPSILON,
            "shot collision trajectory must have a finite positive duration"
        );
        let interval_start = from_time.max(*start_time);
        let interval_end = to_time;
        if interval_end < interval_start {
            return ShotFlightStep::InFlight;
        }

        let board_contact = Self::shot_backboard_contact(
            *from_pos,
            *hoop_pos,
            *aim_pos,
            *start_time,
            *duration,
            rules,
        );
        let rim_contact = Self::shot_rim_contact_in_interval(
            *from_pos,
            *hoop_pos,
            *aim_pos,
            *start_time,
            *duration,
            interval_start,
            interval_end,
            rules,
        );
        let first_contact = [
            rim_contact,
            board_contact
                .filter(|contact| contact.time >= interval_start && contact.time <= interval_end),
        ]
        .into_iter()
        .flatten()
        .min_by(|left, right| left.time.total_cmp(&right.time));

        let hoop_crossing_time = *start_time + *duration;
        if hoop_crossing_time >= interval_start && hoop_crossing_time <= to_time {
            let crossing_position = Self::shot_position_at(
                *from_pos,
                *hoop_pos,
                *aim_pos,
                *start_time,
                *duration,
                hoop_crossing_time,
                rules,
            );
            if let Some(contact) = first_contact {
                if contact.time <= hoop_crossing_time {
                    return ShotFlightStep::Contact(contact);
                }
            }
            if matches!(
                Self::classify_shot_arrival(crossing_position.truncate(), *hoop_pos, rules),
                ShotArrivalOutcome::Made
            ) {
                return ShotFlightStep::Made {
                    time: hoop_crossing_time,
                    ball_position: crossing_position,
                };
            }
            if let Some(contact) = board_contact {
                if contact.time > hoop_crossing_time {
                    if contact.time <= interval_end {
                        return ShotFlightStep::Contact(contact);
                    }
                    return ShotFlightStep::InFlight;
                }
            }
            if rim_contact.is_none() {
                return ShotFlightStep::Miss {
                    time: hoop_crossing_time,
                    ball_position: crossing_position,
                    incoming_velocity: Self::shot_velocity_at(
                        *from_pos,
                        *hoop_pos,
                        *aim_pos,
                        *start_time,
                        *duration,
                        hoop_crossing_time,
                        rules,
                    ),
                };
            }
        }

        if hoop_crossing_time < interval_start {
            let arc = ProjectileArc::solve(
                rules.chest_height_ft,
                rules.rim_height_ft,
                *duration,
                rules.ball_gravity_ftps2,
            );
            let descending_velocity = arc.vz0 - rules.ball_gravity_ftps2 * *duration;
            let gravity = rules.ball_gravity_ftps2.max(f32::EPSILON);
            let exit_time = hoop_crossing_time
                + (descending_velocity
                    + (descending_velocity * descending_velocity
                        + 2.0 * gravity * rules.ball_radius_ft)
                        .sqrt())
                    / gravity;
            if let Some(contact) = board_contact {
                if contact.time >= interval_start && contact.time <= interval_end {
                    return ShotFlightStep::Contact(contact);
                }
            }
            if interval_end >= exit_time {
                return ShotFlightStep::Miss {
                    time: exit_time,
                    ball_position: Self::shot_position_at(
                        *from_pos,
                        *hoop_pos,
                        *aim_pos,
                        *start_time,
                        *duration,
                        exit_time,
                        rules,
                    ),
                    incoming_velocity: Self::shot_velocity_at(
                        *from_pos,
                        *hoop_pos,
                        *aim_pos,
                        *start_time,
                        *duration,
                        exit_time,
                        rules,
                    ),
                };
            }
        }
        if let Some(contact) = first_contact {
            return ShotFlightStep::Contact(contact);
        }
        ShotFlightStep::InFlight
    }

    fn shot_position_at(
        from_pos: Vec2,
        hoop_pos: Vec2,
        aim_pos: Vec2,
        start_time: f32,
        duration: f32,
        time: f32,
        rules: &GameRules,
    ) -> glam::Vec3 {
        let duration = duration.max(f32::EPSILON);
        let elapsed = (time - start_time).max(0.0);
        let direction = (hoop_pos - from_pos).normalize_or_zero();
        let lateral = Vec2::new(-direction.y, direction.x);
        let lateral_offset = (aim_pos - hoop_pos).dot(lateral);
        let xy = from_pos.lerp(hoop_pos, elapsed / duration)
            + lateral * (lateral_offset * elapsed / duration);
        let arc = ProjectileArc::solve(
            rules.chest_height_ft,
            rules.rim_height_ft,
            duration,
            rules.ball_gravity_ftps2,
        );
        glam::Vec3::new(
            xy.x,
            xy.y,
            arc.z_at(elapsed, rules.chest_height_ft, rules.ball_gravity_ftps2)
                .max(0.0),
        )
    }

    fn shot_velocity_at(
        from_pos: Vec2,
        hoop_pos: Vec2,
        aim_pos: Vec2,
        start_time: f32,
        duration: f32,
        time: f32,
        rules: &GameRules,
    ) -> glam::Vec3 {
        let duration = duration.max(f32::EPSILON);
        let direction = (hoop_pos - from_pos).normalize_or_zero();
        let lateral = Vec2::new(-direction.y, direction.x);
        let lateral_offset = (aim_pos - hoop_pos).dot(lateral);
        let horizontal = (hoop_pos - from_pos) / duration + lateral * (lateral_offset / duration);
        let arc = ProjectileArc::solve(
            rules.chest_height_ft,
            rules.rim_height_ft,
            duration,
            rules.ball_gravity_ftps2,
        );
        glam::Vec3::new(
            horizontal.x,
            horizontal.y,
            arc.vz0 - rules.ball_gravity_ftps2 * (time - start_time).max(0.0),
        )
    }

    fn shot_rim_contact_in_interval(
        from_pos: Vec2,
        hoop_pos: Vec2,
        aim_pos: Vec2,
        start_time: f32,
        duration: f32,
        interval_start: f32,
        interval_end: f32,
        rules: &GameRules,
    ) -> Option<ShotFlightContact> {
        if interval_end < interval_start {
            return None;
        }
        let start = Self::shot_position_at(
            from_pos,
            hoop_pos,
            aim_pos,
            start_time,
            duration,
            interval_start,
            rules,
        );
        let end = Self::shot_position_at(
            from_pos,
            hoop_pos,
            aim_pos,
            start_time,
            duration,
            interval_end,
            rules,
        );
        let path_length = (end - start).length();
        let sample_spacing = (rules.ball_radius_ft / 4.0).max(f32::EPSILON);
        let samples = (path_length / sample_spacing).ceil().max(1.0) as u32;
        let distance_to_rim = |time: f32| {
            let position = Self::shot_position_at(
                from_pos, hoop_pos, aim_pos, start_time, duration, time, rules,
            );
            let radial = (position.truncate() - hoop_pos).length() - rules.rim_radius_ft;
            radial.hypot(position.z - rules.rim_height_ft) - rules.ball_radius_ft
        };
        let mut previous_time = interval_start;
        let mut previous_distance = distance_to_rim(previous_time);
        if previous_distance <= 0.0 {
            return Some(Self::rim_contact_at(
                previous_time,
                from_pos,
                hoop_pos,
                aim_pos,
                start_time,
                duration,
                rules,
            ));
        }
        for sample in 1..=samples {
            let sample_time =
                interval_start + (interval_end - interval_start) * sample as f32 / samples as f32;
            let sample_distance = distance_to_rim(sample_time);
            if sample_distance <= 0.0 {
                let mut low = previous_time;
                let mut high = sample_time;
                for _ in 0..20 {
                    let middle = (low + high) * 0.5;
                    if distance_to_rim(middle) <= 0.0 {
                        high = middle;
                    } else {
                        low = middle;
                    }
                }
                return Some(Self::rim_contact_at(
                    high, from_pos, hoop_pos, aim_pos, start_time, duration, rules,
                ));
            }
            previous_time = sample_time;
            previous_distance = sample_distance;
        }
        let _ = previous_distance;
        None
    }

    fn rim_contact_at(
        time: f32,
        from_pos: Vec2,
        hoop_pos: Vec2,
        aim_pos: Vec2,
        start_time: f32,
        duration: f32,
        rules: &GameRules,
    ) -> ShotFlightContact {
        let ball_position = Self::shot_position_at(
            from_pos, hoop_pos, aim_pos, start_time, duration, time, rules,
        );
        let radial = (ball_position.truncate() - hoop_pos).normalize_or_zero();
        let radial = if radial.length_squared() > f32::EPSILON {
            radial
        } else {
            Vec2::X
        };
        let contact_position = glam::Vec3::new(
            hoop_pos.x + radial.x * rules.rim_radius_ft,
            hoop_pos.y + radial.y * rules.rim_radius_ft,
            rules.rim_height_ft,
        );
        let velocity = Self::shot_velocity_at(
            from_pos, hoop_pos, aim_pos, start_time, duration, time, rules,
        );
        ShotFlightContact {
            surface: ShotContactSurface::Rim,
            time,
            ball_position,
            contact_position,
            incoming_velocity: velocity,
        }
    }

    fn shot_backboard_contact(
        from_pos: Vec2,
        hoop_pos: Vec2,
        aim_pos: Vec2,
        start_time: f32,
        duration: f32,
        rules: &GameRules,
    ) -> Option<ShotFlightContact> {
        let direction = (hoop_pos - from_pos).normalize_or_zero();
        let lateral = Vec2::new(-direction.y, direction.x);
        let lateral_offset = (aim_pos - hoop_pos).dot(lateral);
        let displacement = hoop_pos - from_pos + lateral * lateral_offset;
        if displacement.x.abs() <= f32::EPSILON {
            return None;
        }
        let board_x = if hoop_pos.x <= rules.court.width_ft / 2.0 {
            rules.backboard_offset_from_baseline_ft
        } else {
            rules.court.width_ft - rules.backboard_offset_from_baseline_ft
        };
        let beyond_hoop = (board_x - hoop_pos.x) * displacement.x > 0.0
            && (from_pos.x - hoop_pos.x) * displacement.x < 0.0;
        if !beyond_hoop {
            return None;
        }
        let center_x = board_x - displacement.x.signum() * rules.ball_radius_ft;
        let progress = (center_x - from_pos.x) / displacement.x;
        if progress <= 0.0 {
            return None;
        }
        let time = start_time + duration * progress;
        let ball_position = Self::shot_position_at(
            from_pos, hoop_pos, aim_pos, start_time, duration, time, rules,
        );
        if ball_position.z <= 0.0
            || (ball_position.y - rules.court.hoop_y_ft).abs()
                > rules.backboard_width_ft / 2.0 + rules.ball_radius_ft
            || ball_position.z < rules.backboard_bottom_height_ft - rules.ball_radius_ft
            || ball_position.z > rules.backboard_top_height_ft + rules.ball_radius_ft
        {
            return None;
        }
        let contact_position = glam::Vec3::new(
            board_x,
            ball_position.y.clamp(
                rules.court.hoop_y_ft - rules.backboard_width_ft / 2.0,
                rules.court.hoop_y_ft + rules.backboard_width_ft / 2.0,
            ),
            ball_position.z.clamp(
                rules.backboard_bottom_height_ft,
                rules.backboard_top_height_ft,
            ),
        );
        let velocity = Self::shot_velocity_at(
            from_pos, hoop_pos, aim_pos, start_time, duration, time, rules,
        );
        Some(ShotFlightContact {
            surface: ShotContactSurface::Backboard,
            time,
            ball_position,
            contact_position,
            incoming_velocity: velocity,
        })
    }

    /// Computes a shot flight duration from the projectile physics.
    ///
    /// 第一步飞行抛体化：时长由「请求弧顶 + 两端高度」闭式解出
    /// （升段 + 降段），再夹在动作窗口区间内。出手速度由
    /// `hypot(水平速度, vz0)` 交叉校验：超过球速包络时削峰重解，
    /// 保证任何采样点的瞬时速度不超过 `ball_max_speed_ftps`。
    ///
    /// 实测量级（默认规则）：25 ft 三分、弧顶 15 ft → T ≈ 1.19 s，
    /// 出手速度 ≈ 37 ft/s（真实 NBA 三分出手 36-40 ft/s）。
    pub fn shot_duration(distance_ft: f32, peak_z: f32, rules: &GameRules) -> f32 {
        assert!(
            distance_ft.is_finite() && peak_z.is_finite() && rules.ball_gravity_ftps2.is_finite(),
            "shot trajectory inputs must be finite"
        );
        assert!(
            distance_ft >= 0.0
                && peak_z >= rules.chest_height_ft.max(rules.rim_height_ft)
                && rules.ball_gravity_ftps2 > 0.0
                && rules.ball_max_speed_ftps > 0.0
                && rules.min_shot_duration_seconds > 0.0
                && rules.max_shot_duration_seconds >= rules.min_shot_duration_seconds,
            "shot trajectory rules are invalid"
        );
        let distance = distance_ft;
        let g = rules.ball_gravity_ftps2;
        let chest = rules.chest_height_ft;
        let rim = rules.rim_height_ft;
        let higher_end = chest.max(rim);
        // 请求弧顶必须高于两端，否则用两端较高者加最小裕量（近平抛）。
        let peak = peak_z.max(higher_end + f32::EPSILON);
        let t_projectile = ProjectileArc::time_for_peak(chest, rim, peak, g);
        // 速度包络下限（与 pass_duration 同源）：水平速度不得超过球速包络，
        // 唯一手段是延长时长（实测回归：远距离出手在 clamp 上限内也可能超速）。
        let t_envelope = distance / rules.ball_max_speed_ftps.max(f32::EPSILON);
        let t_required = t_projectile.max(t_envelope);
        let minimum_duration = rules.min_shot_duration_seconds;
        let maximum_duration = rules.max_shot_duration_seconds.max(minimum_duration);
        t_required.clamp(minimum_duration, maximum_duration)
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

    /// Samples the instantaneous velocity of the held or dribbled ball.
    ///
    /// The carrier velocity and the derivative of the configured dribble offset
    /// are combined so contact adjudication can use the ball's current motion.
    pub fn sample_ball_velocity(
        state: &BallTrajectoryKind,
        current_time: f32,
        players: &HashMap<String, PlayerPhysicsState>,
        rules: &GameRules,
    ) -> glam::Vec3 {
        if let BallTrajectoryKind::FreeThrowSetup {
            from_pos,
            from_z,
            to_pos,
            to_z,
            start_time,
            duration,
            ..
        } = state
        {
            if current_time < *start_time || current_time >= *start_time + *duration {
                return glam::Vec3::ZERO;
            }
            return Self::sample_linear_setup_velocity(
                *from_pos, *from_z, *to_pos, *to_z, *duration,
            );
        }
        if let BallTrajectoryKind::FreeThrow {
            from_pos,
            hoop_pos,
            aim_pos,
            start_time,
            duration,
            ..
        } = state
        {
            let flight_time = duration.max(f32::EPSILON);
            let elapsed = (current_time - start_time).clamp(0.0, flight_time);
            let arc = ProjectileArc::solve(
                rules.chest_height_ft,
                rules.rim_height_ft,
                flight_time,
                rules.ball_gravity_ftps2,
            );
            let direction = (*hoop_pos - *from_pos).normalize_or_zero();
            let lateral = Vec2::new(-direction.y, direction.x);
            let lateral_offset = (*aim_pos - *hoop_pos).dot(lateral);
            let horizontal_velocity = direction * ((*hoop_pos - *from_pos).length() / flight_time)
                + lateral * (lateral_offset / flight_time);
            return glam::Vec3::new(
                horizontal_velocity.x,
                horizontal_velocity.y,
                arc.vz0 - rules.ball_gravity_ftps2 * elapsed,
            );
        }
        if let BallTrajectoryKind::Shot {
            aim_pos,
            hoop_pos,
            from_pos,
            start_time,
            duration,
            ..
        } = state
        {
            let flight_time = duration.max(f32::EPSILON);
            let elapsed = (current_time - start_time).max(0.0);
            let arc = ProjectileArc::solve(
                rules.chest_height_ft,
                rules.rim_height_ft,
                flight_time,
                rules.ball_gravity_ftps2,
            );
            let direction = (*hoop_pos - *from_pos).normalize_or_zero();
            let lateral = Vec2::new(-direction.y, direction.x);
            let lateral_offset = (*aim_pos - *hoop_pos).dot(lateral);
            let lateral_speed = lateral_offset / flight_time;
            let horizontal_velocity = direction * ((*hoop_pos - *from_pos).length() / flight_time)
                + lateral * lateral_speed;
            return glam::Vec3::new(
                horizontal_velocity.x,
                horizontal_velocity.y,
                arc.vz0 - rules.ball_gravity_ftps2 * elapsed,
            );
        }
        let (carrier_id, lateral_multiplier, height_multiplier) = match state {
            BallTrajectoryKind::Held { carrier_id } => (carrier_id, 0.45, 1.0),
            BallTrajectoryKind::Drive {
                driver_id,
                move_kind,
                ..
            } => {
                let lateral_multiplier = match move_kind {
                    Some(nba_domain::action_window::DribbleMoveKind::Crossover) => 1.2,
                    Some(nba_domain::action_window::DribbleMoveKind::BehindTheBack) => 0.8,
                    Some(nba_domain::action_window::DribbleMoveKind::SpinMove) => 1.5,
                    _ => 0.6,
                };
                (driver_id, lateral_multiplier, 0.95)
            }
            _ => return glam::Vec3::ZERO,
        };
        let Some(carrier) = players.get(carrier_id) else {
            return glam::Vec3::ZERO;
        };
        let speed = carrier.vel_ft.length();
        let forward = if speed > 0.5 {
            carrier.vel_ft / speed
        } else {
            Vec2::X
        };
        let lateral = Vec2::new(-forward.y, forward.x);
        let frequency = rules.ball_bounce_frequency_hz
            * (1.0 + (speed / rules.max_player_speed_ftps.max(f32::EPSILON)).min(0.6));
        let phase = current_time * frequency * std::f32::consts::TAU;
        let lateral_velocity = lateral
            * (rules.ball_holder_offset_ft
                * lateral_multiplier
                * phase.cos()
                * frequency
                * std::f32::consts::TAU);
        let vertical_phase = current_time * frequency * std::f32::consts::PI;
        let vertical_velocity = vertical_phase.cos()
            * (rules.ball_holder_height_ft
                * (1.0 - 0.65 * height_multiplier)
                * frequency
                * std::f32::consts::PI);
        let horizontal_velocity = carrier.vel_ft + lateral_velocity;
        glam::Vec3::new(
            horizontal_velocity.x,
            horizontal_velocity.y,
            vertical_velocity,
        )
    }

    fn sample_linear_setup_velocity(
        from_pos: Vec2,
        from_z: f32,
        to_pos: Vec2,
        to_z: f32,
        duration: f32,
    ) -> glam::Vec3 {
        let duration = duration.max(f32::EPSILON);
        let horizontal = (to_pos - from_pos) / duration;
        glam::Vec3::new(horizontal.x, horizontal.y, (to_z - from_z) / duration)
    }

    fn sample_linear_setup_position(
        from_pos: Vec2,
        from_z: f32,
        to_pos: Vec2,
        to_z: f32,
        start_time: f32,
        duration: f32,
        current_time: f32,
    ) -> (Vec2, f32) {
        let progress = ((current_time - start_time) / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
        (
            from_pos.lerp(to_pos, progress),
            from_z + (to_z - from_z) * progress,
        )
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
                        carrier.facing_dir
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
                let tau = if *duration > f32::EPSILON {
                    ((current_time - start_time) / duration).clamp(f32::from(0u8), f32::from(1u8))
                } else {
                    f32::from(1u8)
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
                let progress = ((current_time - start_time) / duration.max(f32::EPSILON))
                    .clamp(f32::from(0u8), f32::from(1u8));
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
                        driver.facing_dir
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
                from_z,
                to_pos,
                start_time,
                duration,
                ..
            } => {
                let t_flight = duration.max(f32::EPSILON);
                let elapsed = (current_time - start_time).clamp(f32::from(0u8), t_flight);
                let progress = elapsed / t_flight;
                let arc = ProjectileArc::solve(
                    *from_z,
                    rules.chest_height_ft,
                    t_flight,
                    rules.ball_gravity_ftps2,
                );
                let xy = from_pos.lerp(*to_pos, progress);
                let z = arc.z_at(elapsed, *from_z, rules.ball_gravity_ftps2);
                (xy, z.max(f32::from(0u8)))
            }
            BallTrajectoryKind::Shot {
                from_pos,
                aim_pos,
                hoop_pos,
                start_time,
                duration,
                ..
            } => {
                assert!(
                    current_time.is_finite()
                        && start_time.is_finite()
                        && duration.is_finite()
                        && *duration > f32::EPSILON,
                    "shot trajectory time parameters must be finite and positive"
                );
                let t_flight = *duration;
                let elapsed = (current_time - start_time).max(f32::from(0u8));
                let progress = elapsed / t_flight;
                let offset = *aim_pos - *hoop_pos;
                let direction = (*hoop_pos - *from_pos).normalize_or_zero();
                let lateral = Vec2::new(-direction.y, direction.x);
                let lateral_offset = offset.dot(lateral);
                let xy = from_pos.lerp(*hoop_pos, progress) + lateral * lateral_offset * progress;
                let arc = ProjectileArc::solve(
                    rules.chest_height_ft,
                    rules.rim_height_ft,
                    t_flight,
                    rules.ball_gravity_ftps2,
                );
                let z = arc.z_at(elapsed, rules.chest_height_ft, rules.ball_gravity_ftps2);
                (xy, z.max(f32::from(0u8)))
            }
            BallTrajectoryKind::LooseBall { pos, z, .. } => {
                (*pos, (*z).clamp(f32::from(0u8), rules.ball_max_speed_ftps))
            }
            BallTrajectoryKind::FreeThrow {
                from_pos,
                hoop_pos,
                aim_pos,
                start_time,
                duration,
                ..
            } => {
                assert!(
                    duration.is_finite() && *duration > f32::EPSILON,
                    "free-throw trajectory duration must be finite and positive"
                );
                let elapsed = (current_time - start_time).clamp(f32::from(0u8), *duration);
                let progress = elapsed / *duration;
                let direction = (*hoop_pos - *from_pos).normalize_or_zero();
                let lateral = Vec2::new(-direction.y, direction.x);
                let lateral_offset = (*aim_pos - *hoop_pos).dot(lateral);
                let xy = from_pos.lerp(*hoop_pos, progress) + lateral * lateral_offset * progress;
                let arc = ProjectileArc::solve(
                    rules.chest_height_ft,
                    rules.rim_height_ft,
                    *duration,
                    rules.ball_gravity_ftps2,
                );
                let z = arc.z_at(elapsed, rules.chest_height_ft, rules.ball_gravity_ftps2);
                (xy, z.max(f32::from(0u8)))
            }
            BallTrajectoryKind::RimRebound {
                from_pos,
                from_z,
                target_landing,
                start_time,
                duration,
                ..
            } => {
                let t_flight = duration.max(f32::EPSILON);
                let elapsed = (current_time - start_time).clamp(f32::from(0u8), t_flight);
                let progress = elapsed / t_flight;
                // The explicit contact point keeps the trajectory continuous;
                // hoop_pos remains metadata for semantic consumers.
                let xy = from_pos.lerp(*target_landing, progress);
                // 重力抛体：z(0)=触筐高度，z(T)=地面 0（球的触地点）。
                // 触点反弹初速推导在第二步（触筐物理）；本步先服从重力。
                let landing_z = f32::from(0u8);
                let arc =
                    ProjectileArc::solve(*from_z, landing_z, t_flight, rules.ball_gravity_ftps2);
                let z = arc.z_at(elapsed, *from_z, rules.ball_gravity_ftps2);
                (xy, z.max(f32::from(0u8)))
            }
            BallTrajectoryKind::Dead { pos, z, .. } => (*pos, *z),
            BallTrajectoryKind::FreeThrowSetup {
                from_pos,
                from_z,
                to_pos,
                to_z,
                start_time,
                duration,
                ..
            } => Self::sample_linear_setup_position(
                *from_pos,
                *from_z,
                *to_pos,
                *to_z,
                *start_time,
                *duration,
                current_time,
            ),
        }
    }
}

#[cfg(test)]
#[path = "ballistics/landing_tests.rs"]
mod landing_tests;
