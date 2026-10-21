use nba_domain::play::{DecisionActionFamily, PlayInhibitionMode};
use nba_domain::rules::GameRules;
use nba_sim_core::action::{ActionKind, PassKind, ShotKind};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::Phase;
use nba_sim_core::tactics::TacticsTable;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};

fn family_code(family: DecisionActionFamily) -> Option<u8> {
    match family {
        DecisionActionFamily::Shoot => Some(0),
        DecisionActionFamily::Drive => Some(1),
        DecisionActionFamily::Pass => Some(2),
        DecisionActionFamily::PostUp => Some(5),
        DecisionActionFamily::Dwell => Some(6),
        DecisionActionFamily::TripleThreatJab => None,
    }
}

#[test]
fn every_play_slot_exists_in_its_system() {
    let params = Params::load(&workspace_calibration_dir()).expect("tactics load");
    let table = &params.tactics;
    assert_eq!(table.plays.len(), 9, "nine play specs");
    assert_eq!(table.systems.len(), 7, "seven system specs");
    table
        .validate()
        .expect("every rule slot must exist in the systems that list the play");
}

#[test]
fn at_least_two_plays_declare_inhibitions_with_shot_or_drive_preferences() {
    let table = TacticsTable::load(&workspace_calibration_dir()).expect("tactics load");
    let mut qualifying = 0;
    for play in &table.plays {
        if play.inhibitions.is_empty() {
            continue;
        }
        let prefers_shot_or_drive = play.carrier_preferences.iter().any(|preference| {
            matches!(
                preference.action_family,
                DecisionActionFamily::Shoot | DecisionActionFamily::Drive
            )
        }) || play.rules.iter().any(|rule| {
            rule.carrier_preferences.iter().any(|preference| {
                matches!(
                    preference.action_family,
                    DecisionActionFamily::Shoot | DecisionActionFamily::Drive
                )
            })
        });
        if prefers_shot_or_drive {
            qualifying += 1;
        }
    }
    assert!(
        qualifying >= 2,
        "only {qualifying} play declares inhibitions together with Shoot/Drive preferences"
    );
}

#[test]
fn hard_inhibition_removes_the_family_from_the_candidate_set() {
    let params = Params::load(&workspace_calibration_dir()).expect("tactics load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, 13);
    for _ in 0..40_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase, Phase::Live) && world.holder().is_some() {
            break;
        }
    }
    let play = world
        .params
        .tactics
        .plays
        .iter()
        .position(|play| {
            play.inhibitions
                .iter()
                .any(|inhibition| matches!(inhibition.mode, PlayInhibitionMode::Hard))
        })
        .expect("some play declares a hard inhibition");
    world.possession.play = Some(nba_sim_core::tactics::PlayWindow {
        play,
        ends_at: world.now() + 5.0,
    });
    let mut candidates = vec![
        (ActionKind::Shoot(ShotKind::PullUp), 1.0, None, glam::Vec2::ZERO),
        (ActionKind::Drive, 1.0, None, glam::Vec2::ZERO),
        (ActionKind::Pass(PassKind::Chest), 1.0, None, glam::Vec2::ZERO),
        (ActionKind::PostUp, 1.0, None, glam::Vec2::ZERO),
        (ActionKind::Dribble, 1.0, None, glam::Vec2::ZERO),
    ];
    let blocked: Vec<u8> = world.params.tactics.plays[play]
        .inhibitions
        .iter()
        .filter(|inhibition| matches!(inhibition.mode, PlayInhibitionMode::Hard))
        .filter_map(|inhibition| family_code(inhibition.action_family))
        .collect();
    nba_sim_core::tactics::apply_play_effects(&world, &mut candidates);
    assert!(!blocked.is_empty(), "the chosen play must declare a hard inhibition");
    for (kind, _, _, _) in &candidates {
        assert!(
            !blocked.contains(&kind.family()),
            "family {} survived a hard inhibition",
            kind.family()
        );
    }
}

#[test]
fn decisions_inside_a_hard_inhibition_window_avoid_the_blocked_family() {
    let params = Params::load(&workspace_calibration_dir()).expect("tactics load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, 13);
    let mut seen = 0;
    for _ in 0..80_000 {
        let blocked: Vec<u8> = world
            .possession
            .play
            .map(|window| {
                world.params.tactics.plays[window.play]
                    .inhibitions
                    .iter()
                    .filter(|inhibition| matches!(inhibition.mode, PlayInhibitionMode::Hard))
                    .filter_map(|inhibition| family_code(inhibition.action_family))
                    .collect()
            })
            .unwrap_or_default();
        world.step(TICK_SECONDS);
        if world.decision_samples.len() > seen {
            seen = world.decision_samples.len();
            let family = world.decision_samples[seen - 1].family;
            assert!(
                !blocked.contains(&family),
                "family {family} was selected while hard-inhibited by {blocked:?}"
            );
        }
        if matches!(world.phase, Phase::Finished) {
            break;
        }
    }
}
