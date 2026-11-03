//! 机制实验的工具级回归：单项属性能在固定步数内跑完且方向正确。

use nba_validation::mechanism_core::{
    run_mechanism_experiment, MechanismAttribute, MechanismConfig,
};

#[test]
fn a_decision_iq_experiment_moves_the_turnover_rate() {
    let config = MechanismConfig {
        seeds: 8,
        ..MechanismConfig::default()
    };
    let result = run_mechanism_experiment(MechanismAttribute::DecisionIq, config)
        .expect("decision-iq experiment must terminate");
    assert!(
        result.relative_change < 0.0,
        "判断更好的传球人必须更少丢球，实际 {}",
        result.relative_change
    );
}
