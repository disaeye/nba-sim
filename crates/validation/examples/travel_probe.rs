//! 测量走步与收球的分布：每场走步数、收球位移的分布。

use nba_sim_core::action::{Fact, TurnoverKind};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{MatchSetup, Params, World};

fn main() {
    let params = Params::load(&workspace_calibration_dir()).expect("params");
    let setup = MatchSetup::builtin(nba_domain::rules::GameRules::default());
    let games: u64 = std::env::args()
        .nth(1)
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(10);
    let mut travels = 0u64;
    let mut possessions = 0u64;
    let mut double_dribbles = 0u64;
    let mut travels_by_team = [0u64; 2];
    for seed in 1..=games {
        let mut world = World::new(setup.clone(), params.clone(), seed);
        let mut steps = 0usize;
        while !world.finished && steps < 400_000 {
            world.step(nba_sim_core::world::TICK_SECONDS);
            steps += 1;
        }
        for event in &world.events {
            match &event.fact {
                Fact::Turnover {
                    kind: TurnoverKind::Travel,
                    player,
                } => {
                    travels += 1;
                    let index = usize::from(world.players[*player].team == nba_sim_core::TeamSide::Away);
                    travels_by_team[index] += 1;
                }
                Fact::Turnover {
                    kind: TurnoverKind::DoubleDribble,
                    ..
                } => {
                    double_dribbles += 1;
                }
                _ => {}
            }
        }
        possessions += world.possession_log.len() as u64;
    }
    println!(
        "games={games} possessions={possessions} travels={travels} double_dribbles={double_dribbles}"
    );
    println!("travels per possession = {:.4}", travels as f64 / possessions.max(1) as f64);
    println!(
        "travels home={} away={}",
        travels_by_team[0], travels_by_team[1]
    );
}
