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
use crate::play_executor::PlayExecution;
use nba_domain::play::DecisionActionFamily;

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
    pub active_play_id: Option<String>,
    pub play_adjustments: Vec<PlayCandidateAdjustment>,
    pub constraint_flags: Vec<(&'static str, String)>,
    pub flags_full: Vec<(&'static str, String, f32)>,
    pub blocked: Vec<(String, String)>,
    pub probabilities: Vec<(String, f32)>,
    pub active_constraints: Vec<&'static str>,
    /// Runtime/post findings observed while producing the decision.
    pub enforcement: Vec<String>,
}

/// 单个候选动作经过约束与 Play 调整后的稳定追踪记录。
#[derive(Debug, Clone, PartialEq)]
pub struct PlayCandidateAdjustment {
    pub label: String,
    pub action_family: DecisionActionFamily,
    pub constraint_feasible: bool,
    pub bonus: f32,
    pub soft_penalty: f32,
    pub hard_inhibited: bool,
    pub adjusted_utility: Option<f32>,
}

/// 一次已裁决的决策输出。
#[derive(Debug, Clone)]
pub struct DecisionOutput {
    pub action: CandidateAction,
    pub trace: DecisionTrace,
}

/// 持球人决策所需的只读场景输入。
pub struct OnBallDecisionContext<'ctx, 'world> {
    pub constraint_context: &'ctx ConstraintContext<'world>,
    pub carrier_id: &'ctx str,
    pub stamina: f32,
    pub morale_bias: f32,
    pub coach: &'ctx crate::modulation::CoachStrategy,
    pub active_play: Option<&'ctx PlayExecution>,
}

fn action_family(action: &CandidateAction) -> DecisionActionFamily {
    match action {
        CandidateAction::Shoot { .. } => DecisionActionFamily::Shoot,
        CandidateAction::Drive { .. } => DecisionActionFamily::Drive,
        CandidateAction::PostUp { .. } => DecisionActionFamily::PostUp,
        CandidateAction::Pass { .. } | CandidateAction::InboundPass { .. } => {
            DecisionActionFamily::Pass
        }
        CandidateAction::TripleThreatJab { .. } => DecisionActionFamily::TripleThreatJab,
        CandidateAction::Dwell { .. } | CandidateAction::Advance { .. } => {
            DecisionActionFamily::Dwell
        }
    }
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
        let decision = OnBallDecisionContext {
            constraint_context: ctx,
            carrier_id,
            stamina,
            morale_bias,
            coach,
            active_play: None,
        };
        self.decide_on_ball_with_play(&decision, rng)
    }

    /// 持球人决策并应用激活 Play 的动作族偏好与抑制。
    pub fn decide_on_ball_with_play(
        &self,
        decision: &OnBallDecisionContext<'_, '_>,
        rng: &mut impl Rng,
    ) -> Option<DecisionOutput> {
        let ctx = decision.constraint_context;
        let carrier_id = decision.carrier_id;
        let stamina = decision.stamina;
        let morale_bias = decision.morale_bias;
        let coach = decision.coach;
        let active_play = decision.active_play;
        let carrier = ctx.physics.get_player(carrier_id)?;
        let carrier_pos = carrier.pos_ft;
        let offense_team = carrier.team.clone();

        let hoop = ctx.rules.court.hoop_pos(offense_team == "home");
        let dist_to_hoop = (carrier_pos - hoop).length();
        // 底角三分是更近的直线（NBA 22ft vs 弧顶 23.75ft），
        // 必须用几何判定而非单一半径（D5.1b）。
        let is_three = ctx.rules.court.is_three_point_attempt(
            carrier_pos,
            offense_team == "home",
            ctx.rules.league.three_point_distance_ft,
            ctx.rules.league.corner_three_distance_ft,
        );

        // Stamina is normalized against the player's own capacity before it reaches the decision model.
        let stamina_factor = stamina.clamp(0.0, 1.0);
        let stamina_mult = stamina_factor * self.weights.stamina_sensitivity
            + (1.0 - self.weights.stamina_sensitivity);

        // 是否处于后场（决定是否必须提供「推进」候选）。
        let midcourt = ctx.rules.court.width_ft / f32::from(2u8);
        let in_backcourt = if offense_team == "home" {
            carrier_pos.x < midcourt
        } else {
            carrier_pos.x > midcourt
        };
        // Candidate actions are derived from the authoritative spatial view.
        let mut candidates: Vec<CandidateAction> = Vec::with_capacity(8);
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
                    // 球的落点与飞行时长一起求解（不动点），不采用固定领传时长。
                    to_pos: nba_physics::ballistics::BallisticsEngine::solve_pass_landing(
                        carrier_pos,
                        p,
                        ctx.rules,
                    )
                    .0,
                });
            }
        } else if ctx.ball_available_for_action() {
            // 3. 突破通道选择：评估中路、左侧与右侧走廊，避开主防人正面阻挡，寻找进攻切入角度
            let driver_finishing = ctx
                .physics
                .get_player(carrier_id)
                .map(|p| p.attributes.finishing)
                .unwrap_or(0.5);
            let drive_target = Self::select_drive_lane(
                ctx.physics.get_players(),
                ctx.rules,
                carrier_pos,
                hoop,
                &offense_team,
                driver_finishing,
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
            let is_putback_opportunity = ctx.putback_rebounder_id == Some(carrier_id);
            if !is_putback_opportunity {
                candidates.push(CandidateAction::Drive {
                    driver_id: carrier_id.to_string(),
                    from_pos: carrier_pos,
                    target_pos: drive_target,
                    move_kind,
                });
            }
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
            if !is_putback_opportunity
                && !ctx
                    .rules
                    .court
                    .is_in_lane(carrier_pos, offense_team == "home")
            {
                candidates.push(CandidateAction::TripleThreatJab {
                    player_id: carrier_id.to_string(),
                    pivot_pos: carrier_pos,
                    jab_dir,
                });
            }
            // 背身落位目标必须落在限制区外（charter §6.2 攻方三秒）：
            // 真实低位在限制区边缘外要位。目标落在限制区内时沿径向外推。
            let attacking_right = offense_team == "home";
            let base_post_dist = f32::from(8u8);
            let mut post_target = hoop + (carrier_pos - hoop).normalize_or_zero() * base_post_dist;
            let outward = (post_target - hoop).normalize_or_zero();
            let mut push_dist = f32::from(0u8);
            while ctx.rules.court.is_in_lane(post_target, attacking_right)
                && push_dist < base_post_dist + base_post_dist
            {
                push_dist += f32::from(1u8);
                post_target = hoop + outward * (base_post_dist + push_dist);
            }
            // 限制区内的持球人不得生成「停车」类候选（背身要位/原地等待）：
            // 攻方三秒规则下持球停车超过时限即违例，限制区内只保留
            // 出手/传球/突破三类移动选项。18.0 是背身候选的距离上限（ft）。
            let carrier_in_lane = ctx.rules.court.is_in_lane(carrier_pos, attacking_right);
            if !is_putback_opportunity && !carrier_in_lane && (carrier_pos - hoop).length() < 18.0 {
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
                if is_putback_opportunity {
                    break;
                }
                // 球的落点与飞行时长一起求解（不动点），不采用固定领传时长。
                let to_pos = nba_physics::ballistics::BallisticsEngine::solve_pass_landing(
                    carrier_pos,
                    p,
                    ctx.rules,
                )
                .0;
                candidates.push(CandidateAction::Pass {
                    passer_id: carrier_id.to_string(),
                    receiver_id: p.id.clone(),
                    from_pos: carrier_pos,
                    to_pos,
                });
            }
        }
        // 第一性原理：把球推进过半场是进攻方的强制义务（8 秒规则）。
        // 若候选集里没有「推进」，持球人只能在原地 Dwell/试探，直到被判
        // 8 秒违例——实测球 x 在 8 秒内只从 8.4 移到 9.5 ft（需越过 47）。
        if in_backcourt {
            let advance_target =
                Self::advance_target(ctx.rules, offense_team == "home", carrier_pos);
            if advance_target.distance(carrier_pos) > ctx.rules.player_radius_ft {
                candidates.push(CandidateAction::Advance {
                    player_id: carrier_id.to_string(),
                    from_pos: carrier_pos,
                    target_pos: advance_target,
                });
            }
        }
        if ctx.putback_rebounder_id != Some(carrier_id)
            && !ctx
                .rules
                .court
                .is_in_lane(ctx.ball_pos, offense_team == "home")
        {
            candidates.push(CandidateAction::Dwell {
                player_id: carrier_id.to_string(),
            });
        }

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
                CandidateAction::Advance { player_id, .. } => format!("ADVANCE({})", player_id),
                // `PostUp` 与 `TripleThreatJab` 此前都落到 `OTHER`，使两个候选族
                // 在决策追踪与评判器的 `utilities` 里无法区分：诊断「PostUp
                // 从未被选中」时会得到假阴（它其实进了候选，只是标签不叫 POST_UP）。
                CandidateAction::PostUp { player_id, .. } => format!("POST_UP({})", player_id),
                CandidateAction::TripleThreatJab { player_id, .. } => {
                    format!("JAB({})", player_id)
                }
            }
        };

        // --- 约束管线 + 效用评分 ---
        // 零值是候选没有 Play 偏好或抑制时的加法单位元。
        let zero = f32::from(0u8);
        if let Some(play) = active_play {
            for effect in play.family_effects() {
                assert!(
                    effect.bonus.is_finite() && effect.bonus >= zero,
                    "play `{}` family {} bonus must be finite and non-negative",
                    play.play_id(),
                    effect.action_family.as_str()
                );
                assert!(
                    effect.soft_penalty.is_finite() && effect.soft_penalty >= zero,
                    "play `{}` family {} soft penalty must be finite and non-negative",
                    play.play_id(),
                    effect.action_family.as_str()
                );
            }
        }
        let active_constraints: Vec<&'static str> =
            self.registry.active_set(ctx).iter().map(|c| c.id).collect();
        let mut scored: Vec<(ScoredCandidate, f32)> = Vec::with_capacity(candidates.len());
        let mut flags_union: Vec<(&'static str, String)> = Vec::new();
        let mut flags_full: Vec<(&'static str, String, f32)> = Vec::new();
        let mut blocked: Vec<(String, String)> = Vec::new();
        let mut play_adjustments = Vec::with_capacity(candidates.len());
        let mut first_hard_inhibited_family = None;
        for cand in &candidates {
            let action_family = action_family(cand);
            let label = label_of(cand);
            let s = self.registry.evaluate_candidate(ctx, cand);
            if !s.feasible {
                if let Some(id) = s.blocked_by {
                    blocked.push((label.clone(), id.to_string()));
                }
                play_adjustments.push(PlayCandidateAdjustment {
                    label,
                    action_family,
                    constraint_feasible: false,
                    bonus: zero,
                    soft_penalty: zero,
                    hard_inhibited: false,
                    adjusted_utility: None,
                });
                continue; // 硬约束剔除（文档 §6.3）
            }

            let (bonus, soft_penalty, hard_inhibited) = active_play
                .map(|play| {
                    let effect = play.family_effect(action_family);
                    assert!(
                        effect.bonus.is_finite() && effect.bonus >= zero,
                        "play `{}` family {} bonus must be finite and non-negative",
                        play.play_id(),
                        action_family.as_str()
                    );
                    assert!(
                        effect.soft_penalty.is_finite() && effect.soft_penalty >= zero,
                        "play `{}` family {} soft penalty must be finite and non-negative",
                        play.play_id(),
                        action_family.as_str()
                    );
                    (effect.bonus, effect.soft_penalty, effect.hard_inhibited)
                })
                .unwrap_or((zero, zero, false));
            if hard_inhibited {
                first_hard_inhibited_family.get_or_insert(action_family);
                play_adjustments.push(PlayCandidateAdjustment {
                    label,
                    action_family,
                    constraint_feasible: true,
                    bonus,
                    soft_penalty,
                    hard_inhibited: true,
                    adjusted_utility: None,
                });
                continue;
            }

            let utility = self.utility(&s, ctx, dist_to_hoop, stamina_mult, morale_bias, coach);
            let effect_weight = self.weights.play_effect_weight;
            let adjusted_utility = utility + effect_weight * bonus - effect_weight * soft_penalty;
            assert!(
                adjusted_utility.is_finite(),
                "play `{}` produced a non-finite utility for family {}",
                active_play.map_or("<none>", PlayExecution::play_id),
                action_family.as_str()
            );
            play_adjustments.push(PlayCandidateAdjustment {
                label,
                action_family,
                constraint_feasible: true,
                bonus,
                soft_penalty,
                hard_inhibited: false,
                adjusted_utility: Some(adjusted_utility),
            });
            let new_flags: Vec<(&'static str, String)> = s
                .flags
                .iter()
                .filter(|(id, r, _)| !flags_union.iter().any(|(id2, r2)| id2 == id && r2 == r))
                .map(|(id, r, _)| (*id, r.clone()))
                .collect();
            flags_union.extend(new_flags);
            flags_full.extend(s.flags.iter().map(|(id, r, p)| (*id, r.clone(), *p)));
            scored.push((s, adjusted_utility));
        }

        if scored.is_empty() {
            if let (Some(play), Some(action_family)) = (active_play, first_hard_inhibited_family) {
                panic!(
                    "play `{}` hard-inhibits every feasible candidate; action family {}",
                    play.play_id(),
                    action_family.as_str()
                );
            }
            return None;
        }

        // --- softmax 个性化采样（文档 §6.7）---
        //
        // 曾尝试改为「分层 softmax」（先族后目标），假设是"传球族被队友数量
        // 稀释"。**该假设被实测否证**：分层后传球数反而从 1.61 降到 1.12，
        // 且实测 PASS 族效用均值 0.389 < DWELL 0.496 —— 传球效用本身就低
        // （见 `utility` 里 `(0.5 + openness)` 乘子的说明）。
        // 因此保留扁平 softmax，把修复放在效用结构上。
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
                active_play_id: active_play.map(|play| play.play_id().to_owned()),
                play_adjustments,
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
        // ## 士气必须按动作族加权（round-20 接线）
        //
        // 采样是 `exp((u - max_u) / temperature)` 的 softmax。若士气作为
        // 全候选共享的加性常数，它在归一化中完全抵消，`hot_hand_bias` /
        // `clutch_bias` / `frustrated_bias` / `exhausted_bias` 对选择分布
        // 零影响（4 个 seed 实测 0 个行为改变）。把同一个标量乘以各动作族
        // 的权重后再相加，则同一候选集合内各族的修正不同，比值不再恒定，
        // 参数扰动可改变选择分布。权重走 `ModulationRules` 通道。
        let policy = &ctx.rules.modulation;
        // 士气按**动作族**取权重：同一标量对不同候选产生不同修正。
        let morale_affinity = match &s.action {
            CandidateAction::Shoot { .. } => policy.morale_shoot_affinity,
            CandidateAction::Drive { .. } | CandidateAction::PostUp { .. } => {
                policy.morale_drive_affinity
            }
            CandidateAction::Pass { .. } | CandidateAction::InboundPass { .. } => {
                policy.morale_pass_affinity
            }
            CandidateAction::Dwell { .. }
            | CandidateAction::TripleThreatJab { .. }
            | CandidateAction::Advance { .. } => -policy.morale_dwell_affinity,
        };
        let base = match &s.action {
            CandidateAction::Advance { .. } => {
                // 推进的紧迫性：后场停留越久越必须推进（8 秒规则）。
                let one = f32::from(1u8);
                let zero = f32::from(0u8);
                let urgency =
                    (ctx.backcourt_elapsed / ctx.rules.backcourt_seconds.max(one)).clamp(zero, one);
                self.weights.dwell_base * (one + urgency * self.weights.advance_urgency_boost)
            }
            CandidateAction::Shoot {
                shooter_id,
                is_three,
                ..
            } => {
                let openness = ctx.physics.openness(shooter_id);
                let distance_factor = 1.0
                    - (dist_to_hoop / ctx.rules.shot_distance_reference_ft.max(1.0))
                        .clamp(0.0, 1.0);
                let shot_zone = ctx.rules.court.shot_zone(
                    match &s.action {
                        CandidateAction::Shoot { from_pos, .. } => *from_pos,
                        _ => ctx
                            .physics
                            .get_player(s.action.actor_id())
                            .map(|player| player.pos_ft)
                            .unwrap_or_default(),
                    },
                    ctx.possession_team == "home",
                    ctx.rules.league.three_point_distance_ft,
                    ctx.rules.league.corner_three_distance_ft,
                );
                let open_bonus = openness.contest_free_score() * self.weights.shot_openness_weight;
                let shooting_skill = attributes
                    .map(|a| {
                        // 统一出手分区（attributes.md §2.3a）：效用选技与
                        // 裁决层共用 `CourtGeometry::shot_zone`，禁止内联
                        // 距离阈值。射手出手点取自 Shoot 候选的射手位置，
                        // 攻击方向按持有球权方。
                        let shooter_pos = match &s.action {
                            CandidateAction::Shoot { from_pos, .. } => *from_pos,
                            _ => ctx
                                .physics
                                .get_player(s.action.actor_id())
                                .map(|p| p.pos_ft)
                                .unwrap_or_default(),
                        };
                        let zone = ctx.rules.court.shot_zone(
                            shooter_pos,
                            ctx.possession_team == "home",
                            ctx.rules.league.three_point_distance_ft,
                            ctx.rules.league.corner_three_distance_ft,
                        );
                        match zone {
                            nba_domain::ShotZone::Rim => a.finishing,
                            nba_domain::ShotZone::Near => a.shooting_near,
                            nba_domain::ShotZone::Mid => a.shooting_mid,
                            nba_domain::ShotZone::Three => a.shooting_three,
                        }
                    })
                    .unwrap_or(0.5);
                let shoot_preference = tendency.map(|t| t.shoot_frequency).unwrap_or(0.5);
                // G6a 链 2：前场篮板后的篮下二次攻框（putback）。判定条件是
                // 「本回合已有出手 && 持球人在篮下（ShotZone::Rim）」——
                // 这正是抢到前场板的空间事实。效用加成由
                // `effective_putback_bias`（属性曲线，finishing 驱动）提供，
                // 使高 finishing 内线更倾向直接补篮而非运出重新组织。
                // 规则字段 `rebound.putback_distance_discount`
                // 同时把有效距离折扣用于距离因子，保持同一语义通道。
                let putback_bonus = if ctx.possession_had_shot
                    && ctx.putback_rebounder_id == Some(shooter_id.as_str())
                    && dist_to_hoop < nba_domain::court::RIM_ZONE_MAX_DIST_FT
                {
                    attributes
                        .map(|a| {
                            nba_domain::effective_putback_bias(ctx.rules, a)
                                * ctx.rules.resolve.rebound.putback_distance_discount
                        })
                        .unwrap_or(0.0)
                } else {
                    0.0
                };
                let range_bias = if *is_three {
                    centered(style.three_point_emphasis) * coach.three_point_bias
                } else if dist_to_hoop < nba_domain::court::RIM_ZONE_MAX_DIST_FT {
                    centered(style.rim_pressure)
                } else {
                    0.0
                };
                let three_mult = if *is_three {
                    self.weights.three_point_utility_multiplier
                } else {
                    1.0
                };
                // 第一性原理：早出手有**机会成本**——放弃一次可能更好的
                // 后续出手。此前效用只随 shot clock 递减（紧迫加成），没有
                // 「时间价值」项，导致 46% 出手发生在 8 秒内（真实约 15%），
                // 进而使每回合传球仅 1.3 次（真实 ~3.5）。
                //
                // 用 `shot_clock_urgency_seconds` 作为「进入紧迫期」的分界：
                // 在此之前出手按剩余时间打折；进入紧迫期后折扣消失。
                let zero = f32::from(0u8);
                let one = f32::from(1u8);
                let clock_remaining = ctx.shot_clock.max(zero);
                let in_transition_rim = !*is_three
                    && dist_to_hoop < nba_domain::court::RIM_ZONE_MAX_DIST_FT
                    && ctx.possession_elapsed_seconds
                        < ctx.rules.tactics.transition_finish_window_seconds;
                let early_shot_cost = if !in_transition_rim
                    && clock_remaining > ctx.rules.shot_clock_urgency_seconds
                {
                    let slack = (clock_remaining - ctx.rules.shot_clock_urgency_seconds)
                        / (ctx.rules.league.shot_clock_seconds
                            - ctx.rules.shot_clock_urgency_seconds)
                            .max(f32::EPSILON);
                    self.weights.early_shot_penalty * slack.clamp(zero, one)
                } else {
                    zero
                };
                let transition_shoot_bonus = if in_transition_rim {
                    let window = ctx
                        .rules
                        .tactics
                        .transition_finish_window_seconds
                        .max(f32::EPSILON);
                    let remaining = (window - ctx.possession_elapsed_seconds) / window;
                    ctx.rules.tactics.transition_finish_bonus * remaining
                } else {
                    zero
                };
                let midrange_bonus = if shot_zone == nba_domain::ShotZone::Mid {
                    ctx.rules.decision.midrange_utility_bonus
                } else {
                    f32::from(0u8)
                };
                self.weights.shoot_base
                    * three_mult
                    * (0.45 + distance_factor * self.weights.shot_distance_slope + open_bonus)
                    * (1.0 + (shooting_skill - 0.5) * self.weights.tendency_weight)
                    + (shoot_preference - 0.5) * self.weights.tendency_weight
                    + midrange_bonus
                    + range_bias * self.weights.team_style_weight
                    + putback_bonus
                    + transition_shoot_bonus
                    - early_shot_cost
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
                // G6a 链 3：转换期篮下终结。回合前段（防守未落位）且目标
                // 指向篮下时，突破攻框获得窗口内线性衰减的加成；窗口外
                // 零影响，阵地战攻框比例由其余项决定（不动 v66 校准）。
                let transition_finish = {
                    let window = ctx
                        .rules
                        .tactics
                        .transition_finish_window_seconds
                        .max(f32::from(0u8));
                    if ctx.possession_elapsed_seconds < window
                        && (*target_pos - ctx.rules.court.hoop_pos(ctx.possession_team == "home"))
                            .length()
                            <= ctx.rules.tactics.drive_early_finish_dist_ft
                    {
                        let remaining = (window - ctx.possession_elapsed_seconds) / window;
                        ctx.rules.tactics.transition_finish_bonus * remaining
                    } else {
                        f32::from(0u8)
                    }
                };
                self.weights.drive_base
                    * coach.pace_factor
                    * (0.35 + (1.0 - dist_to_hoop / ctx.rules.court.width_ft.max(1.0)) * 0.45)
                    * (1.0 + (finishing_skill - 0.5) * self.weights.tendency_weight)
                    + (drive_preference - 0.5) * self.weights.tendency_weight
                    + (openness.contest_free_score() - 0.5) * self.weights.team_style_weight
                    + centered(style.rim_pressure) * self.weights.team_style_weight
                    + drive_distance.min(ctx.rules.court.width_ft) * 0.001
                    + transition_finish
                    - (1.0 - openness.contest_free_score()) * self.weights.team_style_weight * 0.5
            }
            CandidateAction::Pass {
                receiver_id,
                from_pos,
                to_pos,
                ..
            }
            | CandidateAction::InboundPass {
                receiver_id,
                from_pos,
                to_pos,
                ..
            } => {
                let openness = ctx.physics.openness(receiver_id);
                let passing_skill = attributes.map(|a| a.passing).unwrap_or(0.5);
                let pass_preference = tendency.map(|t| t.pass_frequency).unwrap_or(0.5);
                // ## 距离衰减（round-15 接线，此前声明但从未消费）
                //
                // 实测（6 场）：传球距离中位 28.3 ft、48% 超过 30 ft，
                // 而拦截率随距离单调上升（0-12ft 4.2% → 40ft+ 21.5%），
                // 短传失败率（6.4%）已与真实(~5%)一致。根因是效用只有
                // 「接球人空不空」一个空间项——最空的人往往最远，于是传球
                // 人系统性选择长传。三项参数（free/参考/上限）正是为此
                // 声明的规则通道，本次接线：
                //   factor = 1 − clamp((d − free)/(ref − free), 0, 1) × max_decay
                // 走廊风险**不**在这里重复计入——它已由约束层的
                // PASS_LANE_CONTESTED/NARROW 通道惩罚（单一事实源，charter C1）。
                let pass_distance = (*to_pos - *from_pos).length();
                let free = ctx.rules.decision.pass_distance_free_ft;
                let reference =
                    (ctx.rules.decision.pass_distance_decay_reference_ft - free).max(f32::EPSILON);
                let over = ((pass_distance - free) / reference).clamp(0.0, 1.0);
                // ## 发球传球的义务性压力（round-16）
                //
                // 发球传球受 5 秒规则约束：不发的后果是 FIVE_SECOND 失误。
                // 此前用「豁免距离衰减」修复 5 秒违例（0→22 次/场的回归），
                // 但那丢失了接球人的距离优选（发球后失败率回升）。
                // 正确的做法：保留距离衰减（优选接球人），把「必须出球」的
                // 压力加在替代动作（Dwell/Jab）上——见下方的 inbound_pressure。
                let distance_factor = 1.0 - over * ctx.rules.decision.pass_distance_max_decay;
                // G6a 链 1 的最后一环：接球人处于篮下（BackdoorCut/DipToRim
                // 切入后的落点）时，传球就是一次攻框机会（接球后直接终结）。
                // 效用加成用接球人（非持球人）的 finishing 技能驱动——内线
                // 终结者优先；无篮下接球点时零影响，外线 spacing 传球不受扰动。
                let rim_catch_bonus = {
                    let hoop = ctx.rules.court.hoop_pos(ctx.possession_team == "home");
                    let receiver_finishing = ctx
                        .physics
                        .get_player(receiver_id)
                        .map(|receiver| receiver.attributes.finishing)
                        .unwrap_or(f32::from(0u8));
                    if (*to_pos - hoop).length() < nba_domain::court::RIM_ZONE_MAX_DIST_FT {
                        receiver_finishing.max(f32::from(0u8)) * ctx.rules.decision.rim_catch_bonus
                    } else {
                        f32::from(0u8)
                    }
                };
                self.weights.pass_base
                    * (0.5 + openness.contest_free_score())
                    * distance_factor
                    * (1.0 + (passing_skill - 0.5) * self.weights.tendency_weight)
                    + (pass_preference - 0.5) * self.weights.tendency_weight
                    + centered(style.pace) * self.weights.team_style_weight
                    + rim_catch_bonus
            }
            CandidateAction::Dwell { .. } => {
                // 组织衰减：随着进攻时间消耗，持续运球观察的价值下降，
                // 24 秒违例约束兜底防止无限 Dwell。
                let time_used = (ctx.rules.league.shot_clock_seconds - ctx.shot_clock)
                    .clamp(0.0, ctx.rules.league.shot_clock_seconds);
                let decay = 1.0
                    - (time_used / ctx.rules.league.shot_clock_seconds.max(1.0))
                        .clamp(0.0, self.weights.dwell_decay_max);
                // ## 发球 5 秒压力（round-16）
                //
                // 发球阶段「继续持球」是非法选项的邻近地带：5 秒规则使
                // 不发球成为失误。压力随发球已用时间线性上升（0s 可自由
                // 观察，近 5s 必须出球），使持球观察的效用让位于发球传球
                // ——即使接球人较远（距离衰减后的效用仍高于受压的 Dwell）。
                // 修复前：豁免发球距离衰减 → 5 秒违例归零但长发球失败回升。
                let inbound_pressure = if ctx.rules.inbound_seconds > f32::EPSILON {
                    1.0 - (ctx.inbound_elapsed / ctx.rules.inbound_seconds).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                self.weights.dwell_base * decay * inbound_pressure / coach.pace_factor.max(0.1)
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
                let finishing_skill = attributes
                    .map(|a| a.finishing)
                    .unwrap_or(ctx.rules.capability.neutral_attribute);
                // ## 低位背身是独立的动作族（D27）
                //
                // 本式原先与 `Drive` 共用 `drive_base`，且距离因子用 `dist/15`
                // （而 `PostUp` 只在距篮 18 ft 内生成），因此它在距篮 8 ft 处
                // 只得 ≈0.20，而 Drive 得 ≈0.65：`PostUp` 进入候选 15 次、
                // 被选中 0 次（seed 42、20000 tick 实测）。
                //
                // 低位背身看的是**对位强弱**，不是道路空旷：
                // 用背身者的 `strength` 与对位防守人的物理性对抗得到错位优势，
                // 这是它与 Drive 的结构差异。防守人属性不可知时取中性值。
                let defender_resistance = ctx
                    .physics
                    .openness(s.action.actor_id())
                    .closest_defender_id
                    .and_then(|id| ctx.physics.get_player(&id))
                    .map(|p| {
                        nba_domain::capability::effective_post_defense_physicality(
                            ctx.rules,
                            &p.attributes,
                        )
                    })
                    .unwrap_or(ctx.rules.capability.neutral_attribute)
                    .clamp(0.0, 1.0);
                let neutral = ctx.rules.capability.neutral_attribute;
                let post_strength = attributes.map(|a| a.strength).unwrap_or(neutral);
                let body_mismatch = post_strength - defender_resistance;
                // 距离因子：低位背身在篮下附近成立，远离篮筐时快速衰减。
                // 除数取 18（与候选生成的距离门一致），使它在整个低位区间内
                // 连续取值而不是一离开篮下就被 clamp 压到下限。
                let close_factor = 1.0 - (dist / 18.0).clamp(0.0, 0.8);
                ctx.rules.decision.post_up_base
                    * finishing_skill
                    * close_factor
                    * (ctx.rules.capability.post_defense_resistance_floor
                        + ctx.rules.capability.post_defense_resistance_gain
                            * (neutral - defender_resistance))
                    * (1.0 + ctx.rules.decision.post_up_mismatch_weight * body_mismatch)
            }
        };
        let risk_aversion = match &s.action {
            CandidateAction::Pass { .. } | CandidateAction::InboundPass { .. } => {
                let tolerance = attributes
                    .map(|a| nba_domain::capability::effective_risk_tolerance(ctx.rules, a))
                    .unwrap_or(ctx.rules.capability.neutral_attribute);
                self.weights.risk_aversion * (1.0 - 0.5 * tolerance.clamp(0.0, 1.0))
            }
            _ => self.weights.risk_aversion,
        };
        base * s.feasibility_score * stamina_mult
            + morale_bias * morale_affinity
            + s.constraint_penalty
            - s.risk * risk_aversion
    }

    /// 根据防守人分布采样中路与两侧突破走廊，避免无脑直冲篮下中心
    /// 后场推进的目标点：朝中线方向前进（第一性原理：8 秒内必须过中线）。
    ///
    /// 目标点取「当前 x 与中线之间、再向进攻方向留一点余量」的位置，
    /// 使持球人按最大速度的一小部分推进即可在规则时限内越线。
    fn advance_target(
        rules: &nba_domain::GameRules,
        offense_home: bool,
        carrier_pos: Vec2,
    ) -> Vec2 {
        let midcourt = rules.court.width_ft / 2.0;
        // 越过中线后进入前场少许，避免卡在中线上反复触发后场计时。
        let overshoot = rules.court.width_ft * rules.decision.advance_overshoot_ratio;
        let target_x = if offense_home {
            (midcourt + overshoot).min(rules.court.width_ft)
        } else {
            (midcourt - overshoot).max(0.0)
        };
        Vec2::new(
            target_x,
            carrier_pos.y.clamp(
                rules.player_radius_ft,
                rules.court.height_ft - rules.player_radius_ft,
            ),
        )
    }

    fn select_drive_lane(
        players: &std::collections::HashMap<String, nba_physics::movement::PlayerPhysicsState>,
        rules: &nba_domain::GameRules,
        carrier_pos: Vec2,
        hoop: Vec2,
        offense_team: &str,
        driver_finishing: f32,
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

            // ## 冲框价值折扣（round-18）
            //
            // 拥堵是成本，但冲框有更高的期望收益（篮下 ~1.3 PPP vs 中距
            // ~0.8）。原实现只比成本，篮筐（防守最密处）几乎永不入选——
            // 实测 88% 突破停在离筐 14-18 ft，篮下出手仅 3%（真实 25-50%）。
            // 按持球人终结能力给冲框走廊折扣：终结强者应顶着防守攻框。
            let is_rim_attack =
                (clamped_target - hoop).length() <= rules.tactics.drive_early_finish_dist_ft;
            let effective = if is_rim_attack {
                (congestion
                    - rules.tactics.drive_rim_attack_bias * driver_finishing.clamp(0.0, 1.0))
                .max(0.0)
            } else {
                congestion
            };
            if effective < min_congestion {
                min_congestion = effective;
                best_target = clamped_target;
            }
        }

        best_target
    }
}
