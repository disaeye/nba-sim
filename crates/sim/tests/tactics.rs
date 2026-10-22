use nba_domain::play::{DecisionActionFamily, PlayInhibitionMode};
use nba_domain::rules::GameRules;
use nba_sim_core::action::{ActionKind, PassKind, ShotKind};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::policy::candidate_utilities;
use nba_sim_core::state::Phase;
use nba_sim_core::tactics::{PlayWindow, TacticsTable};
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
        (
            ActionKind::Shoot(ShotKind::PullUp),
            None,
            glam::Vec2::ZERO,
            nba_sim_core::utility::Utility::default(),
        ),
        (
            ActionKind::Drive,
            None,
            glam::Vec2::ZERO,
            nba_sim_core::utility::Utility::default(),
        ),
        (
            ActionKind::Pass(PassKind::Chest),
            None,
            glam::Vec2::ZERO,
            nba_sim_core::utility::Utility::default(),
        ),
        (
            ActionKind::PostUp,
            None,
            glam::Vec2::ZERO,
            nba_sim_core::utility::Utility::default(),
        ),
        (
            ActionKind::Dribble,
            None,
            glam::Vec2::ZERO,
            nba_sim_core::utility::Utility::default(),
        ),
    ];
    let blocked: Vec<u8> = world.params.tactics.plays[play]
        .inhibitions
        .iter()
        .filter(|inhibition| matches!(inhibition.mode, PlayInhibitionMode::Hard))
        .filter_map(|inhibition| family_code(inhibition.action_family))
        .collect();
    nba_sim_core::tactics::apply_play_effects(&world, &mut candidates);
    assert!(
        !blocked.is_empty(),
        "the chosen play must declare a hard inhibition"
    );
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
        let active = world.possession.play.map(|window| window.play);
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
            // 窗口在同一步长里到期时，这次决策发生在抑制解除之后，
            // 不能拿步长开始时的抑制表来判它。
            if world.possession.play.map(|window| window.play) != active {
                continue;
            }
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

/// 战术先验只能改战术项，不能隐式改期望得分。
///
/// 同一个回合分别按「自由阅读」和「跑某个战术」求候选：候选集只允许
/// 因硬抑制而变小，保留下来的候选的 EPV 各项、方差、时钟成本与犯规
/// 成本必须逐字相同，只有 `tactical_bonus` 可以变。
#[test]
fn play_prior_only_touches_the_tactical_term() {
    let params = Params::load(&workspace_calibration_dir()).expect("tactics load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, 21);
    for _ in 0..40_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase, Phase::Live) && world.holder().is_some() {
            break;
        }
    }
    world.possession.play = None;
    let free = candidate_utilities(&world, &world.decision_state());
    assert!(!free.is_empty(), "自由阅读必须产出候选");
    let mut moved = 0;
    for index in 0..world.params.tactics.plays.len() {
        let id = world.params.tactics.plays[index].id.clone();
        world.possession.play = Some(PlayWindow {
            play: index,
            ends_at: world.now() + 5.0,
        });
        let with_play = candidate_utilities(&world, &world.decision_state());
        assert!(!with_play.is_empty(), "{id} 把候选集清空了");
        assert!(
            with_play.len() <= free.len(),
            "{id} 增加了自由阅读里没有的候选"
        );
        for (kind, _, _, utility) in &with_play {
            let baseline = free
                .iter()
                .find(|(free_kind, _, _, _)| free_kind == kind)
                .unwrap_or_else(|| panic!("{id} 引入了自由阅读里不存在的候选 {kind:?}"));
            let base = baseline.3;
            assert!(
                (utility.epv.sum() - base.epv.sum()).abs() < 1e-4,
                "{id} 改动了 {kind:?} 的期望得分"
            );
            assert!(
                (utility.variance - base.variance).abs() < 1e-4,
                "{id} 改动了 {kind:?} 的方差项"
            );
            assert!(
                (utility.clock_cost - base.clock_cost).abs() < 1e-4,
                "{id} 改动了 {kind:?} 的时钟成本"
            );
            assert!(
                (utility.foul_cost - base.foul_cost).abs() < 1e-4,
                "{id} 改动了 {kind:?} 的犯规成本"
            );
            if (utility.tactical_bonus - base.tactical_bonus).abs() > 1e-4 {
                moved += 1;
            }
        }
    }
    assert!(moved > 0, "没有任何剧本改到战术项，这条断言会变成空话");
}
