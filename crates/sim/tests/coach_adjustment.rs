//! 跨回合教练调整的成对证据。
//!
//! 同一种掩护机会反复转成轻松得分会改变防守安排；相同次数的机会若主要
//! 靠高难度命中得分，教练保持原安排。两组只用登记的输入不同。

use nba_sim_core::action::TeamSide;
use nba_sim_core::coach::{adjust, observe_possession, Adjustment, PossessionSignal};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{MatchSetup, Params, World};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(
        MatchSetup::builtin(nba_domain::rules::GameRules::default()),
        params,
        seed,
    )
}

fn repeated(world: &mut World, difficult: bool) -> Adjustment {
    for _ in 0..3 {
        observe_possession(
            world,
            TeamSide::Away,
            PossessionSignal {
                screen_used: true,
                shot: true,
                shot_distance_ft: if difficult { 28.0 } else { 8.0 },
                contest: if difficult { 0.9 } else { 0.1 },
                points: 2,
            },
        );
    }
    adjust(world, TeamSide::Away)
}

#[test]
fn repeated_mechanism_opportunities_change_the_defensive_scheme() {
    let mut easy = world(311);
    let decision = repeated(&mut easy, false);
    assert_eq!(decision, Adjustment::SwitchHeavy);
    assert_eq!(easy.coach_schemes[1], "def_switch_heavy");
    assert!(easy.events.iter().any(|event| matches!(
        event.fact,
        nba_sim_core::Fact::CoachAdjustment { .. }
    )));

    let mut difficult = world(311);
    let decision = repeated(&mut difficult, true);
    assert_eq!(decision, Adjustment::Keep);
    assert_eq!(difficult.coach_schemes[1], difficult.setup.away.default_defense_tactic);
    assert!(!difficult.events.iter().any(|event| matches!(
        event.fact,
        nba_sim_core::Fact::CoachAdjustment { .. }
    )));
}

#[test]
fn unseen_or_non_screen_possessions_do_not_create_adjustment_evidence() {
    let mut world = world(313);
    for _ in 0..6 {
        observe_possession(
            &mut world,
            TeamSide::Away,
            PossessionSignal {
                screen_used: false,
                shot: true,
                shot_distance_ft: 8.0,
                contest: 0.1,
                points: 2,
            },
        );
    }
    assert_eq!(adjust(&mut world, TeamSide::Away), Adjustment::Keep);
    assert_eq!(world.coach_evidence[1].repeated_opportunities, 0);
}
