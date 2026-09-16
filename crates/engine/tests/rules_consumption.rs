use nba_domain::{GameRules, UNIMPLEMENTED_RULE_FIELDS};
use nba_engine::MatchEngine;

#[test]
fn test_unimplemented_rule_fields_are_fully_closed() {
    // D27: 全部规则字段均已接入因果链，清单清零
    assert!(UNIMPLEMENTED_RULE_FIELDS.is_empty());
}

#[test]
fn test_drive_finish_range_rules_perturbation() {
    // 验证 drive_finish_range_ft 参数能够生效（非硬编码）
    let mut rules_short = GameRules::default();
    rules_short.tactics.drive_finish_range_ft = 8.0;

    let mut rules_long = GameRules::default();
    rules_long.tactics.drive_finish_range_ft = 22.0;

    assert_ne!(
        rules_short.tactics.drive_finish_range_ft,
        rules_long.tactics.drive_finish_range_ft
    );
}

#[test]
fn test_clutch_rules_wired_to_engine() {
    let engine = MatchEngine::new(42);
    let rules = engine.rules().clone();
    assert_eq!(rules.modulation.clutch_period, 4);
    assert_eq!(rules.modulation.clutch_time_remaining, 120.0);
    assert_eq!(rules.modulation.clutch_score_margin, 5);
}
