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
