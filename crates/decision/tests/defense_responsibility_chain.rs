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
