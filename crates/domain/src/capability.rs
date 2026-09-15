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
    (base + (attributes.free_throw - 0.5) * weight).clamp(
        rules.resolve.ft_probability_floor,
        rules.resolve.ft_probability_ceiling,
    )
}

/// 突破终结成功率加成（attributes.md §4 / T2 / T5）：根据球员 finishing 属性
/// 和规则通道内的技能权重计算技能修正量。
pub fn drive_finishing_delta(rules: &GameRules, attributes: &PlayerAttributes) -> f32 {
    let weight = rules.resolve.player_skill.finishing_weight;
    (attributes.finishing.max(rules.attribute_response_floor) - 0.5) * weight * 2.0
}

/// 防守干扰有效系数（attributes.md §4 / T5）：外线/内线防守属性加权调制干扰惩罚。
pub fn effective_defense_factor(
    rules: &GameRules,
    attributes: &PlayerAttributes,
    is_interior: bool,
) -> f32 {
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

/// 接球半径（ft）：球到达时接球人能控制住的空间范围。
///
/// ## 用途
///
/// 判断「球到达时，接球人是否**在物理上可能接到**」（层 A，P-1）。
/// 这是有限信息原则的执行点：接球人按自己的预估跑位，可能跑错；
/// 跑错到超出本半径时，球落到空处 → loose ball。
///
/// ## 曲线
///
/// ```text
/// radius = base + skill_gain × ball_handling
/// ```
///
/// - `base` = `rules.catch_radius_base_ft`（无技能时的控制圈）；
/// - `skill_gain` = `rules.catch_radius_skill_gain_ft`（`ball_handling` 0→1 的增量）；
/// - `attribute_response_floor` 避免低属性死区（attributes.md §4）。
///
/// 不消费 `off_ball_sense`：那是**预估精度**（决定跑向哪里），不是
/// **接球能力**（决定能否接住）。两者分离是层 A/层 B 分工的体现。
pub fn effective_catch_radius(rules: &GameRules, attributes: &PlayerAttributes) -> f32 {
    let handling = attributes.ball_handling.max(rules.attribute_response_floor);
    rules.catch_radius_base_ft + rules.catch_radius_skill_gain_ft * handling
}

/// 接球人预估落点的误差上限（ft）。
///
/// ## 用途（P-1）
///
/// 传球人预估路线并传出；接球人**同样只能预估**。本函数给出他预估的
/// 不确定度：`off_ball_sense` 越低，偏差越大。
///
/// ```text
/// noise = rules.receive_estimate_noise_ft × (1 − off_ball_sense)
/// ```
///
/// `receive_estimate_noise_ft == 0` 时退化为 0（接球人精确知道落点）——
/// 那是**旧的全知全能行为**，仅用于向后兼容与对照实验。
pub fn receive_estimate_noise(rules: &GameRules, attributes: &PlayerAttributes) -> f32 {
    let sense = attributes.off_ball_sense.clamp(0.0, 1.0);
    rules.receive_estimate_noise_ft * (1.0 - sense)
}

/// 单次贴身切球尝试的成功概率。
///
/// ## 用途（round-13：失误构成失衡修复）
///
/// 为「持球人被贴身施压后丢球」（real NBA 占比最大的失误类型）提供
/// **能力派生**的概率。与 `resolve_pass_interception` 同形：几何可达性
/// 在调用方判定（防守者进 `poke_pressure_radius_ft`），本函数只给技能项。
///
/// ```text
/// p = ceiling − (ceiling − floor) × 护球强度
/// 护球强度 = handler_skill_weight × ball_handling
///          + handler_tendency_weight × (1 − risk_tolerance)
///            （防守人 steal 能力在另一侧以 defender_skill_weight 抬升）
/// ```
///
/// 结果夹取在 `[poke_success_floor, poke_success_ceiling]`。
///
/// 不消费持球人的 `speed`：被切球是**护球**能力问题，不是跑动能力。
pub fn poke_check_success(
    rules: &GameRules,
    defender: &PlayerAttributes,
    handler: &PlayerAttributes,
    handler_risk_tolerance: f32,
) -> f32 {
    let policy = &rules.resolve.ball_security;
    let floor = policy.poke_success_floor;
    let ceiling = policy.poke_success_ceiling;
    let floor_attr = rules.attribute_response_floor;
    let one = 1.0_f32;

    let defender_skill = defender.steal.max(floor_attr).clamp(0.0_f32, one);
    let handler_skill = handler.ball_handling.max(floor_attr).clamp(0.0_f32, one);
    // `risk_tolerance` 高 = 更愿意冒险暴露球（倾向通道，非能力通道）。
    let exposure = handler_risk_tolerance.clamp(0.0_f32, one);

    // 护球强度：能力为主、倾向为辅；两者都高才真正难以被切。
    let protection = (policy.poke_handler_skill_weight * handler_skill
        + policy.poke_handler_tendency_weight * (one - exposure))
        .clamp(0.0_f32, one);

    // 防守人技能抬升成功概率（0.5 为中性）。
    let defender_lift = policy.poke_defender_skill_weight * (defender_skill - 0.5);

    let span = (ceiling - floor).max(0.0);
    (ceiling - span * protection + defender_lift * span)
        .clamp(floor.min(ceiling), ceiling.max(floor))
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
        assert!(
            effective_defense_factor(&rules, &a, false)
                > effective_defense_factor(&rules, &a, true)
        );
    }

    #[test]
    fn decision_risk_tolerance_is_monotonic_with_mental_attributes() {
        let rules = GameRules::default();
        let mut low = attrs(0.5, 0.5);
        low.decision_iq = 0.2;
        let mut high = attrs(0.5, 0.5);
        high.decision_iq = 0.9;
        assert!(
            effective_decision_risk_tolerance(&rules, &high)
                > effective_decision_risk_tolerance(&rules, &low)
        );
    }

    #[test]
    fn catch_radius_is_monotonic_in_ball_handling() {
        let rules = GameRules::default();
        let (lo, hi) = (0.2f32, 0.9f32);
        let mut low = attrs(0.5, 0.5);
        low.ball_handling = lo;
        let mut high = attrs(0.5, 0.5);
        high.ball_handling = hi;
        let a = effective_catch_radius(&rules, &low);
        let b = effective_catch_radius(&rules, &high);
        assert!(
            b > a,
            "higher ball_handling must widen the catch radius: {a} vs {b}"
        );
        // 边界：不得为负、不得无限
        assert!(a > 0.0 && b.is_finite());
    }

    #[test]
    fn estimate_noise_is_monotonic_in_off_ball_sense() {
        let rules = GameRules::default();
        let mut poor = attrs(0.5, 0.5);
        poor.off_ball_sense = 0.1;
        let mut good = attrs(0.5, 0.5);
        good.off_ball_sense = 0.95;
        let n_poor = receive_estimate_noise(&rules, &poor);
        let n_good = receive_estimate_noise(&rules, &good);
        assert!(
            n_poor > n_good,
            "lower off_ball_sense must produce larger estimate error: {n_poor} vs {n_good}"
        );
        // 全知全能只在显式把规则置 0 时出现 —— 那是旧行为的对照开关
        let zero = GameRules {
            receive_estimate_noise_ft: 0.0,
            ..GameRules::default()
        };
        assert_eq!(receive_estimate_noise(&zero, &poor), 0.0);
    }

    #[test]
    fn catch_radius_and_estimate_noise_are_separable_channels() {
        // 接球能力（ball_handling）与预估精度（off_ball_sense）必须互不干扰：
        // 这是层 A（跑位）与层 B（接稳）分工的机械保证。
        let rules = GameRules::default();
        let mut base = attrs(0.5, 0.5);
        base.ball_handling = 0.5;
        base.off_ball_sense = 0.5;
        let r0 = effective_catch_radius(&rules, &base);
        let n0 = receive_estimate_noise(&rules, &base);

        let mut only_sense = base.clone();
        only_sense.off_ball_sense = 0.9;
        assert_eq!(
            effective_catch_radius(&rules, &only_sense),
            r0,
            "catch radius must ignore off_ball_sense"
        );
        assert!(receive_estimate_noise(&rules, &only_sense) < n0);

        let mut only_handling = base.clone();
        only_handling.ball_handling = 0.9;
        assert_eq!(
            receive_estimate_noise(&rules, &only_handling),
            n0,
            "noise must ignore ball_handling"
        );
        assert!(effective_catch_radius(&rules, &only_handling) > r0);
    }
}
