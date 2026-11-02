//! 机制实验的工具级复现：单项属性能在固定步数内跑完且内存有界。

use nba_validation::mechanism_core::{
    run_mechanism_experiment, MechanismAttribute, MechanismConfig,
};

#[test]
fn the_free_throw_experiment_terminates_at_full_sample() {
    let config = MechanismConfig::default();
    let result = run_mechanism_experiment(MechanismAttribute::FreeThrow, config)
        .expect("free-throw experiment must terminate");
    assert!(result.seeds >= 30, "样本不足：{}", result.seeds);
}
