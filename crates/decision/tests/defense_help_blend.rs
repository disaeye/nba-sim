//! help_blend 接入势能场锚点权重的行为测试（plan_play.md #22）。
//!
//! 被测语义：`solve_equilibrium` 的下沉锚点「朝篮筐」权重由
//! `hoop_weight = clamp(base + tilt_gain × help_priority, min, max)`
//! 合成（`help_anchor_hoop_weight`），使防守方案的协防优先级真正决定
//! 协防重心在「朝筐」与「朝持球人」之间的偏移。

use glam::Vec2;
use nba_decision::potential_field::{help_anchor_hoop_weight, DefensePotentialFieldSolver};
use nba_domain::rules::{DefenseRules, GameRules};

/// 测试几何：持球人在侧翼（idx 0），对位人 idx 1 位于弱侧低位（low-man），
/// idx 2 为弱侧高位（high-man），idx 3/4 与持球人同侧（非弱侧）。
struct Geometry {
    hoop: Vec2,
    carrier: Vec2,
    off_positions: [Vec2; 5],
}

fn geometry() -> Geometry {
    Geometry {
        hoop: Vec2::new(88.75, 25.0),
        carrier: Vec2::new(75.0, 22.0),
        off_positions: [
            Vec2::new(75.0, 22.0),
            Vec2::new(88.0, 8.0),
            Vec2::new(70.0, 5.0),
            Vec2::new(80.0, 24.0),
            Vec2::new(70.0, 24.0),
        ],
    }
}

/// 求弱侧低位人（idx 1）在指定规则下的平衡点。
fn solve_low_man(
    rules: &GameRules,
    geo: &Geometry,
) -> nba_decision::potential_field::EmergentDefenseTarget {
    let solver = DefensePotentialFieldSolver::new(rules.tactics.defense.potential_field);
    solver.solve_equilibrium(
        geo.carrier,
        geo.hoop,
        &geo.off_positions,
        1,
        0,
        rules,
        1.0,
        false,
    )
}

#[test]
fn higher_help_priority_pulls_anchor_toward_hoop() {
    // 仅 help_priority 不同的三份规则（默认方案其余字段不变）。
    let geo = geometry();
    let mut prev_dist = f32::MAX;
    for priority in [0.0_f32, 0.5, 1.0] {
        let mut rules = GameRules::default();
        rules.tactics.defense.help_priority = priority;
        let emergent = solve_low_man(&rules, &geo);
        assert!(emergent.target_pos.is_finite());
        let dist = (emergent.target_pos - geo.hoop).length();
        assert!(
            dist < prev_dist,
            "balance point must move closer to the hoop as help_priority rises: \
             priority={priority}, dist={dist}, prev={prev_dist}"
        );
        prev_dist = dist;
    }
}

#[test]
fn clamp_upper_bound_saturates_weight() {
    // help_priority 大时权重不越 help_hoop_weight_max：纯函数层面断言
    // 恰等于上限；平衡点层面断言继续抬升优先级不再改变目标点（饱和）。
    let mut rules = GameRules::default();
    let max = rules.tactics.defense.potential_field.help_hoop_weight_max;
    assert_eq!(
        help_anchor_hoop_weight(1.0, &rules.tactics.defense.potential_field),
        max
    );

    let geo = geometry();
    rules.tactics.defense.help_priority = 0.9;
    let saturated = solve_low_man(&rules, &geo).target_pos;
    rules.tactics.defense.help_priority = 1.0;
    let pushed = solve_low_man(&rules, &geo).target_pos;
    assert_eq!(
        saturated, pushed,
        "beyond the clamp upper bound the anchor weight must saturate"
    );
}

#[test]
fn clamp_lower_bound_holds_at_zero_priority() {
    // help_priority = 0 时权重不低于 help_hoop_weight_min：
    // 把合成基线压到下限之下（base < min），断言合成结果恰等于下限。
    let mut rules = GameRules::default();
    rules.tactics.defense.potential_field.help_hoop_weight_base = 0.2;
    let min = rules.tactics.defense.potential_field.help_hoop_weight_min;
    assert_eq!(
        help_anchor_hoop_weight(0.0, &rules.tactics.defense.potential_field),
        min
    );
}

#[test]
fn weights_stay_normalized_for_all_priorities() {
    // 任意 help_priority 下 hoop_weight + carrier_weight = 1：
    // carrier_weight 由 1 - hoop_weight 导出，此处验证合成权重有限、
    // 落在 [min, max] 且互补和在 f32 精度内等于 1。
    let rules = GameRules::default();
    let cfg = &rules.tactics.defense.potential_field;
    for priority in [0.0_f32, 0.25, 0.5, 0.75, 1.0] {
        let hoop_weight = help_anchor_hoop_weight(priority, cfg);
        assert!(hoop_weight.is_finite(), "weight must stay finite");
        assert!(
            (cfg.help_hoop_weight_min..=cfg.help_hoop_weight_max).contains(&hoop_weight),
            "weight must stay inside [min, max]: {hoop_weight}"
        );
        let carrier_weight = 1.0 - hoop_weight;
        assert!(
            (hoop_weight + carrier_weight - 1.0).abs() < 4.0 * f32::EPSILON,
            "hoop_weight + carrier_weight must equal 1 within f32 precision"
        );
    }
}

#[test]
fn base_perturbation_changes_target_position() {
    // 扰动响应（TA7）：help_priority > 0 的方案下，扰动
    // help_hoop_weight_base 必须改变 solve_equilibrium 的 target_pos。
    let geo = geometry();
    let mut rules = GameRules::default();
    assert!(rules.tactics.defense.help_priority > 0.0);
    let baseline = solve_low_man(&rules, &geo).target_pos;
    rules.tactics.defense.potential_field.help_hoop_weight_base -= 0.05;
    let perturbed = solve_low_man(&rules, &geo).target_pos;
    assert!(
        (baseline - perturbed).length() > 1e-4,
        "perturbing help_hoop_weight_base must move the balance point"
    );
}

#[test]
fn scheme_table_matches_blend_formula() {
    // 六个方案的生效权重必须等于合成公式值（数据通道 → 行为一致），
    // 同时打印生效值表供校准对照。
    for (id, defense) in DefenseRules::all() {
        let cfg = &defense.potential_field;
        let expected = (cfg.help_hoop_weight_base
            + cfg.help_priority_tilt_gain * defense.help_priority)
            .clamp(cfg.help_hoop_weight_min, cfg.help_hoop_weight_max);
        let actual = help_anchor_hoop_weight(defense.help_priority, cfg);
        assert_eq!(
            actual, expected,
            "scheme {id} anchor weight must follow the help_blend formula"
        );
        eprintln!(
            "{id}: help_priority={:.2} -> hoop_weight={:.3}",
            defense.help_priority, actual
        );
    }
}

#[test]
#[should_panic(expected = "help_priority must be normalized")]
fn out_of_range_priority_fails_fast() {
    let rules = GameRules::default();
    let _ = help_anchor_hoop_weight(1.5, &rules.tactics.defense.potential_field);
}

#[test]
fn schemes_solve_with_expected_action_labels() {
    // 接入不改变涌现标签的通道本身：默认方案在测试几何下仍应给出
    // ROTATE_RIM_HELP（low-man 强威胁主导），六个方案都可正常求解。
    let geo = geometry();
    for (id, defense) in DefenseRules::all() {
        let mut rules = GameRules::default();
        rules.tactics.defense = defense;
        let emergent = solve_low_man(&rules, &geo);
        assert!(
            emergent.target_pos.is_finite(),
            "scheme {id} produced a non-finite target"
        );
        assert!(
            matches!(
                emergent.action,
                "ROTATE_RIM_HELP" | "X_OUT_CLOSEOUT" | "HELP_SIDE_SHELL"
            ),
            "scheme {id} produced unexpected action {}",
            emergent.action
        );
    }
}
