// 吞吐基准：固定种子跑固定 tick 数，输出每秒 tick 吞吐。
// 与旧引擎的 throughput_bench 同形：性能优化需要前后对比证据。
use std::time::Instant;

use nba_domain::rules::GameRules;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};

fn bench(ticks: usize, seed: u64) -> f32 {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, seed);
    let start = Instant::now();
    for _ in 0..ticks {
        world.step(TICK_SECONDS);
    }
    let elapsed = start.elapsed().as_secs_f32();
    let throughput = ticks as f32 / elapsed;
    println!(
        "seed{seed}: ticks={ticks} wall={elapsed:.2}s throughput={throughput:.0} ticks/s"
    );
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

/// 十万 tick 的预算口径：验收要求这一批量在 60 秒内跑完。
#[test]
fn bench_hundred_thousand_ticks() {
    let throughput = bench(100_000, 42);
    assert!(
        throughput > 1666.0,
        "十万 tick 需要 60 秒内跑完，实测吞吐 {throughput:.0} ticks/s"
    );
}
