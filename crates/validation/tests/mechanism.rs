use nba_validation::mechanism_core::{
    run_mechanism_experiment, validate_known_entries, KnownInsufficient, MechanismAttribute,
    MechanismConfig,
};

/// 登记表只接受两种 resolution，`more_games` 必须写明所需场次。
#[test]
fn known_insufficient_entries_require_a_valid_resolution() {
    let more_games = KnownInsufficient {
        attribute: "shooting_mid".to_string(),
        reason: "区间半宽按 1/sqrt(n) 收缩，越过零需要 30 个种子".to_string(),
        resolution: "more_games".to_string(),
        required_games: Some(30),
    };
    assert!(validate_known_entries(std::slice::from_ref(&more_games)).is_ok());

    let missing = KnownInsufficient {
        required_games: None,
        ..more_games.clone()
    };
    let error = validate_known_entries(&[missing]).expect_err("missing required_games must fail");
    assert!(
        error.contains("required_games"),
        "unexpected error: {error}"
    );

    let unknown = KnownInsufficient {
        resolution: "lower_bar".to_string(),
        ..more_games.clone()
    };
    let error = validate_known_entries(&[unknown]).expect_err("unknown resolution must fail");
    assert!(
        error.contains("unknown resolution"),
        "unexpected error: {error}"
    );

    let structural = KnownInsufficient {
        resolution: "mechanism_change".to_string(),
        required_games: None,
        ..more_games
    };
    assert!(validate_known_entries(&[structural]).is_ok());
}

/// 机制实验的契约：每个属性都产出已知裁决、数值有限、样本缺失时给出明确原因。
/// 方向正确性由 `validate-core --mechanism` 的整场批次负责，因为 60 回合的
/// 样本不足以压过出手选择对命中率的补偿效应。
#[test]
fn every_attribute_reports_a_known_verdict_or_an_explicit_gap() {
    let config = MechanismConfig {
        seeds: 1,
        seed_start: 1,
        possessions: Some(20),
        low_value: 0.10,
        high_value: 0.95,
    };
    for attribute in MechanismAttribute::all() {
        match run_mechanism_experiment(attribute, config) {
            Ok(result) => {
                assert!(
                    ["pass", "insufficient", "defect", "not_applicable"]
                        .contains(&result.verdict.as_str()),
                    "attribute {} returned unknown verdict {}",
                    attribute.key(),
                    result.verdict
                );
                assert!(
                    result.baseline.is_finite() && result.perturbed.is_finite(),
                    "attribute {} produced a non-finite metric",
                    attribute.key()
                );
                assert_eq!(result.seeds, 1);
            }
            Err(error) => {
                assert!(
                    error.contains("produced no"),
                    "attribute {} failed for an unexpected reason: {error}",
                    attribute.key()
                );
            }
        }
    }
}
