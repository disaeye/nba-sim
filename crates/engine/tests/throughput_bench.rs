// 吞吐基准（quality.md §4.1 性能预算的 fixture 底座）：
// 固定种子跑固定 tick 数，输出每秒 tick 吞吐。不做断言——带的比对由
// scripts/check_perf_fixture.py 守卫完成（带在 scripts/perf_fixture.json）。
// 碰撞/行为类改动改变每 tick 成本属正常演化：守卫提示用 --write-budget
// 重冻结，须附前后对比证据。
use nba_engine::MatchEngine;
use std::time::Instant;

fn bench(ticks: usize, seed: u64) -> f32 {
    let mut engine = MatchEngine::new(seed);
    engine.set_scope("full").expect("full scope is valid");
    let start = Instant::now();
    for _ in 0..ticks {
        engine.step();
    }
    let elapsed = start.elapsed().as_secs_f32();
    let throughput = ticks as f32 / elapsed;
    println!("seed{seed}: ticks={ticks} wall={elapsed:.2}s throughput={throughput:.0} ticks/s");
    throughput
}

#[test]
fn bench_seed42() {
    // 预热（页缓存/分支预测）后测量。
    bench(500, 7);
    bench(20_000, 42);
}

#[test]
fn bench_seed1() {
    bench(20_000, 1);
}
