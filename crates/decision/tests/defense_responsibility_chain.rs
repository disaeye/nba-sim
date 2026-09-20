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
fn test_potential_field_continuity_and_threat_monotonicity() {
    use nba_decision::potential_field::DefensePotentialFieldSolver;

    let rules = GameRules::default();
    let solver = DefensePotentialFieldSolver::new(rules.tactics.defense.potential_field);
    let hoop = Vec2::new(88.75, 25.0);

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
