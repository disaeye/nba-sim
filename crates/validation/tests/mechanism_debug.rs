//! 机制实验的工具级回归：单项属性在固定步数内跑完，事件日志有界。

use nba_validation::mechanism_core::{
    run_mechanism_experiment, MechanismAttribute, MechanismConfig,
};

#[test]
fn perturbed_attributes_always_keep_a_decision() {
    let config = MechanismConfig {
        seeds: 8,
        ..MechanismConfig::default()
    };
    for attribute in [
        MechanismAttribute::FreeThrow,
        MechanismAttribute::Acceleration,
        MechanismAttribute::DecisionIq,
    ] {
        let result = run_mechanism_experiment(attribute, config)
            .unwrap_or_else(|error| panic!("{attribute:?} experiment failed: {error}"));
        assert!(result.seeds >= 6, "{attribute:?} 样本不足：{}", result.seeds);
    }
}
