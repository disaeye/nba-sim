//! PlaySpec 数据层测试（docs/tactics.md §2.2.4、docs/decisions.md ADR-019）。
//!
//! 覆盖：合法档案反序列化 + 校验通过；未知谓词/动词字符串反序列化失败
//! 且错误信息含非法值；kind 非 Play 被拒绝；hard 抑制覆盖全部族被拒绝；
//! 窗口秒数 <= 0 被拒绝；空 triggers 被拒绝；谓词 serde roundtrip。

use nba_domain::{
    DecisionActionFamily, PlayAction, PlayCarrierPreference, PlayInhibition, PlayInhibitionMode,
    PlayKind, PlayPredicate, PlayRule, PlaySpec, PlayTrigger, PlayVerb,
};

/// 合法基准档案（覆盖全部谓词类别与抑制模式）。
fn valid_spec_json() -> String {
    r#"{
        "schema_version": 1,
        "id": "pnr_drop_lob_v1",
        "name_zh": "高位挡拆吊传",
        "kind": "play",
        "carrier_preferences": [
            {"action_family": "Drive", "bonus": 0.30},
            {"action_family": "Pass", "bonus": 0.15}
        ],
        "inhibitions": [
            {"action_family": "Shoot", "mode": "soft", "penalty": 0.40},
            {"action_family": "PostUp", "mode": "hard"}
        ],
        "rules": [
            {
                "id": "screener_roll_pull",
                "when": [
                    {"predicate": "carrier_possed"},
                    {"predicate": "screen_established"},
                    {"predicate": "help_shading_off"}
                ],
                "then": {"verb": "ScreenRoll", "slot": "screener"},
                "carrier_preferences": [
                    {"action_family": "Pass", "bonus": 0.20}
                ]
            }
        ],
        "triggers": [
            {
                "when": [
                    {"predicate": "halfcourt_possession"},
                    {"predicate": "carrier_possed"},
                    {"predicate": "beyond_circle_ft", "r": 20.0}
                ],
                "window_seconds": 6.0,
                "cooldown_seconds": 10.0
            }
        ]
    }"#
    .to_string()
}

/// 无规则、无偏好、无抑制的最小合法档案。
fn minimal_spec() -> PlaySpec {
    PlaySpec {
        schema_version: 1,
        id: "iso_clear_right_v1".to_string(),
        name_zh: "右侧清空单打".to_string(),
        kind: PlayKind::Play,
        carrier_preferences: Vec::new(),
        inhibitions: Vec::new(),
        rules: Vec::new(),
        triggers: vec![PlayTrigger {
            when: vec![PlayPredicate::CarrierPossed],
            window_seconds: 4.0,
            cooldown_seconds: 0.0,
        }],
    }
}

/// 把基准档案解析成 JSON 值，便于逐字段做负例变形。
fn mutate_valid_json(field: &str, value: serde_json::Value) -> serde_json::Value {
    let mut root: serde_json::Value =
        serde_json::from_str(&valid_spec_json()).expect("基准档案必须是合法 JSON");
    root[field] = value;
    root
}

#[test]
fn documentation_schema_example_parses_through_from_json() {
    let json = r#"{
        "schema_version": 1,
        "id": "pnr_drop_lob_v1",
        "name_zh": "高位挡拆吊传",
        "kind": "play",
        "carrier_preferences": [
            {"action_family": "Drive", "bonus": 0.30},
            {"action_family": "Pass", "bonus": 0.15}
        ],
        "inhibitions": [
            {"action_family": "Shoot", "mode": "soft", "penalty": 0.40},
            {"action_family": "Dwell", "mode": "hard"}
        ],
        "rules": [
            {
                "id": "screener_roll_pull",
                "when": [
                    {"predicate": "carrier_possed"},
                    {"predicate": "screen_established"},
                    {"predicate": "help_shading_off"}
                ],
                "then": {"verb": "ScreenRoll", "slot": "screener"},
                "carrier_preferences": [
                    {"action_family": "Pass", "bonus": 0.20}
                ]
            }
        ],
        "triggers": [
            {
                "when": [
                    {"predicate": "halfcourt_possession"},
                    {"predicate": "carrier_possed"},
                    {"predicate": "beyond_circle_ft", "r": 20.0}
                ],
                "window_seconds": 6.0,
                "cooldown_seconds": 10.0
            }
        ]
    }"#;
    let spec = PlaySpec::from_json(json).expect("tactics.md 中的 schema 示例必须能解析并通过校验");
    assert_eq!(spec.id, "pnr_drop_lob_v1");
}

#[test]
fn valid_spec_deserializes_and_validates() {
    let spec = PlaySpec::from_json(&valid_spec_json()).expect("合法档案必须通过");
    assert_eq!(spec.id, "pnr_drop_lob_v1");
    assert_eq!(spec.kind, PlayKind::Play);
    assert_eq!(spec.carrier_preferences.len(), 2);
    assert_eq!(spec.inhibitions.len(), 2);
    assert_eq!(spec.rules.len(), 1);
    assert_eq!(spec.triggers.len(), 1);
    // 封闭枚举字段命中词汇表。
    assert_eq!(spec.rules[0].then.verb, PlayVerb::ScreenRoll);
    assert_eq!(spec.rules[0].then.slot, "screener");
    assert_eq!(
        spec.rules[0].carrier_preferences[0].action_family,
        DecisionActionFamily::Pass
    );
    assert!(spec.validate().is_ok());
}

#[test]
fn minimal_spec_without_optional_sections_is_valid() {
    let spec = minimal_spec();
    assert!(spec.validate().is_ok());
}

#[test]
fn unknown_predicate_string_fails_deserialization_with_value() {
    let json = r#"{
        "schema_version": 1,
        "id": "bad_predicate_v1",
        "name_zh": "未知谓词",
        "kind": "play",
        "triggers": [
            {"when": [{"predicate": "teleport_to_basket"}], "window_seconds": 5.0, "cooldown_seconds": 1.0}
        ]
    }"#;
    let err = PlaySpec::from_json(json).expect_err("未知谓词必须被拒绝");
    let message = err.to_string();
    assert!(
        message.contains("teleport_to_basket"),
        "错误信息必须含非法值，实际为：{message}"
    );
}

#[test]
fn unknown_verb_string_fails_deserialization_with_value() {
    let json = r#"{
        "schema_version": 1,
        "id": "bad_verb_v1",
        "name_zh": "未知动词",
        "kind": "play",
        "rules": [
            {
                "id": "r1",
                "when": [{"predicate": "carrier_possed"}],
                "then": {"verb": "Levitate", "slot": "screener"}
            }
        ],
        "triggers": [
            {"when": [{"predicate": "carrier_possed"}], "window_seconds": 5.0, "cooldown_seconds": 1.0}
        ]
    }"#;
    let err = PlaySpec::from_json(json).expect_err("未知动词必须被拒绝");
    let message = err.to_string();
    assert!(
        message.contains("Levitate"),
        "错误信息必须含非法值，实际为：{message}"
    );
}

#[test]
fn unknown_action_family_string_fails_deserialization_with_value() {
    let json = r#"{
        "schema_version": 1,
        "id": "bad_family_v1",
        "name_zh": "未知动作族",
        "kind": "play",
        "carrier_preferences": [
            {"action_family": "Teleport", "bonus": 0.5}
        ],
        "triggers": [
            {"when": [{"predicate": "carrier_possed"}], "window_seconds": 5.0, "cooldown_seconds": 1.0}
        ]
    }"#;
    let err = PlaySpec::from_json(json).expect_err("未知动作族必须被拒绝");
    let message = err.to_string();
    assert!(
        message.contains("Teleport"),
        "错误信息必须含非法值，实际为：{message}"
    );
}

#[test]
fn system_kind_is_rejected_at_serde_layer_construction() {
    // kind 两级分流：PlaySpec 携带 System 变体在 serde 构造层即被拒绝，
    // 反序列化错误信息含非法值。
    let json = mutate_valid_json("kind", serde_json::Value::String("System".into()));
    let err =
        PlaySpec::from_json(&json.to_string()).expect_err("PlaySpec 携带 System kind 必须被拒绝");
    let message = err.to_string();
    assert!(
        message.contains("System"),
        "错误信息必须含非法值，实际为：{message}"
    );

    // Rust 构造层同样拒绝：validate 对 kind != Play 报 InvalidKind。
    let mut spec = minimal_spec();
    spec.kind = PlayKind::System;
    match spec.validate() {
        Err(nba_domain::PlaySpecError::InvalidKind { found, .. }) => {
            assert_eq!(found, PlayKind::System);
        }
        other => panic!("期望 InvalidKind，实际为 {other:?}"),
    }
}

#[test]
fn hard_inhibition_covering_all_families_fails_validation() {
    let hard_all: Vec<serde_json::Value> = DecisionActionFamily::ALL
        .iter()
        .map(|family| serde_json::json!({"action_family": family.as_str(), "mode": "hard"}))
        .collect();
    let json = mutate_valid_json("inhibitions", serde_json::Value::Array(hard_all));
    let spec: PlaySpec = serde_json::from_value(json).expect("形状合法，应能反序列化");
    let err = spec.validate().expect_err("hard 抑制覆盖全部族必须失败");
    match err {
        nba_domain::PlaySpecError::HardInhibitionCoversAllFamilies { hard_families, .. } => {
            assert_eq!(hard_families.len(), DecisionActionFamily::ALL.len());
        }
        other => panic!("期望 HardInhibitionCoversAllFamilies，实际为 {other:?}"),
    }
}

#[test]
fn hard_inhibition_leaving_one_family_out_passes() {
    // 合法出路不变量：至少保留一个族不被 hard 抑制（基准档案即满足）。
    let spec: PlaySpec = serde_json::from_str(&valid_spec_json()).expect("基准档案应能反序列化");
    assert!(spec.validate().is_ok());

    // 再 hard 抑制若干族，只要未覆盖全集始终合法（基准档案 hard 抑制
    // PostUp，故还剩 Shoot/Drive/Pass/TripleThreatJab/Dwell 五个保留族）。
    let mut spec = spec;
    for family in [
        DecisionActionFamily::Shoot,
        DecisionActionFamily::Drive,
        DecisionActionFamily::Pass,
        DecisionActionFamily::Dwell,
    ] {
        spec.inhibitions.push(PlayInhibition {
            action_family: family,
            mode: PlayInhibitionMode::Hard,
        });
        assert!(spec.validate().is_ok(), "抑制 {family:?} 后仍有合法出路");
    }
    // 抑制到最后一个保留族才越界失败。
    spec.inhibitions.push(PlayInhibition {
        action_family: DecisionActionFamily::TripleThreatJab,
        mode: PlayInhibitionMode::Hard,
    });
    assert!(spec.validate().is_err());
}

#[test]
fn nonpositive_window_seconds_fails_validation() {
    for window in [0.0_f32, -1.0] {
        let spec = PlaySpec {
            triggers: vec![PlayTrigger {
                when: vec![PlayPredicate::CarrierPossed],
                window_seconds: window,
                cooldown_seconds: 1.0,
            }],
            ..minimal_spec()
        };
        let err = spec.validate().expect_err("窗口秒数 <= 0 必须失败");
        match err {
            nba_domain::PlaySpecError::InvalidTriggerTiming { field, value, .. } => {
                assert_eq!(field, "window_seconds");
                assert_eq!(value, window);
            }
            other => panic!("期望 InvalidTriggerTiming，实际为 {other:?}"),
        }
    }
}

#[test]
fn empty_triggers_fail_validation() {
    let spec = PlaySpec {
        triggers: Vec::new(),
        ..minimal_spec()
    };
    match spec.validate() {
        Err(nba_domain::PlaySpecError::NoTriggers { id }) => {
            assert_eq!(id, "iso_clear_right_v1");
        }
        other => panic!("期望 NoTriggers，实际为 {other:?}"),
    }
}

#[test]
fn empty_trigger_or_rule_when_fails_validation() {
    let trigger_json = mutate_valid_json(
        "triggers",
        serde_json::json!([
            {"when": [], "window_seconds": 5.0, "cooldown_seconds": 1.0}
        ]),
    );
    match PlaySpec::from_json(&trigger_json.to_string()).expect_err("空触发条件必须被拒绝")
    {
        nba_domain::PlaySpecError::EmptyTriggerWhen { trigger_index, .. } => {
            assert_eq!(trigger_index, 0);
        }
        other => panic!("期望 EmptyTriggerWhen，实际为 {other:?}"),
    }

    let rule_json = mutate_valid_json(
        "rules",
        serde_json::json!([
            {
                "id": "empty_when",
                "when": [],
                "then": {"verb": "Lift", "slot": "screener"}
            }
        ]),
    );
    match PlaySpec::from_json(&rule_json.to_string()).expect_err("空规则条件必须被拒绝") {
        nba_domain::PlaySpecError::EmptyRuleWhen { rule_id, .. } => {
            assert_eq!(rule_id, "empty_when");
        }
        other => panic!("期望 EmptyRuleWhen，实际为 {other:?}"),
    }
}

#[test]
fn rules_for_the_same_slot_are_rejected_even_when_conditions_differ() {
    let spec = PlaySpec {
        rules: vec![
            PlayRule {
                id: "roll".to_string(),
                when: vec![PlayPredicate::ScreenEstablished],
                then: PlayAction {
                    verb: PlayVerb::ScreenRoll,
                    slot: "screener".to_string(),
                },
                carrier_preferences: Vec::new(),
            },
            PlayRule {
                id: "pop".to_string(),
                when: vec![PlayPredicate::HalfcourtPossession],
                then: PlayAction {
                    verb: PlayVerb::ScreenPop,
                    slot: "screener".to_string(),
                },
                carrier_preferences: Vec::new(),
            },
        ],
        ..minimal_spec()
    };
    match spec.validate() {
        Err(nba_domain::PlaySpecError::DuplicateRuleSlot { slot, .. }) => {
            assert_eq!(slot, "screener");
        }
        other => panic!("期望 DuplicateRuleSlot，实际为 {other:?}"),
    }
}

#[test]
fn duplicate_family_inside_one_rule_preference_list_fails_validation() {
    let mut spec = minimal_spec();
    spec.rules = vec![PlayRule {
        id: "r1".to_string(),
        when: vec![PlayPredicate::CarrierPossed],
        then: PlayAction {
            verb: PlayVerb::Lift,
            slot: "left_wing".to_string(),
        },
        carrier_preferences: vec![
            PlayCarrierPreference {
                action_family: DecisionActionFamily::Drive,
                bonus: 0.2,
            },
            PlayCarrierPreference {
                action_family: DecisionActionFamily::Drive,
                bonus: 0.3,
            },
        ],
    }];
    match spec.validate() {
        Err(nba_domain::PlaySpecError::DuplicateFamily {
            section, family, ..
        }) => {
            assert_eq!(section, "rules[].carrier_preferences");
            assert_eq!(family, "Drive");
        }
        other => panic!("期望 DuplicateFamily，实际为 {other:?}"),
    }
}

#[test]
fn rules_for_distinct_slots_are_valid() {
    let spec = PlaySpec {
        rules: vec![
            PlayRule {
                id: "roll".to_string(),
                when: vec![PlayPredicate::ScreenEstablished],
                then: PlayAction {
                    verb: PlayVerb::ScreenRoll,
                    slot: "screener".to_string(),
                },
                carrier_preferences: Vec::new(),
            },
            PlayRule {
                id: "lift".to_string(),
                when: vec![PlayPredicate::HalfcourtPossession],
                then: PlayAction {
                    verb: PlayVerb::Lift,
                    slot: "left_wing".to_string(),
                },
                carrier_preferences: Vec::new(),
            },
        ],
        ..minimal_spec()
    };
    assert!(spec.validate().is_ok());
}

#[test]
fn predicate_serde_roundtrip() {
    let predicates = vec![
        PlayPredicate::CarrierPossed,
        PlayPredicate::HalfcourtPossession,
        PlayPredicate::ShotClockUrgent { threshold_s: 4.5 },
        PlayPredicate::BeyondCircleFt { r: 18.0 },
        PlayPredicate::ScreenEstablished,
        PlayPredicate::CornerOccupied {
            side: nba_domain::PlayCourtSide::Left,
        },
        PlayPredicate::HelpShadingOff,
        PlayPredicate::WeakSideVacated,
    ];
    for predicate in &predicates {
        let value = serde_json::to_value(predicate).expect("序列化");
        assert!(
            value.get("predicate").is_some(),
            "tag 字段必须存在：{value}"
        );
        let decoded: PlayPredicate = serde_json::from_value(value).expect("反序列化");
        assert_eq!(&decoded, predicate);
    }
}

#[test]
fn predicate_tags_are_snake_case_strings() {
    assert_eq!(
        serde_json::to_value(PlayPredicate::BeyondCircleFt { r: 20.0 })
            .expect("序列化")
            .get("predicate")
            .and_then(|v| v.as_str()),
        Some("beyond_circle_ft")
    );
}

#[test]
fn verb_and_family_serde_roundtrip() {
    for verb in [
        PlayVerb::ScreenRoll,
        PlayVerb::ScreenPop,
        PlayVerb::SpotUp,
        PlayVerb::Relocate,
        PlayVerb::CutBackdoor,
        PlayVerb::Lift,
    ] {
        let value = serde_json::to_value(verb).expect("序列化");
        let decoded: PlayVerb = serde_json::from_value(value).expect("反序列化");
        assert_eq!(decoded, verb);
    }
    for family in DecisionActionFamily::ALL {
        let value = serde_json::to_value(family).expect("序列化");
        let decoded: DecisionActionFamily = serde_json::from_value(value).expect("反序列化");
        assert_eq!(decoded, family);
    }
}

#[test]
fn negative_bonus_and_penalty_fail_validation() {
    let mut spec = minimal_spec();
    spec.carrier_preferences.push(PlayCarrierPreference {
        action_family: DecisionActionFamily::Drive,
        bonus: -0.1,
    });
    assert!(spec.validate().is_err());

    let mut spec = minimal_spec();
    spec.inhibitions.push(PlayInhibition {
        action_family: DecisionActionFamily::Shoot,
        mode: PlayInhibitionMode::Soft { penalty: -0.2 },
    });
    assert!(spec.validate().is_err());

    let mut spec = minimal_spec();
    spec.inhibitions.push(PlayInhibition {
        action_family: DecisionActionFamily::Shoot,
        mode: PlayInhibitionMode::Soft { penalty: f32::NAN },
    });
    assert!(spec.validate().is_err());
}

#[test]
fn conditional_rule_preference_can_repeat_top_level_family() {
    let spec = PlaySpec {
        rules: vec![PlayRule {
            id: "r1".to_string(),
            when: vec![PlayPredicate::CarrierPossed],
            then: PlayAction {
                verb: PlayVerb::Lift,
                slot: "lift_big".to_string(),
            },
            carrier_preferences: vec![PlayCarrierPreference {
                action_family: DecisionActionFamily::Drive,
                bonus: 0.2,
            }],
        }],
        carrier_preferences: vec![PlayCarrierPreference {
            action_family: DecisionActionFamily::Drive,
            bonus: 0.3,
        }],
        ..minimal_spec()
    };
    assert!(spec.validate().is_ok());
}

#[test]
fn empty_rule_slot_fails_validation() {
    let spec = PlaySpec {
        rules: vec![PlayRule {
            id: "r1".to_string(),
            when: vec![PlayPredicate::CarrierPossed],
            then: PlayAction {
                verb: PlayVerb::SpotUp,
                slot: "  ".to_string(),
            },
            carrier_preferences: Vec::new(),
        }],
        ..minimal_spec()
    };
    match spec.validate() {
        Err(nba_domain::PlaySpecError::EmptyRuleSlot { rule_id, .. }) => {
            assert_eq!(rule_id, "r1");
        }
        other => panic!("期望 EmptyRuleSlot，实际为 {other:?}"),
    }
}

#[test]
fn unknown_field_is_rejected() {
    // deny_unknown_fields 对未知字段整体拒绝。
    let spec: Result<PlaySpec, _> = serde_json::from_value(mutate_valid_json(
        "triggers",
        serde_json::json!([
            {"when": [{"predicate": "carrier_possed"}], "window_seconds": 5.0,
             "cooldown_seconds": 1.0, "mystery_field": 1}
        ]),
    ));
    assert!(spec.is_err(), "未知字段必须被拒绝");
}

#[test]
fn serde_error_message_reports_offending_value() {
    let json = r#"{
        "schema_version": 1,
        "id": "bad_kind_v1",
        "name_zh": "未知 kind",
        "kind": "Flex",
        "triggers": [
            {"when": [{"predicate": "carrier_possed"}], "window_seconds": 5.0, "cooldown_seconds": 1.0}
        ]
    }"#;
    let err = PlaySpec::from_json(json).expect_err("未知 kind 必须被拒绝");
    let message = err.to_string();
    assert!(
        message.contains("Flex"),
        "错误信息必须含非法值，实际为：{message}"
    );
}
