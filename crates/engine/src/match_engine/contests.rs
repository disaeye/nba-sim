//! 对抗裁定：接触检测与结果分类、贴身切球、传球成败、篮板归属。
//!
//! 依据 `docs/architecture.md` §5 与 `docs/quality.md`：几何可达性（接触检测）
//! 与结果分类（概率掷骰）按各自事实发生的时机执行；结果作为事实传播
//! （球变为松球/被断/被接住），不存在逐 tick 概率累积。

use glam::Vec2;
use nba_domain::action_window::ActionTimeWindow;
use nba_domain::Possession;
use nba_officiating::resolution::{ResolutionLayer, ResolutionOutcome};
use nba_physics::ballistics::{BallTrajectoryKind, BallisticsEngine};
use nba_semantics::SemanticEvaluator;
use rand::Rng;

use super::MatchEngine;

#[allow(clippy::too_many_arguments)]
fn pass_contact_event_probability(
    dt: f32,
    clearance: f32,
    speed_ftps: f32,
    steal_skill: f32,
    scale: f32,
    max_speed: f32,
    steal_slope: f32,
    tip_slope: f32,
    steal_floor: f32,
    tip_floor: f32,
) -> (f32, f32) {
    let distance_factor = (1.0 - clearance / scale.max(f32::EPSILON)).clamp(0.0, 1.0);
    let skill_factor = 0.5 + steal_skill.clamp(0.0, 1.0);
    let speed_reference = max_speed.max(1.0) * 0.5;
    let speed_ratio = (speed_ftps.max(0.0) / speed_reference).clamp(0.0, 2.0);
    let transit_factor = (1.0 / (1.0 + speed_ftps.max(0.0) / speed_reference)).clamp(0.15, 1.0);
    let steal_hazard =
        (distance_factor * steal_slope * skill_factor * (1.0 - 0.3 * speed_ratio).clamp(0.25, 1.0))
            .max(steal_floor);
    let tip_hazard =
        (distance_factor * tip_slope * skill_factor * (1.0 + 0.8 * speed_ratio)).max(tip_floor);
    let probability = 1.0 - (-(steal_hazard + tip_hazard) * transit_factor * dt.max(0.0)).exp();
    (probability, steal_hazard / (steal_hazard + tip_hazard))
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod pass_contact_tests {
    use super::pass_contact_event_probability;

    fn probability(dt: f32, clearance: f32, speed: f32) -> f32 {
        pass_contact_event_probability(dt, clearance, speed, 0.7, 2.5, 85.0, 0.06, 0.10, 0.01, 0.02)
            .0
    }

    #[test]
    fn contact_probability_increases_with_dwell_time_and_closeness() {
        assert!(probability(0.4, 0.2, 20.0) > probability(0.1, 0.2, 20.0));
        assert!(probability(0.2, 0.2, 20.0) > probability(0.2, 1.8, 20.0));
    }

    #[test]
    fn fixed_time_hazard_is_stable_across_tick_sizes() {
        let one_step = probability(0.4, 0.8, 30.0);
        let four_steps = 1.0 - (1.0 - probability(0.1, 0.8, 30.0)).powi(4);
        assert!((one_step - four_steps).abs() < 1e-5);
    }

    #[test]
    fn faster_ball_spends_less_exposure_and_has_more_tip_share() {
        let slow =
            pass_contact_event_probability(0.2, 0.8, 10.0, 0.7, 2.5, 85.0, 0.06, 0.10, 0.01, 0.02);
        let fast =
            pass_contact_event_probability(0.2, 0.8, 70.0, 0.7, 2.5, 85.0, 0.06, 0.10, 0.01, 0.02);
        assert!(slow.0 > fast.0);
        assert!(fast.1 < slow.1);
    }
}

impl MatchEngine {
    /// 传球飞行中的逐 tick 接触检测与结果分类。
    ///
    /// ## 几何（接触检测）
    ///
    /// 对全部在场防守者（除接球人）：球的当前采样位置与防守者的水平
    /// 距离 ≤ `player_radius_ft + defender_reach_ft`，且球高不超过其
    /// 摸高，才构成接触。摸高 = 身高英尺 × 一半权重 + 弹跳属性 ×
    /// 一半权重 × 参考臂展（英尺换算）；身高是量纲事实（cm），只在
    /// 档案里，从两队名册按 id 查（与 `block.rs` 同一来源）。z 下限 0：
    /// 低弧平传全程可及，高吊传的弧顶在防守者位置处高于摸高、不可及。
    ///
    /// ## 掷骰（结果分类）
    ///
    /// 多人同时接触取 clearance 最小者（数值相同时按 id 排序，确定性）。
    /// 接触段累计时间，事件概率使用连续时间 hazard；防守者离开范围后
    /// 其接触状态清除，再次进入时建立新的接触段。
    ///
    /// 返回 `Some((防守者, true=抢断 / false=拨掉))`。
    pub(crate) fn resolve_pass_contact(
        &mut self,
        receiver_id: &str,
        is_home: bool,
        dt: f32,
        horizontal_speed_ftps: f32,
    ) -> Option<(String, bool)> {
        let def_team = if is_home { "away" } else { "home" };
        let ball_pos = self.ball.ball_pos_3d.0;
        let ball_z = self.ball.ball_pos_3d.1.max(0.0);
        let reach = self.config.rules.player_radius_ft + self.config.rules.defender_reach_ft;
        let policy = &self.config.rules.resolve.base_rates;
        let scale = policy.intercept_clearance_scale_ft.max(f32::EPSILON);

        let mut candidates: Vec<(String, f32, f32, f32)> = self
            .systems
            .physics
            .get_players()
            .values()
            .filter(|p| p.on_court && p.team == def_team && p.id != receiver_id)
            .filter_map(|p| {
                let clearance = (p.pos_ft - ball_pos).length();
                if clearance > reach {
                    return None;
                }
                let height_cm = self
                    .config
                    .home_team
                    .players
                    .iter()
                    .chain(self.config.away_team.players.iter())
                    .find(|roster| roster.id == p.id)
                    .map(|roster| roster.height_cm)
                    .unwrap_or(200);
                const CM_PER_FOOT: f64 = 30.48;
                const REACH_REFERENCE_FT: f32 = 7.0;
                let height_ft = (f64::from(height_cm) / CM_PER_FOOT) as f32;
                let reach_ft = height_ft * 0.5
                    + p.attributes.vertical.clamp(0.0, 1.0) * 0.5 * REACH_REFERENCE_FT;
                if ball_z > reach_ft {
                    return None;
                }
                Some((p.id.clone(), clearance, p.attributes.steal, reach_ft))
            })
            .collect();
        // 确定性顺序：接触最近者优先（数值相同时按 id 排序）。
        candidates.sort_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });

        let current_ids: std::collections::HashSet<String> = candidates
            .iter()
            .map(|candidate| candidate.0.clone())
            .collect();
        self.ball
            .pass_contact_states
            .retain(|id, _| current_ids.contains(id));
        let speed = horizontal_speed_ftps.max(0.0);
        let policy_factor =
            (policy.intercept_risk_factor_floor + policy.intercept_risk_factor_gain * 0.5).max(0.0);
        let risk_probability_scale = policy_factor;
        for (defender_id, clearance, steal_skill, _) in candidates {
            let contact = self
                .ball
                .pass_contact_states
                .entry(defender_id.clone())
                .or_insert(super::state::PassContactState {
                    duration_seconds: 0.0,
                });
            contact.duration_seconds += dt.max(0.0);
            let (base_probability, steal_share) = pass_contact_event_probability(
                dt,
                clearance,
                speed,
                steal_skill,
                scale,
                self.config.rules.ball_max_speed_ftps,
                policy.intercept_steal_slope,
                policy.intercept_tip_slope,
                policy.intercept_steal_floor,
                policy.intercept_tip_floor,
            );
            let event_probability = (base_probability * risk_probability_scale).clamp(0.0, 1.0);
            if self.systems.rng.gen::<f32>() >= event_probability {
                continue;
            }
            let is_steal = self.systems.rng.gen::<f32>() < steal_share;
            return Some((defender_id, is_steal));
        }
        None
    }

    /// Resolve a pass once at release from spatial facts, player capabilities,
    /// and the configured officiating policy. Arrival only replays this fact.
    pub(crate) fn resolve_pass_success(
        &mut self,
        passer_id: &str,
        receiver_id: &str,
        from_pos: Vec2,
        to_pos: Vec2,
    ) -> bool {
        let (Some(passer), Some(receiver)) = (
            self.systems.physics.get_player(passer_id).cloned(),
            self.systems.physics.get_player(receiver_id).cloned(),
        ) else {
            return false;
        };
        let evaluation = SemanticEvaluator::pass(
            from_pos,
            to_pos,
            passer_id,
            receiver_id,
            &self.systems.physics,
            &self.config.rules,
        );
        let catch_equilibrium = self
            .observations
            .modulation
            .get(receiver_id)
            .map(|state| state.catch_equilibrium)
            .unwrap_or(1.0);
        matches!(
            ResolutionLayer::resolve_pass_arrival(
                &passer,
                &receiver,
                evaluation.target_openness,
                catch_equilibrium,
                &self.config.rules.resolve.pass,
                self.config.rules.resolve.base_rates.pass_success,
                &mut self.systems.rng,
            ),
            ResolutionOutcome::PassReceived { .. }
        )
    }

    /// 贴身切球（on-ball poke check）的一次性裁定。
    ///
    /// ## 语义
    ///
    /// 回答「**这次持球暴露**是否被防守者切掉」。与逐 tick 接触分类同形：
    /// **在事件发生的那一刻裁定一次**，结果作为事实（loose ball）传播。
    /// 因此不存在逐 tick 概率累积。
    ///
    /// ## 为什么需要它（round-13 结构发现）
    ///
    /// 真实 NBA 失误构成中「带球丢球」占 **53.6%**（82games 2024-25 IND），
    /// 是占比最大的一类。此前引擎只有传球失败一条失误路径，
    /// `TurnoverLooseBall` 在 718 回合中只出现 1 次。
    ///
    /// ## 判定依据（均为当前时刻的事实）
    ///
    /// - 防守者是否进入 `poke_pressure_radius_ft`（几何可达）；
    /// - 持球人是否确实处于 `Held` 且未被动作窗口锁定
    ///   （锁定 = 正在投篮/传球，球不在护球状态，由调用方保证）；
    /// - 概率由 `capability::poke_check_success` 从**技能与倾向**派生；
    /// - 逐 tick 按 `poke_attempt_rate_per_sec × dt` 折算尝试频率，
    ///   使「贴身持续越久、被切风险越大」在**时间积分**意义上成立，
    ///   而不是让单 tick 概率随时长累积成必然。
    ///
    /// 返回 `Some(defender_id)` 表示切球成立。
    pub(crate) fn resolve_on_ball_poke(
        &mut self,
        carrier_id: &str,
        dt: f32,
        lock_kinematics: bool,
    ) -> Option<String> {
        // 动作窗口锁定（投篮/传球/上篮进行中）时球不在护球状态。
        if lock_kinematics {
            return None;
        }
        if !matches!(
            self.ball.ball_state,
            BallTrajectoryKind::Held { .. } | BallTrajectoryKind::Drive { .. }
        ) {
            return None;
        }
        let policy = self.config.rules.resolve.ball_security.clone();
        let offense_team = self
            .systems
            .physics
            .get_player(carrier_id)
            .map(|p| p.team.clone())?;
        let def_team = match offense_team.as_str() {
            "home" => "away",
            _ => "home",
        };
        let handler_attrs = self
            .systems
            .physics
            .get_player(carrier_id)
            .map(|p| p.attributes.clone())?;
        let handler_risk = self
            .systems
            .physics
            .get_player(carrier_id)
            .map(|p| p.tendencies.risk_tolerance)
            .unwrap_or(0.5);
        let handler = self.systems.physics.get_player(carrier_id)?;
        let carrier_pos = handler.pos_ft;
        let ball_pos = BallisticsEngine::sample_ball_position(
            &self.ball.ball_state,
            self.clock.current_time,
            self.systems.physics.get_players(),
            &self.config.rules,
        );
        let ball_velocity = BallisticsEngine::sample_ball_velocity(
            &self.ball.ball_state,
            self.clock.current_time,
            self.systems.physics.get_players(),
            &self.config.rules,
        );
        let exposure = nba_domain::capability::poke_ball_exposure(
            &self.config.rules,
            (ball_pos.0 - carrier_pos).length(),
        );
        let mut threats: Vec<(String, f32, f32, f32)> = self
            .systems
            .physics
            .get_players()
            .values()
            .filter(|p| p.on_court && p.team == def_team)
            .filter_map(|p| {
                let to_ball = ball_pos.0 - p.pos_ft;
                let distance = to_ball.length();
                if distance > policy.poke_pressure_radius_ft {
                    return None;
                }
                let direction = to_ball.normalize_or_zero();
                let closing_speed = (p.vel_ft - ball_velocity.truncate())
                    .dot(direction)
                    .max(0.0);
                let facing_pressure = p.facing_dir.dot(direction).clamp(0.0, 1.0);
                Some((p.id.clone(), distance, closing_speed, facing_pressure))
            })
            .collect();
        threats.sort_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        let dt = dt.max(0.0);
        if dt <= f32::EPSILON {
            return None;
        }
        let ball_speed = ball_velocity.length();
        for (defender_id, distance, closing_speed, facing_pressure) in threats {
            let Some(defender_attrs) = self
                .systems
                .physics
                .get_player(&defender_id)
                .map(|p| p.attributes.clone())
            else {
                continue;
            };
            let per_try = nba_domain::capability::poke_check_success_with_context(
                &self.config.rules,
                &defender_attrs,
                &handler_attrs,
                handler_risk,
                exposure,
                ball_speed,
                closing_speed,
                facing_pressure,
            );
            let proximity =
                nba_domain::capability::poke_pressure_factor(&self.config.rules, distance);
            let hazard = policy.poke_attempt_rate_per_sec * proximity * per_try;
            let hit_chance = 1.0 - (-hazard * dt).exp();
            if self.systems.rng.gen::<f32>() < hit_chance {
                return Some(defender_id);
            }
        }
        None
    }

    /// 贴身切球成立时的事实应用：球脱手 → loose ball → 失误归因。
    ///
    /// 与 `PassTipped` 的 loose ball 路径同构：球从持球人处弹出，
    /// 双方争抢；`pending_loose_ball_terminal` 记录**若攻方未能夺回**时的
    /// 回合终止原因（`TurnoverLooseBall` = 带球丢球）。
    ///
    /// 弹出方向：以防守人→持球人方向为基准（切球动作把球从护球位置拨走），
    /// 叠加由 `poke_deflection_spread_rad` 限幅的确定性伪随机偏转。
    pub(crate) fn apply_on_ball_poke(&mut self, carrier_id: &str, defender_id: &str) {
        let Some(carrier_pos) = self
            .systems
            .physics
            .get_player(carrier_id)
            .map(|p| p.pos_ft)
        else {
            return;
        };
        let Some(defender_pos) = self
            .systems
            .physics
            .get_player(defender_id)
            .map(|p| p.pos_ft)
        else {
            return;
        };
        let spread = self
            .config
            .rules
            .resolve
            .ball_security
            .poke_deflection_spread_rad;
        // 基准方向：防守人 → 持球人（球被从持球人身上拨离防守人方向）。
        let base_dir = (carrier_pos - defender_pos).normalize_or_zero();
        let base_dir = if base_dir.length_squared() <= f32::EPSILON {
            // 完全重叠时退化：取持球人朝进攻篮筐方向，保持确定性。
            let hoop = self
                .config
                .rules
                .court
                .hoop_pos(self.flow.possession == Possession::Home);
            (hoop - carrier_pos).normalize_or_zero()
        } else {
            base_dir
        };
        let angle = (self.systems.rng.gen::<f32>() * 2.0 - 1.0) * spread;
        let (sin_a, cos_a) = angle.sin_cos();
        let dir = Vec2::new(
            base_dir.x * cos_a - base_dir.y * sin_a,
            base_dir.x * sin_a + base_dir.y * cos_a,
        );
        // 球的位置取**球当前的实际坐标**，而不是持球人的身体坐标。
        //
        // 持球时球有 `ball_holder_offset_ft` 的前向/侧向偏移与弹跳相位
        // （`ballistics.rs` 的 `Held` 分支），两者并不相等。若用
        // `carrier_pos` 作为松球起点，球会在单帧内跳变该偏移量——实测
        // 3.87 ft/tick，被 `BALL_SPEED` 不变量判为 96.72 ft/s（超上限 85）。
        let ball_pos = self.ball.ball_pos_3d.0;
        // 切球是**小幅拨离**，不是全速发射：球原在持球人手里（近乎静止），
        // 被拨一下只能获得有限初速。取「绝对上限」与「本次球速上限的比例」
        // 的较小者，保证不同规则档案下都不越过 `BALL_SPEED` 不变量。
        let policy = &self.config.rules.resolve.ball_security;
        let cap = (self.config.rules.ball_max_speed_ftps
            - self.config.rules.invariant_speed_tolerance_ftps)
            .max(self.config.rules.invariant_speed_tolerance_ftps);
        let speed = policy
            .poke_ball_speed_ftps
            .min(cap * policy.poke_ball_speed_ratio)
            .max(0.0);

        self.journal
            .pending_events
            .push(nba_domain::GameEvent::BallPokedLoose {
                handler_id: carrier_id.to_string(),
                defender_id: defender_id.to_string(),
                position: (ball_pos.x, ball_pos.y),
            });
        // 归因：带球丢球（真实 NBA 占比最大的失误类型）。
        self.ball.pending_loose_ball_terminal =
            Some(nba_domain::PossessionEndCause::TurnoverLooseBall);
        self.possession_ctx.current_possession_turnover_player = Some(carrier_id.to_string());
        // 传球链断：丢弃接球人估计与传球人记录。
        self.ball.receiver_estimate = None;
        self.ball.last_passer_id = None;
        self.ball.pending_pass_receiver = None;
        self.transition_ball_state(BallTrajectoryKind::LooseBall {
            pos: ball_pos,
            vel: dir * speed,
            // 沿用球**当前的** z，与 `loose_ball_from` 同源。
            //
            // 不可硬置 `ball_holder_height_ft`：那会在长下落中让重力把
            // |v| 累积到超过 `ball_max_speed_ftps`，触发 `BALL_SPEED` Hard
            // 违规（实测 seed 31337 tick 3755 → 96.72 ft/s > 85）。
            // 球被切掉时的高度是**事实**，不应被重置。
            z: self.ball.ball_pos_3d.1,
            vel_z: 0.0,
            last_touch_team: self.flow.possession,
            // 物理最后触球人是拨球的防守人；进攻方丢球人
            // （handler）已写入 `pending_loose_ball_terminal` 归因链。
            last_touch_player: Some(defender_id.to_string()),
        });
    }

    /// Resolves the rebound winner from the landing window, then delegates
    /// the contest probability to the officiating layer.
    pub(crate) fn try_resolve_rebounder(
        &mut self,
        landing: Vec2,
        max_reach: f32,
    ) -> Option<String> {
        let defensive_team = match self.flow.possession {
            Possession::Home => "away",
            Possession::Away => "home",
        };
        let offensive_team = match self.flow.possession {
            Possession::Home => "home",
            Possession::Away => "away",
        };
        let mut offensive_candidates = self.rebound_candidates(landing, offensive_team, max_reach);
        let mut defensive_candidates = self.rebound_candidates(landing, defensive_team, max_reach);
        offensive_candidates.sort_by(|left, right| {
            left.1
                .total_cmp(&right.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        defensive_candidates.sort_by(|left, right| {
            left.1
                .total_cmp(&right.1)
                .then_with(|| left.0.cmp(&right.0))
        });

        if offensive_candidates.is_empty() && defensive_candidates.is_empty() {
            return None;
        }
        let Some((offensive_id, offensive_distance)) = offensive_candidates.first() else {
            return defensive_candidates.first().map(|(id, _)| id.clone());
        };
        let Some((defensive_id, defensive_distance)) = defensive_candidates.first() else {
            return Some(offensive_id.clone());
        };
        let Some(offensive_player) = self.systems.physics.get_player(offensive_id).cloned() else {
            return Some(defensive_id.clone());
        };
        let Some(defensive_player) = self.systems.physics.get_player(defensive_id).cloned() else {
            return Some(offensive_id.clone());
        };

        let def_boxout_bonus = nba_domain::capability::effective_defensive_boxout_bonus(
            &self.config.rules,
            &defensive_player.attributes,
        );
        let off_putback_bias = nba_domain::capability::effective_putback_bias(
            &self.config.rules,
            &offensive_player.attributes,
        );
        // 距离折扣的强度走规则通道（D27）：原为内联 0.25 / 0.15，
        // 把两个能力函数的全部影响压到有效距离的 6% 以内。
        let rebound_policy = &self.config.rules.resolve.rebound;
        let effective_def_dist = (*defensive_distance
            * (1.0 - def_boxout_bonus * rebound_policy.boxout_distance_discount))
            .max(0.0);
        let effective_off_dist = (*offensive_distance
            * (1.0 - off_putback_bias * rebound_policy.putback_distance_discount))
            .max(0.0);

        match ResolutionLayer::resolve_rebound(
            &offensive_player,
            &defensive_player,
            effective_off_dist,
            effective_def_dist,
            &self.config.rules.resolve.rebound,
            &mut self.systems.rng,
        ) {
            ResolutionOutcome::ReboundSecured { rebounder_id, .. } => Some(rebounder_id),
            _ => Some(defensive_id.clone()),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn resolve_rebounder(&mut self, landing: Vec2) -> String {
        let max_r = self.config.rules.player_radius_ft + self.config.rules.defender_reach_ft + 1.5;
        self.try_resolve_rebounder(landing, max_r)
            .or_else(|| {
                let search_radius = self
                    .config
                    .rules
                    .court
                    .width_ft
                    .hypot(self.config.rules.court.height_ft);
                self.try_resolve_rebounder(landing, search_radius)
            })
            .unwrap_or_else(|| self.new_possession_pg())
    }

    pub(crate) fn rebound_candidates(
        &self,
        landing: Vec2,
        team: &str,
        search_radius: f32,
    ) -> Vec<(String, f32)> {
        self.systems
            .physics
            .query_nearby(
                landing,
                search_radius,
                &nba_physics::EntityFilter::Team(team.to_string()),
            )
            .into_iter()
            .filter_map(|id| {
                self.systems
                    .physics
                    .get_player(&id)
                    .map(|player| (id, (player.pos_ft - landing).length()))
            })
            .collect()
    }

    /// 篮板冲抢的目标指派（evidence/problem.md §23.8）。
    ///
    /// 为什么需要：实测球在空中时守方朝球靠近的速率是攻方的约 42 倍
    /// （0.0042 vs 0.0001 ft/tick），且两者绝对值都极小——即双方都没有
    /// 实质性的抢篮板行为，攻方几乎为零。结果是 ORB% 仅 0.07–0.13
    /// （真实 0.245），每次不中直接换手。
    ///
    /// 设计（用已有规则字段，不新增内联常数）：
    /// - 每队取距球的落点最近的若干名在场上球员作为争抢者；
    /// - 指派目标点 = 球的落点，但按 `min_player_separation_ft` 绕开同队
    ///   已派球员，避免互相碰撞（分离约束在物理层仍会生效）；
    /// - 攻方按 `offensive_rebound` 属性加权决定谁去冲抢（属能力通道）；
    /// - 守方速度用其 `max_speed_ftps`，攻方用同一上限（不人为区分快慢，
    ///   因为“谁抢到”由 `resolve_rebound` 的概率决定，几何只负责让双方
    ///   真的到达球的落点附近）。
    ///
    /// 该函数只指派运动目标，不裁定归属——归属仍由 `resolve_rebound`
    /// 在球触地时裁定（概率在事件时刻裁定、事实随后回放）。
    pub(crate) fn assign_rebound_pursuit(&mut self, landing: Vec2, current_t: f32) {
        let offensive_team = match self.flow.possession {
            Possession::Home => "home",
            Possession::Away => "away",
        };
        let defensive_team = match self.flow.possession {
            Possession::Home => "away",
            Possession::Away => "home",
        };
        // 落点可站立化（此处「落点」指球的触地点，篮球术语），
        let landing = self
            .config
            .rules
            .court
            .clamp_playable(landing, self.config.rules.player_radius_ft);

        for team in [offensive_team, defensive_team] {
            let is_offense = team == offensive_team;
            // 候选：在场上球员，按（攻方：篮板属性降序 / 守方：距球落点升序）
            // 排序后取前若干名。攻方用属性是为了让 `offensive_rebound` 真正
            // 决定“谁去冲抢”（能力→行为链），而非全员无差别跑动。
            let mut squad: Vec<(String, Vec2, f32, String, String, f32)> = self
                .systems
                .physics
                .get_players()
                .values()
                .filter(|p| p.on_court && p.team == team)
                .map(|p| {
                    (
                        p.id.clone(),
                        p.pos_ft,
                        p.max_speed_ftps,
                        p.slot.clone(),
                        p.morale.clone(),
                        p.attributes.offensive_rebound,
                    )
                })
                .collect();
            if squad.is_empty() {
                continue;
            }
            if is_offense {
                squad.sort_by(|a, b| {
                    b.5.partial_cmp(&a.5)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| a.0.cmp(&b.0))
                });
            } else {
                squad.sort_by(|a, b| {
                    (a.1 - landing)
                        .length_squared()
                        .partial_cmp(&(b.1 - landing).length_squared())
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| a.0.cmp(&b.0))
                });
            }
            // 派若干名争抢：真实篮球里不是全队都冲抢，且全员挤向球的落点
            // 会立刻触发 `min_player_separation_ft` 碰撞消解（把所有人推离）。
            // 人数经 GameRules 通道（charter C1），不内联在引擎里。
            let quota = if is_offense {
                self.config.rules.rebound_crash_offense_count as usize
            } else {
                self.config.rules.rebound_boxout_defense_count as usize
            };
            let spread = self.config.rules.min_player_separation_ft;
            for (idx, (id, pos, speed, slot, morale, _attr)) in
                squad.into_iter().take(quota).enumerate()
            {
                // 同队错开：以球的落点为圆心，按序号横向排开一个分离距离，
                // 避免两名同队球员被派到同一点后互推。
                let dir = (pos - landing).normalize_or_zero();
                let perp = if dir.length_squared().abs() > f32::EPSILON {
                    Vec2::new(-dir.y, dir.x)
                } else {
                    Vec2::X
                };
                let offset =
                    (idx as f32 - (quota as f32 - f32::from(1u8)) / f32::from(2u8)) * spread;
                let target = self
                    .config
                    .rules
                    .court
                    .clamp_playable(landing + perp * offset, self.config.rules.player_radius_ft);
                let action = if is_offense { "CrashBoards" } else { "BoxOut" };
                self.systems
                    .physics
                    .set_player_target(&id, target, speed, action, &slot, &morale);
                // 动作窗口：篮板起跳准备（复用既有规则字段）。
                self.observations.active_windows.insert(
                    id.clone(),
                    ActionTimeWindow::new_rebound_jump(&id, current_t, &self.config.rules),
                );
            }
        }
    }
}
