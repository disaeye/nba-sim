// 突破推进臂（`resolve_drive_arm` 的实现）。
//
// 划分依据（D29 行数门）：`Drive` 臂单独成篇，因为它是六个臂里唯一
// 同时承担「推进求解」与「中途分流裁定」的——既要按每 tick 重算突破
// 走廊与攻防选择，又要在路径中断时决定是分球、是降级成传球，还是被
// 防守延误后重置战术。其余五个臂（`Pass` / `RimRebound` / `LooseBall`
// / `FloorRebound` / `Shot`）各自只消费一种载荷，互不交叉，留在
// `arms.rs`。
//
// 搬移保真：函数体逐行搬移，任何一行行为改动都会被黄金哈希抓住。

use glam::Vec2;
use nba_domain::action_window::ActionTimeWindow;
use nba_domain::{GameEvent, Possession, SubPhase};
use nba_physics::ballistics::{BallTrajectoryKind, BallisticsEngine};
use nba_semantics::SemanticEvaluator;
use rand::Rng;

use super::super::MatchEngine;
use super::BallFlightOutcome;
use super::FlightContext;

/// 把任意实数收敛到 0..=1，用于概率/比率类派生量。
fn clamp_unit(value: f32) -> f32 {
    value.clamp(f32::from(0u8), f32::from(1u8))
}

impl MatchEngine {
    /// `Drive` 臂：突破推进、中途分流与终结裁定。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn resolve_drive_arm(
        &mut self,
        driver_id: String,
        target_pos: Vec2,
        start_time: f32,
        duration: f32,
        ctx: FlightContext,
        out: &mut BallFlightOutcome,
    ) {
        let tau = ((ctx.current_t - start_time) / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
        let prev_ball = self.ball.ball_pos_3d;
        let (sampled_pos, sampled_z) = BallisticsEngine::sample_ball_position(
            &self.ball.ball_state,
            ctx.current_t,
            self.systems.physics.get_players(),
            &self.config.rules,
        );
        let driver_carrier_pos = self
            .systems
            .physics
            .get_player(&driver_id)
            .map(|player| player.pos_ft);
        self.ball.ball_pos_3d = self.clamp_held_family_ball_pos(
            prev_ball,
            (sampled_pos, sampled_z),
            driver_carrier_pos,
            ctx.dt,
        );

        let driver_pos = self
            .systems
            .physics
            .get_player(&driver_id)
            .map(|player| player.pos_ft)
            .unwrap_or(self.ball.ball_pos_3d.0);
        let hoop_pos = self.config.rules.court.hoop_pos(ctx.is_home);
        let dist_to_hoop = (driver_pos - hoop_pos).length();

        // ## 过人后重定向（round-18 两段式几何的第二段）
        //
        // 起手目标是被过防守人身旁的空档（第一步过人）；越过他
        // （沿突破方向投影领先）后，把目标切回**篮筐**，完成
        // 「过人 → 攻框」的完整几何。
        // 时间基重定向：被过防守人以极速让位，约一个加速窗口
        // （0.35s）后通道即开，届时切回攻框目标。空间基判定
        // （「越过防守人」）在碰撞边界处永不可达，实测停滞。
        if self.ball.beaten_defender_id.is_some() && ctx.current_t - start_time >= 0.35 {
            if let Some(dp) = self.systems.physics.get_player(&driver_id) {
                let drv_spd = dp.target_speed_ftps;
                let morale = dp.morale.clone();
                self.systems.physics.set_player_target(
                    &driver_id,
                    target_pos,
                    drv_spd,
                    "DriveToBasket",
                    "BallHandler",
                    &morale,
                );
                self.ball.beaten_defender_id = None;
            }
        }
        // 1. 中途分流决策（Drive Branching: 突分 Kickout 或急停中投 Pull-up）
        let elapsed = ctx.current_t - start_time;
        let mut branched = false;
        // 出手冻结期间不得分支：该突破者已有一条挂起的投篮释放
        // （execute_shot 冻结的裁定）， Drive 球态保持到 release 才转
        // Shot。分支若在此期间再触发 execute_shot，会撞上 pending
        // 非空的 fast-fail（实测 seed100 全量赛 tick ~35994：
        // pending 挂起 + Drive 臂 pullup 分支二次出手）。
        let frozen_by_pending = self
            .observations
            .pending_shot_release
            .as_ref()
            .is_some_and(|pending| pending.shooter_id == driver_id);
        if elapsed
            >= self
                .config
                .rules
                .tactics
                .drive_decision_check_interval_seconds
            && tau < 0.85
            && !frozen_by_pending
        {
            let spacing = SemanticEvaluator::spacing(
                self.flow.possession,
                driver_pos,
                &self.systems.physics,
                &self.config.rules,
            );
            // 触发度量：突破路线是否被防守封堵（lane_openness 低）。
            // 旧条件 paint_crowding（以篮筐为圆心的**进攻方**队友密度）
            // 语义错位——五外战术全员在外线，禁区内没有队友，crowding
            // 恒低，kickout 在最需要它的战术里永不触发；kickout 的决策
            // 理由是「突破路线被协防封死、外线有空位」，度量对象是
            // 持球人周围的防守密度。
            let lane_blocked = 1.0 - spacing.lane_openness;

            // 突破路线被封且持球人在中远距离时，评估突分（Kickout）给外线空位队友
            if lane_blocked > self.config.rules.tactics.drive_kickout_max_crowding {
                let mut best_kickout: Option<(String, Vec2)> = None;
                let mut min_opp_dist = self.config.rules.tactics.drive_kickout_min_defender_dist_ft;

                let all_players: Vec<_> = self
                    .systems
                    .physics
                    .get_players()
                    .values()
                    .cloned()
                    .collect();
                let off_team_str = match self.flow.possession {
                    Possession::Home => "home",
                    Possession::Away => "away",
                };
                for teammate in &all_players {
                    // ## on_court 过滤（round-18 修复）
                    //
                    // 此前漏掉：突破分球把球传给了**替补席上的队友**
                    // （实测 seed 31337：分球给站在板凳区 y=-4 的
                    // H_08，球被「已下场的人接住」，BALL_HOLDER_ON_COURT
                    // Hard 190 次）。
                    if teammate.on_court
                        && teammate.team == off_team_str
                        && teammate.id != driver_id
                        // 回场条款（charter §6.2）：前场已建立时不得把球分回
                        // 后场的队友——这类传球是白送球权的违例。
                        && !(self.clock.frontcourt_established
                            && self
                                .config
                                .rules
                                .court
                                .is_backcourt(teammate.pos_ft, self.flow.possession == Possession::Home))
                    {
                        let dist_to_team_hoop = (teammate.pos_ft - hoop_pos).length();
                        if dist_to_team_hoop >= self.config.rules.tactics.drive_kickout_pass_dist_ft
                        {
                            // 检查该空位队友最近的防守人距离
                            let mut nearest_def_dist = f32::MAX;
                            for opp in &all_players {
                                if opp.team != off_team_str {
                                    let d = (opp.pos_ft - teammate.pos_ft).length();
                                    if d < nearest_def_dist {
                                        nearest_def_dist = d;
                                    }
                                }
                            }
                            if nearest_def_dist > min_opp_dist {
                                min_opp_dist = nearest_def_dist;
                                best_kickout = Some((teammate.id.clone(), teammate.pos_ft));
                            }
                        }
                    }
                }

                if let Some((target_id, target_spot)) = best_kickout {
                    out.drive_kickout_action = Some((
                        driver_id.clone(),
                        target_id,
                        driver_pos,
                        target_spot,
                        ctx.current_t,
                    ));
                    branched = true;
                } else if lane_blocked > self.config.rules.tactics.drive_pullup_min_crowding
                    && dist_to_hoop > self.config.rules.tactics.drive_early_finish_dist_ft
                    && dist_to_hoop <= self.config.rules.tactics.drive_mid_range_pullup_dist_ft
                {
                    // 人堆受阻，急停中距离跳投 (Pull-up)
                    out.drive_pullup_action = Some((driver_id.clone(), driver_pos, ctx.current_t));
                    branched = true;
                }
            }
        }

        // 2. 提前进入冲框终结判定：进入终结区（Early Finish Gate）或时间耗尽
        let early_finish = dist_to_hoop <= self.config.rules.tactics.drive_early_finish_dist_ft
            && elapsed >= self.config.rules.tactics.drive_min_duration_seconds;
        if !branched && (tau >= 1.0 || early_finish) {
            let driver_id = driver_id.clone();
            self.possession_ctx.current_possession_turnover_player = Some(driver_id.clone());
            let driver_pos = self
                .systems
                .physics
                .get_player(&driver_id)
                .map(|player| player.pos_ft)
                .unwrap_or(self.ball.ball_pos_3d.0);
            let holder_height = self.config.rules.ball_holder_height_ft;
            // ## 事实修正（round-16）：先判定空间门，再申报结果
            //
            // 原实现无条件把预掷的 `finish_made` 写进事件，但空间门
            // （距筐 >16ft）会把该「进球」静默丢弃——实测 6 场 397 次
            // DRIVE_SCORE 中 80%（场均 52.7 次）未变成出手，事件流与
            // 记分簿自相矛盾（事实账目违规）。
            let stall_hoop_pos = self.config.rules.court.hoop_pos(ctx.is_home);
            let driver_now = self.systems.physics.get_player(&driver_id).cloned();
            let openness = self.systems.physics.openness(&driver_id);
            let reached = driver_now.as_ref().is_some_and(|player| {
                let stamina = clamp_unit(player.stamina / player.max_stamina.max(f32::EPSILON));
                let lane_density = SemanticEvaluator::spacing(
                    self.flow.possession,
                    driver_pos,
                    &self.systems.physics,
                    &self.config.rules,
                )
                .paint_crowding;
                let policy = &self.config.rules.resolve.drive;
                let skill_delta = (player.attributes.finishing - f32::from(1u8) / f32::from(2u8))
                    * self.config.rules.resolve.player_skill.finishing_weight
                    * policy.skill_delta_scale;
                let fatigue_delta = (stamina - f32::from(1u8))
                    * self.config.rules.resolve.player_skill.finishing_weight;
                let reach_probability = clamp_unit(
                    self.config.rules.resolve.base_rates.drive_success
                        + skill_delta
                        + fatigue_delta
                        - lane_density * policy.lane_density_penalty
                        - openness.contest_intensity * policy.contest_penalty,
                );
                self.systems.rng.gen_bool(reach_probability as f64)
            });
            let dist_now = (driver_pos - stall_hoop_pos).length();
            let stall_no_finish =
                reached && dist_now > self.config.rules.tactics.drive_finish_range_ft;
            let successful = reached && !stall_no_finish;
            self.journal.pending_events.push(GameEvent::DriveOutcome {
                driver_id: driver_id.clone(),
                successful,
            });

            if successful {
                let hoop_pos = self.config.rules.court.hoop_pos(ctx.is_home);
                let finish_pos = driver_pos;
                let drive_distance = (driver_pos - target_pos).length();
                let early_finish = drive_distance
                    <= self.config.rules.tactics.drive_early_finish_dist_ft
                    && elapsed <= self.config.rules.tactics.drive_min_duration_seconds;
                let dist_to_hoop = (hoop_pos - finish_pos).length();
                // Spatial gate: if driver is still outside the paint / perimeter,
                // this drive was stalled before reaching finishing position.
                let finish_range = self.config.rules.tactics.drive_finish_range_ft;
                if dist_to_hoop > finish_range
                    && drive_distance > self.config.rules.tactics.drive_early_finish_dist_ft
                    && !early_finish
                {
                    self.ball.ball_pos_3d = (driver_pos, holder_height);
                    out.new_ball_state = Some(BallTrajectoryKind::Held {
                        carrier_id: driver_id.clone(),
                    });
                    // ## 停滞不重新发起战术（round-16）
                    //
                    // 原实现转回 Initiation，而 Initiation 在活球下要等
                    // `tactical_initiation_seconds = 6.5s` 才回到可决策
                    // 状态——停滞一次就白燃 6.5s 进攻时钟。突破受阻是
                    // **进攻的延续**，不是新回合，直接回 ActionExecution
                    // 下一决策间隔（2.4s）即可行动。
                    // 实测该机制是 24s 违例（场均 12.8 次，真实 ~0.5）
                    // 的主要时间吞噬器。
                    self.transition_phase(SubPhase::ActionExecution);
                    self.journal.current_event = Some("DRIVE_STOPPED".to_string());
                    self.journal.current_callout =
                        Some(format!("{} 突破被防守延误于外线，重新组织", driver_id));
                } else {
                    let driver_p = self.systems.physics.get_player(&driver_id).cloned();
                    let finishing_skill = driver_p
                        .as_ref()
                        .map(|p| p.attributes.finishing)
                        .unwrap_or(0.5);
                    let lane_density = SemanticEvaluator::spacing(
                        self.flow.possession,
                        driver_pos,
                        &self.systems.physics,
                        &self.config.rules,
                    )
                    .paint_crowding;
                    let takeoff = (hoop_pos - finish_pos).normalize_or_zero()
                        * drive_distance.min(self.config.rules.tactics.drive_finish_extend_ft);
                    let finish_pos = self
                        .config
                        .rules
                        .court
                        .clamp_playable(finish_pos + takeoff, self.config.rules.player_radius_ft);
                    let dist_to_hoop = (hoop_pos - finish_pos).length();
                    let (finish_kind, action_name, callout_action) = if dist_to_hoop
                        < self.config.rules.tactics.drive_dunk_max_dist_ft
                        && finishing_skill > self.config.rules.tactics.drive_dunk_min_finishing
                        && lane_density < self.config.rules.tactics.drive_dunk_max_lane_density
                    {
                        (
                            nba_domain::action_window::RimFinishKind::Dunk,
                            "Dunk",
                            "腾空暴扣！单臂炸筐！",
                        )
                    } else if dist_to_hoop > self.config.rules.tactics.drive_floater_min_dist_ft {
                        (
                            nba_domain::action_window::RimFinishKind::Floater,
                            "Floater",
                            "行进间柔和抛投！",
                        )
                    } else {
                        (
                            nba_domain::action_window::RimFinishKind::Layup,
                            "Layup",
                            "三步并两步，低手上篮！",
                        )
                    };
                    self.transition_phase(SubPhase::ShotAttempt);
                    let contest_val = openness.contest_intensity;
                    let finish_zone = self.config.rules.court.shot_zone(
                        finish_pos,
                        ctx.is_home,
                        self.config.rules.league.three_point_distance_ft,
                        self.config.rules.league.corner_three_distance_ft,
                    );
                    let finish_skill = clamp_unit(
                        driver_p
                            .as_ref()
                            .map(|player| match finish_zone {
                                nba_domain::ShotZone::Rim => {
                                    let contest = clamp_unit(contest_val);
                                    player.attributes.shooting_close * (f32::from(1u8) - contest)
                                        + player.attributes.finishing * contest
                                }
                                nba_domain::ShotZone::Near => player.attributes.shooting_near,
                                nba_domain::ShotZone::Mid => player.attributes.shooting_mid,
                                nba_domain::ShotZone::Three => player.attributes.shooting_three,
                            })
                            .unwrap_or(f32::from(1u8) / f32::from(2u8)),
                    );
                    let base_fg = match finish_zone {
                        nba_domain::ShotZone::Rim => {
                            self.config.rules.resolve.base_rates.shot_make_rim
                        }
                        nba_domain::ShotZone::Near => {
                            self.config.rules.resolve.base_rates.shot_make_near
                        }
                        nba_domain::ShotZone::Mid => {
                            self.config.rules.resolve.base_rates.shot_make_mid
                        }
                        nba_domain::ShotZone::Three => {
                            self.config.rules.resolve.base_rates.shot_make_3pt
                        }
                    };
                    let skill_adjustment = (finish_skill - f32::from(1u8) / f32::from(2u8))
                        * self.config.rules.resolve.player_skill.shooting_weight
                        * f32::from(2u8);
                    let make_probability = (base_fg + skill_adjustment
                        - contest_val * self.config.rules.shot_contest_sensitivity)
                        .clamp(
                            self.config.rules.shot_pct_floor,
                            self.config.rules.shot_pct_ceiling,
                        );
                    let transition_event_id = self.possession_ctx.transition_context_event(
                        ctx.current_t,
                        self.config.rules.tactics.transition_finish_window_seconds,
                    );
                    let transition_context =
                        transition_event_id.is_some() && dist_to_hoop <= finish_range;
                    // 转换语境下的攻框按转换终结归类（描述回合创建语境，
                    // 优先于动作路径）：突破只是到达篮下的手段。
                    let creation_source = if transition_context {
                        nba_domain::ShotCreationSource::TransitionFinish
                    } else {
                        nba_domain::ShotCreationSource::DriveFinish
                    };
                    let source_event_id = if transition_context {
                        transition_event_id
                    } else {
                        None
                    };
                    self.journal.pending_events.push(GameEvent::ShotRelease {
                        shooter_id: driver_id.clone(),
                        pos: (finish_pos.x, finish_pos.y),
                        creation_source,
                        transition_context,
                        transition_event_id: transition_context
                            .then_some(transition_event_id)
                            .flatten(),
                        source_event_id,
                        is_three: finish_zone == nba_domain::ShotZone::Three,
                        contest_level: contest_val,
                        make_probability,
                    });
                    self.possession_ctx.current_possession_shooter = Some(driver_id.clone());
                    self.possession_ctx.current_possession_contest = Some(contest_val);
                    self.journal.current_event = Some("SHOT_RELEASE".to_string());
                    self.clear_recent_shot_creation_context();
                    let window = if finish_kind == nba_domain::action_window::RimFinishKind::Dunk {
                        ActionTimeWindow::new_dunk(&driver_id, ctx.current_t, &self.config.rules)
                    } else {
                        ActionTimeWindow::new_layup(&driver_id, ctx.current_t, &self.config.rules)
                    };
                    self.start_action_window(
                        &driver_id,
                        window,
                        None,
                        Some((hoop_pos.x, hoop_pos.y)),
                    );
                    if let Some(p) = self.systems.physics.get_player_mut(&driver_id) {
                        p.action = action_name.to_string();
                        let hoop_dir = (hoop_pos - p.pos_ft).normalize_or_zero();
                        if hoop_dir.length_squared() > 0.1 {
                            p.facing_dir = hoop_dir;
                        }
                    }
                    let driver_display = self
                        .systems
                        .physics
                        .get_player(&driver_id)
                        .map(|p| format!("{}号", p.jersey))
                        .unwrap_or_else(|| driver_id.clone());
                    self.journal.current_callout =
                        Some(format!("{} {}", driver_display, callout_action));
                    let aim_pos = BallisticsEngine::sample_shot_aim(
                        finish_pos,
                        hoop_pos,
                        make_probability,
                        &mut self.systems.rng,
                        &self.config.rules,
                    );
                    // 释放起点 = 球的实际位置（末次运球采样点）：出手事件与
                    // 终结分区仍按向篮筐延伸的 finish_pos 统计，但球的飞行
                    // 必须从球所在处连续出发，否则单帧向篮筐瞬移
                    // drive_finish_extend_ft，越出球速包络（实测 90–156 ft/s）。
                    let release_origin = self.ball.ball_pos_3d.0;
                    let shot_dur = BallisticsEngine::shot_duration(
                        (aim_pos - release_origin).length(),
                        self.config.rules.rim_height_ft + 1.5,
                        &self.config.rules,
                    );
                    out.new_ball_state = Some(BallTrajectoryKind::Shot {
                        shooter_id: driver_id.clone(),
                        from_pos: release_origin,
                        hoop_pos,
                        aim_pos,
                        start_time: ctx.current_t,
                        duration: shot_dur,
                        collision_checked_until: ctx.current_t,
                        is_three: finish_zone == nba_domain::ShotZone::Three,
                        peak_z: if finish_kind == nba_domain::action_window::RimFinishKind::Dunk {
                            self.config.rules.rim_height_ft + 0.5
                        } else {
                            self.config.rules.rim_height_ft + 1.5
                        },
                        make_probability,
                        contest_intensity: contest_val,
                    });
                }
            } else {
                self.ball.ball_pos_3d = (driver_pos, holder_height);
                out.new_ball_state = Some(BallTrajectoryKind::Held {
                    carrier_id: driver_id.clone(),
                });
                // 同上：突破未成是进攻延续，不重置为战术发起等待。
                self.transition_phase(SubPhase::ActionExecution);
                self.journal.current_event = Some("DRIVE_STOPPED".to_string());
                self.journal.current_callout =
                    Some(format!("{} 突破被防守延误，重新组织", driver_id));
            }
            self.clock.last_decision_time = ctx.current_t;
        }
    }
}
