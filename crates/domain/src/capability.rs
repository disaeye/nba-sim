//! 能力映射层（attributes.md §4 / T2）：归一属性与体格 → 物理量的
//! 唯一换算通道。曲线形状参数全部来自规则，禁止子系统内联属性乘法
//! 与 `.max(0.5)` 类死区。

use crate::data::PlayerAttributes;
use crate::rules::GameRules;

/// 运动学最大速度上限（ft/s）。
///
/// `rules.attribute_response_floor` 是曲线下限参数（规则通道，可校准）；
/// 属性值低于 floor 的区间不再被内联常数整体压平。
pub fn effective_max_speed(rules: &GameRules, attributes: &PlayerAttributes) -> f32 {
    rules.max_player_speed_ftps * attributes.speed.max(rules.attribute_response_floor)
}

/// 加速度上限（ft/s²）。
pub fn effective_max_accel(rules: &GameRules, attributes: &PlayerAttributes) -> f32 {
    rules.max_player_accel_ftps2 * attributes.acceleration.max(rules.attribute_response_floor)
}

/// 罚球命中概率（attributes.md T4）：全局基率 + 技能调制，夹在
/// 规则通道的上下限内。奥尼尔与库里从此不同命中率。
pub fn free_throw_probability(rules: &GameRules, attributes: &PlayerAttributes) -> f32 {
    let base = rules.resolve.base_rates.ft_make;
    let weight = rules.resolve.player_skill.free_throw_weight;
    (base + (attributes.free_throw - 0.5) * weight)
        .clamp(rules.resolve.ft_probability_floor, rules.resolve.ft_probability_ceiling)
}

/// 突破终结成功率加成（attributes.md §4 / T2 / T5）：根据球员 finishing 属性
/// 和规则通道内的技能权重计算技能修正量。
pub fn drive_finishing_delta(rules: &GameRules, attributes: &PlayerAttributes) -> f32 {
    let weight = rules.resolve.player_skill.finishing_weight;
    (attributes.finishing.max(rules.attribute_response_floor) - 0.5) * weight * 2.0
}

/// 防守干扰有效系数（attributes.md §4 / T5）：外线/内线防守属性加权调制干扰惩罚。
pub fn effective_defense_factor(rules: &GameRules, attributes: &PlayerAttributes, is_interior: bool) -> f32 {
    let raw = if is_interior {
        attributes.defense_interior
    } else {
        attributes.defense_perimeter
    };
    raw.max(rules.attribute_response_floor)
}

/// 决策感知与风险规避加成（attributes.md §4 / T5 / M9）：根据球员 basketball_iq / discipline
/// 和规则通道内的技能权重计算软约束惩罚与风险过滤修正量。高智商球员更少做出高风险愚蠢动作。
pub fn effective_decision_risk_tolerance(rules: &GameRules, attributes: &PlayerAttributes) -> f32 {
    let mental = attributes.decision_iq.max(rules.attribute_response_floor);
    // mental 越高（0.0 ~ 1.0），风险惩罚敏感度越高，更偏好稳妥路线
    1.0 + (mental - 0.5) * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::PlayerAttributes;

    fn attrs(speed: f32, ft: f32) -> PlayerAttributes {
        PlayerAttributes {
            speed,
            free_throw: ft,
            ..PlayerAttributes::default()
        }
    }

    #[test]
    fn speed_response_is_monotonic() {
        let rules = GameRules::default();
        let slow = effective_max_speed(&rules, &attrs(0.3, 0.5));
        let mid = effective_max_speed(&rules, &attrs(0.6, 0.5));
        let fast = effective_max_speed(&rules, &attrs(0.9, 0.5));
        assert!(slow < mid && mid < fast);
    }

    #[test]
    fn speed_floor_comes_from_rules_not_inline_constant() {
        let rules = GameRules {
            attribute_response_floor: 0.0,
            ..GameRules::default()
        };
        let zero = effective_max_speed(&rules, &attrs(0.0, 0.5));
        assert_eq!(zero, 0.0, "floor 0 must not flatten low attributes");
    }

    #[test]
    fn free_throw_response_is_monotonic_and_bounded() {
        let rules = GameRules::default();
        let poor = free_throw_probability(&rules, &attrs(0.5, 0.05));
        let avg = free_throw_probability(&rules, &attrs(0.5, 0.5));
        let elite = free_throw_probability(&rules, &attrs(0.5, 0.95));
        assert!(poor < avg && avg < elite);
        for p in [poor, avg, elite] {
            assert!((0.0..=1.0).contains(&p));
        }
    }

    #[test]
    fn drive_finishing_is_monotonic() {
        let rules = GameRules::default();
        let mut a1 = attrs(0.5, 0.5);
        a1.finishing = 0.3;
        let mut a2 = attrs(0.5, 0.5);
        a2.finishing = 0.8;
        assert!(drive_finishing_delta(&rules, &a1) < drive_finishing_delta(&rules, &a2));
    }

    #[test]
    fn defense_factor_differentiates_perimeter_and_interior() {
        let rules = GameRules::default();
        let mut a = attrs(0.5, 0.5);
        a.defense_perimeter = 0.85;
        a.defense_interior = 0.45;
        assert!(effective_defense_factor(&rules, &a, false) > effective_defense_factor(&rules, &a, true));
    }

    #[test]
    fn decision_risk_tolerance_is_monotonic_with_mental_attributes() {
        let rules = GameRules::default();
        let mut low = attrs(0.5, 0.5);
        low.decision_iq = 0.2;
        let mut high = attrs(0.5, 0.5);
        high.decision_iq = 0.9;
        assert!(effective_decision_risk_tolerance(&rules, &high) > effective_decision_risk_tolerance(&rules, &low));
    }
}
