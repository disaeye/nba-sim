//! 机制实验的工具级回归：单项属性在固定步数内跑完且方向正确。

use nba_validation::mechanism_core::{
    run_mechanism_experiment, MechanismAttribute, MechanismConfig,
};

#[test]
fn a_defensive_rebound_experiment_moves_the_secured_share() {
    let config = MechanismConfig {
        seeds: 8,
        ..MechanismConfig::default()
    };
    let result = run_mechanism_experiment(MechanismAttribute::DefensiveRebound, config)
        .expect("defensive-rebound experiment must terminate");
    assert!(
        result.relative_change > 0.0,
        "卡位更强的球队必须控下更多对手投失的球，实际 {}",
        result.relative_change
    );
}
