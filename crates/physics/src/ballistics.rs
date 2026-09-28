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

// 重力抛体纯数学住在 domain（projectile.rs），physics 直接复用同一实现：
// `GameRules::pass_duration` 与 `BallisticsEngine::shot_duration` 必须同源，
// 两处各写一份必然漂移。
pub use nba_domain::projectile::ProjectileArc;

pub struct BallisticsEngine;
impl BallisticsEngine {
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
        let distance = distance_ft.max(0.0);
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
        t_projectile.max(t_envelope).clamp(
            rules.min_shot_duration_seconds,
            rules.max_shot_duration_seconds,
        )
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
                ..
            } => {
                let t_flight = duration.max(f32::EPSILON);
                let elapsed = (current_time - start_time).clamp(0.0, t_flight);
                let progress = elapsed / t_flight;
                let xy = from_pos.lerp(*to_pos, progress);
                // 重力抛体：z(0)=出手胸口高，z(T)=接球胸口高，vz0 闭式反解。
                // 弧顶不再由载荷 peak_z 直接采样，而由抛物线自然产生；
                // peak_z 只在时长推导侧参与（solve_pass_landing 链路）。
                let arc = ProjectileArc::solve(
                    rules.chest_height_ft,
                    rules.chest_height_ft,
                    t_flight,
                    rules.ball_gravity_ftps2,
                );
                let z = arc.z_at(elapsed, rules.chest_height_ft, rules.ball_gravity_ftps2);
                (xy, z.max(0.0))
            }
            BallTrajectoryKind::Shot {
                from_pos,
                hoop_pos,
                start_time,
                duration,
                ..
            } => {
                let t_flight = duration.max(f32::EPSILON);
                let elapsed = (current_time - start_time).clamp(0.0, t_flight);
                let progress = elapsed / t_flight;
                let xy = from_pos.lerp(*hoop_pos, progress);
                // 重力抛体：z(0)=出手胸口高，z(T)=筐高；vz0 闭式反解。
                // 采样弧顶由抛物线自然产生，服从 g。
                let arc = ProjectileArc::solve(
                    rules.chest_height_ft,
                    rules.rim_height_ft,
                    t_flight,
                    rules.ball_gravity_ftps2,
                );
                let z = arc.z_at(elapsed, rules.chest_height_ft, rules.ball_gravity_ftps2);
                (xy, z.max(0.0))
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
                ..
            } => {
                let t_flight = duration.max(f32::EPSILON);
                let elapsed = (current_time - start_time).clamp(0.0, t_flight);
                let progress = elapsed / t_flight;
                // The explicit contact point keeps the trajectory continuous;
                // hoop_pos remains metadata for semantic consumers.
                let xy = from_pos.lerp(*target_landing, progress);
                // 重力抛体：z(0)=触筐高度，z(T)=地面 0（球的触地点）。
                // 触点反弹初速推导在第二步（触筐物理）；本步先服从重力。
                let landing_z = 0.0f32;
                let arc =
                    ProjectileArc::solve(*from_z, landing_z, t_flight, rules.ball_gravity_ftps2);
                let z = arc.z_at(elapsed, *from_z, rules.ball_gravity_ftps2);
                (xy, z.max(0.0))
            }
            BallTrajectoryKind::Dead { pos, z, .. } => (*pos, *z),
        }
    }

    /// 自由球-人接触检测（ADR-017 第三步）：自由球（篮板飞行、地板球）
    /// 对在场球员是可触碰实体。身体半径和篮球半径决定水平接触范围，
    /// 球高还必须低于该球员的摸高。
    ///
    /// 返回最近的命中者（距离相同时按 id 排序），保证候选顺序不会影响结果。
    /// 身高与弹跳由调用方以 `(id, height_cm, vertical, pos, on_court)` 传入。
    pub fn free_ball_player_contact(
        ball_pos: Vec2,
        ball_z: f32,
        candidates: &[(String, u16, f32, Vec2, bool)],
        rules: &GameRules,
    ) -> Option<(String, Vec2)> {
        let mut best: Option<(String, Vec2, f32)> = None;
        for candidate in candidates {
            if !Self::free_ball_player_contact_active(ball_pos, ball_z, candidate, rules) {
                continue;
            }
            let (id, _, _, pos, _) = candidate;
            let dist = (*pos - ball_pos).length();
            let better = match &best {
                Some((best_id, _, best_dist)) => {
                    dist < *best_dist || (dist == *best_dist && id < best_id)
                }
                None => true,
            };
            if better {
                best = Some((id.clone(), *pos, dist));
            }
        }
        best.map(|(id, pos, _)| (id, pos))
    }

    /// 判断自由球是否仍在指定球员的身体接触区内。
    ///
    /// 引擎用这个结果区分“持续接触”和“离开后再次进入”。它只描述几何事实，
    /// 不负责判断抢断、收球或球权归属。
    pub fn free_ball_player_contact_active(
        ball_pos: Vec2,
        ball_z: f32,
        candidate: &(String, u16, f32, Vec2, bool),
        rules: &GameRules,
    ) -> bool {
        const CM_PER_FOOT: f32 = 30.48;
        let (_, height_cm, vertical, player_pos, on_court) = candidate;
        if !on_court {
            return false;
        }
        let radius = rules.body_contact_radius_ft + rules.ball_radius_ft;
        let reach_ft = (*height_cm as f32 / CM_PER_FOOT) * 0.5 + *vertical * 0.5;
        let ball_bottom_z = ball_z - rules.ball_radius_ft;
        (*player_pos - ball_pos).length() <= radius && ball_bottom_z <= reach_ft.max(0.0)
    }

    /// 触筐反弹（ADR-017 第二步）：接触点、反弹初速、落点全部由入射物理
    /// 推导。命中/打铁的统计裁定仍在出手时刻（校准架构不动），本函数只
    /// 负责「打铁之后球怎么弹」：
    ///
    /// 1. 入射速度：水平 = (筐-出手点)/飞行时长；竖直 = 出手抛体在筐高的
    ///    到达速度 vz0 − g·T（下落，负值）。跳投与上篮的飞行时长不同，
    ///    调用方传入实际值；
    /// 2. 接触点在筐环上以「从筐指向出手点」方向为中心的受控扇形内采样；
    /// 3. 反弹初速 = 水平镜像反射 × 恢复系数，方向叠加受控散射；竖直
    ///    弹起 = 入射下落速度 × 竖直弹起系数；
    /// 4. 落点 = 从 (接触点, 筐高) 以反弹初速作抛体，触地时刻的水平位移，
    ///    距离分布从物理中自然产生，不再均匀采样。
    #[allow(clippy::too_many_arguments)]
    pub fn compute_rebound_landing(
        shot_origin: Vec2,
        hoop_pos: Vec2,
        shot_flight_seconds: f32,
        rng: &mut impl Rng,
        rules: &GameRules,
    ) -> ReboundLandingSpot {
        let g = rules.ball_gravity_ftps2.max(f32::EPSILON);
        let shot_vec = hoop_pos - shot_origin;
        let shot_dist = shot_vec.length();
        let shot_dir = shot_vec.normalize_or_zero();
        let t_flight = shot_flight_seconds.max(f32::EPSILON);
        // 入射水平速度（矢量）：沿出手方向匀速逼近筐。
        let v_in_h = shot_dir * (shot_dist / t_flight);
        // 入射竖直速度：出手抛体在到达时刻的速度，负值（下落）。
        let shot_arc =
            ProjectileArc::solve(rules.chest_height_ft, rules.rim_height_ft, t_flight, g);
        let vz_in = shot_arc.vz0 - g * t_flight;
        // 接触点：筐环上以「从筐指向出手点」为中心的受控扇形。
        let to_shooter = -shot_dir;
        let contact_angle = rng.gen_range(
            -rules.rim_contact_angle_spread_radians..rules.rim_contact_angle_spread_radians,
        );
        let contact_dir = Vec2::new(
            to_shooter.x * contact_angle.cos() - to_shooter.y * contact_angle.sin(),
            to_shooter.x * contact_angle.sin() + to_shooter.y * contact_angle.cos(),
        );
        let contact_pos = hoop_pos + contact_dir * rules.rim_radius_ft;
        // 法线：从接触点指向筐心（水平）。镜像反射只翻转法向分量，
        // 入射速度大小不变、方向按接触角重定向。
        let normal = -contact_dir;
        let vn = v_in_h.dot(normal);
        let v_reflected = v_in_h - normal * (vn + vn);
        // 恢复系数随接触角渐变：正面硬碰筐沿保持更多能量（长回弹），
        // 擦筐耗散更多（短回弹）。
        let spread = rules.rim_contact_angle_spread_radians.max(f32::EPSILON);
        let flushness = 1.0 - (contact_angle.abs() / spread).min(1.0);
        let restitution = rules.rim_contact_restitution_graze
            + (rules.rim_contact_restitution_flush - rules.rim_contact_restitution_graze)
                * flushness;
        let v_horizontal = v_reflected * restitution;
        let speed_cap = (rules.ball_max_speed_ftps - rules.invariant_speed_tolerance_ftps)
            .max(rules.invariant_speed_tolerance_ftps);
        let outgoing_horizontal_speed = v_horizontal.length();
        let horizontal_scale = if outgoing_horizontal_speed > speed_cap {
            speed_cap / outgoing_horizontal_speed
        } else {
            1.0
        };
        let v_horizontal = v_horizontal * horizontal_scale;
        // 受控散射：绕竖直轴对称采样小角度旋转。
        let scatter =
            rng.gen_range(-rules.rim_contact_scatter_radians..rules.rim_contact_scatter_radians);
        let scatter_cos = scatter.cos();
        let scatter_sin = scatter.sin();
        let v_out = Vec2::new(
            v_horizontal.x * scatter_cos - v_horizontal.y * scatter_sin,
            v_horizontal.x * scatter_sin + v_horizontal.y * scatter_cos,
        );
        // 竖直弹起：入射下落速度的恢复系数倍。
        let vz_out = -vz_in * rules.rim_contact_vertical_restitution;
        // 落点：从 (接触点, 筐高) 以 (v_out, vz_out) 的抛体触地时刻解。
        let discriminant = (vz_out * vz_out + 2.0 * g * rules.rim_height_ft).max(0.0);
        let t_land = (vz_out + discriminant.sqrt()) / g;
        let raw_landing = contact_pos + v_out * t_land;
        let margin = rules.player_radius_ft.max(0.0);
        let landing_pos = rules.court.clamp_playable(raw_landing, margin);
        let peak_z = if vz_out > 0.0 {
            rules.rim_height_ft + vz_out * vz_out / (2.0 * g)
        } else {
            rules.rim_height_ft
        };
        ReboundLandingSpot {
            contact_pos,
            contact_z: rules.rim_height_ft,
            landing_pos,
            flight_duration: t_land,
            peak_z,
            rebounder_id: None,
        }
    }

    /// 板平面 x 坐标（ADR-017 第三步）：锚点在左半场取板面 = 底线偏移，
    /// 右半场取场宽 − 底线偏移。
    fn backboard_plane_x(anchor_x: f32, rules: &GameRules) -> f32 {
        if anchor_x <= rules.court.width_ft / 2.0 {
            rules.backboard_offset_from_baseline_ft
        } else {
            rules.court.width_ft - rules.backboard_offset_from_baseline_ft
        }
    }

    /// 触板探针（ADR-017 第三步）：出手是否「力度过大越过筐」。
    ///
    /// 判据：出手 → 筐的延长线与板平面相交（交点在筐之后，t > 1），
    /// 且出手弦外推 z（chest→rim 斜率线性延伸到交点）处于板高范围内。
    /// 生产调用方在跳投 Miss 分支用它在打板与近筐沿两条反弹通道之间
    /// 路由。
    pub fn compute_backboard_contact_probe(
        shot_origin: Vec2,
        hoop_pos: Vec2,
        rules: &GameRules,
    ) -> bool {
        let to_hoop = hoop_pos - shot_origin;
        if to_hoop.x.abs() <= f32::EPSILON {
            return false;
        }
        let board_x = Self::backboard_plane_x(hoop_pos.x, rules);
        let t = (board_x - shot_origin.x) / to_hoop.x;
        // 板必须在筐之后，延长线才会穿过板面。
        if t <= 1.0 {
            return false;
        }
        let z_contact = rules.chest_height_ft + (rules.rim_height_ft - rules.chest_height_ft) * t;
        z_contact >= rules.backboard_bottom_height_ft && z_contact <= rules.backboard_top_height_ft
    }

    /// 打板反弹（ADR-017 第三步）：打铁的板通道，入射由瞄准几何推导。
    ///
    /// 打板在真实篮球里是瞄准行为：出手者瞄准板上的点或筐心，瞄准点
    /// 的散射决定命中筐还是打板。本函数把 `aim_point`（筐心或板面上
    /// 的点）沿出手弦延长到板平面：交点在瞄准点之后（t > 1）、弦外推
    /// z 处于板高范围、交点横向处于板宽范围三者同时成立才算触板；
    /// 触板后水平速度 x 分量镜像 × `backboard_restitution`（y 切向
    /// 保持），竖直保持入射下落速度，落点从触板点抛体解出。几何不
    /// 成立时退回近筐沿反射（[`Self::compute_rebound_landing`]），
    /// 两条通道在同一点汇合。
    pub fn compute_rebound_landing_bank(
        shot_origin: Vec2,
        aim_point: Vec2,
        shot_flight_seconds: f32,
        rng: &mut impl Rng,
        rules: &GameRules,
    ) -> ReboundLandingSpot {
        let g = rules.ball_gravity_ftps2.max(f32::EPSILON);
        let aim_vec = aim_point - shot_origin;
        let board_x = Self::backboard_plane_x(aim_point.x, rules);
        let t = if aim_vec.x.abs() <= f32::EPSILON {
            0.0
        } else {
            (board_x - shot_origin.x) / aim_vec.x
        };
        let y_contact = shot_origin.y + aim_vec.y * t;
        // z 沿出手弦线性外推：chest→rim 斜率 × t。
        let z_contact = rules.chest_height_ft + (rules.rim_height_ft - rules.chest_height_ft) * t;
        let touches_board = t > 1.0
            && z_contact >= rules.backboard_bottom_height_ft
            && z_contact <= rules.backboard_top_height_ft
            && (y_contact - rules.court.hoop_y_ft).abs() <= rules.backboard_width_ft / 2.0;
        if !touches_board {
            return Self::compute_rebound_landing(
                shot_origin,
                aim_point,
                shot_flight_seconds,
                rng,
                rules,
            );
        }
        let t_flight = shot_flight_seconds.max(f32::EPSILON);
        let aim_dist = aim_vec.length();
        let aim_dir = aim_vec / aim_dist.max(f32::EPSILON);
        // 入射水平速度：沿瞄准方向匀速逼近板面。
        let v_in_h = aim_dir * (aim_dist / t_flight);
        // 入射竖直速度：出手抛体（chest → 筐高）在到达时刻的速度，下落为负。
        let shot_arc =
            ProjectileArc::solve(rules.chest_height_ft, rules.rim_height_ft, t_flight, g);
        let vz_in = shot_arc.vz0 - g * t_flight;
        // 镜像反射：法向（x）翻转 × 恢复系数，切向（y）保持。
        let v_out = Vec2::new(-v_in_h.x * rules.backboard_restitution, v_in_h.y);
        let contact_pos = Vec2::new(board_x, y_contact);
        // 落点：从 (触板点, 触板高度) 以 (v_out, vz_in) 的抛体触地时刻解。
        let discriminant = (vz_in * vz_in + 2.0 * g * z_contact).max(0.0);
        let t_land = (vz_in + discriminant.sqrt()) / g;
        let raw_landing = contact_pos + v_out * t_land;
        let margin = rules.player_radius_ft.max(0.0);
        let landing_pos = rules.court.clamp_playable(raw_landing, margin);
        let peak_z = if vz_in > 0.0 {
            z_contact + vz_in * vz_in / (2.0 * g)
        } else {
            // 竖直保持下落：反弹后不再升起，弧顶即触板高度。
            z_contact
        };
        ReboundLandingSpot {
            contact_pos,
            contact_z: z_contact,
            landing_pos,
            flight_duration: t_land,
            peak_z,
            rebounder_id: None,
        }
    }

    pub fn compute_rebound_landing_default(
        shot_origin: Vec2,
        hoop_pos: Vec2,
        shot_flight_seconds: f32,
        rng: &mut impl Rng,
    ) -> ReboundLandingSpot {
        Self::compute_rebound_landing(
            shot_origin,
            hoop_pos,
            shot_flight_seconds,
            rng,
            &GameRules::default(),
        )
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
            ball_orientation: nba_domain::action_window::BallOrientation::FaceUp,
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

    /// 自由球-人接触（ADR-017 第三步）：水平半径、摸高门、最近者与
    /// 确定性、板凳豁免。
    #[test]
    fn free_ball_contact_respects_radius_reach_and_priority() {
        let rules = GameRules::default();
        let at = |id: &str, cm: u16, vertical: f32, pos: Vec2, on_court: bool| {
            (id.to_string(), cm, vertical, pos, on_court)
        };
        // 默认规则：人体半径 1.0 ft 加篮球半径 0.4 ft；200cm/0.5 摸高
        // = (200/30.48)*0.5 + 0.25 ≈ 3.53 ft。
        let nearby = vec![at("H_01", 200, 0.5, Vec2::new(95.0 + 1.2, 25.0), true)];
        let hit =
            BallisticsEngine::free_ball_player_contact(Vec2::new(95.0, 25.0), 3.0, &nearby, &rules);
        assert!(hit.is_some(), "ball inside radius and below reach must hit");

        // 摸高门：球高于摸高时不命中。
        let hit_high =
            BallisticsEngine::free_ball_player_contact(Vec2::new(95.0, 25.0), 4.0, &nearby, &rules);
        assert!(
            hit_high.is_none(),
            "ball above the reach ceiling must pass over"
        );

        // 半径门：球在接触半径之外不命中。
        let hit_far = BallisticsEngine::free_ball_player_contact(
            Vec2::new(95.0, 25.0),
            3.0,
            &[at("H_01", 200, 0.5, Vec2::new(95.0 + 2.0, 25.0), true)],
            &rules,
        );
        assert!(
            hit_far.is_none(),
            "ball outside the horizontal radius must miss"
        );

        // 最近者胜；距离相同按 id 字典序取小（确定性）。
        let two = vec![
            at("H_02", 200, 0.5, Vec2::new(95.0 + 0.8, 25.0), true),
            at("H_01", 200, 0.5, Vec2::new(95.0 + 1.2, 25.0), true),
        ];
        let (nearest, _) =
            BallisticsEngine::free_ball_player_contact(Vec2::new(95.0, 25.0), 3.0, &two, &rules)
                .expect("two candidates must hit");
        assert_eq!(nearest, "H_02", "closer candidate must win");

        let tied = vec![
            at("H_02", 200, 0.5, Vec2::new(95.0 - 1.2, 25.0), true),
            at("H_01", 200, 0.5, Vec2::new(95.0 + 1.2, 25.0), true),
        ];
        let (tied_id, _) =
            BallisticsEngine::free_ball_player_contact(Vec2::new(95.0, 25.0), 3.0, &tied, &rules)
                .expect("tied candidates must hit");
        assert_eq!(tied_id, "H_01", "equal distance must tie-break by id order");

        // 板凳球员不参与身体碰撞。
        let bench = vec![at("H_01", 200, 0.5, Vec2::new(95.0, 25.0), false)];
        let hit_bench =
            BallisticsEngine::free_ball_player_contact(Vec2::new(95.0, 25.0), 3.0, &bench, &rules);
        assert!(
            hit_bench.is_none(),
            "bench players must not collide with the ball"
        );
    }
}
