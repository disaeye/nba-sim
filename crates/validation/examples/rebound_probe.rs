//! 篮板归属的机制探针：把防守篮板属性的作用点摊开来看。
//!
//! 目的：找出 `defensive_rebound` 属性在篮板结算链条上的每个作用点，
//! 确认它到底有没有真的改变归属，以及效应量被哪一步稀释。

use nba_sim_core::action::Fact;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};

fn arm(value: f32, seeds: u64) -> (f64, f64, f64, f64, f64) {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut secured = 0.0f64;
    let mut chances = 0.0f64;
    let mut contested_total = 0.0f64;
    let mut boxout_facts = 0.0f64;
    let mut opportunity_total = 0.0f64;
    for offset in 0..seeds {
        let mut setup = MatchSetup::builtin(nba_domain::GameRules::default());
        for player in setup.home.players.iter_mut() {
            player.attributes.defensive_rebound = value;
        }
        let mut world = World::new(setup, params.clone(), 1 + offset);
        let home_lines = 8usize;
        let mut awaiting: Option<bool> = None;
        let mut scanned = 0usize;
        loop {
            for _ in 0..2000 {
                world.step(TICK_SECONDS);
                if world.finished {
                    break;
                }
            }
            while scanned < world.events.len() {
                let event = &world.events[scanned];
                scanned += 1;
                match &event.fact {
                    Fact::Shot { shooter, made, .. } => {
                        let shooter_is_away = *shooter >= home_lines;
                        if shooter_is_away && !made {
                            chances += 1.0;
                            awaiting = Some(true);
                        } else {
                            awaiting = None;
                        }
                    }
                    Fact::ReboundOpportunity { .. } => {
                        opportunity_total += 1.0;
                    }
                    Fact::BoxOut { player, .. } => {
                        if *player < home_lines {
                            boxout_facts += 1.0;
                        }
                    }
                    Fact::Rebound {
                        player, offensive, ..
                    } => {
                        if let Some(true) = awaiting {
                            contested_total += 1.0;
                            if *player < home_lines {
                                secured += 1.0;
                            }
                        }
                        awaiting = None;
                    }
                    Fact::LooseBallRecovered { player, .. } => {
                        if let Some(true) = awaiting {
                            contested_total += 1.0;
                            if *player < home_lines {
                                secured += 1.0;
                            }
                        }
                        awaiting = None;
                    }
                    _ => {}
                }
            }
            if world.finished {
                break;
            }
        }
    }
    (
        secured,
        chances,
        contested_total,
        boxout_facts,
        opportunity_total,
    )
}

fn main() {
    for value in [0.10f32, 0.95f32] {
        let (secured, chances, contested, boxouts, opportunities) = arm(value, 8);
        println!(
            "defensive_rebound={value:.2} secured={secured:.0} chances={chances:.0} rate={:.4} \
             rebound_events={contested:.0} home_boxout_facts={boxouts:.0} opportunities={opportunities:.0}",
            secured / chances.max(1.0),
        );
    }
}
