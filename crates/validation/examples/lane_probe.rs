//! 测量限制区计时：进攻三秒与防守三秒的违例次数。

use nba_sim_core::action::Fact;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{MatchSetup, Params, World};

fn main() {
    let params = Params::load(&workspace_calibration_dir()).expect("params");
    let setup = MatchSetup::builtin(nba_domain::rules::GameRules::default());
    let games: u64 = std::env::args()
        .nth(1)
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(8);
    let mut offense = 0u64;
    let mut defense = 0u64;
    let mut possessions = 0u64;
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
                    kind: nba_sim_core::action::TurnoverKind::LaneViolation,
                    ..
                } => offense += 1,
                Fact::Foul {
                    kind: nba_sim_core::action::FoulKind::Technical,
                    ..
                } => defense += 1,
                _ => {}
            }
        }
        possessions += world.possession_log.len() as u64;
    }
    println!(
        "games={games} possessions={possessions} offensive_three_second={offense} technical_fouls={defense}"
    );
    println!(
        "offensive three second per game = {:.1}",
        offense as f64 / games as f64
    );
    println!(
        "technical fouls per game = {:.1}",
        defense as f64 / games as f64
    );
}
