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

/// 转换跑动倾向缺省值（球员档案未携带倾向时的中性个体）。
const DEFAULT_TRANSITION_SPRINT: f32 = 0.5;

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
        // 槽位倾向（attributes.md §2.6 项 4/5）：按 slot fill 的绑定结果
        // （`filled_ids` 与 `spec.slots` 同序）传入，无球切入与掩护顺下
        // 的深度由占据该槽位的球员倾向调制。
        let slot_tendencies: Vec<nba_domain::PlayerTendencies> = filled_ids
            .iter()
            .filter_map(|id| {
                self.systems
                    .physics
                    .get_player(id)
                    .map(|p| p.tendencies.clone())
            })
            .collect();
        let mut off_targets = TacticalPlanner::plan_offense_from_spec_with_tendencies(
            off_spec,
            self.clock.sub_phase,
            self.flow.possession,
            carrier_slot,
            self.clock.sub_phase_timer,
            self.clock.frontcourt_seconds,
            &self.config.rules,
            Some(&slot_tendencies),
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
        // D26：突破球态传入弱侧激励——突破时弱侧防守人向篮筐收缩。
        let drive_active = matches!(&self.ball.ball_state, BallTrajectoryKind::Drive { .. });
        // 转换窗口：转换开始后的有限秒数内（与 `transition_finish_window`
        // 同一口径），进攻人的跑动速度受 `transition_sprint` 倾向调制。
        let in_transition = self
            .possession_ctx
            .transition_context_event(
                current_t,
                self.config.rules.tactics.transition_sprint_window_seconds,
            )
            .is_some();
        // 防守方倾向按名册顺序传入（与 `bind_targets_with_matchups` 的
        // 绑定口径一致）：协防触发阈值由个体倾向调制
        // （attributes.md §2.6 项 9）。
        let defending_roster = match self.flow.possession {
            Possession::Home => &self.config.away_roster_order,
            Possession::Away => &self.config.home_roster_order,
        };
        let defending_tendencies: Vec<nba_domain::PlayerTendencies> = defending_roster
            .iter()
            .filter_map(|id| {
                self.systems
                    .physics
                    .get_player(id)
                    .map(|p| p.tendencies.clone())
            })
            .collect();
        let (mut home_targets, mut away_targets) =
            TacticalPlanner::plan_possession_targets_with_rules_and_tendencies(
                self.config.tactical_set,
                self.clock.sub_phase,
                self.flow.possession,
                self.ball.ball_pos_3d.0,
                carrier_idx,
                self.clock.sub_phase_timer,
                self.clock.frontcourt_seconds,
                &mut self.systems.rng,
                &self.config.rules,
                Some(&live_off_positions),
                drive_active,
                Some(&defending_tendencies),
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
            TacticalPlanner::bind_targets_with_matchups(
                &mut away_targets,
                &away_roster,
                &home_roster,
            );
        } else {
            away_targets = off_targets;
            TacticalPlanner::bind_targets_with_matchups(
                &mut home_targets,
                &home_roster,
                &away_roster,
            );
        }
        let defending_targets = if self.flow.possession == Possession::Home {
            &away_targets
        } else {
            &home_targets
        };
        self.capture_potential_field_observations(defending_targets);
        self.track_defense_responsibilities(defending_targets);
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
        // 层 A（P-1 有限信息）：接球人**不得**直读传球人的冻结接球点。
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
            // 接球人标记已在 `transition_ball_state` 设置；估计点用于接球判定，
            // 跑位目标仍由战术系统生成。
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
        self.apply_active_play_actions(&mut home_targets, &mut away_targets, current_t);
        let rebound_chase_target = match &self.ball.ball_state {
            BallTrajectoryKind::RimRebound { target_landing, .. } => Some(*target_landing),
            BallTrajectoryKind::LooseBall { pos, .. } => Some(*pos),
            _ => None,
        };
        let home_jumper = self.select_jumper_id(Possession::Home);
        let away_jumper = self.select_jumper_id(Possession::Away);
        // 活球松球争抢优先（charter §6.2 松球事实）：最近代表必须在一切
        // 战术微操（切割路线残留、被过恢复）之前被派向球点——否则切割
        // 残留者的物理动作标签永远是切割类，`still_cutting` 检查永远
        // 通过，连最近的争抢代表也被 continue 跳过，球静躺在地上无人
        // 问津直到节末（实测 seed 11 p123 停滞 42 秒）。选择排除运动学
        // 锁定者（投篮/传球窗口内的人无法移动），每队各保证至少一人。
        let loose_chase_primary: Option<(Vec2, std::collections::HashSet<String>)> =
            if let BallTrajectoryKind::LooseBall { pos, z, .. } = &self.ball.ball_state {
                let spot = *pos;
                let ball_z = *z;
                let mut eligible: Vec<(f32, String, String)> = self
                    .systems
                    .physics
                    .get_players()
                    .values()
                    .filter(|p| p.on_court && !p.is_locked_kinematics)
                    .filter(|p| !(ball_z > 0.5 && (p.id == home_jumper || p.id == away_jumper)))
                    .map(|p| ((p.pos_ft - spot).length(), p.id.clone(), p.team.clone()))
                    .collect();
                eligible.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                let intercept = self.config.rules.intercept_lane_radius_ft;
                let mut primary: std::collections::HashSet<String> = eligible
                    .iter()
                    .filter(|(dist, _, _)| *dist <= intercept)
                    .map(|(_, id, _)| id.clone())
                    .collect();
                let mut seen_teams: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                for (_, id, team) in &eligible {
                    if seen_teams.insert(team.clone()) {
                        primary.insert(id.clone());
                    }
                }
                if primary.is_empty() {
                    None
                } else {
                    Some((spot, primary))
                }
            } else {
                None
            };
        for target in home_targets.into_iter().chain(away_targets) {
            if let Some(player_id) = target.player_id {
                if active_driver_id.as_deref() == Some(player_id.as_str()) {
                    continue;
                }
                // 活球松球争抢优先于一切战术微操：最近代表（循环前算好）
                // 直接奔赴球点，切割残留/被过恢复都不适用。
                if let Some((spot, primary)) = &loose_chase_primary {
                    if primary.contains(&player_id) {
                        let chase_base = self.config.rules.max_player_speed_ftps
                            * self.config.rules.tactics.rebound_chase_speed_ratio;
                        self.systems.physics.set_player_target(
                            &player_id,
                            *spot,
                            target.speed.max(chase_base),
                            "REBOUND_CRASH",
                            &target.slot,
                            &target.morale,
                        );
                        continue;
                    }
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
                let ball_in_hands_for_lock = matches!(
                    &self.ball.ball_state,
                    BallTrajectoryKind::Held { .. } | BallTrajectoryKind::Drive { .. }
                );
                let is_cut_route = self.observations.cut_route_players.contains(&player_id);
                let is_action_locked = ball_in_hands_for_lock
                    && self
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

                if in_active_window || is_action_locked || is_cut_route {
                    if !in_active_window && !is_action_locked {
                        let still_cutting = self
                            .systems
                            .physics
                            .get_player(&player_id)
                            .is_some_and(|player| {
                                matches!(
                                    player.action.as_str(),
                                    "BACKDOOR_CUT"
                                        | "DIP_TO_RIM"
                                        | "ROLL_TO_RIM"
                                        | "PLAY_CutBackdoor"
                                        | "PLAY_ScreenRoll"
                                )
                            });
                        if !still_cutting {
                            self.observations.cut_route_players.remove(&player_id);
                        }
                    }
                    continue;
                }

                // If tactical assignment is setting a high screen, initialize a ScreenSet action window.
                // 仅在已确立持球人时进入定点掩护，避免在争球或者无持球人阶段产生原地停摆。
                let has_carrier = matches!(
                    &self.ball.ball_state,
                    BallTrajectoryKind::Held { .. } | BallTrajectoryKind::InboundReady { .. }
                );
                if target.action == "SET_HIGH_SCREEN"
                    && !self.observations.active_windows.contains_key(&player_id)
                    && has_carrier
                {
                    self.start_action_window(
                        &player_id,
                        ActionTimeWindow::new_screen_set(&player_id, current_t, &self.config.rules),
                        None,
                        Some((target.target_pos.x, target.target_pos.y)),
                    );
                }

                // 第一性原理：后场推进期间不得被战术目标覆盖。
                //
                // `set_player_target` 每 tick 都执行；若持球人正在执行
                // `Advance`（把球推过中线），战术槽位（弧顶 x=66 等）
                // 会把目标改回半场站位，导致推进速度被反复打断
                // （实测仅 3.5–3.9 ft/s，而 8 秒规则需要 ≥4.5 ft/s）。
                // 球已脱手（LooseBall/Pass/Shot 等）时保护对象不存在，
                // 必须照常接受覆盖——否则被 Poke 松球后原推进者被永久
                // 跳过，连松球争抢分支也被跳过，实测 seed13 死锁 608s
                // （球静止在场上无人捡，直到 PERIOD_END 强制收束）。
                let ball_in_hands = matches!(
                    &self.ball.ball_state,
                    BallTrajectoryKind::Held { .. } | BallTrajectoryKind::Drive { .. }
                );
                let is_carrier_advancing = ball_in_hands
                    && self.observations.advancing_player.as_deref() == Some(player_id.as_str())
                    && self.clock.sub_phase != SubPhase::Initiation;
                if is_carrier_advancing {
                    continue;
                }
                let cut_action = matches!(
                    target.action.as_str(),
                    "BACKDOOR_CUT"
                        | "DIP_TO_RIM"
                        | "ROLL_TO_RIM"
                        | "PLAY_CutBackdoor"
                        | "PLAY_ScreenRoll"
                );
                if cut_action {
                    self.observations
                        .cut_route_players
                        .insert(player_id.clone());
                }
                let (target_pos, speed, action) = if let Some((inb_id, inb_pos)) =
                    &inbounder_override
                {
                    if inb_id == &player_id {
                        (*inb_pos, 15.0, "INBOUND_SETUP".to_string())
                    } else {
                        (target.target_pos, target.speed, target.action)
                    }
                } else if let Some((receiver_id, estimate)) = &pass_receiver_override {
                    if receiver_id == &player_id {
                        let (aim, speed) = self.receive_approach(*estimate, &player_id);
                        (aim, speed, "RECEIVE_CUT".to_string())
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

                    // 篮板反弹（RimRebound）：近身者可加入冲抢；松球的最近
                    // 代表已在循环前直接派发（loose_chase_primary），这里
                    // 只处理篮板与残余近身拦截。
                    let is_primary_chaser = if is_tipoff_jumper {
                        false
                    } else {
                        cur_dist <= 25.0
                    };

                    if is_primary_chaser && !is_tipoff_jumper {
                        let chase_base = self.config.rules.max_player_speed_ftps
                            * self.config.rules.tactics.rebound_chase_speed_ratio;
                        let chase_speed = chase_base.max(0.0);
                        (reb_spot, chase_speed, "REBOUND_CRASH".to_string())
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
                // 转换跑动倾向（attributes.md §2.6 项 11）：转换窗口内
                // 进攻人按 `transition_sprint` 缩放跑动速度——快的人冲得
                // 更快，慢的人在后面跑。速度比走 `tactics.transition_sprint_ratio`
                // 通道（团队基准），倾向是个体修正。
                let effective_speed = if in_transition
                    && self
                        .systems
                        .physics
                        .get_player(&player_id)
                        .is_some_and(|p| {
                            p.team
                                == match self.flow.possession {
                                    Possession::Home => "home",
                                    Possession::Away => "away",
                                }
                        }) {
                    let sprint_tendency = self
                        .systems
                        .physics
                        .get_player(&player_id)
                        .map(|p| p.tendencies.transition_sprint)
                        .unwrap_or(DEFAULT_TRANSITION_SPRINT);
                    let sprint_scale = self.config.rules.tactics.transition_sprint_floor
                        + sprint_tendency * self.config.rules.tactics.transition_sprint_span;
                    effective_speed * sprint_scale
                } else {
                    effective_speed
                };
                // 回场条款的运动层体现（charter §6.2）：前场已建立时，持球人
                // 的战术目标不得指向后场——真实球员知道规则，不会把球
                // 带回后场。只改目标不做位置钳制，避免制造速度瞬移。
                let target_pos = if self.clock.frontcourt_established
                    && self.flow.game_flow.allows_live_ball_actions()
                    && matches!(
                        &self.ball.ball_state,
                        BallTrajectoryKind::Held { carrier_id }
                            | BallTrajectoryKind::Drive { driver_id: carrier_id, .. }
                            if carrier_id == &player_id
                    )
                    && self
                        .config
                        .rules
                        .court
                        .is_backcourt(target_pos, self.flow.possession == Possession::Home)
                {
                    let midcourt = self.config.rules.court.width_ft * 0.5;
                    // 中线本身不属于后场（两侧 `is_backcourt` 均为假），
                    // 把目标钳到中线即回到合法前场。
                    glam::Vec2::new(midcourt, target_pos.y)
                } else {
                    target_pos
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

    fn apply_active_play_actions(
        &mut self,
        home_targets: &mut [nba_decision::tactics::TargetAssignment],
        away_targets: &mut [nba_decision::tactics::TargetAssignment],
        current_t: f32,
    ) {
        let possession = self.flow.possession;
        if self.clock.sub_phase != SubPhase::ActionExecution
            || self.flow.game_flow != nba_domain::GameFlowState::LiveBall
        {
            return;
        }
        let carrier_id = match &self.ball.ball_state {
            BallTrajectoryKind::Held { carrier_id } => Some(carrier_id.as_str()),
            BallTrajectoryKind::Drive { driver_id, .. } => Some(driver_id.as_str()),
            BallTrajectoryKind::ControlTransfer { carrier_id, .. } => Some(carrier_id.as_str()),
            _ => None,
        };
        let Some(carrier_id) = carrier_id else {
            return;
        };
        let Some(selection_context) = self.build_play_selection_context(carrier_id) else {
            return;
        };
        let (play_id, actions) = {
            let active = match possession {
                Possession::Home => self.observations.home_active_play.as_ref(),
                Possession::Away => self.observations.away_active_play.as_ref(),
            };
            let Some(active) = active else {
                return;
            };
            (
                active.spec.id.clone(),
                nba_decision::evaluate_active_play(&active.spec, &selection_context)
                    .matched_rule_actions()
                    .to_vec(),
            )
        };
        let offense_targets = match possession {
            Possession::Home => home_targets,
            Possession::Away => away_targets,
        };
        let mut windows_to_start = Vec::new();
        for action in actions {
            let target = offense_targets
                .iter_mut()
                .find(|target| target.slot == action.slot)
                .unwrap_or_else(|| {
                    panic!(
                        "play `{}` rule `{}` references missing slot `{}` in active System",
                        play_id, action.rule_id, action.slot
                    )
                });
            let player_id = target
                .player_id
                .as_deref()
                .unwrap_or_else(|| panic!("play slot `{}` has no filled player", action.slot))
                .to_string();
            // 松球争抢优先于 Play 跑位：本 tick 战术规划已把松球最近的球员
            // 派去捡球（REBOUND_CRASH），Play 的槽位动词不得把他拉回站位——
            // 否则球静止在场上无人捡（实测 seed13 死锁 608s 直到 PERIOD_END）。
            if self
                .systems
                .physics
                .get_player(&player_id)
                .is_some_and(|player| player.action == "REBOUND_CRASH")
            {
                continue;
            }
            let actor = self
                .systems
                .physics
                .get_player(&player_id)
                .unwrap_or_else(|| {
                    panic!("play slot `{}` player `{player_id}` is absent", action.slot)
                });
            let carrier_id = self.active_carrier_or_focus_id();
            let carrier = self
                .systems
                .physics
                .get_player(&carrier_id)
                .unwrap_or_else(|| panic!("play carrier `{carrier_id}` is absent"));
            let opponent = self
                .systems
                .physics
                .get_players()
                .values()
                .filter(|player| player.on_court && player.team != actor.team)
                .min_by(|left, right| {
                    let left_dist = (left.pos_ft - actor.pos_ft).length();
                    let right_dist = (right.pos_ft - actor.pos_ft).length();
                    left_dist
                        .total_cmp(&right_dist)
                        .then(left.id.cmp(&right.id))
                });
            let is_home = actor.team == "home";
            let context = nba_decision::play_actions::VerbContext {
                actor_pos: actor.pos_ft,
                hoop_pos: self.config.rules.court.hoop_pos(is_home),
                carrier_pos: carrier.pos_ft,
                defender_pos: opponent.map(|defender| defender.pos_ft),
                court: self.config.rules.court,
                clamp_margin_ft: self.config.rules.player_radius_ft,
                three_point_distance_ft: self.config.rules.league.three_point_distance_ft,
            };
            let resolution = nba_decision::play_actions::resolve_verb(action.verb, &context);
            let next_target =
                nba_decision::play_actions::resolve_verb_target(action.verb, &context);
            let verb_progress = (self.observations.possession_ticks as f32
                * self.config.rules.tick_seconds)
                .min(self.config.rules.tactics.action_duration_seconds);
            let should_start_window = verb_progress
                >= self.config.rules.tactics.action_duration_seconds
                && !self.observations.active_windows.contains_key(&player_id);
            let target_speed = target.speed;
            let target_slot = target.slot.clone();
            let target_morale = target.morale.clone();
            target.target_pos = next_target;
            target.action = format!("PLAY_{}", action.verb.as_str());
            self.systems.physics.set_player_target(
                &player_id,
                next_target,
                target_speed,
                &target.action,
                &target_slot,
                &target_morale,
            );
            if should_start_window {
                let window = if let Some(action_type) = resolution.window_action {
                    match action_type {
                        nba_domain::action_window::ActionType::JumpShot => {
                            Some(nba_domain::action_window::ActionTimeWindow::new_jump_shot(
                                &player_id,
                                current_t,
                                &self.config.rules,
                            ))
                        }
                        nba_domain::action_window::ActionType::PassRelease => {
                            Some(nba_domain::action_window::ActionTimeWindow::new_pass(
                                &player_id,
                                current_t,
                                &self.config.rules,
                            ))
                        }
                        nba_domain::action_window::ActionType::ScreenSet => {
                            Some(nba_domain::action_window::ActionTimeWindow::new_screen_set(
                                &player_id,
                                current_t,
                                &self.config.rules,
                            ))
                        }
                        nba_domain::action_window::ActionType::ScreenRoll => Some(
                            nba_domain::action_window::ActionTimeWindow::new_screen_roll(
                                &player_id,
                                current_t,
                                &self.config.rules,
                            ),
                        ),
                        nba_domain::action_window::ActionType::ScreenPop => {
                            Some(nba_domain::action_window::ActionTimeWindow::new_screen_pop(
                                &player_id,
                                current_t,
                                &self.config.rules,
                            ))
                        }
                        nba_domain::action_window::ActionType::CutBackdoor => Some(
                            nba_domain::action_window::ActionTimeWindow::new_cut_backdoor(
                                &player_id,
                                current_t,
                                &self.config.rules,
                            ),
                        ),
                        nba_domain::action_window::ActionType::Cut => {
                            Some(nba_domain::action_window::ActionTimeWindow::new_cut(
                                &player_id,
                                current_t,
                                &self.config.rules,
                            ))
                        }
                        nba_domain::action_window::ActionType::BoxOut => {
                            Some(nba_domain::action_window::ActionTimeWindow::new_box_out(
                                &player_id,
                                current_t,
                                &self.config.rules,
                            ))
                        }
                        nba_domain::action_window::ActionType::Putback => {
                            Some(nba_domain::action_window::ActionTimeWindow::new_putback(
                                &player_id,
                                current_t,
                                &self.config.rules,
                            ))
                        }
                        nba_domain::action_window::ActionType::Layup => {
                            Some(nba_domain::action_window::ActionTimeWindow::new_layup(
                                &player_id,
                                current_t,
                                &self.config.rules,
                            ))
                        }
                        nba_domain::action_window::ActionType::Dunk => {
                            Some(nba_domain::action_window::ActionTimeWindow::new_dunk(
                                &player_id,
                                current_t,
                                &self.config.rules,
                            ))
                        }
                        nba_domain::action_window::ActionType::CloseoutContest => Some(
                            nba_domain::action_window::ActionTimeWindow::new_closeout_contest(
                                &player_id,
                                current_t,
                                &self.config.rules,
                            ),
                        ),
                        nba_domain::action_window::ActionType::ReboundJump => Some(
                            nba_domain::action_window::ActionTimeWindow::new_rebound_jump(
                                &player_id,
                                current_t,
                                &self.config.rules,
                            ),
                        ),
                    }
                } else {
                    match action.verb {
                        nba_domain::play::PlayVerb::ScreenRoll => Some(
                            nba_domain::action_window::ActionTimeWindow::new_screen_roll(
                                &player_id,
                                current_t,
                                &self.config.rules,
                            ),
                        ),
                        nba_domain::play::PlayVerb::CutBackdoor => Some(
                            nba_domain::action_window::ActionTimeWindow::new_cut_backdoor(
                                &player_id,
                                current_t,
                                &self.config.rules,
                            ),
                        ),
                        _ => None,
                    }
                };
                if let Some(w) = window {
                    windows_to_start.push((player_id, w, next_target));
                }
            }
        }
        for (pid, w, next_target) in windows_to_start {
            self.start_action_window(&pid, w, None, Some((next_target.x, next_target.y)));
        }
    }

    fn capture_potential_field_observations(
        &mut self,
        defending_targets: &[nba_decision::tactics::TargetAssignment],
    ) {
        self.observations.potential_field.clear();
        for assignment in defending_targets {
            let Some(potential) = assignment.potential_field.as_ref() else {
                continue;
            };
            let Some(player_id) = assignment.player_id.as_deref() else {
                continue;
            };
            let Some(player) = self.systems.physics.get_player(player_id) else {
                continue;
            };
            if !player.on_court {
                continue;
            }
            let solver = nba_decision::DefensePotentialFieldSolver::new(
                self.config.rules.tactics.defense.potential_field,
            );
            solver.observe_field(
                potential,
                self.observations
                    .field_hysteresis
                    .entry(player.id.clone())
                    .or_default(),
            );
            self.observations
                .potential_field
                .push(super::state::PotentialFieldObservation {
                    player_id: player.id.clone(),
                    team: player.team.clone(),
                    position: player.pos_ft,
                    target: potential.target_pos,
                    drive: potential.drive,
                    action: potential.action.to_string(),
                    threat_ratio: potential.threat_ratio,
                    void_ratio: potential.void_ratio,
                });
        }
    }

    fn track_defense_responsibilities(
        &mut self,
        defending_targets: &[nba_decision::tactics::TargetAssignment],
    ) {
        let mut events = Vec::new();
        for target in defending_targets {
            let Some(player_id) = &target.player_id else {
                continue;
            };
            let Some(new_resp) = target.responsibility else {
                continue;
            };
            let old_resp = self
                .observations
                .defense_responsibilities
                .get(player_id)
                .copied()
                .unwrap_or(nba_domain::DefenseResponsibility::PrimaryMatchup);
            if old_resp != new_resp {
                self.observations
                    .defense_responsibilities
                    .insert(player_id.clone(), new_resp);
                let off_id = target.offensive_player_id.clone().unwrap_or_default();
                events.push(nba_domain::GameEvent::DefenseResponsibilityChanged {
                    defender_id: player_id.clone(),
                    old_responsibility: old_resp,
                    new_responsibility: new_resp,
                    offensive_player_id: off_id,
                    prior_defender_id: None,
                    trigger: Some(target.action.clone()),
                });
            }
        }
        self.journal.pending_events.extend(events);
    }
}
