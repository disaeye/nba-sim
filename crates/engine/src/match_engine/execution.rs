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
use nba_physics::movement::PlayerPhysicsState;
use nba_semantics::SemanticEvaluator;
use rand::Rng;
use std::collections::HashMap;

use super::projection::convert_trace;
use super::MatchEngine;

#[derive(Debug, Clone)]
struct DriveGeometryResolution {
    successful: bool,
    primary_defender_id: Option<String>,
    bypass_target: Option<Vec2>,
    lateral_direction: f32,
}

fn drive_speed_and_duration(
    driver: &PlayerPhysicsState,
    drive_dist: f32,
    rules: &nba_domain::GameRules,
) -> (f32, f32) {
    let drive_speed = (driver.max_speed_ftps * rules.tactics.drive_speed_ratio).max(1.0);
    let accel = rules.max_player_accel_ftps2.max(f32::EPSILON);
    let duration = (drive_dist / drive_speed + drive_speed / accel).clamp(
        rules.tactics.drive_min_duration_seconds,
        rules.tactics.drive_max_duration_seconds,
    );
    (drive_speed, duration)
}

/// 归一到 [0,1]（数值边界模式，非行为系数）。
fn clamp_unit(value: f32) -> f32 {
    value.clamp(f32::from(0u8), f32::from(1u8))
}

/// 负值归零（数值边界模式，非行为系数）。
fn non_negative(value: f32) -> f32 {
    value.max(f32::from(0u8))
}

/// 突破接触技巧的加权组合：控球 + 敏捷 + 力量（攻方用 ball_handling，
/// 守方用 defense_perimeter），权重来自 DriveGeometryPolicy。
fn drive_contact_skill(
    primary: f32,
    agility: f32,
    strength: f32,
    policy: &nba_domain::DriveGeometryPolicy,
) -> f32 {
    primary * policy.contact_skill_primary_weight
        + agility * policy.contact_skill_agility_weight
        + strength * policy.contact_skill_strength_weight
}

fn resolve_drive_geometry(
    driver: &PlayerPhysicsState,
    from_pos: Vec2,
    target_pos: Vec2,
    players: &HashMap<String, PlayerPhysicsState>,
    drive_speed: f32,
    duration: f32,
    rules: &nba_domain::GameRules,
) -> DriveGeometryResolution {
    let policy = &rules.resolve.drive_geometry;
    let contact_skill = drive_contact_skill(
        driver.attributes.ball_handling,
        driver.attributes.agility,
        driver.attributes.strength,
        policy,
    );
    let drive_vector = target_pos - from_pos;
    let drive_dist = drive_vector.length();
    if drive_dist <= f32::EPSILON {
        return DriveGeometryResolution {
            successful: false,
            primary_defender_id: None,
            bypass_target: None,
            lateral_direction: 0.0,
        };
    }
    let drive_dir = drive_vector / drive_dist;
    let perp = Vec2::new(-drive_dir.y, drive_dir.x);
    let minimum_separation = rules.min_player_separation_ft;
    let max_path_distance = drive_speed * duration;
    let lane_width = rules.tactics.drive_lane_offset_ft.max(minimum_separation);
    let mut primary_defender: Option<(&PlayerPhysicsState, (f32, f32))> = None;
    for candidate in players
        .values()
        .filter(|player| player.on_court && player.team != driver.team && player.id != driver.id)
    {
        let relative = candidate.pos_ft - from_pos;
        let along = relative.dot(drive_dir);
        let lateral = (relative - drive_dir * along).length();
        if along <= 0.0 || along >= drive_dist || lateral > lane_width + minimum_separation {
            continue;
        }
        let is_primary = primary_defender.as_ref().is_none_or(|(current, distance)| {
            (along, lateral, candidate.id.as_str()).partial_cmp(&(
                distance.0,
                distance.1,
                current.id.as_str(),
            )) == Some(std::cmp::Ordering::Less)
        });
        if is_primary {
            primary_defender = Some((candidate, (along, lateral)));
        }
    }
    let direct_path = [from_pos, target_pos];
    let direct_check = DrivePathCheck {
        points: &direct_path,
        offense_team: driver.team.as_str(),
        contact_skill,
        players,
        drive_speed,
        duration,
        minimum_separation,
        beaten_defender: None,
        policy,
    };
    if let Some(margin) = drive_path_margin(&direct_check) {
        if margin > 0.0 {
            if let Some((defender, _)) = primary_defender {
                let defender_side = (defender.pos_ft - from_pos).dot(perp);
                return DriveGeometryResolution {
                    successful: true,
                    primary_defender_id: Some(defender.id.clone()),
                    bypass_target: None,
                    lateral_direction: if defender_side >= 0.0 { -1.0 } else { 1.0 },
                };
            }
            return DriveGeometryResolution {
                successful: true,
                primary_defender_id: None,
                bypass_target: None,
                lateral_direction: 0.0,
            };
        }
    }

    let mut best_route: Option<(f32, f32, String, Vec2, f32)> = None;
    for defender in players
        .values()
        .filter(|player| player.on_court && player.team != driver.team && player.id != driver.id)
    {
        let relative = defender.pos_ft - from_pos;
        let along = relative.dot(drive_dir);
        let lateral = (relative - drive_dir * along).length();
        if along <= 0.0 || along >= drive_dist || lateral > lane_width + minimum_separation {
            continue;
        }
        let projected = from_pos + drive_dir * along;
        let clearance = minimum_separation * 2.0 + rules.player_radius_ft;
        for lateral_direction in [-1.0, 1.0] {
            let bypass_target = rules.court.clamp_playable(
                projected + perp * lateral_direction * clearance,
                rules.player_radius_ft,
            );
            let route = [from_pos, bypass_target, target_pos];
            let route_check = DrivePathCheck {
                points: &route,
                offense_team: driver.team.as_str(),
                contact_skill,
                players,
                drive_speed,
                duration,
                minimum_separation,
                beaten_defender: Some((&defender.id, 1)),
                policy,
            };
            let Some(margin) = drive_path_margin(&route_check) else {
                continue;
            };
            let path_distance: f32 = route.windows(2).map(|pair| pair[0].distance(pair[1])).sum();
            if margin <= 0.0 || path_distance > max_path_distance {
                continue;
            }
            let candidate = (
                path_distance,
                -margin,
                defender.id.clone(),
                bypass_target,
                lateral_direction,
            );
            let replace = best_route.as_ref().is_none_or(|best| {
                candidate
                    .0
                    .partial_cmp(&best.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        candidate
                            .1
                            .partial_cmp(&best.1)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .then_with(|| candidate.2.cmp(&best.2))
                    .then_with(|| {
                        candidate
                            .4
                            .partial_cmp(&best.4)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .is_lt()
            });
            if replace {
                best_route = Some(candidate);
            }
        }
    }

    if let Some((_, _, defender_id, bypass_target, lateral_direction)) = best_route {
        DriveGeometryResolution {
            successful: true,
            primary_defender_id: Some(defender_id),
            bypass_target: Some(bypass_target),
            lateral_direction,
        }
    } else if let Some((defender, _)) = primary_defender {
        DriveGeometryResolution {
            successful: false,
            primary_defender_id: Some(defender.id.clone()),
            bypass_target: None,
            lateral_direction: 0.0,
        }
    } else {
        DriveGeometryResolution {
            successful: false,
            primary_defender_id: None,
            bypass_target: None,
            lateral_direction: 0.0,
        }
    }
}

/// 突破路径接触余量评估的共享输入：路径折线、双方技巧与规则参数。
struct DrivePathCheck<'a> {
    points: &'a [Vec2],
    offense_team: &'a str,
    contact_skill: f32,
    players: &'a HashMap<String, PlayerPhysicsState>,
    drive_speed: f32,
    duration: f32,
    minimum_separation: f32,
    /// 已被绕过的防守人从指定段起不再参与接触评估。
    beaten_defender: Option<(&'a str, usize)>,
    policy: &'a nba_domain::DriveGeometryPolicy,
}

fn drive_path_margin(check: &DrivePathCheck<'_>) -> Option<f32> {
    let points = check.points;
    let path_distance: f32 = points
        .windows(2)
        .map(|pair| pair[0].distance(pair[1]))
        .sum();
    if path_distance > check.drive_speed * check.duration {
        return None;
    }

    let mut minimum_margin = f32::INFINITY;
    let mut elapsed = 0.0;
    for (segment_index, segment) in points.windows(2).enumerate() {
        let start = segment[0];
        let vector = segment[1] - start;
        let length_squared = vector.length_squared();
        if length_squared <= f32::EPSILON {
            continue;
        }
        let length = length_squared.sqrt();
        for defender in check.players.values().filter(|player| {
            player.on_court
                && player.team != check.offense_team
                && !check.beaten_defender.is_some_and(|(id, after_segment)| {
                    player.id == id && segment_index >= after_segment
                })
        }) {
            let initial_fraction =
                clamp_unit((defender.pos_ft - start).dot(vector) / length_squared);
            let arrival_time = (elapsed + length * initial_fraction / check.drive_speed)
                .clamp(f32::from(0u8), check.duration);
            let max_speed = non_negative(defender.max_speed_ftps);
            let velocity_speed = defender.vel_ft.length();
            let velocity = if velocity_speed > max_speed && velocity_speed > f32::EPSILON {
                defender.vel_ft * (max_speed / velocity_speed)
            } else {
                defender.vel_ft
            };
            let predicted_pos = defender.pos_ft + velocity * arrival_time;
            let fraction = clamp_unit((predicted_pos - start).dot(vector) / length_squared);
            let closest = start + vector * fraction;
            let defender_can_close = fraction > f32::from(0u8) && fraction < f32::from(1u8);
            let distance = (predicted_pos - closest).length();
            let stamina = clamp_unit(defender.stamina / defender.max_stamina.max(f32::EPSILON));
            let defensive_skill = drive_contact_skill(
                defender.attributes.defense_perimeter,
                defender.attributes.agility,
                defender.attributes.strength,
                check.policy,
            );
            // 防守人改变移动方向需要启动时间；只计算启动后的加速度可达距离。
            let response_time =
                non_negative(arrival_time - check.policy.defender_response_delay_seconds);
            let acceleration_reach =
                (0.5 * non_negative(defender.max_accel_ftps2) * response_time * response_time)
                    .min(non_negative(defender.max_speed_ftps) * response_time)
                    * (check.policy.reach_base_factor
                        + defensive_skill * check.policy.reach_skill_gain)
                    * stamina;
            let contact_margin = if defender_can_close {
                distance - check.minimum_separation - acceleration_reach
                    + (check.contact_skill - defensive_skill) * check.policy.contact_skill_scale
            } else {
                f32::INFINITY
            };
            minimum_margin = minimum_margin.min(contact_margin);
        }
        elapsed += length / check.drive_speed;
    }
    Some(minimum_margin)
}

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
        let defender = self.systems.physics.openness(driver_id);
        let lane_density = SemanticEvaluator::spacing(
            self.flow.possession,
            target_pos,
            &self.systems.physics,
            &self.config.rules,
        )
        .paint_crowding;
        let stamina = clamp_unit(driver.stamina / driver.max_stamina.max(f32::EPSILON));
        let defender_id = defender.closest_defender_id.clone();
        let foul_rate = defender_id
            .as_ref()
            .map(|_| self.config.rules.resolve.base_rates.foul_on_drive_rate)
            .unwrap_or(0.0);
        let drive_dist = (target_pos - from_pos).length();
        let (drive_speed, drive_duration) =
            drive_speed_and_duration(&driver, drive_dist, &self.config.rules);
        let geometry = resolve_drive_geometry(
            &driver,
            from_pos,
            target_pos,
            self.systems.physics.get_players(),
            drive_speed,
            drive_duration,
            &self.config.rules,
        );
        let policy = &self.config.rules.resolve.drive;
        let skill_delta = (driver.attributes.finishing - 0.5)
            * self.config.rules.resolve.player_skill.finishing_weight
            * policy.skill_delta_scale;
        let fatigue_delta =
            (stamina - 1.0) * self.config.rules.resolve.player_skill.finishing_weight;
        let foul_probability = (foul_rate
            * (policy.foul_base_share
                + defender.contest_intensity
                    * policy.foul_contest_weight
                    * policy.foul_contest_scale))
            .clamp(f32::from(0u8), f32::from(1u8));
        let shooting_foul = self.systems.rng.gen_bool(foul_probability as f64);
        let finish_probability = (self.config.rules.resolve.shot_type_rates.drive_finish_2pt
            + skill_delta
            + fatigue_delta
            - defender.contest_intensity
                * policy.finish_contest_penalty
                * self.config.rules.resolve.shot_type_block_bias.drive_finish
            - lane_density * policy.lane_density_penalty * policy.finish_lane_density_scale)
            .clamp(0.0, 1.0);
        let finish_made = geometry.successful
            && !shooting_foul
            && self.systems.rng.gen_bool(finish_probability as f64);
        let resolution = DriveResolution {
            successful: geometry.successful,
            finish_made,
            shooting_foul,
        };
        let fouler_id = resolution.shooting_foul.then_some(defender_id).flatten();
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
        // 只有几何路径可达且通过接触抗衡时才标记突破成功；被绕过的主防守人
        // 按既有恢复窗口追防，其他协防球员保持原有目标。
        let mut target_pos_override = None;
        if resolution.successful {
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
            successful: resolution.successful,
            finish_made: resolution.finish_made,
            fouler_id,
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
                self.start_free_throw_rebound(&shooter_id, shooter_is_home, ft_pos);
            }
        }
    }

    pub(crate) fn start_free_throw_rebound(
        &mut self,
        shooter_id: &str,
        shooter_is_home: bool,
        ft_pos: Vec2,
    ) {
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
            // 物理最后触球人是罚球出手人。
            last_touch_player: Some(shooter_id.to_string()),
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
                    &shooter_id,
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
