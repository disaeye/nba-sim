//! 单项机制实验的独立运行：按登记表给出的样本量跑一个属性。
//!
//! 用途：机制实验的样本量由 `known_insufficient.json` 登记（某项属性需要
//! 多少个种子才判得出来），这里按那个样本量单独跑一项，把结果打出来。

use nba_validation::mechanism_core::{
    run_mechanism_experiment, MechanismAttribute, MechanismConfig,
};

fn main() {
    let attribute = match std::env::args().nth(1).as_deref() {
        Some("defensive_rebound") => MechanismAttribute::DefensiveRebound,
        Some("offensive_rebound") => MechanismAttribute::OffensiveRebound,
        Some(other) => panic!("unknown attribute {other}"),
        None => panic!("usage: mechanism_probe <attribute> [seeds]"),
    };
    let seeds = std::env::args()
        .nth(2)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(640);
    let config = MechanismConfig {
        seeds,
        ..MechanismConfig::default()
    };
    let result = run_mechanism_experiment(attribute, config)
        .expect("mechanism experiment must terminate");
    println!(
        "{} verdict={} rel={:.4} ci=[{:.4},{:.4}] seeds={}/{} direction={}",
        result.attribute,
        result.verdict,
        result.relative_change,
        result.ci_low,
        result.ci_high,
        result.seeds_in_expected_direction,
        result.seeds,
        result.seeds_in_expected_direction as f64 / result.seeds as f64,
    );
}
