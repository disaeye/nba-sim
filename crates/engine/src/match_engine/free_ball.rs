//! 自由球-人碰撞（ADR-017 第三步）：自由球对在场球员弹开。
//!
//! 弹道（`ball_flight`）在 RimRebound 飞行中与 LooseBall 滚动/弹跳中调用
//! 本模块：检测球是否撞上在场球员身体，命中则做几何弹开并重解飞行参数。
//! 归属裁定（谁抢到球）保持既有路径（`try_resolve_rebounder` /
//! `LooseBallSecured`），本模块只改变球的运动，不改变球权语义。

use glam::Vec2;
use nba_physics::ballistics::{BallTrajectoryKind, BallisticsEngine};

use super::MatchEngine;

impl MatchEngine {
    /// 收集在场球员的接触检测摘要切片。
    ///
    /// 身高是量纲事实（cm），只在名册档案里；按 id 从两队名册查
    /// （与 `block.rs` 的封盖身高查询同一先例）。physics 层不依赖名册类型，
    /// 因此以 `(id, height_cm, vertical, pos, on_court)` 摘要传入。
    pub(crate) fn free_ball_contact_candidates(&self) -> Vec<(String, u16, f32, Vec2, bool)> {
        let height_cm = |id: &str| -> u16 {
            self.config
                .home_team
                .players
                .iter()
                .chain(self.config.away_team.players.iter())
                .find(|p| p.id == id)
                .map(|p| p.height_cm)
                .unwrap_or(200)
        };
        let mut candidates: Vec<_> = self
            .systems
            .physics
            .get_players()
            .values()
            .map(|p| {
                (
                    p.id.clone(),
                    height_cm(&p.id),
                    p.attributes.vertical,
                    p.pos_ft,
                    p.on_court,
                )
            })
            .collect();
        // 确定性：检测函数按距离与 id 排序裁决，这里按 id 排序保证
        // 候选顺序本身不依赖 HashMap 遍历序。
        candidates.sort_by(|left, right| left.0.cmp(&right.0));
        candidates
    }

    /// 检测自由球与在场球员的接触并构造弹开后的新球态。
    ///
    /// 命中时返回 `(新球态, 接触球员 id)`。弹开几何：
    ///
    /// 1. 入射水平速度：篮板飞行 = `(落点 − from_pos) / duration`（采样
    ///    是水平匀速插值，同源）；地板球 = 球态携带的 `vel`；
    /// 2. 反射：以「球员指向球」为法线的水平镜像反射 ×
    ///    `loose_ball_player_restitution`（规则通道）；
    /// 3. 落点重解：从当前 `(球位, 球高)` 以弹后水平速度与当前下落速度
    ///    内联抛体触地解（与 `compute_rebound_landing` 的落点解同式），
    ///    夹回可站立区域；
    /// 4. 速度 clamp 到球速包络（保 `BALL_SPEED` 不变量）。
    ///
    /// 返回的球态保留原自由球种类（RimRebound/LooseBall），起始时间取
    /// 当前时刻，使采样与真实飞行连续。
    pub(crate) fn resolve_free_ball_player_contact(
        &mut self,
        ball_3d: (Vec2, f32),
        current_t: f32,
    ) -> Option<(BallTrajectoryKind, String)> {
        let (ball_pos, ball_z) = ball_3d;
        let candidates = self.free_ball_contact_candidates();
        // 记录当前接触区间，而不是永久屏蔽某名球员。球离开身体范围后，
        // 再次进入时应当允许新的身体碰撞。
        self.ball.loose_contact_resolved.retain(|resolved_id| {
            candidates.iter().any(|candidate| {
                candidate.0 == *resolved_id
                    && BallisticsEngine::free_ball_player_contact_active(
                        ball_pos,
                        ball_z,
                        candidate,
                        &self.config.rules,
                    )
            })
        });
        let (player_id, player_pos) = BallisticsEngine::free_ball_player_contact(
            ball_pos,
            ball_z,
            &candidates,
            &self.config.rules,
        )?;
        // 同一接触区间只结算一次：弹开后的球仍可能贴着同一人，
        // 逐 tick 重检会连续改写弹道。球离开后，前面的 retain 会清除记录。
        if self
            .ball
            .loose_contact_resolved
            .iter()
            .any(|id| id == &player_id)
        {
            return None;
        }

        // 入射水平速度。
        let v_horizontal = match &self.ball.ball_state {
            BallTrajectoryKind::RimRebound {
                from_pos,
                target_landing,
                duration,
                ..
            } => {
                let t_flight = duration.max(f32::EPSILON);
                (*target_landing - *from_pos) / t_flight
            }
            BallTrajectoryKind::LooseBall { vel, .. } => *vel,
            _ => return None,
        };

        // 镜像反射：法线 = 从球员指向球（水平），翻转法向分量，切向保留。
        let normal = (ball_pos - player_pos).normalize_or_zero();
        let vn = v_horizontal.dot(normal);
        // 球已经离开人体时不应再次反射。
        if vn >= 0.0 {
            return None;
        }
        let restitution = self.config.rules.loose_ball_player_restitution;
        let reflected = (v_horizontal - normal * (vn + vn)) * restitution;

        // 当前下落速度（竖直）：篮板飞行由抛体导数给出；地板球取 vel_z。
        let g = self.config.rules.ball_gravity_ftps2;
        let vz_current = match &self.ball.ball_state {
            BallTrajectoryKind::RimRebound {
                from_z,
                start_time,
                duration,
                ..
            } => {
                let t_flight = duration.max(f32::EPSILON);
                let arc = nba_domain::projectile::ProjectileArc::solve(*from_z, 0.0, t_flight, g);
                let elapsed = (current_t - start_time).clamp(0.0, t_flight);
                arc.vz0 - g * elapsed
            }
            BallTrajectoryKind::LooseBall { vel_z, .. } => *vel_z,
            _ => return None,
        };

        // 速度包络作用于完整三维速度，避免分别限制水平和竖直分量后
        // 合速度仍然超过 BALL_SPEED。
        let cap = (self.config.rules.ball_max_speed_ftps
            - self.config.rules.invariant_speed_tolerance_ftps)
            .max(self.config.rules.invariant_speed_tolerance_ftps);
        let outgoing_speed = (reflected.length_squared() + vz_current * vz_current).sqrt();
        let speed_scale = if outgoing_speed > cap {
            cap / outgoing_speed
        } else {
            1.0
        };
        let v_out = reflected * speed_scale;
        let vz_out = vz_current * speed_scale;

        // 内联抛体触地解：从 (球位, 球高) 以最终弹后速度抛出，
        // 触地时刻的水平位移即新落点（与 compute_rebound_landing 同式）。
        let disc = (vz_out * vz_out + 2.0 * g * ball_z.max(0.0)).max(0.0);
        let t_land = (vz_out + disc.sqrt()) / g;
        let margin = self.config.rules.player_radius_ft.max(0.0);
        let raw_landing = ball_pos + v_out * t_land;
        let landing = self.config.rules.court.clamp_playable(raw_landing, margin);

        let next_state = match &self.ball.ball_state {
            BallTrajectoryKind::RimRebound {
                hoop_pos,
                peak_z,
                last_touch_team,
                last_touch_player,
                ..
            } => {
                // 弹后抛体的自然弧顶（仅元数据）。
                let peak = if vz_out > 0.0 {
                    ball_z + vz_out * vz_out / (2.0 * g)
                } else {
                    ball_z
                };
                BallTrajectoryKind::RimRebound {
                    from_pos: ball_pos,
                    from_z: ball_z,
                    hoop_pos: *hoop_pos,
                    target_landing: landing,
                    start_time: current_t,
                    duration: t_land,
                    peak_z: peak.max(peak_z.min(self.config.rules.ball_z_max_ft)),
                    last_touch_team: *last_touch_team,
                    // 身体弹开不是触球：最后触球人沿当前载荷延续。
                    last_touch_player: last_touch_player.clone(),
                }
            }
            BallTrajectoryKind::LooseBall {
                last_touch_team,
                last_touch_player,
                ..
            } => BallTrajectoryKind::LooseBall {
                pos: ball_pos,
                vel: v_out,
                z: ball_z,
                vel_z: vz_out,
                last_touch_team: *last_touch_team,
                // 身体弹开不是触球：最后触球人沿当前载荷延续。
                last_touch_player: last_touch_player.clone(),
            },
            _ => return None,
        };
        Some((next_state, player_id))
    }
}
