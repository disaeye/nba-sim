use crate::constraint::ConstraintContext;
use crate::tactics::DefensiveTactic;
use glam::Vec2;
use nba_domain::data::{PlayerAttributes, PlayerTendencies};

/// 防守感知上下文：防守自主体进行多方案效用权衡时的环境切面。
pub struct DefensiveContext<'a> {
    pub defender_id: &'a str,
    pub defender_pos: Vec2,
    pub defender_vel: Vec2,
    pub defender_attrs: &'a PlayerAttributes,
    pub defender_tendencies: &'a PlayerTendencies,
    pub assignment_offense_id: Option<&'a str>,
    pub assignment_pos: Vec2,
    pub assignment_is_shooter: bool,
    pub ball_pos: Vec2,
    pub ball_flight_segment: Option<(Vec2, Vec2, f32)>, // from, to, progress
    pub ball_carrier_is_driving: bool,
    pub hoop_pos: Vec2,
    pub scheme: DefensiveTactic,
    pub base_ctx: &'a ConstraintContext<'a>,
}

/// 防守自主体生成的候选决策动作。
#[derive(Debug, Clone, PartialEq)]
pub enum DefensiveCandidateAction {
    /// 赌博抢断：脱离原对位防守人，全速扑向传球线段截断点。
    GambleInterception {
        intercept_spot: Vec2,
        failure_recovery_spot: Vec2,
    },
    /// 稳健盯防：保持在对位人与球/篮筐的合法防守位置。
    StayOnAssignment {
        target_pos: Vec2,
    },
    /// 贴身施压与试探切球 (On-Ball Pressure & Poke Check)
    OnBallPressure {
        target_pos: Vec2,
    },
    /// 扑防外线投篮并扬手起跳干扰 (Closeout & Contest)
    CloseoutContest {
        contest_pos: Vec2,
    },
    /// 提前站定造进攻犯规 (Take a Charge)
    TakeACharge {
        charge_spot: Vec2,
    },
    /// 协防护框：离开弱侧对位人，收缩至禁区封盖/干扰突破持球人。
    RotateRimHelp {
        contest_pos: Vec2,
    },
    /// 坚守外线射手：即便内线被突破，仍紧贴底角/翼侧射手防突分。
    StayOnShooter {
        shooter_pos: Vec2,
    },
    /// 绕过/挤过掩护：根据持球人射程与自身横移选择挤过(Over)或绕过(Under)。
    NavigateScreenOver {
        target_pos: Vec2,
    },
    NavigateScreenUnder {
        target_pos: Vec2,
    },
    /// 沉退防守：大个防守人在挡拆中沉退油漆区边缘封堵抛投上篮。
    DropAndContain {
        drop_pos: Vec2,
    },
    /// 延误回防：上前横移延误持球人突破路线，直至后卫重新夺回身位。
    HedgeAndRecover {
        hedge_pos: Vec2,
    },
    /// 换防：与队友完全交换对位职责。
    SwitchAssignment {
        target_pos: Vec2,
        new_assignment_id: String,
    },
}

/// 防守动作评分载荷
#[derive(Debug, Clone)]
pub struct ScoredDefensiveAction {
    pub action: DefensiveCandidateAction,
    pub utility: f32,
    pub risk: f32,
}

impl<'a> DefensiveContext<'a> {
    /// 评估赌博抢断 (GambleInterception) 的期望收益与失位代价效用。
    /// 绝不硬编码扑线！严格由到达时间可行性、抢断倾向、失位代价权衡。
    pub fn evaluate_gamble_interception(
        &self,
        from: Vec2,
        to: Vec2,
        ball_progress: f32,
    ) -> ScoredDefensiveAction {
        let segment = to - from;
        let len_sq = segment.length_squared();
        let rules = self.base_ctx.rules;
        let dec_rules = &rules.decision;

        if len_sq < 1e-4 {
            return ScoredDefensiveAction {
                action: DefensiveCandidateAction::StayOnAssignment {
                    target_pos: self.assignment_pos,
                },
                utility: 0.0,
                risk: 1.0,
            };
        }

        // 截断点：传球线段上靠近防守人位置的投影点
        let t = ((self.defender_pos - from).dot(segment) / len_sq).clamp(0.0, 1.0);
        let intercept_spot = from + segment * t;
        let dist_to_intercept = (self.defender_pos - intercept_spot).length();

        // 到达时间与球到达时间比较（物理可行性）
        let defender_speed = rules.max_player_speed_ftps * self.defender_attrs.speed.max(0.5);
        let time_to_reach = dist_to_intercept / defender_speed.max(1.0);
        let ball_remaining_ratio = (1.0 - ball_progress).clamp(0.0, 1.0);
        let ball_remaining_time = ball_remaining_ratio * rules.pass_prep_seconds.max(0.5);

        // 截断成功概率：由时间差与拦截能力综合决定
        let time_margin = ball_remaining_time - time_to_reach;
        let p_intercept = if time_margin > 0.0 {
            let reach_bonus =
                self.defender_attrs.steal * 0.4 + self.defender_attrs.off_ball_sense * 0.3;
            (0.3 + reach_bonus).clamp(0.1, 0.85)
        } else {
            0.05
        };

        // 收益：成功断球转化球权价值（受防守人抢断倾向调节）
        let reward = p_intercept
            * dec_rules.def_steal_gamble_base
            * (1.0 + (self.defender_tendencies.risk_tolerance - 0.5));

        // 代价：抢断扑空导致的严重失位代价（对位人彻底空位）
        let blow_by_risk = (1.0 - p_intercept)
            * dec_rules.def_steal_risk_penalty
            * (1.0 - self.defender_attrs.decision_iq * 0.5);

        let net_utility = reward - blow_by_risk;

        ScoredDefensiveAction {
            action: DefensiveCandidateAction::GambleInterception {
                intercept_spot,
                failure_recovery_spot: self.assignment_pos,
            },
            utility: net_utility,
            risk: blow_by_risk,
        }
    }

    /// 评估弱侧协防护框 vs 坚守底角空位射手的经典“两瓶毒药”博弈。
    /// 绝不硬编码一进油漆区就协防！
    pub fn evaluate_rim_help_vs_shooter(
        &self,
        driver_pos: Vec2,
        driver_finishing_skill: f32,
    ) -> (ScoredDefensiveAction, ScoredDefensiveAction) {
        let rules = self.base_ctx.rules;
        let dec_rules = &rules.decision;

        // 协防护框效用：阻止篮下高期望终结的收益
        let dist_to_rim = (driver_pos - self.hoop_pos).length();
        let rim_threat = (1.0 - (dist_to_rim / 15.0).clamp(0.0, 1.0)) * driver_finishing_skill;
        let contest_spot = self.hoop_pos + (driver_pos - self.hoop_pos).normalize_or_zero() * 3.5;
        let help_reward = rim_threat
            * dec_rules.def_rim_help_base
            * (self.defender_attrs.block * 0.5 + self.defender_attrs.defense_interior * 0.5);

        // 漏空位射手代价：对位射手的三分杀伤期望
        let shooter_threat = if self.assignment_is_shooter {
            dec_rules.def_corner_threat_weight * (1.0 + self.defender_attrs.decision_iq * 0.3)
        } else {
            dec_rules.def_corner_threat_weight * 0.3
        };

        let help_utility = help_reward - shooter_threat;
        let stay_utility = shooter_threat - rim_threat * 0.5;

        (
            ScoredDefensiveAction {
                action: DefensiveCandidateAction::RotateRimHelp {
                    contest_pos: contest_spot,
                },
                utility: help_utility,
                risk: shooter_threat,
            },
            ScoredDefensiveAction {
                action: DefensiveCandidateAction::StayOnShooter {
                    shooter_pos: self.assignment_pos,
                },
                utility: stay_utility,
                risk: rim_threat,
            },
        )
    }

    /// 依据防守方案中的 ScreenDefenseRules 评估挡拆防守责任动作（D17）。
    pub fn evaluate_screen_coverage(
        &self,
        screener_pos: Vec2,
        carrier_pos: Vec2,
        screener_defender_id: &str,
    ) -> ScoredDefensiveAction {
        let defense_rules = &self.base_ctx.rules.tactics.defense;
        let screen_rules = defense_rules.screen_defense;
        let dist_to_screen = (self.defender_pos - screener_pos).length();

        // 1. 换防判定 (Switch Heavy)
        if defense_rules.switch_aggressiveness > 0.6
            || (defense_rules.switch_aggressiveness > 0.2
                && dist_to_screen <= screen_rules.switch_trigger_distance_ft)
        {
            let utility = 0.7 + defense_rules.switch_aggressiveness * 0.3;
            return ScoredDefensiveAction {
                action: DefensiveCandidateAction::SwitchAssignment {
                    target_pos: carrier_pos,
                    new_assignment_id: screener_defender_id.to_string(),
                },
                utility,
                risk: 0.2,
            };
        }

        // 2. 沉退防守判定 (Drop Coverage)
        if screen_rules.drop_depth_ft > 0.0 {
            let to_hoop = (self.hoop_pos - screener_pos).normalize_or_zero();
            let drop_pos = screener_pos + to_hoop * screen_rules.drop_depth_ft;
            return ScoredDefensiveAction {
                action: DefensiveCandidateAction::DropAndContain { drop_pos },
                utility: 0.85,
                risk: 0.15,
            };
        }

        // 3. 延误回防判定 (Hedge and Recover)
        if screen_rules.hedge_distance_ft > 0.0 {
            let to_carrier = (carrier_pos - screener_pos).normalize_or_zero();
            let hedge_pos = screener_pos + to_carrier * screen_rules.hedge_distance_ft;
            return ScoredDefensiveAction {
                action: DefensiveCandidateAction::HedgeAndRecover { hedge_pos },
                utility: 0.80,
                risk: 0.25,
            };
        }

        // 4. 基准人盯人：挤过(Over)或绕过(Under)
        if self.defender_attrs.defense_perimeter > 75.0 {
            let over_pos = carrier_pos + (carrier_pos - screener_pos).normalize_or_zero() * 2.0;
            ScoredDefensiveAction {
                action: DefensiveCandidateAction::NavigateScreenOver { target_pos: over_pos },
                utility: 0.75,
                risk: 0.3,
            }
        } else {
            let under_pos = screener_pos + (self.hoop_pos - screener_pos).normalize_or_zero() * 3.0;
            ScoredDefensiveAction {
                action: DefensiveCandidateAction::NavigateScreenUnder { target_pos: under_pos },
                utility: 0.70,
                risk: 0.2,
            }
        }
    }
}
