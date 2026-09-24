//! 场输出滞回稳定通道（plan_play.md #21 / tactics.md §2.5）。
//!
//! 判据：双阈值之间的保持区维持上一稳定值；越过进入阈值且连续保持满
//! `min_hold_ticks` 才翻转为进入态；进入后回落到退出阈值以下且连续保持满
//! 才翻回退出态；保持计数不足时拒绝翻转；各防守人状态互相独立；
//! 阈值参数改变会改变翻转时刻。
//!
//! `solve_equilibrium` 与 `plan_possession_targets_with_geometry` 的既有
//! 行为路径不在本文件断言范围内（golden_hash 负责）；这里只验证新增的
//! 伴生通道。

use glam::Vec2;
use nba_decision::potential_field::{
    DefenseHysteresisState, DefensePotentialFieldSolver, FieldObservation,
};
use nba_domain::rules::GameRules;
use nba_domain::PotentialFieldRules;

/// 构造一份滞回参数可调的规则档案。
fn hysteresis_rules(enter: f32, exit: f32, min_hold_ticks: u32) -> PotentialFieldRules {
    PotentialFieldRules {
        help_off_enter_threat_ratio: enter,
        help_off_exit_threat_ratio: exit,
        weak_vacant_enter_void_ratio: enter,
        weak_vacant_exit_void_ratio: exit,
        min_hold_ticks,
        ..PotentialFieldRules::default()
    }
}

/// 按 `threat` 通道推 N tick（void 喂退出阈值以下的恒低值，保持该通道不动），
/// 返回 N tick 后的稳定态。
fn push_threat(
    state: &mut DefenseHysteresisState,
    rules: &PotentialFieldRules,
    threat: f32,
    ticks: usize,
) -> bool {
    for _ in 0..ticks {
        state.update(
            FieldObservation {
                threat_ratio: threat,
                void_ratio: 0.0,
            },
            rules,
        );
    }
    state.snapshot().help_pulled_off
}

#[test]
fn hold_band_between_thresholds_keeps_previous_state() {
    // 进入 0.60 > enter 0.45，退出 0.20 < exit 0.30；在 0.30–0.45 之间震荡。
    let rules = hysteresis_rules(0.45, 0.30, 2);
    let mut state = DefenseHysteresisState::new();

    // 先进入稳定态。
    assert!(push_threat(&mut state, &rules, 0.60, 4));
    // 在双阈值之间上下震荡 8 tick：稳定态必须维持。
    for i in 0..8 {
        let threat = if i % 2 == 0 { 0.40 } else { 0.35 };
        let out = state.update(
            FieldObservation {
                threat_ratio: threat,
                void_ratio: 0.0,
            },
            &rules,
        );
        assert!(
            out.help_pulled_off,
            "hold band must keep the previous stable state at tick {i}"
        );
        assert!(!out.weak_side_vacant);
    }
}

#[test]
fn enter_after_crossing_high_threshold_for_min_hold_ticks() {
    let rules = hysteresis_rules(0.45, 0.30, 3);
    let mut state = DefenseHysteresisState::new();

    // 前 2 tick 不足保持时间：不翻转。
    assert!(!push_threat(&mut state, &rules, 0.60, 2));
    // 第 3 tick 计满：翻转为进入态。
    assert!(push_threat(&mut state, &rules, 0.60, 1));
    // 之后即使数值在高位波动，进入态维持。
    assert!(push_threat(&mut state, &rules, 0.50, 2));
}

#[test]
fn exit_after_falling_below_low_threshold_for_min_hold_ticks() {
    let rules = hysteresis_rules(0.45, 0.30, 3);
    let mut state = DefenseHysteresisState::new();

    assert!(push_threat(&mut state, &rules, 0.60, 4));
    // 回落到低阈值以下，前 2 tick 不足保持时间：维持进入态。
    assert!(push_threat(&mut state, &rules, 0.20, 2));
    // 第 3 tick 计满：翻回退出态。
    assert!(!push_threat(&mut state, &rules, 0.20, 1));
}

#[test]
fn single_tick_above_enter_threshold_does_not_flip() {
    let rules = hysteresis_rules(0.45, 0.30, 4);
    let mut state = DefenseHysteresisState::new();

    // 只越过进入阈值 1 tick，随后立刻回到阈值以下（退出区间）：
    // 计数被打断，永不翻转。
    assert!(!push_threat(&mut state, &rules, 0.60, 1));
    assert!(!push_threat(&mut state, &rules, 0.10, 6));
    assert!(!push_threat(&mut state, &rules, 0.60, 1));
    assert!(!push_threat(&mut state, &rules, 0.10, 6));
    assert!(!state.snapshot().help_pulled_off);
}

#[test]
fn per_defender_states_are_independent() {
    let rules = hysteresis_rules(0.45, 0.30, 2);
    let mut a = DefenseHysteresisState::new();
    let mut b = DefenseHysteresisState::new();

    // 防守人 A 进入稳定态；B 保持在低观测值。
    assert!(push_threat(&mut a, &rules, 0.60, 3));
    assert!(!push_threat(&mut b, &rules, 0.10, 3));

    // A 的高观测不再影响判定本身；B 继续维持退出态，A 维持进入态。
    for _ in 0..5 {
        let out_a = a.update(
            FieldObservation {
                threat_ratio: 0.50,
                void_ratio: 0.0,
            },
            &rules,
        );
        let out_b = b.update(
            FieldObservation {
                threat_ratio: 0.20,
                void_ratio: 0.0,
            },
            &rules,
        );
        assert!(out_a.help_pulled_off);
        assert!(!out_b.help_pulled_off);
    }
}

#[test]
fn threshold_change_moves_flip_tick() {
    // 同一观测序列，更宽的滞回带（更低的进入阈值）应更晚翻转。
    let observation = FieldObservation {
        threat_ratio: 0.40,
        void_ratio: 0.0,
    };

    let mut strict = DefenseHysteresisState::new();
    let mut loose = DefenseHysteresisState::new();
    // strict：进入阈值 0.35——第 1 tick 就已越过，第 min_hold tick 翻转。
    let strict_rules = hysteresis_rules(0.35, 0.10, 3);
    // loose：进入阈值 0.45——观测值始终在进入阈值以下，永不翻转。
    let loose_rules = hysteresis_rules(0.45, 0.10, 3);

    let mut strict_flip: Option<usize> = None;
    let mut loose_flip: Option<usize> = None;
    for tick in 1..=6 {
        if strict_flip.is_none() && strict.update(observation, &strict_rules).help_pulled_off {
            strict_flip = Some(tick);
        }
        if loose_flip.is_none() && loose.update(observation, &loose_rules).help_pulled_off {
            loose_flip = Some(tick);
        }
    }
    assert_eq!(
        strict_flip,
        Some(3),
        "strict rules must flip at min_hold tick"
    );
    assert_eq!(
        loose_flip, None,
        "observation below enter threshold must never flip"
    );

    // 同向对照：提高进入阈值把翻转时刻推迟（0.50 需要更高观测才翻转）。
    let mut high = DefenseHysteresisState::new();
    let high_rules = hysteresis_rules(0.50, 0.10, 3);
    let mut high_flip: Option<usize> = None;
    for tick in 1..=6 {
        if high_flip.is_none() && high.update(observation, &high_rules).help_pulled_off {
            high_flip = Some(tick);
        }
    }
    assert_eq!(
        high_flip, None,
        "raising the enter threshold must delay the flip past the window"
    );
}

#[test]
fn stable_output_derives_from_solve_equilibrium() {
    // 观测量直接取 solve_equilibrium 的既有输出，不重算几何。
    let mut rules = GameRules::default();
    rules
        .tactics
        .defense
        .potential_field
        .help_off_enter_threat_ratio = 0.45;
    rules
        .tactics
        .defense
        .potential_field
        .help_off_exit_threat_ratio = 0.28;
    rules
        .tactics
        .defense
        .potential_field
        .weak_vacant_enter_void_ratio = 0.35;
    rules
        .tactics
        .defense
        .potential_field
        .weak_vacant_exit_void_ratio = 0.20;
    rules.tactics.defense.potential_field.min_hold_ticks = 2;

    let solver = DefensePotentialFieldSolver::new(rules.tactics.defense.potential_field);
    let hoop = Vec2::new(88.75, 25.0);
    let off_positions = [
        Vec2::new(60.0, 25.0),
        Vec2::new(86.0, 5.0),
        Vec2::new(72.0, 8.0),
        Vec2::new(82.0, 35.0),
        Vec2::new(72.0, 42.0),
    ];

    let mut low_man_state = DefenseHysteresisState::new();
    let mut far_state = DefenseHysteresisState::new();

    // 持球人远在外线（距筐 34 ft）：低位人 threat_ratio ≈ 0.245 低于退出阈值，
    // 双方都保持退出态。
    let far_carrier = hoop - Vec2::new(34.0, 0.0);
    for _ in 0..4 {
        let emergent =
            solver.solve_equilibrium(far_carrier, hoop, &off_positions, 1, 0, &rules, 1.0, false);
        let out = solver.observe_field(&emergent, &mut low_man_state);
        assert!(!out.help_pulled_off);
        assert!(!out.weak_side_vacant);
    }

    // 持球人深入禁区（距筐 6 ft）：低位人 threat_ratio ≈ 0.63 越过进入阈值，
    // 保持满 min_hold_ticks 后稳定进入；void 通道保持在退出区间不动。
    let deep_carrier = hoop - Vec2::new(6.0, 0.0);
    let mut low_man_flipped = false;
    for _ in 0..4 {
        let emergent =
            solver.solve_equilibrium(deep_carrier, hoop, &off_positions, 1, 0, &rules, 1.0, false);
        let out = solver.observe_field(&emergent, &mut low_man_state);
        low_man_flipped |= out.help_pulled_off;
    }
    assert!(
        low_man_flipped,
        "deep drive must pull the low-man helper into the stable entered state"
    );

    // 与持球人对位无关的远端防守人（索引 4）在同一几何下保持自己的稳定态：
    // 逐防守人伴生状态互不串扰。
    for _ in 0..4 {
        let emergent =
            solver.solve_equilibrium(deep_carrier, hoop, &off_positions, 4, 0, &rules, 1.0, false);
        let out = solver.observe_field(&emergent, &mut far_state);
        assert!(!out.weak_side_vacant);
    }
}
