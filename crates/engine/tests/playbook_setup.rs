use nba_domain::GameRules;
use nba_engine::MatchSetup;

#[test]
fn legacy_setup_json_defaults_missing_playbooks() {
    let setup = MatchSetup::builtin(GameRules::default());
    let mut value = serde_json::to_value(setup).expect("serialize setup");
    let object = value.as_object_mut().expect("setup object");
    object.remove("home_playbook");
    object.remove("away_playbook");

    let decoded: MatchSetup = serde_json::from_value(value).expect("deserialize legacy setup");
    assert!(decoded.home_playbook.is_empty());
    assert!(decoded.away_playbook.is_empty());
    decoded.validate().expect("legacy setup remains valid");
}

#[test]
fn builtin_playbooks_only_include_plays_with_existing_tactic_slots() {
    let setup = MatchSetup::builtin(GameRules::default());
    let home_play_ids: Vec<&str> = setup
        .home_playbook
        .iter()
        .map(|play| play.id.as_str())
        .collect();
    let away_play_ids: Vec<&str> = setup
        .away_playbook
        .iter()
        .map(|play| play.id.as_str())
        .collect();

    assert_eq!(home_play_ids, ["high_pnr_roll_v1"]);
    assert_eq!(away_play_ids, ["weak_side_lift_v1", "corner_backdoor_v1"]);
    assert!(setup
        .home_playbook
        .iter()
        .all(|play| { play.rules.iter().all(|rule| rule.then.slot == "screener") }));
    assert!(setup.away_playbook.iter().all(|play| {
        play.rules
            .iter()
            .all(|rule| rule.then.slot == "left_wing" || rule.then.slot == "left_corner")
    }));
    setup.validate().expect("built-in playbooks are valid");
}

#[test]
fn validate_rejects_duplicate_play_ids() {
    let mut setup = MatchSetup::builtin(GameRules::default());
    setup.away_playbook.push(setup.away_playbook[0].clone());

    let error = setup.validate().expect_err("duplicate play id must fail");
    assert!(error.contains("away"));
    assert!(error.contains("weak_side_lift_v1"));
    assert!(error.contains("duplicate play id"));
}

#[test]
fn validate_rejects_play_rules_with_unknown_slots() {
    let mut setup = MatchSetup::builtin(GameRules::default());
    let play = &mut setup.away_playbook[0];
    play.rules[0].then.slot = "unknown_slot".to_string();

    let error = setup.validate().expect_err("unknown slot must fail");
    assert!(error.contains("away"));
    assert!(error.contains("weak_side_lift_v1"));
    assert!(error.contains("lift_into_weak_side_void"));
    assert!(error.contains("unknown_slot"));
}
