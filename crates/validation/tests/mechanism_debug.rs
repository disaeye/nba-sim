//! 机制实验的工具级回归：单项属性在固定步数内跑完，样本非空。

use nba_validation::mechanism_core::{
    run_mechanism_experiment, MechanismAttribute, MechanismConfig,
};

#[test]
fn a_defensive_rebound_experiment_finishes_with_samples() {
    let config = MechanismConfig {
        seeds: 8,
        ..MechanismConfig::default()
    };
    let result = run_mechanism_experiment(MechanismAttribute::DefensiveRebound, config)
        .expect("defensive-rebound experiment must terminate");
    assert!(result.seeds >= 6, "样本不足：{}", result.seeds);
}
