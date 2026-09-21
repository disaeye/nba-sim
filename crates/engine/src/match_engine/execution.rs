//! 动作执行：把决策输出的候选动作写入权威状态与事件。
//!
//! 依据 `docs/architecture.md` §5.2 的执行重校验原则：本模块只负责**执行**，
//! 候选已经过约束管线；执行点不再重新决策，只把动作写入球态、动作窗口与事件流。

use glam::Vec2;
use nba_decision::constraint::CandidateAction;
use nba_decision::pipeline::DecisionOutput;
use nba_domain::action_window::ActionTimeWindow;
use nba_domain::court::Court;
use nba_domain::{GameEvent, GameFlowState, Possession, SubPhase};
use nba_officiating::resolution::DriveResolution;
use nba_physics::ballistics::{BallTrajectoryKind, BallisticsEngine};
use nba_semantics::SemanticEvaluator;
use rand::Rng;

use super::projection::convert_trace;
use super::MatchEngine;

impl MatchEngine {
    pub(crate) fn execute_drive(
        &mut self,
        driver_id: &str,
        from_pos: Vec2,
        target_pos: Vec2,
        move_kind: Option<nba_domain::action_window::DribbleMoveKind>,
        current_t: f32,
    ) {
        let Some(driver) = self.systems.physics.get_player(driver_id).cloned() else {
            return;
        };
        let defender = self.systems.physics.openness(driver_id);
        let lane_density = SemanticEvaluator::spacing(
            self.flow.possession,
            target_pos,
            &self.systems.physics,
            &self.config.rules,
        )
        .paint_crowding;
        let stamina = (driver.stamina / driver.max_stamina.max(f32::EPSILON)).clamp(0.0, 1.0);
        let defender_id = defender.closest_defender_id.clone();
        let foul_rate = defender_id
            .as_ref()
            .map(|_| self.config.rules.resolve.base_rates.foul_on_drive_rate)
            .unwrap_or(0.0);
        let resolution = DriveResolution::resolve(
            self.config.rules.resolve.base_rates.drive_success,
            self.config.rules.resolve.shot_type_rates.drive_finish_2pt,
            foul_rate,
            driver.attributes.finishing,
            stamina,
            lane_density,
            defender.contest_intensity,
            self.config.rules.resolve.shot_type_block_bias.drive_finish,
            self.config.rules.resolve.player_skill.finishing_weight,
            &self.config.rules.resolve.drive,
            &mut self.systems.rng,
        );
        let fouler_id = resolution.shooting_foul.then_some(defender_id).flatten();
        let target_pos = self
            .config
            .rules
            .court
            .clamp_playable(target_pos, self.config.rules.player_radius_ft);
        let drive_dist = (target_pos - from_pos).length();
        let drive_speed =
            (driver.max_speed_ftps * self.config.rules.tactics.drive_speed_ratio).max(1.0);
        // ## 时长必须包含加速坡（round-18）
        //
        // 原公式 `dist/speed` 假设瞬时达到极速。实测从静止加速
        // （max_player_accel 35 ft/s²）到 ~25 ft/s 需 ~0.7s、损失 ~9 ft
        // 里程——tau=1 时持球人仍距目标 5-10 ft，只能在 7-16 ft 抛投
        // （篮下≤4ft 出手占比 2.4%，真实 25-50%）。加入 `speed/accel`
        // 的加速坡项，使时长覆盖真实到达时间。
        let accel = self.config.rules.max_player_accel_ftps2.max(f32::EPSILON);
        let drive_duration = (drive_dist / drive_speed + drive_speed / accel).clamp(
            self.config.rules.tactics.drive_min_duration_seconds,
            self.config.rules.tactics.drive_max_duration_seconds,
        );
        let action_str = match move_kind {
            Some(nba_domain::action_window::DribbleMoveKind::Crossover) => "Crossover",
            Some(nba_domain::action_window::DribbleMoveKind::BetweenTheLegs) => "BetweenTheLegs",
            Some(nba_domain::action_window::DribbleMoveKind::BehindTheBack) => "BehindTheBack",
            Some(nba_domain::action_window::DribbleMoveKind::SpinMove) => "SpinMove",
            _ => "DriveToBasket",
        };
        let callout_text = match move_kind {
            Some(nba_domain::action_window::DribbleMoveKind::Crossover) => {
                format!("{} 变向晃开防守，大幅变向突破！", driver.jersey)
            }
            Some(nba_domain::action_window::DribbleMoveKind::BetweenTheLegs) => {
                format!("{} 胯下换手运球，加速直插内线！", driver.jersey)
            }
            Some(nba_domain::action_window::DribbleMoveKind::BehindTheBack) => {
                format!("{} 背后运球摆脱，直切篮下！", driver.jersey)
            }
            Some(nba_domain::action_window::DribbleMoveKind::SpinMove) => {
                format!("{} 陀螺转身过人，撕裂防线！", driver.jersey)
            }
            _ => format!("{} 持球强突，冲击篮筐！", driver.jersey),
        };
        // ## 被过掉的防守人必须真的被过掉（round-18）
        //
        // 根因链：突破预掷 `successful=true`，但物理层持球人直撞贴身防守人
        // （最近防守人 3.7 ft ≈ min_player_separation），碰撞消解每 tick
        // 清零速度——实测 0% 突破到达 ≤4.5ft，65% 停在离筐 18.6 ft。
        //
        // 语义一致性（与拦截锚定球到抢断者同一原则——概率在事件时刻裁定，
        // 事实随后回放）：`successful` 意味着**过掉了对位防守人**。把该
        // 防守人实际位移出突破走廊（侧向 + 分离余量），他随后由防守战术
        // 重新追防——这正是真实篮球「被过掉后回追」的几何。
        let mut target_pos_override: Option<Vec2> = None;
        if resolution.successful {
            // ## 过人变向（round-18 修订版：绕行而非瞬移）
            //
            // 初版把被过的防守人瞬移出通道——触发 PLAYER_TELEPORT 不变量
            // （实测 70-94 Hard/seed）。不变量是对的：位置跳变是伪造事实。
            //
            // 物理一致的过人语义：**持球人变向绕过**防守人（真实的 crossover
            // 几何）。突破目标点侧移一个分离余量，路径绕开贴身防守人；
            // 防守人随后由战术层追防（真实「被过掉后回追」）。
            let drive_dir = (target_pos - from_pos).normalize_or_zero();
            let perp = Vec2::new(-drive_dir.y, drive_dir.x);
            // ## 让位整条通道（round-19：从单人到全体）
            //
            // round-18 只让位**最近的一名**通道内防守人——过掉第一人对
            // 后，护框者（第二道防线）仍在篮下挡住最后几米，实测篮下
            // ≤4ft 出手仅 3.8%（真实 25-50%）。
            //
            // 语义：`successful` 预掷的是「这次突破**整体**打成了」——
            // 包括过掉对位人与顶开/绕过护框。因此通道内**所有**防守人
            // 都应让位（各自向远离突破方向的侧向清空点极速移动）；
            // 对抗强度已由 successful 的掷骰（防守能力加权）承担，
            // 几何层只负责让事实成立。
            let clear = self.config.rules.min_player_separation_ft * 2.0
                + self.config.rules.player_radius_ft;
            let mut beaten_ids: Vec<String> = Vec::new();
            let mut beat_spots: Vec<(String, Vec2, f32, String, String)> = Vec::new();
            for q in self.systems.physics.get_players().values() {
                if !q.on_court || q.team == driver.team {
                    continue;
                }
                let rel = q.pos_ft - from_pos;
                let along = rel.dot(drive_dir);
                let lateral = (rel - drive_dir * along).length();
                if along > 0.0
                    && along < drive_dist
                    && lateral < self.config.rules.tactics.drive_lane_offset_ft.max(4.0)
                {
                    let side = if perp.dot(rel) >= 0.0 { -1.0 } else { 1.0 };
                    let spot = self.config.rules.court.clamp_playable(
                        q.pos_ft + perp * side * clear,
                        self.config.rules.player_radius_ft,
                    );
                    beat_spots.push((
                        q.id.clone(),
                        spot,
                        q.max_speed_ftps,
                        q.slot.clone(),
                        q.morale.clone(),
                    ));
                    beaten_ids.push(q.id.clone());
                }
            }
            // 主对位人（离持球人最近者）驱动两段式过人几何；其余
            // （护框者等）只让位，不参与重定向判定。
            let primary = self
                .systems
                .physics
                .get_players()
                .values()
                .filter(|q| beaten_ids.contains(&q.id))
                .min_by(|a, b| {
                    (a.pos_ft - from_pos)
                        .length_squared()
                        .partial_cmp(&(b.pos_ft - from_pos).length_squared())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|q| q.id.clone());
            for (bid, spot, sp, slot, morale) in beat_spots {
                self.systems.physics.set_player_target(
                    &bid,
                    spot,
                    sp,
                    "BeatenRecovery",
                    &slot,
                    &morale,
                );
                self.observations.beaten_recovery_until.insert(
                    bid,
                    self.clock.current_time
                        + self.config.rules.tactics.drive_beaten_recovery_seconds,
                );
            }
            if let Some(pid) = primary {
                if let Some(q) = self.systems.physics.get_player(&pid) {
                    let side = if perp.dot(q.pos_ft - from_pos) >= 0.0 {
                        -1.0
                    } else {
                        1.0
                    };
                    let beat_spot = self.config.rules.court.clamp_playable(
                        q.pos_ft + perp * side * clear,
                        self.config.rules.player_radius_ft,
                    );
                    // 两段式过人几何（真实 crossover）：
                    //   第一段：持球人目标 = 过人点；第二段：重定向攻框。
                    target_pos_override = Some(beat_spot);
                    self.ball.beaten_defender_id = Some(pid);
                }
            }
        }
        let initial_target = target_pos_override.unwrap_or(target_pos);
        self.systems.physics.set_player_target(
            driver_id,
            initial_target,
            drive_speed,
            action_str,
            "BallHandler",
            &driver.morale,
        );
        self.transition_ball_state(BallTrajectoryKind::Drive {
            driver_id: driver_id.to_string(),
            from_pos,
            target_pos,
            move_kind,
            start_time: current_t,
            duration: drive_duration,
            successful: resolution.successful,
            finish_made: resolution.finish_made,
            fouler_id,
        });
        self.ball.ball_pos_3d = (from_pos, self.config.rules.ball_holder_height_ft);
        self.transition_phase(SubPhase::ActionExecution);
        self.journal.pending_events.push(GameEvent::DriveInitiated {
            driver_id: driver_id.to_string(),
            from_pos: (from_pos.x, from_pos.y),
            target_pos: (target_pos.x, target_pos.y),
        });
        self.journal.current_event = Some("DRIVE_INITIATED".to_string());
        self.journal.current_callout = Some(callout_text);
        self.journal.current_intensity = Some("Climax".to_string());
    }

    /// Resolve shot quality at release; the resulting outcome is then replayable
    /// independently of the later presentation trajectory.
    pub(crate) fn execute_shot(
        &mut self,
        shooter_id: &str,
        from_pos: Vec2,
        is_three_hint: bool,
        jumper_kind: Option<nba_domain::action_window::JumperKind>,
        current_t: f32,
    ) {
        let is_home = self.flow.possession == Possession::Home;
        let hoop = self.config.rules.court.hoop_pos(is_home);
        let shooter = self.systems.physics.get_player(shooter_id);
        let shooter_pos = shooter.map(|player| player.pos_ft).unwrap_or(from_pos);
        let dist_to_hoop = (shooter_pos - hoop).length();
        // 底角三分是更近的直线（NBA 22ft vs 弧顶 23.75ft），必须几何判定。
        let is_three_by_distance = self.config.rules.court.is_three_point_attempt(
            shooter_pos,
            is_home,
            self.config.rules.league.three_point_distance_ft,
            self.config.rules.league.corner_three_distance_ft,
        );
        let is_three = is_three_by_distance || is_three_hint;
        let openness = self.systems.physics.openness(shooter_id);
        let spacing_bonus = self
            .observations
            .latest_spacing
            .map(|spacing| spacing.shot_quality_bonus)
            .unwrap_or(0.0);
        let skill = shooter
            .map(|player| {
                if dist_to_hoop < self.config.rules.rim_shot_distance_ft {
                    // 近筐出手按**对抗强度**在两维技能间过渡
                    // （attributes.md §2.3 的可辨识性配对）：
                    //
                    // - `shooting_close` ↔ 非对抗近筐（挑篮/勾手）
                    // - `finishing`      ↔ 对抗近筐（顶人上篮 / and-1）
                    //
                    // 两维解释同一出手族，若只用其一会使另一维成为无效维度，
                    // 且使「无对抗的近筐准度」与「对抗下的完成度」不可区分。
                    // 过渡权重取自 `contest_intensity`（距离 + 朝向 + 逼近速度），
                    // 不另立阈值。
                    let contest = openness
                        .contest_intensity
                        .clamp(f32::from(0u8), f32::from(1u8));
                    let uncontested = f32::from(1u8) - contest;
                    player.attributes.shooting_close * uncontested
                        + player.attributes.finishing * contest
                } else if is_three {
                    player.attributes.shooting_three
                } else {
                    player.attributes.shooting_mid
                }
            })
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);
        let stamina = shooter
            .map(|player| (player.stamina / player.max_stamina.max(f32::EPSILON)).clamp(0.0, 1.0))
            .unwrap_or(1.0);
        let skill_adjustment =
            (skill - 0.5) * self.config.rules.resolve.player_skill.shooting_weight * 2.0;
        let stamina_adjustment =
            (stamina - 1.0) * self.config.rules.resolve.player_skill.shooting_weight;
        let contest_penalty =
            openness.contest_intensity * self.config.rules.shot_contest_sensitivity;
        // 分区命中基准（charter C1：三种基准走 GameRules 数据通道）：
        //   廊下      dist < rim_shot_distance_ft        -> shot_make_2pt
        //   中距离    rim 以外、三分线以内             -> shot_make_mid
        //   三分      is_three                          -> shot_make_3pt
        // 此前中距离与廊下共用 shot_make_2pt，使 8ft–三分线的出手被按廊下
        // 结算（真实 0.42 vs 0.63），形成结构性高估：evidence/problem.md §21.3。
        let base_fg = if dist_to_hoop < self.config.rules.rim_shot_distance_ft {
            self.config.rules.resolve.base_rates.shot_make_2pt
        } else if is_three {
            self.config.rules.resolve.base_rates.shot_make_3pt
        } else {
            self.config.rules.resolve.base_rates.shot_make_mid
        };
        let final_fg_pct = (base_fg + skill_adjustment + stamina_adjustment + spacing_bonus
            - contest_penalty)
            .clamp(
                self.config.rules.shot_pct_floor,
                self.config.rules.shot_pct_ceiling,
            );
        let is_made = self.systems.rng.gen_bool(final_fg_pct as f64);

        // ## 跳投犯规（evidence/problem.md §23.9）
        //
        // 此前全仓 `shooting_foul` 只在 `DriveResolution` 产生，即**只有突破
        // 能被犯规**；跳投（含三分）在被干扰时没有任何造犯规可能。实测 seed42
        // 全场仅 12 次犯规（全部来自突破，真实 NBA 约 40），使
        // `free_throw_rate` 只有 0.110（带 [0.20, 0.35]）。
        //
        // 判定口径：犯规概率 = 基准 × 干扰强度。干扰是自变量（不受干扰的空位
        // 跳投不会被犯规），基准经 GameRules 通道（charter C1）。
        // 犯规与命中相互独立，因此不能从 `is_made` 反推——
        // and-one（犯规且命中）与投篮犯规（犯规且不中）都要能表达。
        let foul_probability = (self.config.rules.resolve.base_rates.foul_on_shot_rate
            * openness.contest_intensity)
            .clamp(f32::EPSILON, f32::from(1u8));
        let fouled = self.systems.rng.gen_bool(foul_probability as f64);
        let fouler_id = if fouled {
            openness.closest_defender_id.clone()
        } else {
            None
        };
        // 没有防守人在附近就不可能犯规（与概率为 0 一致，防御性一致）。
        let fouled = fouled && fouler_id.is_some();

        // 峰值必须服从规则通道的高度上限：base + dist×factor 在超远距离
        // （约 >84ft）会算出高于 `ball_z_max_ft` 的弧顶，直接违反
        // BALL_HEIGHT_BOUNDS（本轮 seed 6 full 实测 35.17ft > 35.0ft）。
        // 在生成端收敛到上限，而不是事后由不变量检查器发现。
        let peak_z = (self.config.rules.shot_peak_base_ft
            + dist_to_hoop * self.config.rules.shot_peak_distance_factor)
            .min(self.config.rules.ball_z_max_ft);
        let flight_time = BallisticsEngine::shot_duration(dist_to_hoop, peak_z, &self.config.rules);
        self.observations.active_windows.insert(
            shooter_id.to_string(),
            ActionTimeWindow::new_jump_shot(shooter_id, current_t, &self.config.rules),
        );
        self.possession_ctx.current_possession_shooter = Some(shooter_id.to_string());
        self.possession_ctx.current_possession_contest = Some(openness.contest_intensity);

        let release_pos = self.ball.ball_pos_3d.0;
        self.transition_ball_state(BallTrajectoryKind::Shot {
            shooter_id: shooter_id.to_string(),
            from_pos: release_pos,
            hoop_pos: hoop,
            start_time: current_t,
            duration: flight_time,
            is_made,
            is_three,
            peak_z,
            fouled,
            fouler_id,
        });
        self.transition_phase(SubPhase::ShotAttempt);
        self.journal.pending_events.push(GameEvent::ShotRelease {
            shooter_id: shooter_id.to_string(),
            pos: (release_pos.x, release_pos.y),
            is_three,
            contest_level: openness.contest_intensity,
            make_probability: final_fg_pct,
        });
        let action_name = match jumper_kind {
            Some(nba_domain::action_window::JumperKind::StepBack) => "StepBackShot",
            Some(nba_domain::action_window::JumperKind::PullUp) => "PullUpShot",
            Some(nba_domain::action_window::JumperKind::TurnaroundFadeaway) => "TurnaroundFadeaway",
            _ => {
                if is_three {
                    "ThreePointShot"
                } else {
                    "JumpShot"
                }
            }
        };
        let callout_detail = match jumper_kind {
            Some(nba_domain::action_window::JumperKind::StepBack) => {
                "撤步拉开空间，命中高难度后撤步！"
            }
            Some(nba_domain::action_window::JumperKind::PullUp) => "急停干拔，教科书般起跳出手！",
            Some(nba_domain::action_window::JumperKind::TurnaroundFadeaway) => {
                "翻身极致后仰，飘逸出手！"
            }
            _ => {
                if is_three {
                    "果断张手三分出手！"
                } else {
                    "迎着防守干拔跳投！"
                }
            }
        };
        if let Some(p) = self.systems.physics.get_player_mut(shooter_id) {
            p.action = action_name.to_string();
            let hoop_dir = (hoop - p.pos_ft).normalize_or_zero();
            if hoop_dir.length_squared() > 0.1 {
                p.facing_dir = hoop_dir;
            }
        }
        self.possession_ctx.current_possession_turnover_player = Some(shooter_id.to_string());
        let shooter_name = self
            .systems
            .physics
            .get_player(shooter_id)
            .map(|p| p.jersey.clone())
            .unwrap_or_else(|| shooter_id.to_string());
        self.journal.current_callout = Some(format!("{} {}", shooter_name, callout_detail));
        self.journal.current_intensity = Some("Climax".to_string());
    }

    /// 执行决策输出（意图执行重校验，architecture.md §5.2）。
    ///
    /// 硬约束校验失败时拒绝执行该动作并保留 Dwell 保护，但 trace 仍必须
    /// 完整发布（architecture.md §5.3：禁止「决策了但没有 trace」的路径），
    /// 供评判器统计「执行时改变」率。
    pub(crate) fn apply_decision_output(&mut self, out: DecisionOutput, current_t: f32) {
        let ctx = self.constraint_ctx();
        if let Err(blocked_reason) = self
            .systems
            .decision
            .registry
            .revalidate_intent(&ctx, &out.action)
        {
            self.journal
                .current_enforcements
                .push(format!("INTENT_REVALIDATION_BLOCKED:{}", blocked_reason));
            let mut debug = convert_trace(&out.trace);
            debug
                .enforcement
                .extend(self.journal.current_enforcements.iter().cloned());
            self.observations.last_decision_trace = Some(Box::new(debug));
            return;
        }
        let trace = out.trace.clone();
        match out.action {
            CandidateAction::Shoot {
                shooter_id,
                from_pos,
                is_three,
                jumper_kind,
            } => {
                self.execute_shot(&shooter_id, from_pos, is_three, jumper_kind, current_t);
            }
            CandidateAction::Drive {
                driver_id,
                from_pos,
                target_pos,
                move_kind,
            } => {
                self.execute_drive(&driver_id, from_pos, target_pos, move_kind, current_t);
            }
            CandidateAction::Pass {
                passer_id,
                receiver_id,
                from_pos,
                to_pos,
            } => {
                self.execute_pass(&passer_id, &receiver_id, from_pos, to_pos, current_t, false);
            }
            CandidateAction::InboundPass {
                passer_id,
                receiver_id,
                from_pos,
                to_pos,
            } => {
                self.execute_pass(&passer_id, &receiver_id, from_pos, to_pos, current_t, true);
            }
            CandidateAction::Dwell { .. } => {
                // 观察等待：无操作。
            }
            CandidateAction::Advance {
                player_id,
                target_pos,
                ..
            } => {
                // 推进过半场：以运球速度把持球人朝中线方向驱动。
                let target = self
                    .config
                    .rules
                    .court
                    .clamp_playable(target_pos, self.config.rules.player_radius_ft);
                let morale = self
                    .systems
                    .physics
                    .get_player(&player_id)
                    .map(|p| p.morale.clone())
                    .unwrap_or_else(|| "Normal".to_string());
                self.systems.physics.set_player_target(
                    &player_id,
                    target,
                    self.config.rules.max_player_speed_ftps
                        * self.config.rules.tactics.carrier_speed_ratio,
                    "Advance",
                    "BallHandler",
                    &morale,
                );
                if let Some(p) = self.systems.physics.get_player_mut(&player_id) {
                    p.action = "ADVANCE".to_string();
                }
                self.observations.advancing_player = Some(player_id.clone());
                self.journal.current_callout = Some("持球推进，尽快越过中线！".to_string());
                self.journal.current_event = Some("ADVANCE".to_string());
                self.clock.last_decision_time = current_t;
            }
            CandidateAction::PostUp {
                player_id,
                target_pos,
                ..
            } => {
                let target = self
                    .config
                    .rules
                    .court
                    .clamp_playable(target_pos, self.config.rules.player_radius_ft);
                let morale = self
                    .systems
                    .physics
                    .get_player(&player_id)
                    .map(|p| p.morale.clone())
                    .unwrap_or_else(|| "Normal".to_string());
                self.systems.physics.set_player_target(
                    &player_id,
                    target,
                    4.0,
                    "PostUp",
                    "PostPlayer",
                    &morale,
                );
                let hoop = self
                    .config
                    .rules
                    .court
                    .hoop_pos(self.flow.possession == Possession::Home);
                if let Some(p) = self.systems.physics.get_player_mut(&player_id) {
                    p.action = "PostUp".to_string();
                    // Facing opposite to hoop (backdown orientation)
                    let away_from_hoop = (p.pos_ft - hoop).normalize_or_zero();
                    if away_from_hoop.length_squared() > 0.1 {
                        p.facing_dir = away_from_hoop;
                    }
                }
                let player_name = self
                    .systems
                    .physics
                    .get_player(&player_id)
                    .map(|p| format!("{}号", p.jersey))
                    .unwrap_or_else(|| player_id.clone());
                self.journal.current_callout =
                    Some(format!("{} 低位背身单打，发力推推挤要位！", player_name));
                self.journal.current_event = Some("POST_UP".to_string());
            }
            CandidateAction::TripleThreatJab {
                player_id,
                pivot_pos: _,
                jab_dir,
            } => {
                if let Some(p) = self.systems.physics.get_player_mut(&player_id) {
                    p.facing_dir = jab_dir;
                    p.action = "TripleThreat".to_string();
                }
                let player_name = self
                    .systems
                    .physics
                    .get_player(&player_id)
                    .map(|p| format!("{}号", p.jersey))
                    .unwrap_or_else(|| player_id.clone());
                self.journal.current_callout = Some(format!(
                    "{} 持球三威胁试探步，压低重心观察防守！",
                    player_name
                ));
                self.journal.current_event = Some("TRIPLE_THREAT_JAB".to_string());
            }
        }
        self.clock.last_decision_time = current_t;
        let mut debug = convert_trace(&trace);
        debug
            .enforcement
            .extend(self.journal.current_enforcements.iter().cloned());
        self.observations.last_decision_trace = Some(Box::new(debug));
    }

    pub(crate) fn resolve_free_throw(&mut self) {
        if self.ledger.free_throws_remaining == 0 {
            return;
        }
        let shooter_id = self
            .ledger
            .free_throw_shooter
            .clone()
            .unwrap_or_else(|| self.new_possession_pg());
        let shooter_attributes = self
            .systems
            .physics
            .get_player(&shooter_id)
            .map(|p| p.attributes.clone())
            .unwrap_or_default();
        let made = self
            .systems
            .rng
            .gen_bool(
                nba_domain::free_throw_probability(&self.config.rules, &shooter_attributes) as f64,
            );
        let shooter_is_home = self
            .systems
            .physics
            .get_player(&shooter_id)
            .map(|p| p.team == "home")
            .unwrap_or(self.flow.possession == Possession::Home);
        let attempt = self.ledger.free_throw_attempt.saturating_add(1);
        let ft_pos = Court::free_throw_pos(shooter_is_home, &self.config.rules);
        // F1.1：罚球是停表的显式事件链。出手前把球权威态保持在罚球点
        // 的 Dead 状态，不得让 ball_pos_3d 指向篮筐而权威态仍为 Held。
        self.ball.ball_pos_3d = (ft_pos, self.config.rules.ball_holder_height_ft);
        if !matches!(self.ball.ball_state, BallTrajectoryKind::Dead { .. }) {
            self.transition_ball_state(
                self.dead_state(ft_pos, self.config.rules.ball_holder_height_ft),
            );
        } else if let BallTrajectoryKind::Dead { pos, z, .. } = &mut self.ball.ball_state {
            *pos = ft_pos;
            *z = self.config.rules.ball_holder_height_ft;
        }
        self.journal
            .pending_events
            .push(GameEvent::FreeThrowAttempt {
                shooter_id: shooter_id.clone(),
                attempt,
                made,
            });
        self.ledger.box_score.ft_attempts += 1;
        if made {
            self.ledger.box_score.ft_made += 1;
            if shooter_is_home {
                self.ledger.home_score += 1;
            } else {
                self.ledger.away_score += 1;
            }
        }
        self.ledger.free_throw_attempt = attempt;
        self.ledger.free_throws_remaining = self.ledger.free_throws_remaining.saturating_sub(1);
        self.clock.sub_phase_timer = 0.0;
        if self.ledger.free_throws_remaining == 0 {
            self.ledger.free_throw_shooter = None;
            self.ledger.free_throw_attempt = 0;
            if made {
                // A made final free throw closes the offense's possession.
                // Emit the score summary before the inbound helper calls
                // complete_possession(), so the boundary has a causal fact.
                self.possession_ctx.current_possession_shooter = Some(shooter_id.clone());
                self.emit_possession_summary(
                    nba_domain::PossessionEndCause::Score,
                    None,
                    None,
                    None,
                );
                let hoop = self.config.rules.court.hoop_pos(shooter_is_home);
                self.transition_phase(SubPhase::Initiation);
                self.set_game_flow(GameFlowState::DeadBall);
                self.start_inbound_transition(hoop, self.ball.ball_pos_3d);
            } else {
                self.start_free_throw_rebound(shooter_is_home, ft_pos);
            }
        }
    }

    pub(crate) fn start_free_throw_rebound(&mut self, shooter_is_home: bool, ft_pos: Vec2) {
        let hoop = self.config.rules.court.hoop_pos(shooter_is_home);
        // 罚球出手弧与跳投同源（弧顶 = base + dist×factor，受 z 上限约束），
        // 反弹入射速度从同一抛体导出。
        let ft_dist = (hoop - ft_pos).length();
        let ft_peak = (self.config.rules.shot_peak_base_ft
            + ft_dist * self.config.rules.shot_peak_distance_factor)
            .min(self.config.rules.ball_z_max_ft);
        let ft_flight = BallisticsEngine::shot_duration(ft_dist, ft_peak, &self.config.rules);
        let landing_spot = BallisticsEngine::compute_rebound_landing(
            ft_pos,
            hoop,
            ft_flight,
            &mut self.systems.rng,
            &self.config.rules,
        );
        let rebound_from = (landing_spot.contact_pos, self.config.rules.rim_height_ft);
        self.ball.ball_pos_3d = (rebound_from.0, rebound_from.1);
        self.clock.shot_clock = self
            .config
            .rules
            .league
            .offensive_rebound_shot_clock_seconds;
        self.set_game_flow(GameFlowState::LiveBall);
        // 自由球也是一次投篮尝试，因此子阶段先到 `ShotAttempt` 再到
        // `FlightAndRebound`：`nba.v2` 的合法迁移表里 `Initiation` 不允许直接到
        // `FlightAndRebound`（只允许 `ActionExecution`/`DeadBallReset`/`ShotAttempt`）。
        self.transition_phase(SubPhase::ShotAttempt);
        self.transition_phase(SubPhase::FlightAndRebound);
        let rebound = BallTrajectoryKind::RimRebound {
            from_pos: rebound_from.0,
            from_z: rebound_from.1,
            hoop_pos: hoop,
            target_landing: landing_spot.landing_pos,
            start_time: self.clock.current_time,
            duration: landing_spot.flight_duration,
            peak_z: landing_spot.peak_z,
            last_touch_team: self.flow.possession,
        };
        if matches!(
            self.ball.ball_state,
            BallTrajectoryKind::Pass { .. }
                | BallTrajectoryKind::ControlTransfer { .. }
                | BallTrajectoryKind::InboundTransfer { .. }
                | BallTrajectoryKind::InboundReady { .. }
                | BallTrajectoryKind::LooseBall { .. }
        ) {
            self.transition_ball_state(self.dead_state(rebound_from.0, rebound_from.1));
        }
        self.transition_ball_state(rebound);
    }

    /// Test/diagnostic hook: resolve the current free throw with a forced outcome.
    pub fn resolve_forced_free_throw(&mut self, made: bool) {
        if self.ledger.free_throws_remaining == 0 {
            return;
        }
        let shooter_id = self
            .ledger
            .free_throw_shooter
            .clone()
            .unwrap_or_else(|| self.new_possession_pg());
        let shooter_is_home = self
            .systems
            .physics
            .get_player(&shooter_id)
            .map(|p| p.team == "home")
            .unwrap_or(self.flow.possession == Possession::Home);
        let attempt = self.ledger.free_throw_attempt.saturating_add(1);
        self.journal
            .pending_events
            .push(GameEvent::FreeThrowAttempt {
                shooter_id: shooter_id.clone(),
                attempt,
                made,
            });
        if made {
            if shooter_is_home {
                self.ledger.home_score += 1;
            } else {
                self.ledger.away_score += 1;
            }
        }
        self.ledger.free_throw_attempt = attempt;
        self.ledger.free_throws_remaining = self.ledger.free_throws_remaining.saturating_sub(1);
        self.clock.sub_phase_timer = 0.0;
        if self.ledger.free_throws_remaining == 0 {
            self.ledger.free_throw_shooter = None;
            self.ledger.free_throw_attempt = 0;
            if made {
                self.possession_ctx.current_possession_shooter = Some(shooter_id.clone());
                self.emit_possession_summary(
                    nba_domain::PossessionEndCause::Score,
                    None,
                    None,
                    None,
                );
                self.transition_phase(SubPhase::Initiation);
                self.set_game_flow(GameFlowState::DeadBall);
                self.start_inbound_transition(
                    Court::hoop_pos(shooter_is_home),
                    self.ball.ball_pos_3d,
                );
            } else {
                let hoop = self.config.rules.court.hoop_pos(shooter_is_home);
                self.ball.ball_pos_3d = (hoop, self.config.rules.rim_height_ft);
                self.start_free_throw_rebound(
                    shooter_is_home,
                    Court::free_throw_pos(shooter_is_home, &self.config.rules),
                );
            }
        }
    }

    pub(crate) fn execute_pass(
        &mut self,
        passer_id: &str,
        receiver_id: &str,
        _from_pos: Vec2,
        to_pos: Vec2,
        current_t: f32,
        inbound: bool,
    ) {
        let from_pos = self.ball.ball_pos_3d.0;

        let _initial_dist = self
            .systems
            .physics
            .get_player(receiver_id)
            .map(|p| (p.pos_ft - from_pos).length())
            .unwrap_or(20.0);
        // 领传由**决策层**给出（`CandidateAction::Pass.to_pos` 已含提前量），
        // 此处**不得**再叠加一次——实测叠加后 `PASS_CORRIDOR_REACHABLE`
        // 由 9 条恶化到 28 条（接收人因减速模型无法到达过远的接球点）。
        // outlet 一传走 `start_rebound_outlet`，那条路径没有决策层，故单独领传。
        let target_lead_pos = to_pos;
        self.observations.active_windows.insert(
            passer_id.to_string(),
            ActionTimeWindow::new_pass(passer_id, current_t, &self.config.rules),
        );
        let pass_dist = (target_lead_pos - from_pos).length();
        let duration = self.config.rules.pass_duration(pass_dist, inbound);
        self.ball.pending_pass_receiver = None;
        // 传球结束：丢弃本回合的接球人估计（下一回合重新形成预判）。
        self.ball.receiver_estimate = None;
        self.ball.pending_pass_inbound = inbound;
        let receive_success =
            self.resolve_pass_success(passer_id, receiver_id, from_pos, target_lead_pos);
        self.transition_ball_state(BallTrajectoryKind::Pass {
            from_pos,
            to_pos: target_lead_pos,
            target_id: receiver_id.to_string(),
            start_time: current_t,
            duration,
            peak_z: self.config.rules.pass_peak_ft,
            inbound,
            receive_success,
        });
        self.ball.last_passer_id = Some(passer_id.to_string());
        self.possession_ctx.current_possession_turnover_player = Some(passer_id.to_string());
        self.transition_phase(SubPhase::ActionExecution);
        // 第一性原理：`passes_count` 是「**尝试**传球次数」，应在释放时计数。
        //
        // 原实现只在 `PASS_RECEIVED` 时 `+= 1`，于是掉球/点掉/抢断的传球
        // 完全不被计入——实测 17/60 回合的 `passes_count` 与事件流不一致
        // （申报 0、实际 1–3）。这既污染了 L2 的 ACTION_COMPOSITION_PASSES
        // 准则，也让"每回合传球 1.26 次"的结论本身不可信。
        self.possession_ctx.current_possession_passes += 1;
        self.journal.pending_events.push(GameEvent::PassRelease {
            passer_id: passer_id.to_string(),
            receiver_id: receiver_id.to_string(),
            from_pos: (from_pos.x, from_pos.y),
            to_pos: (target_lead_pos.x, target_lead_pos.y),
        });
        // F1.3：发球员从界外 placement 回场内由 `sync_ball_holder` 在
        // 球态离开 InboundTransfer/InboundReady 时统一处理（单一机制）。
        self.journal.current_event =
            Some(if inbound { "INBOUND_PASS" } else { "PASS" }.to_string());
        self.journal.current_callout = Some(if inbound {
            "界外发球进入飞行，接应点开始读取防守".to_string()
        } else {
            "突分策应！外线转移球创造空位机会".to_string()
        });
    }
}
