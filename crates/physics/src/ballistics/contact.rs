// 自由球-人接触与篮板落点（`BallisticsEngine` 的第二段实现）。
//
// 划分依据（D29 行数门，与 movement/kinematics.rs 同款分层）：本文件
// 只保留「球已经脱手，接下来由接触与落点决定去向」的几何——
//
// - 自由球（篮板飞行、地板球）对在场球员的可触碰性（ADR-017 第三步）；
// - 篮板/筐接触后的反弹落点族，含打板路径与反弹体飞行时长。
//
// 「出手瞄准、投篮飞行、传球落点、球位置采样」留在 `../ballistics.rs`：
// 那半是发射端，本半是接触端，两者只通过 `ReboundLandingSpot` 交换事实。

use glam::Vec2;
use nba_domain::GameRules;
use rand::Rng;

use super::{BallisticsEngine, ProjectileArc, ReboundLandingSpot};

impl BallisticsEngine {
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

    /// 使用已观测的筐环接触点推导反弹落点，避免事后另选触点。
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
        let shot_dir = (hoop_pos - shot_origin).normalize_or_zero();
        let to_shooter = -shot_dir;
        let contact_angle = rng.gen_range(
            -rules.rim_contact_angle_spread_radians..rules.rim_contact_angle_spread_radians,
        );
        let contact_dir = Vec2::new(
            to_shooter.x * contact_angle.cos() - to_shooter.y * contact_angle.sin(),
            to_shooter.x * contact_angle.sin() + to_shooter.y * contact_angle.cos(),
        );
        let contact_pos = hoop_pos + contact_dir * rules.rim_radius_ft;
        let incoming_velocity =
            Self::sample_incoming_shot_velocity(shot_origin, hoop_pos, shot_flight_seconds, rules);
        Self::compute_rebound_landing_at_contact_velocity(
            shot_origin,
            hoop_pos,
            shot_flight_seconds,
            incoming_velocity,
            contact_pos,
            rng,
            rules,
        )
    }

    pub fn compute_rebound_landing_from_contact_velocity(
        shot_origin: Vec2,
        hoop_pos: Vec2,
        incoming_velocity: glam::Vec3,
        contact_pos: Vec2,
        rng: &mut impl Rng,
        rules: &GameRules,
    ) -> ReboundLandingSpot {
        let distance = (hoop_pos - shot_origin).length();
        let horizontal_speed = incoming_velocity.truncate().length();
        let flight_seconds = distance / horizontal_speed.max(f32::EPSILON);
        Self::compute_rebound_landing_at_contact_velocity(
            shot_origin,
            hoop_pos,
            flight_seconds,
            incoming_velocity,
            contact_pos,
            rng,
            rules,
        )
    }

    pub fn compute_rebound_landing_from_contact(
        shot_origin: Vec2,
        hoop_pos: Vec2,
        shot_flight_seconds: f32,
        contact_pos: Vec2,
        rng: &mut impl Rng,
        rules: &GameRules,
    ) -> ReboundLandingSpot {
        let incoming_velocity =
            Self::sample_incoming_shot_velocity(shot_origin, hoop_pos, shot_flight_seconds, rules);
        Self::compute_rebound_landing_at_contact_velocity(
            shot_origin,
            hoop_pos,
            shot_flight_seconds,
            incoming_velocity,
            contact_pos,
            rng,
            rules,
        )
    }

    fn sample_incoming_shot_velocity(
        shot_origin: Vec2,
        hoop_pos: Vec2,
        shot_flight_seconds: f32,
        rules: &GameRules,
    ) -> glam::Vec3 {
        let duration = shot_flight_seconds.max(f32::EPSILON);
        let arc = ProjectileArc::solve(
            rules.chest_height_ft,
            rules.rim_height_ft,
            duration,
            rules.ball_gravity_ftps2,
        );
        let direction = (hoop_pos - shot_origin).normalize_or_zero();
        let horizontal = direction * ((hoop_pos - shot_origin).length() / duration);
        glam::Vec3::new(
            horizontal.x,
            horizontal.y,
            arc.vz0 - rules.ball_gravity_ftps2 * duration,
        )
    }

    fn compute_rebound_landing_at_contact_velocity(
        shot_origin: Vec2,
        hoop_pos: Vec2,
        _shot_flight_seconds: f32,
        incoming_velocity: glam::Vec3,
        contact_pos: Vec2,
        rng: &mut impl Rng,
        rules: &GameRules,
    ) -> ReboundLandingSpot {
        let g = rules.ball_gravity_ftps2.max(f32::EPSILON);
        let to_shooter = (shot_origin - hoop_pos).normalize_or_zero();
        let contact_dir = (contact_pos - hoop_pos).normalize_or_zero();
        let contact_angle = to_shooter
            .perp_dot(contact_dir)
            .atan2(to_shooter.dot(contact_dir));
        // 入射水平速度（矢量）：沿出手方向匀速逼近筐。
        let v_in_h = incoming_velocity.truncate();
        let vz_in = incoming_velocity.z;
        let contact_dir = (contact_pos - hoop_pos).normalize_or_zero();
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

    /// 从检测到的触板位置与入射速度生成反弹落点。
    pub fn compute_rebound_landing_bank_from_contact_velocity(
        contact_pos: Vec2,
        contact_z: f32,
        incoming_velocity: glam::Vec3,
        rules: &GameRules,
    ) -> ReboundLandingSpot {
        let gravity = rules.ball_gravity_ftps2.max(f32::EPSILON);
        let board_x = if contact_pos.x <= rules.court.width_ft / 2.0 {
            rules.backboard_offset_from_baseline_ft
        } else {
            rules.court.width_ft - rules.backboard_offset_from_baseline_ft
        };
        let reflected = Vec2::new(
            -incoming_velocity.x * rules.backboard_restitution,
            incoming_velocity.y,
        );
        let vertical = (-incoming_velocity.z * rules.backboard_vertical_restitution).max(0.0);
        let flight_duration =
            (vertical + (vertical * vertical + 2.0 * gravity * contact_z).sqrt()) / gravity;
        let landing_pos = rules.court.clamp_playable(
            contact_pos + reflected * flight_duration,
            rules.player_radius_ft,
        );
        ReboundLandingSpot {
            contact_pos: Vec2::new(board_x, contact_pos.y),
            contact_z,
            landing_pos,
            flight_duration,
            peak_z: contact_z + vertical * vertical / (2.0 * gravity),
            rebounder_id: None,
        }
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
        // 竖直：入射下落速度 × 板面弹起系数——真实板反弹球会向上弹起
        // （竖直分量衰减但不归零），滞空 1~2s 给篮板争抢留出窗口；
        // 保持入射下落速度会让球贴板滑落（滞空 0.4s），篮板全部
        // 变成落地地板球。
        let vz_out = -vz_in * rules.backboard_vertical_restitution;
        let v_out = Vec2::new(-v_in_h.x * rules.backboard_restitution, v_in_h.y);
        let contact_pos = Vec2::new(board_x, y_contact);
        // 落点：从 (触板点, 触板高度) 以 (v_out, vz_out) 的抛体触地时刻解。
        let discriminant = (vz_out * vz_out + 2.0 * g * z_contact).max(0.0);
        let t_land = (vz_out + discriminant.sqrt()) / g;
        let raw_landing = contact_pos + v_out * t_land;
        let margin = rules.player_radius_ft.max(0.0);
        let landing_pos = rules.court.clamp_playable(raw_landing, margin);
        let peak_z = if vz_out > 0.0 {
            z_contact + vz_out * vz_out / (2.0 * g)
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
