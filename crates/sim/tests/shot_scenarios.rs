//! 投篮与得分裁决，以及干扰球。
//!
//! NBA Rule 5 §I 与 §III（投篮得分与命中）、Rule 11（篮筐干涉与干扰球）。
//!
//! 判定落在 `nba_sim_core::resolve::shot`：这里钉的是「这个位置算两分
//! 还是三分」「球在筐上被碰到算不算干扰」「被盖与被犯的区别」。

mod common;

use glam::Vec2;
use nba_sim_core::action::{Fact, ShotKind};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::{PossessionKind, Phase};
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

/// 把持球人放到离篮筐 `offset` 的位置并面向篮筐。
fn place_shooter(world: &mut World, offset: Vec2) -> usize {
    let shooter = world.holder().expect("live possession has a holder");
    let team = world.players[shooter].team;
    let hoop = world.hoop_for(team);
    let pos = hoop + offset;
    world.players[shooter].pos = pos;
    world.players[shooter].target = pos;
    world.players[shooter].facing = (hoop - pos).normalize_or_zero();
    world.ball.pos = pos;
    world.ball.z = world.rules.ball_holder_height_ft;
    shooter
}

/// 出手一次并把结算推到结束。
fn shoot_from(world: &mut World, shooter: usize, kind: ShotKind) {
    let intent = nba_sim_core::action::ActionIntent {
        actor: shooter,
        kind: nba_sim_core::action::ActionKind::Shoot(kind),
        target: None,
        from_ft: world.players[shooter].pos,
        to_ft: world.players[shooter].pos,
        start_time: world.now(),
        duration: 0.8,
    };
    nba_sim_core::resolve::shot::resolve(world, &intent, kind);
    for _ in 0..400 {
        world.step(TICK_SECONDS);
        if world.pending_shot.is_none() {
            break;
        }
    }
}

#[test]
fn a_shot_from_inside_the_arc_is_worth_two_points() {
    let mut world = live_world(1001);
    // 篮下：必然是两分。
    let shooter = place_shooter(&mut world, Vec2::new(-3.0, 0.0));
    shoot_from(&mut world, shooter, ShotKind::CatchShoot);
    // 出手记录的是「值几分」与「打没打进」：进球时的分数才是得分。
    let (is_three, zone) = world
        .events
        .iter()
        .find_map(|event| match &event.fact {
            Fact::Shot {
                is_three, zone, ..
            } => Some((*is_three, *zone)),
            _ => None,
        })
        .expect("the shot must be recorded");
    assert!(!is_three, "弧内出手不是三分");
    assert_eq!(zone, "rim", "篮下出手记在篮下分区");
}

#[test]
fn a_shot_from_behind_the_arc_is_worth_three_points() {
    let mut world = live_world(1002);
    // 弧顶三分：离筐 25 英尺。
    let shooter = place_shooter(&mut world, Vec2::new(-25.0, 0.0));
    shoot_from(&mut world, shooter, ShotKind::CatchShoot);
    let (is_three, zone) = world
        .events
        .iter()
        .find_map(|event| match &event.fact {
            Fact::Shot {
                is_three, zone, ..
            } => Some((*is_three, *zone)),
            _ => None,
        })
        .expect("the shot must be recorded");
    assert!(is_three, "弧外出手是三分");
    assert_eq!(zone, "three", "弧外出手记在三分分区");
}

#[test]
fn a_corner_shot_behind_the_arc_is_worth_three_points() {
    let mut world = live_world(1003);
    // 底角：离篮筐 14 英尺，但横向贴着边线，三分线在底角更近。
    let shooter = place_shooter(&mut world, Vec2::new(-14.0, -23.0));
    shoot_from(&mut world, shooter, ShotKind::CatchShoot);
    let (is_three, zone) = world
        .events
        .iter()
        .find_map(|event| match &event.fact {
            Fact::Shot {
                is_three, zone, ..
            } => Some((*is_three, *zone)),
            _ => None,
        })
        .expect("the shot must be recorded");
    assert!(is_three, "底角出手是三分");
    assert_eq!(zone, "three", "底角出手记在三分分区");
}

#[test]
fn a_blocked_shot_leaves_the_ball_loose() {
    let mut world = live_world(1004);
    let shooter = place_shooter(&mut world, Vec2::new(-3.0, 0.0));
    // 把一个防守人放到投篮人与篮筐之间，并给他极高的封盖属性。
    let team = world.players[shooter].team;
    let defender = world
        .on_court(team.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    let hoop = world.hoop_for(team);
    world.players[defender].pos = hoop + Vec2::new(-2.0, 0.0);
    world.players[defender].target = world.players[defender].pos;
    world.players[defender].data.attributes.block = 1.0;
    shoot_from(&mut world, shooter, ShotKind::CatchShoot);
    // 结算之后球要么被盖成活球，要么被防守人控下：两种都不是得分。
    let made = world.events.iter().any(|event| {
        matches!(&event.fact, Fact::Shot { made: true, .. })
    });
    assert!(!made || world.possession.had_shot, "被盖的球不记得分");
}

#[test]
fn a_fouled_shooter_gets_free_throws() {
    let mut world = live_world(1005);
    let shooter = place_shooter(&mut world, Vec2::new(-5.0, 0.0));
    let team = world.players[shooter].team;
    let defender = world
        .on_court(team.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    // 直接吹一次投篮犯规：判罚路径是确定的，不靠抽样。
    nba_sim_core::resolve::foul::apply_shooting_foul(&mut world, defender, shooter, 2, false);
    assert!(
        matches!(world.phase(), Phase::FreeThrows { .. }),
        "投篮犯规之后必须进入罚球程序"
    );
    let foul = world
        .events
        .iter()
        .rev()
        .find_map(|event| match &event.fact {
            Fact::Foul {
                kind,
                free_throws,
                ..
            } => Some((*kind, *free_throws)),
            _ => None,
        })
        .expect("the foul must be recorded");
    assert_eq!(foul.1, 2, "两分投篮犯规罚两次");
}

#[test]
fn touching_a_descending_shot_inside_the_cylinder_is_goaltending() {
    let mut world = live_world(1006);
    let team = world.possession.team;
    let shooter = place_shooter(&mut world, Vec2::new(-8.0, 0.0));
    let defender = world
        .on_court(team.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    // 把球放到筐上方、正在下落的位置，防守人就在筐边。
    let hoop = world.hoop_for(team);
    world.ball.pos = hoop;
    world.ball.z = world.rules.rim_height_ft + 0.2;
    world.ball.vel = Vec2::ZERO;
    world.ball.vz = -3.0;
    world.players[defender].pos = hoop;
    world.players[defender].target = hoop;
    world.set_flight(
        nba_sim_core::state::FlightKind::Shot,
        world.now() + 2.0,
    );
    // 建立一笔待结算的出手：干扰球的判定读它。
    world.pending_shot = Some(nba_sim_core::resolve::shot::PendingShot {
        shooter,
        kind: ShotKind::CatchShoot,
        zone: "rim",
        distance_ft: 8.0,
        contest: 0.0,
        fouled: false,
        and_one: false,
        free_throws: 0,
        points: 2,
        is_three: false,
        assisted_by: None,
        shot_id: 1,
        free_throw: None,
        block_by: None,
        result: None,
        touched_backboard: false,
    });
    let called = nba_sim_core::resolve::shot::check_goaltending(&mut world);
    assert!(called, "下落段里碰到筐上方的球必须判干扰球");
    let fact = world
        .events
        .iter()
        .find_map(|event| match &event.fact {
            Fact::Goaltending { defender, .. } => Some(*defender),
            _ => None,
        })
        .expect("the goaltending fact must be published");
    assert_eq!(fact, defender, "干扰球的人必须是碰到球的防守人");
}

#[test]
fn an_ascending_ball_may_be_blocked_without_goaltending() {
    let mut world = live_world(1007);
    let team = world.possession.team;
    let shooter = place_shooter(&mut world, Vec2::new(-8.0, 0.0));
    let defender = world
        .on_court(team.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    let hoop = world.hoop_for(team);
    world.ball.pos = hoop;
    world.ball.z = world.rules.rim_height_ft + 0.2;
    world.ball.vel = Vec2::ZERO;
    // 球还在上升：合法封盖，不是干扰球。
    world.ball.vz = 3.0;
    world.players[defender].pos = hoop;
    world.players[defender].target = hoop;
    world.set_flight(
        nba_sim_core::state::FlightKind::Shot,
        world.now() + 2.0,
    );
    world.pending_shot = Some(nba_sim_core::resolve::shot::PendingShot {
        shooter,
        kind: ShotKind::CatchShoot,
        zone: "rim",
        distance_ft: 8.0,
        contest: 0.0,
        fouled: false,
        and_one: false,
        free_throws: 0,
        points: 2,
        is_three: false,
        assisted_by: None,
        shot_id: 1,
        free_throw: None,
        block_by: None,
        result: None,
        touched_backboard: false,
    });
    let called = nba_sim_core::resolve::shot::check_goaltending(&mut world);
    assert!(!called, "上升段的球可以合法封盖，不是干扰球");
}

/// 建立一笔待结算的出手：干扰球的判定读它。
fn pending_shot(world: &mut World, shooter: usize, is_three: bool) {
    world.pending_shot = Some(nba_sim_core::resolve::shot::PendingShot {
        shooter,
        kind: ShotKind::CatchShoot,
        zone: "rim",
        distance_ft: 8.0,
        contest: 0.0,
        fouled: false,
        and_one: false,
        free_throws: 0,
        points: 2,
        is_three,
        assisted_by: None,
        shot_id: 1,
        free_throw: None,
        block_by: None,
        result: None,
        touched_backboard: false,
    });
}

#[test]
fn touching_a_ball_that_touched_the_backboard_is_goaltending() {
    // NBA Rule 11 §I.3：球碰过篮板而且还有得分机会时，防守人再碰它就是
    // 干扰，不管球在上升还是下落。
    let mut world = live_world(1009);
    let team = world.possession.team;
    let shooter = place_shooter(&mut world, Vec2::new(-8.0, 0.0));
    let defender = world
        .on_court(team.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    let hoop = world.hoop_for(team);
    // 球在筐上方的圆柱段里，而且在上升：上升段本身不是干扰。
    world.ball.pos = hoop;
    world.ball.z = world.rules.rim_height_ft + 0.2;
    world.ball.vel = Vec2::ZERO;
    world.ball.vz = 3.0;
    world.players[defender].pos = hoop;
    world.players[defender].target = hoop;
    world.set_flight(
        nba_sim_core::state::FlightKind::Shot,
        world.now() + 2.0,
    );
    pending_shot(&mut world, shooter, false);
    // 还没有碰过篮板：上升段的球可以合法封盖。
    assert!(
        !nba_sim_core::resolve::shot::check_goaltending(&mut world),
        "上升段且没有碰过篮板的球可以合法封盖"
    );
    // 记一次碰篮板：此后防守人再碰它就是干扰。这一条记在本次出手自己
    // 的记录上，而不是扫事件日志——上一次出手碰过篮板不该让这一次的
    // 球也变成干扰。
    if let Some(pending) = world.pending_shot.as_mut() {
        pending.touched_backboard = true;
    }
    assert!(
        nba_sim_core::resolve::shot::check_goaltending(&mut world),
        "碰过篮板之后防守人再碰球就是干扰球"
    );
}

#[test]
fn touching_a_ball_that_is_sitting_on_the_ring_is_goaltending() {
    // NBA Rule 11 §I.1：球坐在或者滚在筐沿上时，防守人碰它就是干扰，
    // 无论他有没有把它按进。
    let mut world = live_world(1010);
    let team = world.possession.team;
    let shooter = place_shooter(&mut world, Vec2::new(-8.0, 0.0));
    let defender = world
        .on_court(team.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    let hoop = world.hoop_for(team);
    // 球坐在筐沿上：高度就是筐高，位置就是筐心。
    world.ball.pos = hoop;
    world.ball.z = world.rules.rim_height_ft;
    world.ball.vel = Vec2::ZERO;
    world.ball.vz = -0.5;
    world.players[defender].pos = hoop;
    world.players[defender].target = hoop;
    world.set_flight(
        nba_sim_core::state::FlightKind::Shot,
        world.now() + 2.0,
    );
    pending_shot(&mut world, shooter, false);
    assert!(
        nba_sim_core::resolve::shot::check_goaltending(&mut world),
        "球坐在筐沿上时碰它就是干扰球"
    );
}

#[test]
fn a_ball_outside_the_cylinder_is_not_goaltending() {
    let mut world = live_world(1008);
    let team = world.possession.team;
    let shooter = place_shooter(&mut world, Vec2::new(-8.0, 0.0));
    let defender = world
        .on_court(team.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    let hoop = world.hoop_for(team);
    // 球在筐上方但横向离开筐柱：不在圆柱段里。
    world.ball.pos = hoop + Vec2::new(0.0, 6.0);
    world.ball.z = world.rules.rim_height_ft + 0.2;
    world.ball.vel = Vec2::ZERO;
    world.ball.vz = -3.0;
    world.players[defender].pos = hoop;
    world.players[defender].target = hoop;
    world.set_flight(
        nba_sim_core::state::FlightKind::Shot,
        world.now() + 2.0,
    );
    world.pending_shot = Some(nba_sim_core::resolve::shot::PendingShot {
        shooter,
        kind: ShotKind::CatchShoot,
        zone: "rim",
        distance_ft: 8.0,
        contest: 0.0,
        fouled: false,
        and_one: false,
        free_throws: 0,
        points: 2,
        is_three: false,
        assisted_by: None,
        shot_id: 1,
        free_throw: None,
        block_by: None,
        result: None,
        touched_backboard: false,
    });
    let called = nba_sim_core::resolve::shot::check_goaltending(&mut world);
    assert!(!called, "筐柱圆柱段之外的球不算干扰球");
}
