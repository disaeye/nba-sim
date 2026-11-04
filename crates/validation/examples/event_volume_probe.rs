//! 诊断完整比赛里每场的事件量：哪种事实占得最多。

use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};
use nba_domain::rules::GameRules;

fn main() {
    let params = Params::load(&workspace_calibration_dir()).expect("params");
    let setup = MatchSetup::builtin(GameRules::default());
    let games: u64 = std::env::args()
        .nth(1)
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(2);
    let mut counts: std::collections::BTreeMap<String, u64> = Default::default();
    let mut total = 0u64;
    for seed in 1..=games {
        let mut world = World::new(setup.clone(), params.clone(), seed);
        let mut steps = 0usize;
        while !world.finished && steps < 400_000 {
            world.step(TICK_SECONDS);
            steps += 1;
        }
        for event in &world.events {
            *counts.entry(event.fact.name().to_string()).or_default() += 1;
            total += 1;
        }
    }
    let mut ranked: Vec<(&String, &u64)> = counts.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1));
    println!(
        "total {total} over {games} games = {:.0} per game",
        total as f64 / games as f64
    );
    for (name, count) in ranked.iter().take(20) {
        println!(
            "  {name:32} {count:8} ({:.0}/game)",
            **count as f64 / games as f64
        );
    }
}
