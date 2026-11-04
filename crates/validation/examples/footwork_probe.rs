//! 诊断收球步的落点分布：收球点到终结点的距离。

use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{MatchSetup, Params, World};

fn main() {
    let params = Params::load(&workspace_calibration_dir()).expect("params");
    let setup = MatchSetup::builtin(nba_domain::rules::GameRules::default());
    let games: u64 = std::env::args()
        .nth(1)
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(3);
    let mut distances: Vec<f32> = Vec::new();
    for seed in 1..=games {
        let mut world = World::new(setup.clone(), params.clone(), seed);
        let mut steps = 0usize;
        while !world.finished && steps < 400_000 {
            world.step(nba_sim_core::world::TICK_SECONDS);
            steps += 1;
            // 收球点与终结点：收球后的落点离收球点多远。
            if let Some(gather) = world.possession.gather_pos {
                if let Some(pivot) = world.possession.pivot {
                    let moved = (pivot.position - gather).length();
                    if moved > 0.05 {
                        distances.push(moved);
                    }
                }
            }        }
    }
    distances.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    if distances.is_empty() {
        println!("no samples");
        return;
    }
    let quantile = |q: f32| distances[(distances.len() as f32 * q) as usize % distances.len()];
    println!(
        "samples={} min={:.2} p50={:.2} p90={:.2} p99={:.2} max={:.2}",
        distances.len(),
        distances[0],
        quantile(0.5),
        quantile(0.9),
        quantile(0.99),
        distances[distances.len() - 1]
    );
}
