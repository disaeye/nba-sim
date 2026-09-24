//! 势能场的体能衰减通道（plan_play.md #24）。
//!
//! 判据：防守人体能下降时，其势能场分量中的护筐引力与真空吸力
//! （两个主动跑动分量）按 `stamina_multiplier` 衰减，对位牵引（被动跟随）
//! 不衰。具体成立四件事：
//!
//! 1. 中性：stamina = 1.0 时衰减系数恒为 1，`solve_equilibrium` 输出与
//!    衰减系数恒 1 的旧常量路径逐位一致（引擎级证明：黄金哈希
//!    0xee0429df2bfdde79 在满体能路径下实测不变）；
//! 2. 单调：同一几何下体能更低，`threat_ratio` / `void_ratio` 不增
//!    （对位牵引占比相应不降）；
//! 3. floor：stamina = 0 时衰减系数恰为 `stamina_floor`；
//! 4. 扰动响应（TA7）：`stamina_floor` / `stamina_gain` 改变输出；
//!    体能越界立即 panic（fast-fail）。

use glam::Vec2;
use nba_decision::potential_field::{
    help_anchor_hoop_weight, stamina_multiplier, DefensePotentialFieldSolver,
};
use nba_domain::rules::GameRules;
use nba_domain::PotentialFieldRules;

/// 测试几何：持球人深入禁区，弱侧低位人（索引 1）与高位人（索引 2）在场，
/// 低位人的护筐引力处于激发态。
fn stamina_geometry() -> (Vec2, [Vec2; 5]) {
    let hoop = Vec2::new(88.75, 25.0);
    let off_positions = [
        Vec2::new(82.75, 25.0), // 持球人：距筐 6 ft，深度突破
        Vec2::new(86.0, 5.0),   // Low-man 对位人（被求解的防守人）
        Vec2::new(72.0, 8.0),   // High-man 对位人
        Vec2::new(82.0, 35.0),
        Vec2::new(72.0, 42.0),
    ];
    (hoop, off_positions)
}

fn solver() -> DefensePotentialFieldSolver {
    DefensePotentialFieldSolver::new(PotentialFieldRules::default())
}

#[test]
fn neutral_at_full_stamina_matches_pre_decay_bit_pattern() {
    // 中性证明方式：衰减通道的引入点只有 `w_threat` / `w_void` 两处乘法，
    // stamina = 1.0 时 `stamina_multiplier` 严格返回 1.0（IEEE 754 下
    // `x × 1.0 = x` 逐位成立，floor/gain 取任意合法值都不影响），因此
    // 两个权重与旧常量路径在浮点上完全一致。下面把衰减系数恒 1 时的
    // 权重合成（含 help_blend 锚点配方）逐表达式复刻，与新路径输出对比。
    let (hoop, off_positions) = stamina_geometry();
    let rules = GameRules::default();
    let solver = solver();
    let carrier_pos = hoop - Vec2::new(6.0, 0.0);

    let emergent =
        solver.solve_equilibrium(carrier_pos, hoop, &off_positions, 1, 0, &rules, 1.0, false);

    // 旧常量路径（衰减系数恒 1，逐表达式复刻 `solve_equilibrium`）：
    let config = &rules.tactics.defense.potential_field;
    let assigned_pos = off_positions[1];
    let carrier_dist_to_hoop = (carrier_pos - hoop).length();
    let global_threat = 1.0 / (1.0 + (carrier_dist_to_hoop / config.threat_radius_ft).powi(2));
    let assigned_dist_to_hoop = (assigned_pos - hoop).length();
    let local_rim_proximity =
        1.0 / (1.0 + (assigned_dist_to_hoop / config.rim_response_radius_ft).powi(2));
    let sag_mult = rules
        .tactics
        .defense
        .sag_multiplier
        .clamp(config.sag_multiplier_min, config.sag_multiplier_max);
    let base_sag = if assigned_dist_to_hoop > config.perimeter_attribution_ft {
        config.sag_distance_perimeter_ft
    } else {
        config.sag_distance_interior_ft
    };
    let to_hoop = (hoop - assigned_pos).normalize_or_zero();
    let to_carrier = (carrier_pos - assigned_pos).normalize_or_zero();
    let hoop_weight = help_anchor_hoop_weight(rules.tactics.defense.help_priority, config);
    let carrier_weight = 1.0 - hoop_weight;
    let shell_anchor = assigned_pos
        + (to_hoop * hoop_weight + to_carrier * carrier_weight).normalize_or_zero()
            * (base_sag * sag_mult);
    let rim_target = hoop + (carrier_pos - hoop).normalize_or_zero() * config.rim_buffer_ft;
    let w_man = config.k_man_base;
    // 索引 1 是弱侧低位人（threat_gain 2.8 路径），`is_high_man` 为 false，
    // `w_void` 保持初值 0。
    let w_threat = config.k_threat_base
        * global_threat
        * local_rim_proximity
        * config.low_man_threat_gain
        * sag_mult;
    let w_void = 0.0_f32;
    let total_weight = (w_man + w_threat + w_void).max(config.total_weight_floor);
    let equilibrium =
        (shell_anchor * w_man + rim_target * w_threat + assigned_pos * w_void) / total_weight;

    assert_eq!(emergent.threat_ratio, w_threat / total_weight);
    assert_eq!(emergent.void_ratio, w_void / total_weight);
    assert_eq!(emergent.target_pos, equilibrium);
}

#[test]
fn multiplier_is_one_at_full_stamina_for_every_floor_gain() {
    // 满体能恒等式对参数网格逐点成立：floor/gain 任取合法值，
    // stamina = 1.0 的衰减系数都严格等于 1（满体能行为与参数解耦，
    // 这是「衰减通道建立但引擎接线前行为零变化」的解析基础）。
    for floor in [0.0_f32, 0.3, 0.6, 0.9] {
        for gain in [0.5_f32, 1.0, 1.5, 4.0] {
            let config = PotentialFieldRules {
                stamina_floor: floor,
                stamina_gain: gain,
                ..PotentialFieldRules::default()
            };
            let m = stamina_multiplier(1.0, &config);
            assert_eq!(
                m, 1.0,
                "floor={floor}, gain={gain}: multiplier at full stamina must be exactly 1"
            );
        }
    }
}

#[test]
fn lower_stamina_does_not_increase_threat_or_void_ratio() {
    // 单调性：同一几何下，体能更低的防守人 threat_ratio / void_ratio 不增
    // （w_man 不衰，其占比相应不降）。
    let (hoop, off_positions) = stamina_geometry();
    let rules = GameRules::default();
    let solver = solver();

    let mut prev_threat = f32::INFINITY;
    let mut prev_void = f32::INFINITY;
    for &stamina in &[1.0_f32, 0.8, 0.6, 0.4] {
        let emergent = solver.solve_equilibrium(
            hoop - Vec2::new(6.0, 0.0),
            hoop,
            &off_positions,
            1,
            0,
            &rules,
            stamina,
            false,
        );
        assert!(
            emergent.threat_ratio <= prev_threat,
            "stamina={stamina}: threat_ratio={} rose above previous {}",
            emergent.threat_ratio,
            prev_threat
        );
        assert!(
            emergent.void_ratio <= prev_void,
            "stamina={stamina}: void_ratio={} rose above previous {}",
            emergent.void_ratio,
            prev_void
        );
        prev_threat = emergent.threat_ratio;
        prev_void = emergent.void_ratio;
    }
    // 体能耗尽时护筐占比严格低于满体能：衰减通道确实改变了输出。
    let fresh = solver.solve_equilibrium(
        hoop - Vec2::new(6.0, 0.0),
        hoop,
        &off_positions,
        1,
        0,
        &rules,
        1.0,
        false,
    );
    assert!(
        prev_threat < fresh.threat_ratio,
        "exhausted defender must hold strictly lower threat share than a fresh one"
    );
}

#[test]
fn floor_multiplier_reached_at_zero_stamina() {
    // floor 生效：stamina = 0 时衰减系数恰为 `stamina_floor`，与 gain 无关。
    for gain in [0.5_f32, 1.5, 8.0] {
        let config = PotentialFieldRules {
            stamina_floor: 0.6,
            stamina_gain: gain,
            ..PotentialFieldRules::default()
        };
        let m = stamina_multiplier(0.0, &config);
        assert_eq!(
            m, 0.6,
            "gain={gain}: multiplier at zero stamina must equal stamina_floor"
        );
    }
}

#[test]
fn parameter_perturbation_changes_output() {
    // TA7 扰动响应：`stamina_floor` / `stamina_gain` 任一扰动都必须
    // 改变低体能下的输出（场位置与护筐占比）。
    let (hoop, off_positions) = stamina_geometry();
    let rules = GameRules::default();
    let solver = solver();
    let baseline = solver.solve_equilibrium(
        hoop - Vec2::new(6.0, 0.0),
        hoop,
        &off_positions,
        1,
        0,
        &rules,
        0.4,
        false,
    );

    let mut floor_perturbed = rules.clone();
    floor_perturbed
        .tactics
        .defense
        .potential_field
        .stamina_floor = 0.2;
    let by_floor = solver.solve_equilibrium(
        hoop - Vec2::new(6.0, 0.0),
        hoop,
        &off_positions,
        1,
        0,
        &floor_perturbed,
        0.4,
        false,
    );
    assert_ne!(
        (baseline.target_pos, baseline.threat_ratio),
        (by_floor.target_pos, by_floor.threat_ratio),
        "perturbing stamina_floor must change the exhausted defender's output"
    );

    let mut gain_perturbed = rules.clone();
    gain_perturbed.tactics.defense.potential_field.stamina_gain = 6.0;
    let by_gain = solver.solve_equilibrium(
        hoop - Vec2::new(6.0, 0.0),
        hoop,
        &off_positions,
        1,
        0,
        &gain_perturbed,
        0.4,
        false,
    );
    assert_ne!(
        (baseline.target_pos, baseline.threat_ratio),
        (by_gain.target_pos, by_gain.threat_ratio),
        "perturbing stamina_gain must change the exhausted defender's output"
    );
}

#[test]
#[should_panic(expected = "stamina must be normalized in [0, 1]")]
fn out_of_range_stamina_fails_fast() {
    let (hoop, off_positions) = stamina_geometry();
    let rules = GameRules::default();
    let solver = solver();
    let _ = solver.solve_equilibrium(hoop, hoop, &off_positions, 1, 0, &rules, 1.5, false);
}
