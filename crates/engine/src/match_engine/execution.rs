//! 动作执行：把决策输出的候选动作写入权威状态与事件。
//!
//! 依据 `docs/architecture.md` §5.2 的执行重校验原则：本模块只负责执行，
//! 候选已经过约束管线；执行点把动作写入球态、动作窗口与事件流。

use glam::Vec2;
use nba_decision::constraint::CandidateAction;
use nba_decision::pipeline::DecisionOutput;
use nba_domain::action_window::ActionTimeWindow;
use nba_domain::court::Court;
use nba_domain::{GameEvent, GameFlowState, Possession, SubPhase};
use nba_physics::ballistics::{BallTrajectoryKind, BallisticsEngine};

mod drive_geometry;
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
        let target_pos = self
            .config
            .rules
            .court
            .clamp_playable(target_pos, self.config.rules.player_radius_ft);
        let drive_dist = (target_pos - from_pos).length();
        let (drive_speed, drive_duration) =
            drive_geometry::drive_speed_and_duration(&driver, drive_dist, &self.config.rules);
        let geometry = drive_geometry::resolve_drive_geometry(
            &driver,
            from_pos,
            target_pos,
            self.systems.physics.get_players(),
            drive_speed,
            drive_duration,
            &self.config.rules,
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
        let mut target_pos_override = None;
        if geometry.successful {
            if let Some(defender_id) = geometry.primary_defender_id.as_ref() {
                if let Some(defender) = self.systems.physics.get_player(defender_id).cloned() {
                    let drive_dir = (target_pos - from_pos).normalize_or_zero();
                    let perp = Vec2::new(-drive_dir.y, drive_dir.x);
                    let clear = self.config.rules.min_player_separation_ft;
                    let recovery_target = self.config.rules.court.clamp_playable(
                        defender.pos_ft - perp * geometry.lateral_direction * clear,
                        self.config.rules.player_radius_ft,
                    );
                    self.systems.physics.set_player_target(
                        defender_id,
                        recovery_target,
                        defender.max_speed_ftps,
                        "BeatenRecovery",
                        &defender.slot,
                        &defender.morale,
                    );
                    self.observations.beaten_recovery_until.insert(
                        defender_id.clone(),
                        self.clock.current_time
                            + self.config.rules.tactics.drive_beaten_recovery_seconds,
                    );
                    target_pos_override = geometry.bypass_target;
                    self.ball.beaten_defender_id = Some(defender_id.clone());
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
            target_pos: initial_target,
            move_kind,
            start_time: current_t,
            duration: drive_duration,
        });
        self.ball.ball_pos_3d = (from_pos, self.config.rules.ball_holder_height_ft);
        self.transition_phase(SubPhase::ActionExecution);
        self.journal.pending_events.push(GameEvent::DriveInitiated {
            driver_id: driver_id.to_string(),
            from_pos: (from_pos.x, from_pos.y),
            target_pos: (initial_target.x, initial_target.y),
        });
        self.journal.current_event = Some("DRIVE_INITIATED".to_string());
        self.journal.current_callout = Some(callout_text);
        self.journal.current_intensity = Some("Climax".to_string());
    }

    pub(crate) fn clear_recent_shot_creation_context(&mut self) {
        self.possession_ctx.last_cut_reception_time = None;
        self.possession_ctx.last_cut_reception_player = None;
        self.possession_ctx.last_cut_reception_event_parent = None;
        self.possession_ctx.last_offensive_rebound_time = None;
        self.possession_ctx.last_offensive_rebound_player = None;
        self.possession_ctx.last_offensive_rebound_event_parent = None;
    }

    pub(crate) fn shot_creation_source(
        &self,
        shooter_id: &str,
        current_t: f32,
    ) -> nba_domain::ShotCreationSource {
        let offense = match self.flow.possession {
            Possession::Home => "home",
            Possession::Away => "away",
        };
        let possession_elapsed = current_t - self.possession_ctx.current_possession_start_time;
        let source_age_limit = self.config.rules.decision_interval_seconds;
        let recent_offensive_rebound = self
            .possession_ctx
            .recent_offensive_rebounder(current_t, source_age_limit)
            == Some(shooter_id)
            && self
                .systems
                .physics
                .get_player(shooter_id)
                .is_some_and(|player| {
                    let attacking_right = player.team == "home";
                    player.team == offense
                        && self.config.rules.court.shot_zone(
                            player.pos_ft,
                            attacking_right,
                            self.config.rules.league.three_point_distance_ft,
                            self.config.rules.league.corner_three_distance_ft,
                        ) == nba_domain::ShotZone::Rim
                });
        let recent_cut_reception = self.possession_ctx.last_cut_reception_player.as_deref()
            == Some(shooter_id)
            && self
                .possession_ctx
                .last_cut_reception_event_parent
                .is_some()
            && self
                .possession_ctx
                .last_cut_reception_time
                .is_some_and(|time| current_t >= time && current_t - time <= source_age_limit)
            && self
                .systems
                .physics
                .get_player(shooter_id)
                .is_some_and(|player| {
                    let attacking_right = player.team == "home";
                    player.team == offense
                        && self.config.rules.court.shot_zone(
                            player.pos_ft,
                            attacking_right,
                            self.config.rules.league.three_point_distance_ft,
                            self.config.rules.league.corner_three_distance_ft,
                        ) == nba_domain::ShotZone::Rim
                });
        if recent_offensive_rebound {
            nba_domain::ShotCreationSource::OffensiveReboundPutback
        } else if recent_cut_reception {
            nba_domain::ShotCreationSource::CutReception
        } else if self
            .possession_ctx
            .transition_context_event(
                current_t,
                self.config.rules.tactics.transition_finish_window_seconds,
            )
            .is_some_and(|_| {
                possession_elapsed <= self.config.rules.tactics.transition_finish_window_seconds
                    && self
                        .systems
                        .physics
                        .get_player(shooter_id)
                        .is_some_and(|player| {
                            let attacking_right = player.team == "home";
                            let in_rim_zone = self.config.rules.court.shot_zone(
                                player.pos_ft,
                                attacking_right,
                                self.config.rules.league.three_point_distance_ft,
                                self.config.rules.league.corner_three_distance_ft,
                            ) == nba_domain::ShotZone::Rim
                                || self.config.rules.court.shot_zone(
                                    self.ball.ball_pos_3d.0,
                                    attacking_right,
                                    self.config.rules.league.three_point_distance_ft,
                                    self.config.rules.league.corner_three_distance_ft,
                                ) == nba_domain::ShotZone::Rim;
                            player.team == offense && in_rim_zone
                        })
            })
        {
            nba_domain::ShotCreationSource::TransitionFinish
        } else {
            nba_domain::ShotCreationSource::SetPlay
        }
    }

    pub(crate) fn is_cut_reception(
        &self,
        receiver_id: &str,
        position: Vec2,
        current_t: f32,
    ) -> bool {
        let Some(receiver) = self.systems.physics.get_player(receiver_id) else {
            return false;
        };
        let hoop = self.config.rules.court.hoop_pos(receiver.team == "home");
        let ball_distance = (position - hoop).length();
        self.observations.cut_route_players.contains(receiver_id)
            && ball_distance < nba_domain::court::NEAR_ZONE_MAX_DIST_FT
            && current_t.is_finite()
    }

    pub(crate) fn consume_pending_shot_release(&mut self, current_t: f32) {
        let Some(pending) = self.observations.pending_shot_release.take() else {
            return;
        };
        assert!(
            self.observations
                .active_windows
                .get(&pending.shooter_id)
                .is_some_and(|window| {
                    window.action_type == nba_domain::action_window::ActionType::JumpShot
                        || window.action_type == nba_domain::action_window::ActionType::Putback
                }),
            "pending shot release for `{}` lost its JumpShot window",
            pending.shooter_id
        );
        assert!(
            current_t + f32::EPSILON >= pending.release_time,
            "shot release consumed before its frozen release time (t={current_t}, release={})",
            pending.release_time
        );
        let release_pos = self.ball.ball_pos_3d.0;
        let distance = (pending.hoop_pos - release_pos).length();
        let peak_z = (self.config.rules.shot_peak_base_ft
            + distance * self.config.rules.shot_peak_distance_factor)
            .min(self.config.rules.ball_z_max_ft);
        let aim_pos = BallisticsEngine::sample_shot_aim(
            release_pos,
            pending.hoop_pos,
            pending.make_probability,
            &mut self.systems.rng,
            &self.config.rules,
        );
        let flight_time = BallisticsEngine::shot_duration(
            (aim_pos - release_pos).length(),
            peak_z,
            &self.config.rules,
        );
        assert!(
            (pending.hoop_pos - release_pos).length() <= 1.0e-4
                || flight_time >= self.config.rules.min_shot_duration_seconds,
            "shot duration fell below the configured minimum"
        );
        self.transition_ball_state(BallTrajectoryKind::Shot {
            shooter_id: pending.shooter_id.clone(),
            from_pos: release_pos,
            hoop_pos: pending.hoop_pos,
            aim_pos,
            start_time: pending.release_time,
            duration: flight_time,
            is_three: pending.is_three,
            peak_z,
            make_probability: pending.make_probability,
            contest_intensity: pending.contest_intensity,
        });
        self.transition_phase(SubPhase::ShotAttempt);
        self.journal.pending_events.push(GameEvent::ShotRelease {
            shooter_id: pending.shooter_id,
            pos: (release_pos.x, release_pos.y),
            creation_source: pending.creation_source,
            transition_context: pending.transition_context,
            transition_event_id: pending.transition_event_id,
            source_event_id: pending.source_event_id,
            is_three: pending.is_three,
            contest_level: pending.contest_intensity,
            make_probability: pending.make_probability,
        });
    }

    pub(crate) fn execute_shot(
        &mut self,
        shooter_id: &str,
        from_pos: Vec2,
        is_three_hint: bool,
        jumper_kind: Option<nba_domain::action_window::JumperKind>,
        creation_source: nba_domain::ShotCreationSource,
        current_t: f32,
    ) {
        let is_home = self.flow.possession == Possession::Home;
        let hoop = self.config.rules.court.hoop_pos(is_home);
        let shooter = self.systems.physics.get_player(shooter_id);
        let shooter_pos = shooter.map(|player| player.pos_ft).unwrap_or(from_pos);
        let dist_to_hoop = (shooter_pos - hoop).length();
        let shot_zone = self.config.rules.court.shot_zone(
            shooter_pos,
            is_home,
            self.config.rules.league.three_point_distance_ft,
            self.config.rules.league.corner_three_distance_ft,
        );
        let is_three = shot_zone == nba_domain::ShotZone::Three || is_three_hint;
        let openness = self.systems.physics.openness(shooter_id);
        let spacing_bonus = self
            .observations
            .latest_spacing
            .map(|spacing| spacing.shot_quality_bonus)
            .unwrap_or(0.0);
        let skill = shooter
            .map(|player| match shot_zone {
                nba_domain::ShotZone::Rim => {
                    let contest = openness
                        .contest_intensity
                        .clamp(f32::from(0u8), f32::from(1u8));
                    let uncontested = f32::from(1u8) - contest;
                    player.attributes.shooting_close * uncontested
                        + player.attributes.finishing * contest
                }
                nba_domain::ShotZone::Near => player.attributes.shooting_near,
                nba_domain::ShotZone::Mid => player.attributes.shooting_mid,
                nba_domain::ShotZone::Three => player.attributes.shooting_three,
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
        let base_fg = match shot_zone {
            nba_domain::ShotZone::Rim => self.config.rules.resolve.base_rates.shot_make_rim,
            nba_domain::ShotZone::Near => self.config.rules.resolve.base_rates.shot_make_near,
            nba_domain::ShotZone::Mid => self.config.rules.resolve.base_rates.shot_make_mid,
            nba_domain::ShotZone::Three => self.config.rules.resolve.base_rates.shot_make_3pt,
        };
        let final_fg_pct = (base_fg + skill_adjustment + stamina_adjustment + spacing_bonus
            - contest_penalty)
            .clamp(
                self.config.rules.shot_pct_floor,
                self.config.rules.shot_pct_ceiling,
            );
        let peak_z = (self.config.rules.shot_peak_base_ft
            + dist_to_hoop * self.config.rules.shot_peak_distance_factor)
            .min(self.config.rules.ball_z_max_ft);
        let flight_time = BallisticsEngine::shot_duration(dist_to_hoop, peak_z, &self.config.rules);
        let rim_release = shot_zone == nba_domain::ShotZone::Rim;
        let transition_event_id = self.possession_ctx.transition_context_event(
            current_t,
            self.config.rules.tactics.transition_finish_window_seconds,
        );
        let transition_context = transition_event_id.is_some()
            && (rim_release || creation_source == nba_domain::ShotCreationSource::TransitionFinish);
        let window = if creation_source == nba_domain::ShotCreationSource::OffensiveReboundPutback {
            ActionTimeWindow::new_putback(shooter_id, current_t, &self.config.rules)
        } else {
            ActionTimeWindow::new_jump_shot(shooter_id, current_t, &self.config.rules)
        };
        let (prep_duration, exec_duration) = (window.prep_duration, window.exec_duration);
        self.start_action_window(shooter_id, window, None, Some((hoop.x, hoop.y)));
        self.possession_ctx.current_possession_shooter = Some(shooter_id.to_string());
        self.possession_ctx.current_possession_contest = Some(openness.contest_intensity);
        let source_event_id = match creation_source {
            nba_domain::ShotCreationSource::DrivePullUp => Some(
                self.journal
                    .causal_links
                    .get("drive")
                    .copied()
                    .expect("drive pull-up must retain its DriveInitiated event ID"),
            ),
            nba_domain::ShotCreationSource::CutReception => Some(
                self.possession_ctx
                    .last_cut_reception_event_parent
                    .expect("cut-reception shot must retain its PassReceived event ID"),
            ),
            nba_domain::ShotCreationSource::OffensiveReboundPutback => Some(
                self.possession_ctx
                    .last_offensive_rebound_event_parent
                    .expect("putback shot must retain its offensive Rebound event ID"),
            ),
            nba_domain::ShotCreationSource::TransitionFinish => Some(
                transition_event_id
                    .expect("transition finish must retain its TRANSITION_STARTED event ID"),
            ),
            nba_domain::ShotCreationSource::DriveFinish
            | nba_domain::ShotCreationSource::SetPlay => None,
        };
        self.clear_recent_shot_creation_context();
        assert!(
            self.observations.pending_shot_release.is_none(),
            "a pending shot release already exists for `{}` while `{shooter_id}` shoots",
            self.observations
                .pending_shot_release
                .as_ref()
                .map(|pending| pending.shooter_id.as_str())
                .unwrap_or("?")
        );
        self.observations.pending_shot_release = Some(super::state::PendingShotRelease {
            shooter_id: shooter_id.to_string(),
            hoop_pos: hoop,
            release_time: current_t + prep_duration + exec_duration,
            flight_time,
            is_three,
            contest_intensity: openness.contest_intensity,
            make_probability: final_fg_pct,
            creation_source,
            transition_context,
            transition_event_id: transition_context.then_some(transition_event_id).flatten(),
            source_event_id,
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
        if let Some(p) = self.systems.physics.get_player_mut(shooter_id) {
            p.action = action_name.to_string();
            let hoop_dir = (hoop - p.pos_ft).normalize_or_zero();
            if hoop_dir.length_squared() > 0.1 {
                p.facing_dir = hoop_dir;
            }
        }
        self.possession_ctx.current_possession_turnover_player = Some(shooter_id.to_string());
    }

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
        let is_putback_decision = self
            .possession_ctx
            .recent_offensive_rebounder(current_t, self.config.rules.decision_interval_seconds)
            .is_some_and(|rebounder| rebounder == out.action.actor_id());
        if !matches!(&out.action, CandidateAction::Shoot { .. }) && !is_putback_decision {
            self.clear_recent_shot_creation_context();
        }
        let trace = out.trace.clone();
        match out.action {
            CandidateAction::Shoot {
                shooter_id,
                from_pos,
                is_three,
                jumper_kind,
            } => {
                let creation_source = self.shot_creation_source(&shooter_id, current_t);
                self.execute_shot(
                    &shooter_id,
                    from_pos,
                    is_three,
                    jumper_kind,
                    creation_source,
                    current_t,
                );
                if is_putback_decision {
                    self.clear_recent_shot_creation_context();
                }
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
            } => self.execute_pass(&passer_id, &receiver_id, from_pos, to_pos, current_t, true),
            CandidateAction::Dwell { .. } => {}
            CandidateAction::Advance {
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
        if self.ledger.free_throws_remaining == 0
            || !matches!(self.ball.ball_state, BallTrajectoryKind::Dead { .. })
        {
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
        let ft_pos = Court::free_throw_pos(shooter_is_home, &self.config.rules);
        self.start_free_throw_flight(
            &shooter_id,
            shooter_is_home,
            ft_pos,
            None,
            self.ledger.free_throws_remaining == 1,
        );
    }

    pub(crate) fn start_free_throw_flight(
        &mut self,
        shooter_id: &str,
        shooter_is_home: bool,
        ft_pos: Vec2,
        forced_result: Option<bool>,
        is_final: bool,
    ) {
        let from_position = self.ball.ball_pos_3d;
        let ft_height = self.config.rules.ball_holder_height_ft;
        let placement_distance = (ft_pos - from_position.0)
            .length()
            .hypot(ft_height - from_position.1);
        let speed_budget = (self.config.rules.ball_max_speed_ftps
            - self.config.rules.invariant_speed_tolerance_ftps)
            .max(self.config.rules.invariant_speed_tolerance_ftps);
        let placement_duration =
            (placement_distance / speed_budget).max(self.config.rules.tick_seconds);
        self.clock.shot_clock = self
            .config
            .rules
            .league
            .offensive_rebound_shot_clock_seconds;
        self.set_game_flow(GameFlowState::FreeThrow);
        if placement_distance > f32::EPSILON || (from_position.1 - ft_height).abs() > f32::EPSILON {
            self.transition_ball_state(BallTrajectoryKind::FreeThrowSetup {
                shooter_id: shooter_id.to_string(),
                from_pos: from_position.0,
                from_z: from_position.1,
                to_pos: ft_pos,
                to_z: ft_height,
                start_time: self.clock.current_time,
                duration: placement_duration,
                forced_result,
                is_final,
            });
        } else {
            self.begin_free_throw_flight(
                shooter_id,
                shooter_is_home,
                ft_pos,
                forced_result,
                is_final,
            );
        }
    }

    pub(crate) fn begin_free_throw_flight(
        &mut self,
        shooter_id: &str,
        shooter_is_home: bool,
        ft_pos: Vec2,
        forced_result: Option<bool>,
        is_final: bool,
    ) {
        let hoop_pos = self.config.rules.court.hoop_pos(shooter_is_home);
        let probability = nba_domain::free_throw_probability(
            &self.config.rules,
            &self
                .systems
                .physics
                .get_player(shooter_id)
                .map(|player| player.attributes.clone())
                .unwrap_or_default(),
        );
        let aim_pos = BallisticsEngine::sample_free_throw_aim(
            ft_pos,
            hoop_pos,
            probability,
            forced_result,
            &mut self.systems.rng,
            &self.config.rules,
        );
        let distance = (aim_pos - ft_pos).length();
        let peak_z = (self.config.rules.shot_peak_base_ft
            + (hoop_pos - ft_pos).length() * self.config.rules.shot_peak_distance_factor)
            .min(self.config.rules.ball_z_max_ft);
        let duration = BallisticsEngine::shot_duration(distance, peak_z, &self.config.rules);
        let ft_height = self.config.rules.ball_holder_height_ft;
        self.ball.ball_pos_3d = (ft_pos, ft_height);
        if self.clock.sub_phase != SubPhase::FlightAndRebound {
            self.transition_phase(SubPhase::ShotAttempt);
            self.transition_phase(SubPhase::FlightAndRebound);
        }
        self.transition_ball_state(BallTrajectoryKind::FreeThrow {
            shooter_id: shooter_id.to_string(),
            from_pos: ft_pos,
            hoop_pos,
            aim_pos,
            start_time: self.clock.current_time,
            duration,
            peak_z,
            is_final,
        });
    }

    pub fn resolve_forced_free_throw(&mut self, made: bool) {
        if self.ledger.free_throws_remaining == 0 {
            return;
        }
        if let BallTrajectoryKind::FreeThrowSetup {
            shooter_id,
            from_pos,
            from_z,
            to_pos,
            to_z,
            start_time,
            duration,
            ..
        } = &self.ball.ball_state
        {
            self.transition_ball_state(BallTrajectoryKind::FreeThrowSetup {
                shooter_id: shooter_id.clone(),
                from_pos: *from_pos,
                from_z: *from_z,
                to_pos: *to_pos,
                to_z: *to_z,
                start_time: *start_time,
                duration: *duration,
                forced_result: Some(made),
                is_final: self.ledger.free_throws_remaining == 1,
            });
            return;
        }
        if !matches!(self.ball.ball_state, BallTrajectoryKind::Dead { .. }) {
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
        let ft_pos = Court::free_throw_pos(shooter_is_home, &self.config.rules);
        self.start_free_throw_flight(
            &shooter_id,
            shooter_is_home,
            ft_pos,
            Some(made),
            self.ledger.free_throws_remaining == 1,
        );
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
        let from_z = self.ball.ball_pos_3d.1;
        let target_lead_pos = to_pos;
        self.start_action_window(
            passer_id,
            ActionTimeWindow::new_pass(passer_id, current_t, &self.config.rules),
            Some(receiver_id.to_string()),
            Some((target_lead_pos.x, target_lead_pos.y)),
        );
        let pass_dist = (target_lead_pos - from_pos).length();
        let duration = self.config.rules.pass_duration(pass_dist, inbound);
        self.ball.pending_pass_receiver = None;
        self.ball.receiver_estimate = None;
        self.ball.pending_pass_inbound = inbound;
        self.transition_ball_state(BallTrajectoryKind::Pass {
            from_pos,
            from_z,
            to_pos: target_lead_pos,
            target_id: receiver_id.to_string(),
            start_time: current_t,
            duration,
            peak_z: self.config.rules.pass_peak_ft,
            inbound,
        });
        self.ball.last_passer_id = Some(passer_id.to_string());
        self.possession_ctx.current_possession_turnover_player = Some(passer_id.to_string());
        self.transition_phase(SubPhase::ActionExecution);
        self.possession_ctx.current_possession_passes += 1;
        self.journal.pending_events.push(GameEvent::PassRelease {
            passer_id: passer_id.to_string(),
            receiver_id: receiver_id.to_string(),
            from_pos: (from_pos.x, from_pos.y),
            to_pos: (target_lead_pos.x, target_lead_pos.y),
        });
        self.journal.current_event = Some(if inbound {
            "INBOUND_PASS".to_string()
        } else {
            "PASS".to_string()
        });
        self.journal.current_callout = Some(if inbound {
            "发球人将球传入场内".to_string()
        } else {
            "队友之间传球".to_string()
        });
    }
}

#[cfg(test)]
mod shot_creation_source_tests {
    use super::MatchEngine;
    use nba_domain::ShotCreationSource;

    #[test]
    fn cut_reception_source_expires_after_decision_interval() {
        let mut engine = MatchEngine::new(1);
        let hoop = engine.rules().court.hoop_pos(true);
        engine.set_ball_pos_for_test(hoop, engine.rules().ball_holder_height_ft);
        if let Some(player) = engine.physics_mut_for_test().get_player_mut("H_01") {
            player.pos_ft = hoop;
        }
        engine.possession_ctx.last_cut_reception_time = Some(f32::from(0u8));
        engine.possession_ctx.last_cut_reception_player = Some("H_01".to_string());
        engine.possession_ctx.last_cut_reception_event_parent = Some(10);
        assert_eq!(
            engine.shot_creation_source("H_01", f32::from(0u8)),
            ShotCreationSource::CutReception
        );
        assert_eq!(
            engine.shot_creation_source(
                "H_01",
                engine.rules().decision_interval_seconds + engine.rules().tick_seconds
            ),
            ShotCreationSource::SetPlay
        );
    }

    #[test]
    fn offensive_rebound_source_expires_after_decision_interval() {
        let mut engine = MatchEngine::new(1);
        let hoop = engine.rules().court.hoop_pos(true);
        engine.set_ball_pos_for_test(hoop, engine.rules().ball_holder_height_ft);
        if let Some(player) = engine.physics_mut_for_test().get_player_mut("H_01") {
            player.pos_ft = hoop;
        }
        engine.possession_ctx.last_offensive_rebound_time = Some(f32::from(0u8));
        engine.possession_ctx.last_offensive_rebound_player = Some("H_01".to_string());
        engine.possession_ctx.last_offensive_rebound_event_parent = Some(10);
        assert_eq!(
            engine.shot_creation_source("H_01", f32::from(0u8)),
            ShotCreationSource::OffensiveReboundPutback
        );
        assert_eq!(
            engine.shot_creation_source(
                "H_01",
                engine.rules().decision_interval_seconds + engine.rules().tick_seconds
            ),
            ShotCreationSource::SetPlay
        );
    }

    #[test]
    fn transition_source_expires_at_transition_window() {
        let mut engine = MatchEngine::new(1);
        let hoop = engine.rules().court.hoop_pos(true);
        engine.set_ball_pos_for_test(hoop, engine.rules().ball_holder_height_ft);
        engine.possession_ctx.transition_start_time = Some(f32::from(0u8));
        engine.possession_ctx.last_transition_event_parent = Some(10);
        let window = engine.rules().tactics.transition_finish_window_seconds;
        assert_eq!(
            engine.shot_creation_source("H_01", window),
            ShotCreationSource::TransitionFinish
        );
        assert_eq!(
            engine.shot_creation_source("H_01", window + engine.rules().tick_seconds),
            ShotCreationSource::SetPlay
        );
    }

    #[test]
    fn transition_source_requires_a_rim_attempt() {
        let mut engine = MatchEngine::new(4);
        let hoop = engine.rules().court.hoop_pos(true);
        engine.set_ball_pos_for_test(
            hoop + glam::Vec2::X * (nba_domain::court::RIM_ZONE_MAX_DIST_FT + f32::from(1u8)),
            engine.rules().ball_holder_height_ft,
        );
        engine.possession_ctx.transition_start_time = Some(f32::from(0u8));
        engine.possession_ctx.last_transition_event_parent = Some(10);
        assert_eq!(
            engine.shot_creation_source(
                "H_01",
                engine.rules().tactics.transition_finish_window_seconds
            ),
            ShotCreationSource::SetPlay
        );
    }

    #[test]
    fn non_shoot_decision_retains_putback_parent_context() {
        let mut engine = MatchEngine::new(3);
        let hoop = engine.rules().court.hoop_pos(true);
        engine.set_game_flow_for_test(nba_domain::GameFlowState::LiveBall);
        engine.set_sub_phase_for_test(nba_domain::SubPhase::ActionExecution);
        engine.set_ball_state_for_test(super::BallTrajectoryKind::Held {
            carrier_id: "H_01".to_string(),
        });
        engine.set_ball_pos_for_test(hoop, engine.rules().ball_holder_height_ft);
        engine.possession_ctx.last_offensive_rebound_time = Some(engine.current_time());
        engine.possession_ctx.last_offensive_rebound_player = Some("H_01".to_string());
        engine.possession_ctx.last_offensive_rebound_event_parent = Some(10);

        engine.apply_decision_output(
            nba_decision::pipeline::DecisionOutput {
                action: nba_decision::constraint::CandidateAction::Dwell {
                    player_id: "H_01".to_string(),
                },
                trace: nba_decision::pipeline::DecisionTrace::default(),
            },
            engine.current_time(),
        );

        assert_eq!(
            engine.possession_ctx.last_offensive_rebound_event_parent,
            Some(10)
        );
    }
}
