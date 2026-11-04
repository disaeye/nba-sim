//! 诊断封盖与站位动作的相互影响。

use nba_sim_core::action::Fact;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};
use nba_domain::rules::GameRules;

fn main() {
    let params = Params::load(&workspace_calibration_dir()).expect("params");
    let setup = MatchSetup::builtin(GameRules::default().shortened(120.0, 60.0));
    let mut counts: std::collections::BTreeMap<String, u32> = Default::default();
    for seed in 1..=8u64 {
        let mut world = World::new(setup.clone(), params.clone(), seed);
        let mut steps = 0usize;
        while !world.finished && steps < 400_000 {
            world.step(TICK_SECONDS);
            steps += 1;
        }
        for event in &world.events {
            match &event.fact {
                Fact::DefensiveActionStarted { kind, .. } => {
                    *counts.entry((*kind).to_string()).or_default() += 1;
                }
                _ => {}
            }
        }
    }
    println!("{counts:?}");
}
