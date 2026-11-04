//! 复现选板硬抑制清空候选集：按机制实验的方式跑整场比赛，
//! 逐个种子记录崩溃点。

use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{MatchSetup, Params, World};

fn main() {
    let params = Params::load(&workspace_calibration_dir()).expect("params");
    let value: f32 = std::env::args()
        .nth(1)
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(0.95);
    let seed_start: u64 = std::env::args()
        .nth(2)
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(1);
    let seed_end: u64 = std::env::args()
        .nth(3)
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(640);
    for seed in seed_start..=seed_end {
        let mut setup = MatchSetup::builtin(nba_domain::rules::GameRules::default());
        for player in setup.home.players.iter_mut() {
            player.attributes.defensive_rebound = value;
        }
        let mut world = World::new(setup, params.clone(), seed);
        let mut steps = 0usize;
        let mut crashed = false;
        while !world.finished && steps < 400_000 {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                world.step(nba_sim_core::world::TICK_SECONDS);
            }));
            if outcome.is_err() {
                crashed = true;
                println!(
                    "seed={seed} value={value} step={steps} period={} clock={:.2} phase={:?} \
                     holder={:?} play={:?} shot_clock={:.2}",
                    world.clock.period(),
                    world.clock.game_clock(),
                    world.phase(),
                    world.holder(),
                    world.possession.play.map(|window| window.play),
                    world.clock.shot_clock(),
                );
                break;
            }
            steps += 1;
        }
        if crashed {
            return;
        }
        eprintln!("seed {seed} ok ({steps} steps)");
    }
    println!("no crash");
}
