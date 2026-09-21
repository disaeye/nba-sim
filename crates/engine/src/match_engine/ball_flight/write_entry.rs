//! 球态的写入口与物理派生标志同步。
//!
//! 本模块是 `ball_state` 的**唯一写通道**（`architecture.md` §3.2/§3.3）：
//! 经领域层纯函数转换表校验，非法边被拒绝并记入强制项。它同时维护物理层的
//! 派生态（`has_ball` / `is_receiving_pass` / `is_driving_to_rim` /
//! `out_of_bounds_placement`），这些标志必须与权威球态同一步更新，
//! 否则物理会按上一 tick 的旧标志执行。
//!
//! 与 `resolve.rs` 的分工：这里**写**球态，那里**裁决**球态在飞行中的接球点
//! 与归属变化。写入口被两者共用，因此单独立文件。

use glam::Vec2;
use nba_domain::GameEvent;
use nba_physics::ballistics::BallTrajectoryKind;

use super::super::projection::is_inbound_role_action;
use crate::match_engine::MatchEngine;

impl MatchEngine {
    /// 球弹道状态的唯一写入口（BallState Consolidation 的第一步）。
    ///
    /// 所有 `ball_state` 变更都必须经过此方法，保证：
    /// - 归属派生（physics 层的 `has_ball`）随状态同步，不出现"两人持球"；
    /// - 未来可在此插入状态转换合法性校验与领域事件，无需改各调用点。
    ///
    /// `current_t` 用于同步持有/受控状态的持球人参考；非持有状态传当前时间即可。
    /// 标记/清除本 tick 的接球人（round-10）。
    ///
    /// 接球人向球收敛期间需豁免 APF 排斥，并在到位后立即停住
    /// （见 `physics::movement` 的 `is_receiving_pass` 分支）。
    /// 传 `None` 表示清除全部标记。
    pub(crate) fn mark_receiver(&mut self, receiver_id: Option<&str>) {
        let ids: Vec<String> = self.systems.physics.get_players().keys().cloned().collect();
        for id in ids {
            let should = receiver_id == Some(id.as_str());
            if let Some(p) = self.systems.physics.get_player_mut(&id) {
                p.is_receiving_pass = should;
            }
        }
    }

    pub(crate) fn transition_ball_state(&mut self, next: BallTrajectoryKind) {
        // M2：经领域层纯函数转换表校验，非法边被拒绝并记入强制项
        // （architecture.md §3.2 唯一写入口 + §3.3 转换表穷举）。
        let next_transfer_id = match &next {
            BallTrajectoryKind::ControlTransfer { carrier_id, .. } => Some(carrier_id.clone()),
            _ => None,
        };
        let released_transfer_id = match (&self.ball.ball_state, &next) {
            (BallTrajectoryKind::ControlTransfer { carrier_id, .. }, next)
                if !matches!(next, BallTrajectoryKind::ControlTransfer { .. }) =>
            {
                Some(carrier_id.clone())
            }
            _ => None,
        };
        // ## 攻框旗标（round-18）：进入 Drive 置位，离开清除。
        //
        // 唯一写入口统一管理（与接球人标记同一模式）。旗标让物理层的
        // APF 排斥豁免攻框者——对抗交由终结裁决处理，转向墙挡不住攻框。
        let drive_flag: Option<(String, bool)> = match (&self.ball.ball_state, &next) {
            (_, BallTrajectoryKind::Drive { driver_id, .. }) => Some((driver_id.clone(), true)),
            (BallTrajectoryKind::Drive { driver_id, .. }, next)
                if !matches!(next, BallTrajectoryKind::Drive { .. }) =>
            {
                Some((driver_id.clone(), false))
            }
            _ => None,
        };
        // 球-人弹开账目（ADR-017 第三步）：进入**新的**自由球阶段时清空。
        //
        // 「同一飞行对同一球员只弹一次」的作用域是单段飞行：弹开后的
        // 自环（RimRebound → RimRebound / LooseBall → LooseBall）必须
        // 保留账目，否则球还在同一人身边时会逐 tick 重复弹开；而换段
        // （传球被拨掉、打铁后球触地变松球、封盖拍出等）是新飞行，同一
        // 球员应重新参与碰撞。
        let entering_free_flight = matches!(
            next,
            BallTrajectoryKind::RimRebound { .. } | BallTrajectoryKind::LooseBall { .. }
        );
        let same_kind = matches!(
            (&self.ball.ball_state, &next),
            (BallTrajectoryKind::RimRebound { .. }, BallTrajectoryKind::RimRebound { .. })
                | (BallTrajectoryKind::LooseBall { .. }, BallTrajectoryKind::LooseBall { .. })
        );
        if entering_free_flight && !same_kind {
            self.ball.loose_contact_resolved.clear();
        }
        match nba_domain::transition_ball_state(&self.ball.ball_state, next) {
            Ok(next) => {
                // ## 接球人标记必须在**写入口**设置（round-10）
                //
                // 物理步进（`step` 内 `physics.step`）发生在战术规划**之前**，
                // 因此若在战术规划里才标记接球人，第一个 tick 的物理仍按旧标志执行，
                // 接球人会带着上一 tick 的速度滑离接球点（实测 17.26 ft/s，
                // 一 tick 滑 1.73 ft > catch_radius）。
                //
                // 球态进入 `Pass` 时，接球人的身份已确定（`target_id`），
                // 在唯一写入口立即标记，保证下一 tick 的物理就生效。
                if let BallTrajectoryKind::Pass { target_id, .. } = &next {
                    let rid = target_id.clone();
                    self.mark_receiver(Some(&rid));
                    // 新的一次传球出手：清空接触锁存，让本次飞行重新逐
                    // tick 检测防守者接触（每个防守者对每对传球只掷一次）。
                    self.ball.pass_contact_resolved.clear();
                }
                self.ball.ball_state = next;
                self.sync_ball_holder();

                // During the frozen control-transfer flight the receiving body
                // must not move away from the endpoint. Otherwise the state
                // would end with a Held label at a stale ball coordinate.
                if let Some(player_id) = next_transfer_id {
                    if let Some(player) = self.systems.physics.get_player_mut(&player_id) {
                        player.is_locked_kinematics = true;
                        player.vel_ft = Vec2::ZERO;
                        player.accel_ft = Vec2::ZERO;
                        player.target_pos_ft = player.pos_ft;
                        player.target_speed_ftps = 0.0;
                    }
                }
                if let Some(player_id) = released_transfer_id {
                    self.systems
                        .physics
                        .set_player_locked(&player_id, false, None);
                }
                if let Some((player_id, entering)) = drive_flag {
                    if let Some(p) = self.systems.physics.get_player_mut(&player_id) {
                        p.is_driving_to_rim = entering;
                    }
                }
            }
            Err(reason) => {
                self.journal
                    .current_enforcements
                    .push(format!("ILLEGAL_BALL_TRANSITION:{}", reason));
            }
        }
    }

    /// 将 physics 层的逐球员 `has_ball` 与 `ball_state` 的归属保持一致。
    /// Held / Drive / ControlTransfer 视为"有明确持球人"，其余状态清空持球标志。
    pub(crate) fn sync_ball_holder(&mut self) {
        let holder: Option<&str> = match &self.ball.ball_state {
            BallTrajectoryKind::Held { carrier_id }
            | BallTrajectoryKind::InboundReady {
                inbounder_id: carrier_id,
                ..
            }
            | BallTrajectoryKind::Drive {
                driver_id: carrier_id,
                ..
            } => Some(carrier_id.as_str()),
            _ => None,
        };
        self.systems.physics.set_ball_holder(holder);

        // F1.3：player.out_of_bounds_placement 是派生态：只有“当前权威球态
        // 处于发球程序”（InboundTransfer/InboundReady）时才允许界外豁免。
        // 一旦球进入其他状态（传球飞行、被断、死球等），发球员必须恢复
        // 普通在场约束，不能永久保持界外豁免（本轮实测 476–1324 条
        // PLAYER_IN_BOUNDS 即由 sticky flag 造成）。
        let exempt_inbounder: Option<String> = match &self.ball.ball_state {
            BallTrajectoryKind::InboundTransfer { inbounder_id, .. }
            | BallTrajectoryKind::InboundReady { inbounder_id, .. } => Some(inbounder_id.clone()),
            _ => None,
        };
        let ids: Vec<String> = self.systems.physics.get_players().keys().cloned().collect();
        let mut placements: Vec<(String, Vec2, Vec2)> = Vec::new();
        for id in ids {
            let should_exempt = exempt_inbounder.as_deref() == Some(id.as_str());
            let mut release_pos = None;
            let mut clear_action = false;
            if let Some(p) = self.systems.physics.get_player_mut(&id) {
                let was_exempt = p.out_of_bounds_placement;
                p.out_of_bounds_placement = should_exempt;
                if was_exempt && !should_exempt {
                    release_pos = Some(p.pos_ft);
                    clear_action = is_inbound_role_action(&p.action);
                }
                // 注：非发球球员在发球程序中被卡界外的几何僵局修复，已移至
                // 物理步进的 InboundReady 分支（每 tick 执行），不在此——本函数
                // 只在球态转换时调用，覆盖不到恒为 InboundReady 的停滞段。
            }
            // 豁免被取消且球员仍在界外时，必须做一次显式离散 placement
            // 把它放回界内，而不是让下一 tick 的物理 clamp 产生
            // PLAYER_SPEED 伪造超速（gap.md §4.3）。
            if let Some(from_pos) = release_pos {
                // 选择界内且不与任何在场球员重叠的接球点。若直接放在被
                // 他人占据的边界点上，下一 tick 的分离投影会产生巨大
                // 瞬时修正（本轮 seed 21 实测 PLAYER_SPEED 55–117 ft/s）。
                let to = self.free_in_court_spot(from_pos, &exempt_inbounder);
                if let Some(p) = self.systems.physics.get_player_mut(&id) {
                    // 发球程序结束：清除发球角色动作，否则评估/展示层仍会
                    // 把它当作界外豁免对象。
                    if clear_action {
                        p.action = "SpotUp".to_string();
                    }
                    if (to - from_pos).length() > f32::EPSILON {
                        p.pos_ft = to;
                        p.target_pos_ft = to;
                        p.vel_ft = Vec2::ZERO;
                        p.accel_ft = Vec2::ZERO;
                        placements.push((id.clone(), from_pos, to));
                    }
                }
            }
        }
        for (id, from, to) in placements {
            self.systems.physics.teleport_player(&id, to);
            self.journal
                .pending_events
                .push(GameEvent::PlacementApplied {
                    player_id: id,
                    from: (from.x, from.y),
                    to: (to.x, to.y),
                    reason: "INBOUND_PROGRAM_EXIT".to_string(),
                    phase: self.phase_type().as_str().to_string(),
                });
        }
    }
}
