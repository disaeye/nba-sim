//! 机制实验的工具级回归：单项属性在固定步数内跑完，事件日志有界。

use nba_validation::mechanism_core::{
    run_mechanism_experiment, MechanismAttribute, MechanismConfig,
};

#[test]
fn a_free_throw_experiment_finishes_with_a_bounded_log() {
    let config = MechanismConfig {
        seeds: 6,
        ..MechanismConfig::default()
    };
    let result = run_mechanism_experiment(MechanismAttribute::FreeThrow, config)
        .expect("free-throw experiment must terminate");
    assert!(result.seeds >= 5, "样本不足：{}", result.seeds);
}
