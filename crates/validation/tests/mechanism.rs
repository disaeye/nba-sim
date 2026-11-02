use nba_validation::mechanism_core::{
    run_mechanism_experiment, validate_known_entries, KnownInsufficient, MechanismAttribute,
    MechanismConfig, Metric,
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

/// 力量的观测量必须对应属性规格中的「掩护推开距离」。
#[test]
fn strength_mechanism_uses_screen_displacement_observable() {
    assert_eq!(
        MechanismAttribute::Strength.metric(),
        Metric::ScreenDisplacement
    );
}

/// 决策智商的观测量使用每次传球的失误比，避免回合长度改变分母。
#[test]
fn decision_iq_mechanism_uses_turnovers_per_pass() {
    assert_eq!(
        MechanismAttribute::DecisionIq.metric(),
        Metric::TurnoversPerPass
    );
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
    // 各属性的实验互不依赖，四个线程分摊。
    let attributes: Vec<MechanismAttribute> = MechanismAttribute::all().to_vec();
    let chunk_size = attributes.len().div_ceil(4);
    let shared = (&attributes, &config);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .filter_map(|chunk| {
                let start = chunk * chunk_size;
                let end = (start + chunk_size).min(shared.0.len());
                (start < end).then(|| &shared.0[start..end])
            })
            .map(|chunk| {
                scope.spawn(move || {
                    for attribute in chunk {
                        check_mechanism_experiment(*attribute, shared.1);
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("mechanism thread must not panic");
        }
    });
}

/// 单属性的实验契约检查：裁决已知、数值有限、失败原因明确。
fn check_mechanism_experiment(attribute: MechanismAttribute, config: &MechanismConfig) {
    match run_mechanism_experiment(attribute, config.clone()) {
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
