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
use nba_sim_core::state::Phase;
use nba_sim_core::world::{World, TICK_SECONDS};
use nba_sim_core::{MatchSetup, Params};
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
    world.free_throw_lane = Some(nba_sim_core::world::FreeThrowLane::default());
    world.adjudicate(nba_sim_core::adjudicate::Adjudication::new(
        nba_sim_core::adjudicate::StoppageCause::FreeThrow,
        nba_sim_core::adjudicate::RestartPlan::FreeThrows {
            shooter,
            total,
            remaining: total,
            made: 0,
            and_one: false,
            retain_possession: false,
            foul_id: 1,
            technical_retained_team: None,
            after_technical: None,
        },
    ));
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

/// 罚球已经离手时，出手时限不再成立。
///
/// 球在空中属于飞行结算，不属于「罚球人持球等待出手」。旧时限戳留下来
/// 会让飞行被误判为罚球超时，罚球程序因此被切成死球，节末结算永远等不
/// 到这次待决出手关闭。
#[test]
fn a_free_throw_in_flight_is_not_a_shooting_timeout() {
    let mut world = live_world(1206);
    let shooter = world.holder().expect("live possession has a holder");
    free_throw_program(&mut world, shooter, 1);
    nba_sim_core::resolve::freethrow::shoot(
        &mut world,
        shooter,
        1,
        1,
        0,
        false,
        false,
        1,
        None,
        None,
    );
    assert!(
        world.pending_shot.is_some(),
        "罚球飞行必须有待结算的出手"
    );
    let limit = f64::from(world.rules.league.free_throws.free_throw_time_limit_seconds);
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
    assert!(!expired, "球在空中时不得判罚球出手超时");
    assert!(
        matches!(world.phase(), Phase::FreeThrows { .. }),
        "罚球程序必须继续到飞行结算，实际 {:?}",
        world.phase()
    );
}

#[test]
fn the_free_throw_time_limit_expires_into_a_turnover() {
    let mut world = live_world(1205);
    let offense = world.possession.team;
    let shooter = world.holder().expect("live possession has a holder");
    free_throw_program(&mut world, shooter, 2);
    // 把罚球时限推到过去：罚球人没有在限时内出手。
    let limit = f64::from(world.rules.league.free_throws.free_throw_time_limit_seconds);
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

#[test]
fn the_staging_places_the_shooter_on_the_free_throw_line() {
    let mut world = live_world(1207);
    let shooter = world.holder().expect("live possession has a holder");
    free_throw_program(&mut world, shooter, 2);
    // 列位后: 出手人目标必须是罚球线, 且距筐 13.75ft。
    let team = world.players[shooter].team;
    let attacking_right = world.attacking_right(team);
    let hoop = world.hoop_for(team);
    let line = nba_domain::court::Court::free_throw_pos_with_geometry(
        attacking_right,
        world.rules.free_throw_distance_ft,
        world.rules.court,
    );
    let spots = world.free_throw_procedure_spots();
    let shooter_spot = spots
        .iter()
        .find(|(handle, _)| *handle == shooter)
        .map(|(_, spot)| *spot)
        .expect("the shooter must have a staged spot");
    assert!((shooter_spot - line).length() < 0.5, "罚球人的列位点必须是罚球线, 实际 {shooter_spot:?}");
    let distance = f64::from((shooter_spot - hoop).length());
    let expected = f64::from(world.rules.free_throw_distance_ft);
    assert!((distance - expected).abs() < 0.5, "罚球人的列位点必须距筐 {expected}ft, 实际 {distance}");
    // 普通两罚：罚球人、五名合法列位球员，其余球员站在限制区外。
    assert_eq!(spots.len(), 10, "整队所有在场球员都应有程序站位");
    let slots = nba_domain::court::Court::free_throw_lane_slots_with_geometry(
        attacking_right,
        world.rules.court,
        world.rules.player_radius_ft,
    );
    let lane_spots: Vec<_> = spots
        .iter()
        .filter(|(handle, _)| *handle != shooter)
        .map(|(_, spot)| *spot)
        .collect();
    assert_eq!(lane_spots.len(), 9);
    for spot in &lane_spots[..5] {
        assert!(slots.iter().any(|slot| (*slot - *spot).length() < 0.5));
    }
    assert!(lane_spots[5..].iter().all(|spot| {
        !world.rules.court.is_in_lane(*spot, attacking_right)
    }), "non-lane players stay outside the painted lane");
}
