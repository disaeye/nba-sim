//! 机制实验的工具级回归：单项属性在固定步数内跑完，事件日志有界。

use nba_validation::mechanism_core::{
    run_mechanism_experiment, MechanismAttribute, MechanismConfig,
};

#[test]
fn a_stamina_experiment_moves_the_minutes_metric() {
    let config = MechanismConfig {
        seeds: 8,
        ..MechanismConfig::default()
    };
    let result = run_mechanism_experiment(MechanismAttribute::Stamina, config)
        .expect("stamina experiment must terminate");
    assert!(
        result.relative_change > 0.0,
        "耐力更高的球队必须让主力打得更久，实际 {}",
        result.relative_change
    );
}
