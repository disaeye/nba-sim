use glam::Vec2;
use nba_decision::tactics::{TacticalPlanner, TacticalSet};
use nba_domain::rules::{DefenseRules, GameRules};
use nba_domain::{Possession, SubPhase};

#[test]
fn test_drop_coverage_structure_difference() {
    let mut rules = GameRules::default();
    rules.tactics.defense = DefenseRules::for_scheme("def_drop_coverage").expect("valid scheme");
    assert!(rules.tactics.defense.screen_defense.drop_depth_ft > 0.0);

    let carrier_pos = Vec2::new(25.0, 25.0);
    let screener_pos = Vec2::new(26.0, 24.0);
    let off_positions = [
        carrier_pos,
        Vec2::new(10.0, 10.0),
        Vec2::new(10.0, 40.0),
        screener_pos,
        Vec2::new(5.0, 25.0),
    ];

    let mut rng = rand::rngs::mock::StepRng::new(42, 1);
    let (_off, def) = TacticalPlanner::plan_possession_targets_with_rules(
        TacticalSet::HighPickAndRoll,
        SubPhase::ActionExecution,
        Possession::Home,
        carrier_pos,
        0,
        2.0,
        &mut rng,
        &rules,
        Some(&off_positions),
    );

    let screener_def = &def[3];
    assert_eq!(screener_def.action, "DROP_CONTAIN");
    assert_eq!(screener_def.slot, "DropAnchor");
}

#[test]
fn test_switch_assignment_structure_difference() {
    let mut rules = GameRules::default();
    rules.tactics.defense = DefenseRules::for_scheme("def_switch_heavy").expect("valid scheme");
    assert!(rules.tactics.defense.switch_aggressiveness > 0.5);

    let carrier_pos = Vec2::new(25.0, 25.0);
    let screener_pos = Vec2::new(26.0, 24.0);
    let off_positions = [
        carrier_pos,
        Vec2::new(10.0, 10.0),
        Vec2::new(10.0, 40.0),
        screener_pos,
        Vec2::new(5.0, 25.0),
    ];

    let mut rng = rand::rngs::mock::StepRng::new(42, 1);
    let (_off, def) = TacticalPlanner::plan_possession_targets_with_rules(
        TacticalSet::HighPickAndRoll,
        SubPhase::ActionExecution,
        Possession::Home,
        carrier_pos,
        0,
        2.0,
        &mut rng,
        &rules,
        Some(&off_positions),
    );

    assert_eq!(def[0].action, "SWITCH_ASSIGNMENT");
    assert_eq!(def[3].action, "SWITCH_ASSIGNMENT");
}

#[test]
fn test_hedge_and_recover_structure_difference() {
    let mut rules = GameRules::default();
    rules.tactics.defense = DefenseRules::for_scheme("def_hedge_recover").expect("valid scheme");
    assert!(rules.tactics.defense.screen_defense.hedge_distance_ft > 0.0);

    let carrier_pos = Vec2::new(25.0, 25.0);
    let screener_pos = Vec2::new(26.0, 24.0);
    let off_positions = [
        carrier_pos,
        Vec2::new(10.0, 10.0),
        Vec2::new(10.0, 40.0),
        screener_pos,
        Vec2::new(5.0, 25.0),
    ];

    let mut rng = rand::rngs::mock::StepRng::new(42, 1);
    let (_off, def) = TacticalPlanner::plan_possession_targets_with_rules(
        TacticalSet::HighPickAndRoll,
        SubPhase::ActionExecution,
        Possession::Home,
        carrier_pos,
        0,
        2.0,
        &mut rng,
        &rules,
        Some(&off_positions),
    );

    let screener_def = &def[3];
    assert_eq!(screener_def.action, "HEDGE_AND_RECOVER");
    assert_eq!(screener_def.slot, "HedgeDefender");
}

#[test]
fn test_conservative_baseline_no_screen_mutation() {
    let mut rules = GameRules::default();
    rules.tactics.defense = DefenseRules::for_scheme("def_man_conservative").expect("valid scheme");

    let carrier_pos = Vec2::new(25.0, 25.0);
    let screener_pos = Vec2::new(26.0, 24.0);
    let off_positions = [
        carrier_pos,
        Vec2::new(10.0, 10.0),
        Vec2::new(10.0, 40.0),
        screener_pos,
        Vec2::new(5.0, 25.0),
    ];

    let mut rng = rand::rngs::mock::StepRng::new(42, 1);
    let (_off, def) = TacticalPlanner::plan_possession_targets_with_rules(
        TacticalSet::HighPickAndRoll,
        SubPhase::ActionExecution,
        Possession::Home,
        carrier_pos,
        0,
        2.0,
        &mut rng,
        &rules,
        Some(&off_positions),
    );

    assert_eq!(def[0].action, "ON_BALL_CONTEST");
    assert_eq!(def[3].action, "HELP_SIDE_SHELL");
}

#[test]
fn test_weak_side_help_and_x_out_rotation_chain() {
    use nba_decision::defense::{DefensiveCandidateAction, DefensiveContext};
    use nba_decision::tactics::DefensiveTactic;
    use nba_domain::data::{PlayerAttributes, PlayerTendencies};

    let rules = GameRules::default();
    let attrs = PlayerAttributes::default();
    let tendencies = PlayerTendencies::default();

    let ctx = DefensiveContext {
        defender_id: "def_low_man",
        defender_pos: Vec2::new(10.0, 10.0),
        defender_vel: Vec2::ZERO,
        defender_attrs: &attrs,
        defender_tendencies: &tendencies,
        assignment_offense_id: Some("corner_shooter"),
        assignment_pos: Vec2::new(5.0, 5.0),
        assignment_is_shooter: true,
        ball_pos: Vec2::new(20.0, 25.0),
        ball_flight_segment: None,
        ball_carrier_is_driving: true,
        hoop_pos: Vec2::new(5.25, 25.0),
        scheme: DefensiveTactic::DropCoverage,
        rules: &rules,
        base_ctx: None,
    };

    // 1. 弱侧 Low-man 测试：突破深入禁区，Low-man 必须生成下沉护筐 RotateRimHelp 动作
    let low_man_action = ctx.evaluate_weak_side_rotation(
        Vec2::new(12.0, 25.0),
        Vec2::new(5.0, 5.0),
        Vec2::new(18.0, 8.0),
        true,
    );
    assert!(matches!(
        low_man_action.action,
        DefensiveCandidateAction::RotateRimHelp { .. }
    ));

    // 2. 弱侧 High-man 测试：High-man 必须生成补位 XOutCloseout 动作
    let high_man_action = ctx.evaluate_weak_side_rotation(
        Vec2::new(12.0, 25.0),
        Vec2::new(5.0, 5.0),
        Vec2::new(18.0, 8.0),
        false,
    );
    assert!(matches!(
        high_man_action.action,
        DefensiveCandidateAction::XOutCloseout { .. }
    ));
}

#[test]
fn test_tactical_planner_weak_side_rotation_and_x_out_integration() {
    let rules = GameRules::default();
    let mut rng = rand::rngs::mock::StepRng::new(42, 1);

    // 进攻方布局（Home 攻右侧篮筐 hoop=(88.75, 25.0)）：
    // 0: 持球突破人（杀入禁区 80.0, 25.0，距篮筐 8.75 尺）
    // 1: 弱侧底角射手 (86.0, 5.0) -> Low-man 防守人应该下沉护筐
    // 2: 弱侧 45 度射手 (72.0, 8.0) -> High-man 防守人应该执行 X-Out
    // 3: 强侧掩护人 (82.0, 35.0)
    // 4: 强侧翼侧射手 (72.0, 42.0)
    let deep_carrier_pos = Vec2::new(80.0, 25.0);
    let off_positions_drive = [
        deep_carrier_pos,
        Vec2::new(86.0, 5.0),
        Vec2::new(72.0, 8.0),
        Vec2::new(82.0, 35.0),
        Vec2::new(72.0, 42.0),
    ];

    let (_off, def_drive) = TacticalPlanner::plan_possession_targets_with_rules(
        TacticalSet::HighPickAndRoll,
        SubPhase::ActionExecution,
        Possession::Home,
        deep_carrier_pos,
        0,
        2.0,
        &mut rng,
        &rules,
        Some(&off_positions_drive),
    );

    // 弱侧 Low-man (index 1) 必须分配到 ROTATE_RIM_HELP
    assert_eq!(def_drive[1].action, "ROTATE_RIM_HELP");
    assert_eq!(def_drive[1].slot, "LowManRimHelp");
    // 弱侧 High-man (index 2) 必须分配到 X_OUT_CLOSEOUT
    assert_eq!(def_drive[2].action, "X_OUT_CLOSEOUT");
    assert_eq!(def_drive[2].slot, "HighManXOut");

    // 反事实对照组：持球人退回外线（65.0, 25.0，距篮筐 23.75 尺），未深入禁区
    let perimeter_carrier_pos = Vec2::new(65.0, 25.0);
    let off_positions_perimeter = [
        perimeter_carrier_pos,
        Vec2::new(86.0, 5.0),
        Vec2::new(72.0, 8.0),
        Vec2::new(82.0, 35.0),
        Vec2::new(72.0, 42.0),
    ];

    let (_off, def_perimeter) = TacticalPlanner::plan_possession_targets_with_rules(
        TacticalSet::HighPickAndRoll,
        SubPhase::ActionExecution,
        Possession::Home,
        perimeter_carrier_pos,
        0,
        2.0,
        &mut rng,
        &rules,
        Some(&off_positions_perimeter),
    );

    // 外线持球时，弱侧防守人必须保持常规 HELP_SIDE_SHELL 站位，不可盲目下沉放空射手
    assert_eq!(def_perimeter[1].action, "HELP_SIDE_SHELL");
    assert_eq!(def_perimeter[2].action, "HELP_SIDE_SHELL");
}

#[test]
fn test_potential_field_continuity_and_threat_monotonicity() {
    use nba_decision::potential_field::{DefensePotentialFieldSolver, PotentialFieldConfig};

    let solver = DefensePotentialFieldSolver::new(PotentialFieldConfig::default());
    let hoop = Vec2::new(88.75, 25.0);
    let rules = GameRules::default();

    let off_positions = [
        Vec2::new(60.0, 25.0),
        Vec2::new(86.0, 5.0), // Low-man 对位人
        Vec2::new(72.0, 8.0), // High-man 对位人
        Vec2::new(82.0, 35.0),
        Vec2::new(72.0, 42.0),
    ];

    // 持球人从 30 尺外向禁区突破：距离篮筐逐步递减
    let distances = [30.0, 24.0, 18.0, 14.0, 10.0, 6.0];
    let mut prev_threat = 0.0_f32;
    let mut prev_low_man_dist_to_hoop = f32::MAX;

    for &dist in &distances {
        let carrier_pos = hoop - Vec2::new(dist, 0.0);
        let low_man = solver.solve_equilibrium(carrier_pos, hoop, &off_positions, 1, 0, &rules);

        // 威胁占比必须随着突破深入严格单调递增！
        assert!(
            low_man.threat_ratio >= prev_threat,
            "Threat ratio must increase monotonically with drive depth: dist={}, curr={}, prev={}",
            dist,
            low_man.threat_ratio,
            prev_threat
        );
        prev_threat = low_man.threat_ratio;

        // Low-man 的平衡点距离篮筐必须平滑下沉靠近！
        let curr_dist_to_hoop = (low_man.target_pos - hoop).length();
        assert!(
            curr_dist_to_hoop <= prev_low_man_dist_to_hoop + 0.001,
            "Low-man must sink smoothly toward rim: dist={}, curr_d={}, prev_d={}",
            dist,
            curr_dist_to_hoop,
            prev_low_man_dist_to_hoop
        );
        prev_low_man_dist_to_hoop = curr_dist_to_hoop;
    }
}
