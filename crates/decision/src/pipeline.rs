//! 决策流水线（架构文档 §6）：约束如何进入决策系统。
//!
//! 流程：感知 → 生成候选 → 硬约束过滤 → 物理可行性 → 软约束惩罚 → 效用评分 → 个性化采样。
//!
//! 效用公式（文档 §6.6）：
//!   FinalUtility = BaseValue × Feasibility × TacticalFit × RoleFit
//!                  − SoftPenalty − RiskPenalty + PreferenceBonus

use glam::Vec2;
use rand::Rng;

use crate::constraint::{CandidateAction, ConstraintContext, ConstraintRegistry, ScoredCandidate};

/// 决策效用权重（由比赛规则统一提供）。
pub type DecisionWeights = nba_domain::DecisionRules;

/// 单次决策的完整解释。
#[derive(Debug, Clone, Default)]
pub struct DecisionTrace {
    pub player_id: String,
    pub chosen_kind: &'static str,
    /// 选中候选的稳定标签（如 PASS→H_3）。
    pub chosen_label: String,
    pub utilities: Vec<(String, f32)>,
    pub constraint_flags: Vec<(&'static str, String)>,
    pub flags_full: Vec<(&'static str, String, f32)>,
    pub blocked: Vec<(String, String)>,
    pub probabilities: Vec<(String, f32)>,
    pub active_constraints: Vec<&'static str>,
    /// Runtime/post findings observed while producing the decision.
    pub enforcement: Vec<String>,
}

/// 一次已裁决的决策输出。
#[derive(Debug, Clone)]
pub struct DecisionOutput {
    pub action: CandidateAction,
    pub trace: DecisionTrace,
}

/// 决策系统：候选生成 → 约束管线 → 效用评分 → softmax 采样。
pub struct DecisionSystem {
    pub registry: ConstraintRegistry,
    pub weights: DecisionWeights,
}

impl Default for DecisionSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl DecisionSystem {
    pub fn new() -> Self {
        Self::with_weights(DecisionWeights::default())
    }

    pub fn with_weights(weights: DecisionWeights) -> Self {
        Self {
            registry: ConstraintRegistry::new(),
            weights,
        }
    }

    /// Replace decision policy at a match/setup boundary.
    pub fn set_weights(&mut self, weights: DecisionWeights) {
        self.weights = weights;
    }

    /// 持球人决策（文档 §6.2）：候选生成、约束过滤、效用评分与采样。
    /// `stamina` is the normalized current/max fraction in [0,1];
    /// `morale_bias` and `coach` are supplied by the match policy layer.
    pub fn decide_on_ball(
        &self,
        ctx: &ConstraintContext,
        carrier_id: &str,
        stamina: f32,
        morale_bias: f32,
        coach: &crate::modulation::CoachStrategy,
        rng: &mut impl Rng,
    ) -> Option<DecisionOutput> {
        let carrier = ctx.physics.get_player(carrier_id)?;
        let carrier_pos = carrier.pos_ft;
        let offense_team = carrier.team.clone();

        let hoop = ctx.rules.court.hoop_pos(offense_team == "home");
        let dist_to_hoop = (carrier_pos - hoop).length();
        let is_three = dist_to_hoop >= ctx.rules.league.three_point_distance_ft;

        // Stamina is normalized against the player's own capacity before it reaches the decision model.
        let stamina_factor = stamina.clamp(0.0, 1.0);
        let stamina_mult = stamina_factor * self.weights.stamina_sensitivity
            + (1.0 - self.weights.stamina_sensitivity);

        // Candidate actions are derived from the authoritative spatial view.
        let mut candidates: Vec<CandidateAction> = Vec::with_capacity(7);
        if ctx.game_flow == nba_domain::GameFlowState::DeadBall
            && ctx.phase == nba_domain::PhaseType::Inbound
        {
            let mut receivers: Vec<&nba_physics::movement::PlayerPhysicsState> = ctx
                .physics
                .get_players()
                .values()
                .filter(|p| p.on_court && p.team == offense_team && p.id != carrier_id)
                .collect();

            receivers.sort_by(|left, right| left.id.cmp(&right.id));
            for p in receivers {
                candidates.push(CandidateAction::InboundPass {
                    passer_id: carrier_id.to_string(),
                    receiver_id: p.id.clone(),
                    from_pos: carrier_pos,
                    to_pos: nba_physics::ballistics::BallisticsEngine::extrapolate_receiver_pos(
                        p,
                        self.weights.pass_lead_time_seconds,
                        ctx.rules,
                    ),
                });
            }
        } else if ctx.ball_available_for_action() {
            // 3. 突破通道选择：评估中路、左侧与右侧走廊，避开主防人正面阻挡，寻找进攻切入角度
            let drive_target = Self::select_drive_lane(
                ctx.physics.get_players(),
                ctx.rules,
                carrier_pos,
                hoop,
                &offense_team,
            );
            let carrier_skill = carrier.attributes.ball_handling;
            let closest_def_dist = ctx
                .physics
                .get_players()
                .values()
                .filter(|p| p.on_court && p.team != offense_team)
                .map(|d| (d.pos_ft - carrier_pos).length())
                .fold(f32::MAX, f32::min);
            let move_kind = if carrier_skill > 0.75 && closest_def_dist < 4.5 {
                Some(nba_domain::action_window::DribbleMoveKind::Crossover)
            } else if carrier_skill > 0.65 && closest_def_dist < 6.0 {
                Some(nba_domain::action_window::DribbleMoveKind::BetweenTheLegs)
            } else {
                Some(nba_domain::action_window::DribbleMoveKind::DirectDrive)
            };
            candidates.push(CandidateAction::Drive {
                driver_id: carrier_id.to_string(),
                from_pos: carrier_pos,
                target_pos: drive_target,
                move_kind,
            });
            let jumper_kind = if is_three {
                if closest_def_dist < 4.0 && carrier_skill > 0.7 {
                    Some(nba_domain::action_window::JumperKind::StepBack)
                } else {
                    Some(nba_domain::action_window::JumperKind::CatchAndShoot)
                }
            } else if dist_to_hoop > 12.0 {
                Some(nba_domain::action_window::JumperKind::PullUp)
            } else {
                Some(nba_domain::action_window::JumperKind::CatchAndShoot)
            };
            candidates.push(CandidateAction::Shoot {
                shooter_id: carrier_id.to_string(),
                from_pos: carrier_pos,
                is_three,
                jumper_kind,
            });
            let jab_dir = (hoop - carrier_pos).normalize_or_zero();
            candidates.push(CandidateAction::TripleThreatJab {
                player_id: carrier_id.to_string(),
                pivot_pos: carrier_pos,
                jab_dir,
            });
            let post_target = hoop + (carrier_pos - hoop).normalize_or_zero() * 8.0;
            if (carrier_pos - hoop).length() < 18.0 {
                candidates.push(CandidateAction::PostUp {
                    player_id: carrier_id.to_string(),
                    from_pos: carrier_pos,
                    target_pos: post_target,
                });
            }

            let mut ordered_teammates: Vec<&nba_physics::movement::PlayerPhysicsState> = ctx
                .physics
                .get_players()
                .values()
                .filter(|p| p.on_court && p.team == offense_team && p.id != carrier_id)
                .collect();
            ordered_teammates.sort_by(|left, right| left.id.cmp(&right.id));
            for p in ordered_teammates {
                let to_pos = nba_physics::ballistics::BallisticsEngine::extrapolate_receiver_pos(
                    p,
                    self.weights.pass_lead_time_seconds,
                    ctx.rules,
                );
                candidates.push(CandidateAction::Pass {
                    passer_id: carrier_id.to_string(),
                    receiver_id: p.id.clone(),
                    from_pos: carrier_pos,
                    to_pos,
                });
            }
        }
        candidates.push(CandidateAction::Dwell {
            player_id: carrier_id.to_string(),
        });

        let label_of = |c: &CandidateAction| -> String {
            match c {
                CandidateAction::Shoot {
                    shooter_id,
                    is_three,
                    ..
                } => {
                    format!(
                        "SHOOT({}{})",
                        shooter_id,
                        if *is_three { ", 3PT" } else { "" }
                    )
                }
                CandidateAction::Drive { driver_id, .. } => format!("DRIVE({})", driver_id),
                CandidateAction::Pass { receiver_id, .. }
                | CandidateAction::InboundPass { receiver_id, .. } => {
                    format!("{}→{}", c.kind_str(), receiver_id)
                }
                CandidateAction::Dwell { player_id } => format!("DWELL({})", player_id),
                _ => "OTHER".to_string(),
            }
        };

        // --- 约束管线 + 效用评分 ---
        let active_constraints: Vec<&'static str> =
            self.registry.active_set(ctx).iter().map(|c| c.id).collect();
        let mut scored: Vec<(ScoredCandidate, f32)> = Vec::with_capacity(candidates.len());
        let mut flags_union: Vec<(&'static str, String)> = Vec::new();
        let mut flags_full: Vec<(&'static str, String, f32)> = Vec::new();
        let mut blocked: Vec<(String, String)> = Vec::new();
        for cand in &candidates {
            let s = self.registry.evaluate_candidate(ctx, cand);
            if !s.feasible {
                if let Some(id) = s.blocked_by {
                    blocked.push((label_of(cand), id.to_string()));
                }
                continue; // 硬约束剔除（文档 §6.3）
            }
            let utility = self.utility(&s, ctx, dist_to_hoop, stamina_mult, morale_bias, coach);
            let new_flags: Vec<(&'static str, String)> = s
                .flags
                .iter()
                .filter(|(id, r, _)| !flags_union.iter().any(|(id2, r2)| id2 == id && r2 == r))
                .map(|(id, r, _)| (*id, r.clone()))
                .collect();
            flags_union.extend(new_flags);
            flags_full.extend(s.flags.iter().map(|(id, r, p)| (*id, r.clone(), *p)));
            scored.push((s, utility));
        }

        if scored.is_empty() {
            return None;
        }

        // --- softmax 个性化采样（文档 §6.7）---
        let max_u = scored
            .iter()
            .map(|(_, u)| *u)
            .fold(f32::NEG_INFINITY, f32::max);
        let exps: Vec<f32> = scored
            .iter()
            .map(|(_, u)| ((u - max_u) / self.weights.temperature).exp())
            .collect();
        let sum: f32 = exps.iter().sum();
        let probs: Vec<f32> = exps.iter().map(|e| e / sum).collect();

        let r = rng.gen::<f32>();
        let mut cum = 0.0;
        let mut chosen = scored.len() - 1;
        for (i, p) in probs.iter().enumerate() {
            cum += p;
            if r <= cum {
                chosen = i;
                break;
            }
        }

        let (s, _) = &scored[chosen];
        Some(DecisionOutput {
            action: s.action.clone(),
            trace: DecisionTrace {
                player_id: carrier_id.to_string(),
                chosen_kind: s.action.kind_str(),
                chosen_label: label_of(&s.action),
                utilities: scored
                    .iter()
                    .map(|(s, u)| (label_of(&s.action), *u))
                    .collect(),
                constraint_flags: flags_union,
                flags_full,
                blocked,
                probabilities: scored
                    .iter()
                    .zip(probs.iter())
                    .map(|((s, _), p)| (label_of(&s.action), *p))
                    .collect(),
                active_constraints,
                enforcement: Vec::new(),
            },
        })
    }

    fn utility(
        &self,
        s: &ScoredCandidate,
        ctx: &ConstraintContext,
        dist_to_hoop: f32,
        stamina_mult: f32,
        morale_bias: f32,
        coach: &crate::modulation::CoachStrategy,
    ) -> f32 {
        let actor = ctx.physics.get_player(s.action.actor_id());
        let tendency = actor.map(|player| &player.tendencies);
        let attributes = actor.map(|player| &player.attributes);
        let team_traits = ctx.team_traits.get(ctx.possession_team);
        let style = team_traits.cloned().unwrap_or_default();
        let centered = |value: f32| value.clamp(0.0, 1.0) - 0.5;
        let base = match &s.action {
            CandidateAction::Shoot {
                shooter_id,
                is_three,
                ..
            } => {
                let openness = ctx.physics.openness(shooter_id);
                let distance_factor = 1.0
                    - (dist_to_hoop / ctx.rules.shot_distance_reference_ft.max(1.0))
                        .clamp(0.0, 1.0);
                let open_bonus = openness.contest_free_score() * 0.5;
                let shooting_skill = attributes
                    .map(|a| {
                        if dist_to_hoop < ctx.rules.rim_shot_distance_ft {
                            a.finishing
                        } else if dist_to_hoop >= ctx.rules.league.three_point_distance_ft {
                            a.shooting_three
                        } else {
                            a.shooting_mid
                        }
                    })
                    .unwrap_or(0.5);
                let shoot_preference = tendency.map(|t| t.shoot_frequency).unwrap_or(0.5);
                let range_bias = if *is_three {
                    centered(style.three_point_emphasis) * coach.three_point_bias
                } else if dist_to_hoop <= ctx.rules.rim_shot_distance_ft {
                    centered(style.rim_pressure)
                } else {
                    0.0
                };
                let three_mult = if *is_three {
                    self.weights.three_point_utility_multiplier
                } else {
                    1.0
                };
                self.weights.shoot_base
                    * three_mult
                    * (0.45 + distance_factor * 0.7 + open_bonus)
                    * (1.0 + (shooting_skill - 0.5) * self.weights.tendency_weight)
                    + (shoot_preference - 0.5) * self.weights.tendency_weight
                    + range_bias * self.weights.team_style_weight
            }
            CandidateAction::Drive {
                driver_id,
                from_pos,
                target_pos,
                ..
            } => {
                let drive_distance = (*target_pos - *from_pos).length();
                let openness = ctx.physics.openness(driver_id);
                let drive_preference = tendency.map(|t| t.drive_frequency).unwrap_or(0.5);
                let finishing_skill = attributes.map(|a| a.finishing).unwrap_or(0.5);
                self.weights.drive_base
                    * coach.pace_factor
                    * (0.35 + (1.0 - dist_to_hoop / ctx.rules.court.width_ft.max(1.0)) * 0.45)
                    * (1.0 + (finishing_skill - 0.5) * self.weights.tendency_weight)
                    + (drive_preference - 0.5) * self.weights.tendency_weight
                    + (openness.contest_free_score() - 0.5) * self.weights.team_style_weight
                    + centered(style.rim_pressure) * self.weights.team_style_weight
                    + drive_distance.min(ctx.rules.court.width_ft) * 0.001
                    - (1.0 - openness.contest_free_score()) * self.weights.team_style_weight * 0.5
            }
            CandidateAction::Pass { receiver_id, .. }
            | CandidateAction::InboundPass { receiver_id, .. } => {
                let openness = ctx.physics.openness(receiver_id);
                let passing_skill = attributes.map(|a| a.passing).unwrap_or(0.5);
                let pass_preference = tendency.map(|t| t.pass_frequency).unwrap_or(0.5);
                self.weights.pass_base
                    * (0.5 + openness.contest_free_score())
                    * (1.0 + (passing_skill - 0.5) * self.weights.tendency_weight)
                    + (pass_preference - 0.5) * self.weights.tendency_weight
                    + centered(style.pace) * self.weights.team_style_weight
            }
            CandidateAction::Dwell { .. } => {
                // 组织衰减：随着进攻时间消耗，持续运球观察的价值下降，
                // 24 秒违例约束兜底防止无限 Dwell。
                let time_used = (ctx.rules.league.shot_clock_seconds - ctx.shot_clock)
                    .clamp(0.0, ctx.rules.league.shot_clock_seconds);
                let decay = 1.0
                    - (time_used / ctx.rules.league.shot_clock_seconds.max(1.0))
                        .clamp(0.0, self.weights.dwell_decay_max);
                self.weights.dwell_base * decay / coach.pace_factor.max(0.1)
            }
            CandidateAction::TripleThreatJab { .. } => {
                let time_used = (ctx.rules.league.shot_clock_seconds - ctx.shot_clock)
                    .clamp(0.0, ctx.rules.league.shot_clock_seconds);
                let decay = 1.0
                    - (time_used / ctx.rules.league.shot_clock_seconds.max(1.0))
                        .clamp(0.0, self.weights.dwell_decay_max);
                let early_clock_bonus = if time_used < 4.0 { 1.1 } else { 0.1 };
                self.weights.dwell_base * decay * early_clock_bonus / coach.pace_factor.max(0.1)
            }
            CandidateAction::PostUp { target_pos, .. } => {
                let hoop = ctx.rules.court.hoop_pos(ctx.possession_team == "home");
                let dist = (*target_pos - hoop).length();
                let finishing_skill = attributes.map(|a| a.finishing).unwrap_or(0.5);
                self.weights.drive_base * finishing_skill * (1.0 - (dist / 15.0).clamp(0.0, 0.8))
            }
        };
        base * s.feasibility_score * stamina_mult + morale_bias + s.constraint_penalty
            - s.risk * self.weights.risk_aversion
    }

    /// 根据防守人分布采样中路与两侧突破走廊，避免无脑直冲篮下中心
    fn select_drive_lane(
        players: &std::collections::HashMap<String, nba_physics::movement::PlayerPhysicsState>,
        rules: &nba_domain::GameRules,
        carrier_pos: Vec2,
        hoop: Vec2,
        offense_team: &str,
    ) -> Vec2 {
        let to_hoop = hoop - carrier_pos;
        let dist = to_hoop.length();
        if dist <= rules.tactics.drive_early_finish_dist_ft {
            return hoop;
        }

        let forward = to_hoop.normalize_or_zero();
        // 侧向垂直向量
        let perp = Vec2::new(-forward.y, forward.x);
        let lane_offset = rules.tactics.drive_lane_offset_ft;

        // 不把所有突破都锁到篮筐中心：中路/两侧先攻击到肘区或罚球线附近，
        // 只有对应走廊足够干净时才选择真正的冲框终点。
        let entry_dist = rules
            .tactics
            .drive_mid_range_pullup_dist_ft
            .min(dist - rules.tactics.drive_early_finish_dist_ft)
            .max(rules.tactics.drive_early_finish_dist_ft);
        let entry_center = carrier_pos + forward * (dist - entry_dist);
        let candidate_targets = [
            hoop,
            entry_center + perp * lane_offset,
            entry_center - perp * lane_offset,
        ];

        let mut best_target = hoop;
        let mut min_congestion = f32::INFINITY;

        for &target in &candidate_targets {
            let clamped_target = rules.court.clamp_playable(target, rules.player_radius_ft);

            // 评估走廊沿线防守拥挤度
            let seg_dir = clamped_target - carrier_pos;
            let seg_len = seg_dir.length();
            if seg_len <= f32::EPSILON {
                continue;
            }
            let seg_norm = seg_dir / seg_len;

            let mut congestion = 0.0;
            for p in players.values() {
                if !p.on_court || p.team == offense_team {
                    continue;
                }
                let to_p = p.pos_ft - carrier_pos;
                let proj = to_p.dot(seg_norm);
                if proj > 0.0 && proj < seg_len {
                    let perp_dist = (to_p - seg_norm * proj).length();
                    if perp_dist < rules.tactics.apf_repulsion_radius_ft {
                        let factor = (rules.tactics.apf_repulsion_radius_ft - perp_dist)
                            / rules.tactics.apf_repulsion_radius_ft;
                        congestion += factor;
                    }
                }
            }

            if congestion < min_congestion {
                min_congestion = congestion;
                best_target = clamped_target;
            }
        }

        best_target
    }
}
