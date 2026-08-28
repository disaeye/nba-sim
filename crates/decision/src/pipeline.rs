//! 决策流水线（架构文档 §6）：约束如何进入决策系统。
//!
//! 流程：感知 → 生成候选 → 硬约束过滤 → 物理可行性 → 软约束惩罚 → 效用评分 → 个性化采样。
//!
//! 效用公式（文档 §6.6）：
//!   FinalUtility = BaseValue × Feasibility × TacticalFit × RoleFit
//!                  − SoftPenalty − RiskPenalty + PreferenceBonus

use rand::Rng;

use crate::constraint::{CandidateAction, ConstraintContext, ConstraintRegistry, ScoredCandidate};
use nba_physics::spatial::SpatialGeometry;

/// 决策效用权重（数据驱动，可调参）。
#[derive(Debug, Clone)]
pub struct DecisionWeights {
    pub shoot_base: f32,
    pub pass_base: f32,
    pub dwell_base: f32,
    /// 体力对动作欲望的折减强度
    pub stamina_sensitivity: f32,
    /// softmax 温度基准
    pub temperature: f32,
    /// 风险厌恶系数
    pub risk_aversion: f32,
}

impl Default for DecisionWeights {
    fn default() -> Self {
        Self {
            shoot_base: 0.85,
            pass_base: 0.70,
            dwell_base: 0.30,
            stamina_sensitivity: 0.5,
            temperature: 0.22,
            risk_aversion: 0.8,
        }
    }
}

/// 单次决策的完整解释（文档 Phase 8 调试层：能解释球员为什么这样做）。
#[derive(Debug, Clone, Default)]
pub struct DecisionTrace {
    pub player_id: String,
    pub chosen_kind: &'static str,
    pub utilities: Vec<(&'static str, f32)>,
    pub constraint_flags: Vec<(&'static str, String)>,
    /// 含惩罚值的完整约束触发明细
    pub flags_full: Vec<(&'static str, String, f32)>,
    /// 被硬约束剔除的候选
    pub blocked: Vec<&'static str>,
    pub probabilities: Vec<(&'static str, f32)>,
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
        Self {
            registry: ConstraintRegistry::new(),
            weights: DecisionWeights::default(),
        }
    }

    /// 持球人决策（文档 §6.2 持球人候选：投篮/传球/观察）。
    /// stamina ∈ [0,100]；morale_bias 由士气状态机调制。
    pub fn decide_on_ball(
        &self,
        ctx: &ConstraintContext,
        carrier_id: &str,
        stamina: f32,
        morale_bias: f32,
        rng: &mut impl Rng,
    ) -> Option<DecisionOutput> {
        let carrier = ctx.players.get(carrier_id)?;
        let carrier_pos = carrier.pos_ft;
        let offense_team = carrier.team.clone();

        let hoop = nba_domain::court::Court::hoop_pos(offense_team == "home");
        let dist_to_hoop = (carrier_pos - hoop).length();
        let is_three = dist_to_hoop >= 23.75;

        // 体力调制：低体力降低动作欲望（文档 §3.1.5）
        let stamina_factor = (stamina / 100.0).clamp(0.0, 1.0);
        let stamina_mult = stamina_factor * self.weights.stamina_sensitivity + (1.0 - self.weights.stamina_sensitivity);

        // --- 候选生成 ---
        let mut candidates: Vec<CandidateAction> = Vec::with_capacity(6);
        candidates.push(CandidateAction::Shoot {
            shooter_id: carrier_id.to_string(),
            from_pos: carrier_pos,
            is_three,
        });

        for p in ctx.players.values() {
            if p.team == offense_team && p.id != carrier_id {
                let to_pos = nba_physics::ballistics::BallisticsEngine::extrapolate_receiver_pos(p, 0.65);
                candidates.push(CandidateAction::Pass {
                    passer_id: carrier_id.to_string(),
                    receiver_id: p.id.clone(),
                    from_pos: carrier_pos,
                    to_pos,
                });
            }
        }
        candidates.push(CandidateAction::Dwell { player_id: carrier_id.to_string() });

        // --- 约束管线 + 效用评分 ---
        let mut scored: Vec<(ScoredCandidate, f32)> = Vec::with_capacity(candidates.len());
        let mut flags_union: Vec<(&'static str, String)> = Vec::new();
        let mut flags_full: Vec<(&'static str, String, f32)> = Vec::new();
        let mut blocked: Vec<&'static str> = Vec::new();
        for cand in &candidates {
            let s = self.registry.evaluate_candidate(ctx, cand);
            if !s.feasible {
                if let Some(id) = s.blocked_by {
                    blocked.push(id);
                }
                continue; // 硬约束剔除（文档 §6.3）
            }
            let utility = self.utility(&s, ctx, dist_to_hoop, stamina_mult, morale_bias);
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
        let max_u = scored.iter().map(|(_, u)| *u).fold(f32::NEG_INFINITY, f32::max);
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
                utilities: scored.iter().map(|(s, u)| (s.action.kind_str(), *u)).collect(),
                constraint_flags: flags_union,
                flags_full,
                blocked,
                probabilities: scored.iter().zip(probs.iter()).map(|((s, _), p)| (s.action.kind_str(), *p)).collect(),
            },
        })
    }

    /// 效用评分（文档 §6.6 公式实现）。
    fn utility(
        &self,
        s: &ScoredCandidate,
        ctx: &ConstraintContext,
        dist_to_hoop: f32,
        stamina_mult: f32,
    morale_bias: f32,
    ) -> f32 {
        let base = match &s.action {
            CandidateAction::Shoot { shooter_id, .. } => {
                let openness = SpatialGeometry::get_openness(shooter_id, ctx.players);
                // 距离衰减 × 空位加成
                let dist_factor = 1.0 - (dist_to_hoop / 40.0).clamp(0.0, 0.85);
                let open_bonus = if openness.is_open_shot { 0.5 } else { 0.0 };
                self.weights.shoot_base * (0.45 + dist_factor * 0.7 + open_bonus)
            }
            CandidateAction::Pass { receiver_id, .. } => {
                let openness = SpatialGeometry::get_openness(receiver_id, ctx.players);
                self.weights.pass_base * (0.5 + openness.contest_free_score())
            }
            CandidateAction::Dwell { .. } => self.weights.dwell_base,
        };

        // 约束惩罚与风险（soft penalty / preference bonus 已并入 constraint_penalty）
        base * stamina_mult + morale_bias - s.constraint_penalty - s.risk * self.weights.risk_aversion
    }
}
