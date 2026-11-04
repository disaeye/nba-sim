//! 机制实验的工具级回归：单项属性在固定步数内跑完，样本非空。

use nba_validation::mechanism_core::{
    run_mechanism_experiment, MechanismAttribute, MechanismConfig,
};

#[test]
fn a_defensive_rebound_experiment_finishes_with_samples() {
    // 限定回合数的快速档：登记表里 `more_games` 的属性按登记的场次跑
    // （640 个种子），那是一次小时级的完整实验，不属于这里的工具级回归。
    // 快速档只看前若干个回合，因此不套用登记的场次。
    let config = MechanismConfig {
        seeds: 8,
        possessions: Some(30),
        ..MechanismConfig::default()
    };
    let result = run_mechanism_experiment(MechanismAttribute::DefensiveRebound, config)
        .expect("defensive-rebound experiment must terminate");
    assert!(result.seeds >= 6, "样本不足：{}", result.seeds);
    assert!(
        result.per_seed_change.len() >= 6,
        "样本不足：{}",
        result.per_seed_change.len()
    );
}
