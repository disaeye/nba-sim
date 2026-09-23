use nba_domain::{
    DecisionActionFamily, PlayCourtSide, PlayKind, PlayPredicate, PlaySpec, PlayVerb,
};

const WEAK_SIDE_LIFT: &str = include_str!("../../../data/tactics/plays/weak_side_lift_v1.json");
const HIGH_PNR_ROLL: &str = include_str!("../../../data/tactics/plays/high_pnr_roll_v1.json");
const CORNER_BACKDOOR: &str = include_str!("../../../data/tactics/plays/corner_backdoor_v1.json");

#[test]
fn playbook_data_loads_and_matches_expected_semantics() {
    let weak_side_lift = PlaySpec::from_json(WEAK_SIDE_LIFT).expect("弱侧上提样例必须通过校验");
    assert_eq!(weak_side_lift.id, "weak_side_lift_v1");
    assert_eq!(weak_side_lift.kind, PlayKind::Play);
    assert_eq!(weak_side_lift.triggers.len(), 1);
    assert_eq!(weak_side_lift.rules.len(), 1);
    assert!(weak_side_lift.triggers[0]
        .when
        .contains(&PlayPredicate::WeakSideVacated));
    assert!(weak_side_lift.triggers[0]
        .when
        .contains(&PlayPredicate::HalfcourtPossession));
    assert_eq!(weak_side_lift.rules[0].then.verb, PlayVerb::Lift);
    assert_eq!(weak_side_lift.rules[0].then.slot, "left_wing");

    let high_pnr_roll = PlaySpec::from_json(HIGH_PNR_ROLL).expect("高位挡拆样例必须通过校验");
    assert_eq!(high_pnr_roll.id, "high_pnr_roll_v1");
    assert_eq!(high_pnr_roll.kind, PlayKind::Play);
    assert_eq!(high_pnr_roll.triggers.len(), 1);
    assert_eq!(high_pnr_roll.rules.len(), 1);
    assert!(high_pnr_roll.triggers[0]
        .when
        .contains(&PlayPredicate::ScreenEstablished));
    assert!(high_pnr_roll.triggers[0]
        .when
        .contains(&PlayPredicate::HelpShadingOff));
    assert_eq!(high_pnr_roll.rules[0].then.verb, PlayVerb::ScreenRoll);
    assert_eq!(high_pnr_roll.rules[0].then.slot, "screener");
    assert_eq!(
        high_pnr_roll.rules[0].carrier_preferences[0].action_family,
        DecisionActionFamily::Pass
    );

    let corner_backdoor = PlaySpec::from_json(CORNER_BACKDOOR).expect("底角背切样例必须通过校验");
    assert_eq!(corner_backdoor.id, "corner_backdoor_v1");
    assert_eq!(corner_backdoor.kind, PlayKind::Play);
    assert_eq!(corner_backdoor.triggers.len(), 1);
    assert_eq!(corner_backdoor.rules.len(), 1);
    assert!(corner_backdoor.triggers[0]
        .when
        .contains(&PlayPredicate::CornerOccupied {
            side: PlayCourtSide::Left,
        }));
    assert!(corner_backdoor.triggers[0]
        .when
        .contains(&PlayPredicate::HalfcourtPossession));
    assert_eq!(corner_backdoor.rules[0].then.verb, PlayVerb::CutBackdoor);
    assert_eq!(corner_backdoor.rules[0].then.slot, "left_corner");
}

#[test]
fn playbook_data_rejects_unknown_predicate() {
    let invalid_json = WEAK_SIDE_LIFT.replace(
        "\"predicate\": \"weak_side_vacated\"",
        "\"predicate\": \"unknown_playbook_predicate\"",
    );
    assert_ne!(invalid_json, WEAK_SIDE_LIFT);
    let error = PlaySpec::from_json(&invalid_json).expect_err("未知谓词必须被拒绝");
    assert!(error.to_string().contains("unknown_playbook_predicate"));
}
