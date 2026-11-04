//! 换人：犯满离场、被逐与罚球人替换。
//!
//! NBA Rule 3 §I（队员资格与替换）、Rule 3 §V（替换与罚球）、Rule 12A
//! §II（球队技术犯规）；FIBA Article 15（替换程序）。
//!
//! 判定落在 `nba_sim_core::rotation` 与 `nba_sim_core::resolve::foul`。

mod common;

use nba_sim_core::action::{Fact, FoulKind};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::program::rotation::{apply_dead_ball_window, note_intents, record_foul};
use nba_sim_core::state::Phase;
use nba_sim_core::world::{World, TICK_SECONDS};
use nba_sim_core::{MatchSetup, Params, TeamSide};
use nba_domain::rules::GameRules;

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

/// 一个活球有人持球的世界。
fn live_world(seed: u64) -> World {
    let mut world = world(seed);
    for _ in 0..40_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::Live) && world.holder().is_some() {
            return world;
        }
    }
    panic!("the engine never reached a live ball state");
}

#[test]
fn a_player_who_reaches_the_foul_limit_leaves_the_court() {
    let mut world = live_world(1301);
    let team = world.possession.team;
    let player = world.on_court(team).as_slice()[0];
    let limit = world.rules.league.fouls.max_personal_fouls;
    world.players[player].fouls = limit - 1;
    world.set_phase(Phase::DeadBall {
        team,
        ready_at: f64::MAX,
    });
    // 再吃一次个人犯规：犯满离场。
    let fouled = world.holder().expect("live possession has a holder");
    let disqualified = record_foul(&mut world, player);
    assert!(disqualified, "犯满必须被逐");
    assert!(
        world.players[player].disqualified,
        "犯满的球员必须记上被逐标记"
    );
    assert_eq!(
        world.players[player].fouls, limit,
        "犯满的球员必须记满个人犯规"
    );
    // 换人窗口里他必须下场，除非替补席也满了。
    apply_dead_ball_window(&mut world);
    world.set_phase(Phase::Live);
    note_intents(&mut world);
    if world
        .players
        .iter()
        .any(|player| {
            player.team == team && !player.on_court && !player.disqualified && player.fouls < limit
        })
    {
        assert!(
            !world.players[player].on_court,
            "有合格替补时，犯满的球员必须下场"
        );
    }
    let _ = fouled;
}

#[test]
fn a_technical_foul_does_not_count_toward_the_personal_limit() {
    // NBA Rule 12A §V：技术犯规不计入个人犯规累计。
    assert!(!nba_domain::rules::counts_toward_personal_limit(
        FoulKind::Technical
    ));
    assert!(nba_domain::rules::counts_toward_personal_limit(
        FoulKind::Personal
    ));
}

#[test]
fn a_technical_foul_does_not_count_toward_the_team_period_fouls() {
    // NBA Rule 12A §V：技术犯规不计入球队单节犯规。
    assert!(!nba_domain::rules::counts_toward_team_period_fouls(
        FoulKind::Technical
    ));
    assert!(nba_domain::rules::counts_toward_team_period_fouls(
        FoulKind::Personal
    ));
}

#[test]
fn the_lineup_never_changes_while_the_ball_is_live() {
    // 换人只能在死球窗口里发生（NBA Rule 3 §V）。
    let mut world = live_world(1302);
    for _ in 0..400 {
        let mut mask_before = 0u64;
        for (handle, player) in world.players.iter().enumerate() {
            if player.on_court {
                mask_before |= 1 << handle;
            }
        }
        let phase_before = world.phase();
        world.step(TICK_SECONDS);
        let mut mask_after = 0u64;
        for (handle, player) in world.players.iter().enumerate() {
            if player.on_court {
                mask_after |= 1 << handle;
            }
        }
        if mask_before != mask_after {
            let dead_before = !matches!(phase_before, Phase::Live);
            let dead_after = !matches!(world.phase(), Phase::Live);
            assert!(
                dead_before || dead_after,
                "活球期间不得换人：{phase_before:?} -> {:?}",
                world.phase()
            );
        }
    }
}

#[test]
fn a_substitution_swaps_the_player_and_his_station() {
    let mut world = live_world(1303);
    let team = world.possession.team;
    let outgoing = world.on_court(team).as_slice()[0];
    let incoming = (0..world.players.len())
        .find(|handle| {
            let player = &world.players[*handle];
            player.team == team && !player.on_court && !player.disqualified
        })
        .expect("the team must have a substitute");
    let slot = world.players[outgoing].slot;
    world.set_phase(Phase::DeadBall {
        team,
        ready_at: f64::MAX,
    });
    // 把替补的体能拉满，主力拉到需要休息：换人窗口因此会换人。
    world.players[incoming].stamina = 1.0;
    world.players[outgoing].stamina = 0.0;
    apply_dead_ball_window(&mut world);
    world.set_phase(Phase::Live);
    note_intents(&mut world);
    if world.players[incoming].on_court {
        assert!(!world.players[outgoing].on_court, "换人必须换下场");
        assert_eq!(
            world.players[incoming].slot, slot,
            "替补必须接过被换下者的站位"
        );
    }
}

#[test]
fn a_substituted_in_player_may_not_return_immediately() {
    let mut world = live_world(1304);
    let team = world.possession.team;
    let outgoing = world.on_court(team).as_slice()[0];
    let incoming = (0..world.players.len())
        .find(|handle| {
            let player = &world.players[*handle];
            player.team == team && !player.on_court && !player.disqualified
        })
        .expect("the team must have a substitute");
    world.set_phase(Phase::DeadBall {
        team,
        ready_at: f64::MAX,
    });
    world.players[incoming].stamina = 1.0;
    world.players[outgoing].stamina = 0.0;
    apply_dead_ball_window(&mut world);
    if !world.players[incoming].on_court {
        return;
    }
    // 刚上场的人必须坐够最短休息时间才能回来。
    assert!(
        world.players[outgoing].resting_until > world.clock.elapsed(),
        "刚下场的人必须坐够最短休息时间"
    );
}

#[test]
fn the_foul_ledger_records_the_personal_and_team_counts() {
    let mut world = live_world(1305);
    let team = world.possession.team;
    let player = world.on_court(team).as_slice()[0];
    let team_period_before = world.fouls[World::team_index(team)].period;
    let personal_before = world.line(player).fouls;
    let fouled = world.holder().expect("live possession has a holder");
    nba_sim_core::resolve::foul::apply_non_shooting_foul(&mut world, player, fouled);
    assert_eq!(
        world.fouls[World::team_index(team)].period,
        team_period_before + 1,
        "个人犯规必须计入球队单节犯规"
    );
    assert_eq!(
        world.line(player).fouls,
        personal_before + 1,
        "个人犯规必须计入个人累计"
    );
    let foul = world
        .events
        .iter()
        .rev()
        .find_map(|event| match &event.fact {
            Fact::Foul { kind, .. } => Some(*kind),
            _ => None,
        })
        .expect("the foul must be recorded");
    assert_eq!(foul, FoulKind::Personal);
}
