//! 战术目标生成与移动导航（每 tick）。
//!
//! 依据 `docs/tactics.md` 与 `docs/architecture.md` §2 [8]：进攻槽位来自
//! `data/tactics/` 档案，防守目标来自对位与协防责任链；本阶段只把目标点
//! 交给物理层，不裁判任何归属或结果。

use glam::Vec2;
use nba_decision::tactics::TacticalPlanner;
use nba_domain::action_window::ActionTimeWindow;
use nba_domain::{Possession, SubPhase};
use nba_physics::ballistics::BallTrajectoryKind;

use super::MatchEngine;

impl MatchEngine {
    pub(crate) fn plan_tactics_and_navigation(&mut self, current_t: f32) {
        // ============================================================
        // 4. 战术目标生成 & 移动导航（每 tick）
        // ============================================================
        let off_roster = match self.flow.possession {
            Possession::Home => &self.config.home_roster_order,
            Possession::Away => &self.config.away_roster_order,
        };
        let live_off_positions: Vec<Vec2> = off_roster
            .iter()
            .filter_map(|pid| self.systems.physics.get_player(pid).map(|p| p.pos_ft))
            .collect();
        // D5.1b：进攻槽位由**战术档案**决定（底角/内线/弧顶距离各异），
        // 防守目标仍由对位逻辑生成。
        let off_roster_owned: Vec<String> = off_roster.clone();
        let off_spec = match self.flow.possession {
            Possession::Home => &self.config.home_offense_spec,
            Possession::Away => &self.config.away_offense_spec,
        };
        // slot fill：按能力把槽位分配给在场球员（不再按 roster 顺序绑定）。
        let fitness: Vec<nba_domain::PlayerSlotFitness> = off_roster_owned
            .iter()
            .filter_map(|pid| {
                self.systems
                    .physics
                    .get_player(pid)
                    .filter(|p| p.on_court)
                    .map(|_| pid.as_str())
            })
            .filter_map(|pid| {
                self.config
                    .home_team
                    .players
                    .iter()
                    .chain(self.config.away_team.players.iter())
                    .find(|pl| pl.id == pid)
            })
            .map(nba_domain::PlayerSlotFitness::from_player)
            .collect();
        let (filled_ids, fit_error) = TacticalPlanner::fill_slots_or_roster_order(
            off_spec,
            &fitness,
            &off_roster_owned,
            &self.config.rules,
        );
        if let Some(err) = fit_error {
            self.journal
                .current_enforcements
                .push(format!("SLOT_FIT_FALLBACK:{}", err));
        }
        // 持球槽位：由 slot fill 选出「最擅长处理球」的球员所占据的槽位。
        // 这样持球权归属来自能力适配，而不是 roster 索引（D5.1b）。
        let mut carrier_slot = 0usize;
        let mut best_handle = f32::MIN;
        for (i, pid) in filled_ids.iter().enumerate() {
            if let Some(p) = fitness.iter().find(|f| &f.player_id == pid) {
                let s = TacticalPlanner::handler_score(off_spec, &p.attributes);
                if s > best_handle {
                    best_handle = s;
                    carrier_slot = i;
                }
            }
        }
        let mut off_targets = TacticalPlanner::plan_offense_from_spec(
            off_spec,
            self.clock.sub_phase,
            self.flow.possession,
            carrier_slot,
            self.clock.sub_phase_timer,
            &self.config.rules,
        );
        TacticalPlanner::bind_targets(&mut off_targets, &filled_ids);

        // 场内战术对位焦点（ADR-010）：
        // 1. 若球在场内被持有/突破/合球，焦点为持球人；
        // 2. 若球在传球飞行中，焦点为接球人（防守人保持对位）；
        // 3. 界外发球/死球/争球/投篮飞行等无场内持球人状态，焦点为场内战术发起人（PG/Playmaker）。
        let focus_player_id = match &self.ball.ball_state {
            BallTrajectoryKind::Held { carrier_id }
            | BallTrajectoryKind::Drive {
                driver_id: carrier_id,
                ..
            }
            | BallTrajectoryKind::ControlTransfer { carrier_id, .. } => carrier_id.clone(),
            BallTrajectoryKind::Pass { target_id, .. } => target_id.clone(),
            _ => self.new_possession_pg(),
        };
        let carrier_idx = off_roster_owned
            .iter()
            .position(|id| id == &focus_player_id)
            .unwrap_or(0);

        // 防守目标沿用对位/协防逻辑（含 D5.2 的执行器）。
        let (mut home_targets, mut away_targets) =
            TacticalPlanner::plan_possession_targets_with_rules(
                self.config.tactical_set,
                self.clock.sub_phase,
                self.flow.possession,
                self.ball.ball_pos_3d.0,
                carrier_idx,
                self.clock.sub_phase_timer,
                &mut self.systems.rng,
                &self.config.rules,
                Some(&live_off_positions),
            );
        let home_roster = self.config.home_roster_order.clone();
        let away_roster = self.config.away_roster_order.clone();
        // 进攻方用档案槽位覆盖（球员绑定已在 off_targets 内按能力完成）；
        // 防守方保留原对位结果并按 roster 绑定。
        // 注意：不能再对 off_targets 调用 bind_targets——它按 roster 顺序
        // 覆盖 player_id，会把 slot fill 的结果抹掉并把替补拉进场内
        // （本轮实测 116 条 PLAYER_SEPARATION：替补 A_7 与在场球员重叠 1.19ft）。
        if self.flow.possession == Possession::Home {
            home_targets = off_targets;
            TacticalPlanner::bind_targets(&mut away_targets, &away_roster);
        } else {
            away_targets = off_targets;
            TacticalPlanner::bind_targets(&mut home_targets, &home_roster);
        }
        // 取为 owned String，避免与后续 `&mut self` 调用（接球人估计）冲突。
        let active_driver_id = match &self.ball.ball_state {
            BallTrajectoryKind::Drive { driver_id, .. } => Some(driver_id.clone()),
            _ => None,
        };
        let inbounder_override = match &self.ball.ball_state {
            BallTrajectoryKind::InboundTransfer {
                inbounder_id,
                baseline_pos,
                ..
            }
            | BallTrajectoryKind::InboundReady {
                inbounder_id,
                baseline_pos,
                ..
            } => Some((inbounder_id.clone(), *baseline_pos)),
            _ => None,
        };
        // 层 A（P-1 有限信息）：接球人**不得**直读传球人的冻结落点。
        //
        // 旧实现把 `BallState::Pass.to_pos`（传球人的私有意图）直接注入接球人的
        // 运动目标，于是接球人必然到位——全知全能，违反真实性。实测：无论把
        // 接球人的 `off_ball_sense`（预估能力）设为 0.95 还是 0.05，2/4 种子的
        // 接球轨迹**逐位相同**（见 `tests/pass_information.rs`）。
        //
        // 现改为：接球人按**自己的感知**估算球会到哪里，并向该估计值收敛。
        // 估计可能错 ⇒ 他可能接不到（真实：大个策应给小个传提前量，小个
        // 可能启动方向不同而接不到）。
        // 先从球态取出所需字段（避免与 `estimate_receiver_landing` 的
        // `&mut self` 冲突），再计算接球人的**自身估计**。
        let pass_fields = match &self.ball.ball_state {
            BallTrajectoryKind::Pass {
                target_id, to_pos, ..
            } => Some((target_id.clone(), *to_pos)),
            _ => None,
        };
        let pass_receiver_override = if let Some((rid, frozen)) = pass_fields {
            let est = self.estimate_receiver_landing(&rid, frozen);
            // 接球人标记已在 `transition_ball_state`（唯一写入口）设置，
            // 确保物理步进先于战术规划时也能生效。
            Some((rid, est))
        } else {
            match &self.ball.ball_state {
                BallTrajectoryKind::ControlTransfer {
                    carrier_id,
                    target_pos,
                    ..
                } => Some((carrier_id.clone(), *target_pos)),
                _ => None,
            }
        };
        let rebound_chase_target = match &self.ball.ball_state {
            BallTrajectoryKind::RimRebound { target_landing, .. } => Some(*target_landing),
            BallTrajectoryKind::LooseBall { pos, .. } => Some(*pos),
            _ => None,
        };
        let home_jumper = self.select_jumper_id(Possession::Home);
        let away_jumper = self.select_jumper_id(Possession::Away);
        for target in home_targets.into_iter().chain(away_targets) {
            if let Some(player_id) = target.player_id {
                if active_driver_id.as_deref() == Some(player_id.as_str()) {
                    continue;
                }
                // 被过恢复窗口（round-19）：被过掉的防守人正扑向回追位，
                // 战术层不得立即把他派回原位（否则让位形同虚设）。
                if self
                    .observations
                    .beaten_recovery_until
                    .get(player_id.as_str())
                    .is_some_and(|&until| current_t < until)
                {
                    continue;
                }
                // If player is currently executing a locked action window (shot, layup, dunk, pass, screen),
                // protect their action and kinematics from tactical overwrite
                let in_active_window = self
                    .observations
                    .active_windows
                    .get(&player_id)
                    .map(|w| !w.is_finished(current_t))
                    .unwrap_or(false);
                let is_action_locked = self
                    .systems
                    .physics
                    .get_player(&player_id)
                    .map(|p| {
                        let a = p.action.as_str();
                        a == "TripleThreat"
                            || a == "PostUp"
                            || a.ends_with("Shot")
                            || a == "Layup"
                            || a == "Dunk"
                            || a == "Floater"
                    })
                    .unwrap_or(false);

                if in_active_window || is_action_locked {
                    continue;
                }

                // If tactical assignment is setting a high screen, initialize a ScreenSet action window
                if target.action == "SET_HIGH_SCREEN"
                    && !self.observations.active_windows.contains_key(&player_id)
                {
                    self.observations.active_windows.insert(
                        player_id.clone(),
                        ActionTimeWindow::new_screen_set(&player_id, current_t, &self.config.rules),
                    );
                }

                // 第一性原理：后场推进期间不得被战术目标覆盖。
                //
                // `set_player_target` 每 tick 都执行；若持球人正在执行
                // `Advance`（把球推过中线），战术槽位（弧顶 x=66 等）
                // 会把目标改回半场落位，导致推进速度被反复打断
                // （实测仅 3.5–3.9 ft/s，而 8 秒规则需要 ≥4.5 ft/s）。
                let is_carrier_advancing = self.observations.advancing_player.as_deref()
                    == Some(player_id.as_str())
                    && self.clock.sub_phase != SubPhase::Initiation;
                if is_carrier_advancing {
                    continue;
                }
                let (target_pos, speed, action) = if let Some((inb_id, inb_pos)) =
                    &inbounder_override
                {
                    if inb_id == &player_id {
                        (*inb_pos, 15.0, "INBOUND_SETUP".to_string())
                    } else if let Some((rx_id, rx_pos)) = &pass_receiver_override {
                        if rx_id == &player_id {
                            let (aim, spd) = self.receive_approach(*rx_pos, &player_id);
                            (aim, spd, "RECEIVE_CUT".to_string())
                        } else {
                            (target.target_pos, target.speed, target.action)
                        }
                    } else {
                        (target.target_pos, target.speed, target.action)
                    }
                } else if let Some((rx_id, rx_pos)) = &pass_receiver_override {
                    if rx_id == &player_id {
                        let (aim, spd) = self.receive_approach(*rx_pos, &player_id);
                        (aim, spd, "RECEIVE_CUT".to_string())
                    } else {
                        (target.target_pos, target.speed, target.action)
                    }
                } else if let Some(reb_spot) = rebound_chase_target {
                    let cur_dist = self
                        .systems
                        .physics
                        .get_player(&player_id)
                        .map(|p| (p.pos_ft - reb_spot).length())
                        .unwrap_or(99.0);
                    let is_tipoff_jumper = matches!(&self.ball.ball_state, BallTrajectoryKind::LooseBall { z, .. } if *z > 0.5)
                        && (player_id == home_jumper || player_id == away_jumper);
                    // ## 地板球追逐不受距离限制（round-17 活锁修复）
                    //
                    // 原实现的 `cur_dist <= 25.0` 硬半径在球停于空档区时失效：
                    // 实测 seed 6，罚球后松球停在 (85.6, 29.0)，最近球员
                    // 59.1 ft——无人满足 25 ft 条件 → 全场站桩 247 秒直到节末
                    // （POSSESSION_DURATION_BOUNDS Hard: 258.4s > 40s）。
                    //
                    // 第一性原理：活球是场上**唯一完全可观测**的对象（不是
                    // 任何人的私有信息），地板上躺着一颗活球时，「去抢球」
                    // 压倒一切战术站位——真实篮球里所有近处球员都会扑向球。
                    // 篮板追逐（RimRebound 的落点预判）保留 25 ft 半径；
                    // 松球（LooseBall）无条件追逐。
                    let is_live_loose_ball =
                        matches!(self.ball.ball_state, BallTrajectoryKind::LooseBall { .. });
                    if (is_live_loose_ball || cur_dist <= 25.0) && !is_tipoff_jumper {
                        (reb_spot, 16.0, "REBOUND_CRASH".to_string())
                    } else {
                        (target.target_pos, target.speed, target.action)
                    }
                } else {
                    (target.target_pos, target.speed, target.action)
                };
                let effective_speed = if action == "ROTATE_RIM_HELP"
                    || action == "X_OUT_CLOSEOUT"
                    || action == "HELP_SIDE_SHELL"
                {
                    let awareness = self
                        .systems
                        .physics
                        .get_player(&player_id)
                        .map(|p| {
                            nba_domain::capability::effective_help_awareness(
                                &self.config.rules,
                                &p.attributes,
                            )
                        })
                        .unwrap_or(0.5);
                    speed * (0.95 + awareness * 0.15)
                } else {
                    speed
                };
                self.systems.physics.set_player_target(
                    &player_id,
                    target_pos,
                    effective_speed,
                    &action,
                    &target.slot,
                    &target.morale,
                );
            }
        }
    }
}
