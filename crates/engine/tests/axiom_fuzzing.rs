//! 篮球底层形式化公理长时 Fuzzing 自动化属性测试。
//!
//! 跨多种子运行长时比赛模拟，覆盖攻防转换、阵地进攻、抢断、投篮、篮板、罚球与死球发球，
//! 严格断言每一 tick 的输出帧必须 100% 满足 7 组正交公理，0 容忍任何常识性违例。

use nba_engine::MatchEngine;

#[test]
fn axiom_fuzzing_multi_seed_long_run() {
    let seeds = [42, 7, 100, 999, 1201, 31337, 2024, 8888];
    for seed in seeds {
        let mut engine = MatchEngine::new(seed);
        for tick_idx in 0..2500 {
            let _tick = engine.step();
            assert!(
                engine.last_tick_violations.is_empty(),
                "Seed {} at tick {} violated axioms: {:?}",
                seed,
                tick_idx,
                engine.last_tick_violations
            );
            if engine.is_finished() {
                break;
            }
        }
    }
}
