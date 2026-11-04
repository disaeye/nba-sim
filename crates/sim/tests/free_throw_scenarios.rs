//! 罚球：禁区违例与罚球人替换。
//!
//! NBA Rule 8 §III（罚球程序与禁区站位）、Rule 9 §II（罚球人替换）；
//! FIBA Article 17（罚球程序）。
//!
//! 判定落在 `World::check_free_throw_lane` 与 `World::advance_phase` 的
//! 罚球分支。

mod common;

use nba_sim_core::action::Fact;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::{Phase, PossessionKind};
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

/// 开一场罚球程序：罚球人站在罚球线上。
fn free_throw_program(world: &mut World, shooter: usize, total: u8) {
    world.set_phase(Phase::FreeThrows {
        shooter,
        total,
        remaining: total,
        made: 0,
        next_at: world.now() + 0.5,
        and_one: false,
        retain_possession: false,
        foul_id: 1,
        technical_retained_team: None,
        after_technical: None,
    });
    world.free_throw_started = Some(world.now());
    world.free_throw_lane = Some(nba_sim_core::world::FreeThrowLane::default());
    world.set_ball_held(shooter);
}

/// 罚球线延长线与底线之间的禁区：把一个球员放进去。
fn put_in_lane(world: &mut World, handle: usize, attacking_right: bool) {
    let court = world.rules.court;
    let baseline = if attacking_right {
        court.width_ft
    } else {
        0.0
    };
    world.players[handle].pos = glam::Vec2::new(baseline - 8.0, court.hoop_y_ft);
    world.players[handle].target = world.players[handle].pos;
}

#[test]
fn an_early_offensive_entry_ends_the_free_throw_with_a_turnover() {
    let mut world = live_world(1201);
    let offense = world.possession.team;
    let shooter = world.holder().expect("live possession has a holder");
    free_throw_program(&mut world, shooter, 2);
    let attacking_right = world.attacking_right(offense);
    // 进攻方有人提前进区。
    let early = world
        .on_court(offense)
        .as_slice()
        .iter()
        .copied()
        .find(|handle| *handle != shooter)
        .expect("the offence has another on-court player");
    put_in_lane(&mut world, early, attacking_right);
    world.check_free_throw_lane();
    assert!(
        world
            .free_throw_lane
            .expect("禁区记录必须存在")
            .offense_early,
        "进攻方提前进区必须被记下"
    );
}

#[test]
fn an_early_defensive_entry_is_recorded_separately() {
    let mut world = live_world(1202);
    let offense = world.possession.team;
    let shooter = world.holder().expect("live possession has a holder");
    free_throw_program(&mut world, shooter, 2);
    let attacking_right = world.attacking_right(offense);
    // 防守方有人提前进区。
    let early = world
        .on_court(offense.other())
        .as_slice()
        .first()
        .copied()
        .expect("the defence has an on-court player");
    put_in_lane(&mut world, early, attacking_right);
    world.check_free_throw_lane();
    let lane = world.free_throw_lane.expect("禁区记录必须存在");
    assert!(lane.defense_early, "防守方提前进区必须被记下");
    assert!(
        !lane.offense_early,
        "防守方提前进区不得同时记成进攻方进区"
    );
}

#[test]
fn a_player_outside_the_lane_does_not_trigger_a_violation() {
    let mut world = live_world(1203);
    let offense = world.possession.team;
    let shooter = world.holder().expect("live possession has a holder");
    free_throw_program(&mut world, shooter, 2);
    // 把双方的人都放到禁区之外。
    for handle in 0..world.players.len() {
        if !world.players[handle].on_court {
            continue;
        }
        world.players[handle].pos = glam::Vec2::new(40.0, 8.0);
        world.players[handle].target = world.players[handle].pos;
    }
    world.check_free_throw_lane();
    let lane = world.free_throw_lane.expect("禁区记录必须存在");
    assert!(!lane.offense_early, "禁区外的人不算提前进区");
    assert!(!lane.defense_early, "禁区外的人不算提前进区");
}

#[test]
fn the_shooter_is_replaced_when_his_own_spot_leaves_the_court() {
    let mut world = live_world(1204);
    let offense = world.possession.team;
    let shooter = world.holder().expect("live possession has a holder");
    free_throw_program(&mut world, shooter, 2);
    // 罚球人被换下场：剩下的罚球由在场队友罚。
    world.players[shooter].on_court = false;
    world.sync_on_court();
    let stand_in = world
        .on_court(offense)
        .as_slice()
        .first()
        .copied()
        .expect("the offence must still field a player");
    world.step(TICK_SECONDS);
    match world.phase() {
        Phase::FreeThrows { shooter: actual, .. } => {
            assert_eq!(
                actual, stand_in,
                "罚球人离场后由在场队友罚球"
            );
        }
        other => panic!("罚球人离场之后必须继续罚球程序，实际 {other:?}"),
    }
}

#[test]
fn the_free_throw_time_limit_expires_into_a_turnover() {
    let mut world = live_world(1205);
    let offense = world.possession.team;
    let shooter = world.holder().expect("live possession has a holder");
    free_throw_program(&mut world, shooter, 2);
    // 把罚球时限推到过去：罚球人没有在限时内出手。
    let limit = f64::from(world.rules.league.free_throw_time_limit_seconds);
    world.free_throw_started = Some(world.now() - limit - 1.0);
    world.step(TICK_SECONDS);
    let expired = world.events.iter().any(|event| {
        matches!(
            &event.fact,
            Fact::PossessionEnded {
                termination_detail: "free_throw_timeout",
                ..
            }
        )
    });
    assert!(expired, "罚球人超时必须判违例");
    // 罚则：对方在底线发球，球权交给对方（NBA Rule 9 §II）。
    match world.phase() {
        Phase::DeadBall { team, .. } => assert_eq!(
            team,
            offense.other(),
            "罚球超时之后由对方发球"
        ),
        other => panic!("罚球超时之后必须停表换边，实际 {other:?}"),
    }
}
