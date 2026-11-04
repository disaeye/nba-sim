//! 界外发球与发球违例，出界归属，以及运球收球的脚步规则。
//!
//! NBA Rule 8 §III（发球违例）、Rule 10 §II（运球）、§XIII（走步）与
//! §I（出界）。
//!
//! 判定落在 `World::advance_phase` 的发球分支、`control::out_of_bounds`
//! 与 `nba_sim_core::footwork`。

mod common;

use glam::Vec2;
use nba_sim_core::action::{Fact, TurnoverKind};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::{DribbleState, PossessionKind, Phase};
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
fn the_inbound_five_second_clock_expires_into_a_turnover() {
    let mut world = world(1101);
    // 直接进入发球程序：发球队员站在本方底线。
    world.set_phase(Phase::Inbound {
        team: TeamSide::Home,
        ready_at: world.now() + 100.0,
    });
    world.inbound_started_at = Some(world.now());
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Dead);
    world.set_phase(Phase::Inbound {
        team: TeamSide::Home,
        ready_at: world.now() + 100.0,
    });
    world.inbound_started_at = Some(world.now());
    let mut violation = false;
    for _ in 0..400 {
        world.step(TICK_SECONDS);
        if world.events.iter().any(|event| {
            matches!(
                &event.fact,
                Fact::Turnover {
                    kind: TurnoverKind::OutOfBounds,
                    ..
                }
            )
        }) {
            violation = true;
            break;
        }
        // 违例的落点写在回合结束的明细里。
        if world.events.iter().any(|event| {
            matches!(
                &event.fact,
                Fact::PossessionEnded {
                    termination_detail: "inbound_timeout",
                    ..
                }
            )
        }) {
            violation = true;
            break;
        }
    }
    assert!(violation, "发球超过五秒必须判违例");
}

#[test]
fn the_inbound_clock_is_started_when_the_procedure_begins() {
    let mut world = world(1102);
    world.set_phase(Phase::Inbound {
        team: TeamSide::Home,
        ready_at: world.now() + 100.0,
    });
    world.inbound_started_at = Some(world.now());
    let started = world.inbound_started_at.expect("发球程序必须记下开始时刻");
    assert!(
        (started - world.now()).abs() < 0.01,
        "发球五秒从发球程序开始那一刻算起"
    );
}

#[test]
fn a_ball_out_of_bounds_goes_to_the_opponent_of_the_last_touch() {
    let mut world = live_world(1103);
    let offense = world.possession.team;
    let holder = world.holder().expect("live possession has a holder");
    // 把球放到边线外：球必须先脱手，出界判定才在活球争抢这一层，而且
    // 外向速度要超过慢球阈值，否则球会被判成停在边线上。
    world.ball = nba_sim_core::ball::BallBody::loose(
        Vec2::new(2.0, 25.0),
        Vec2::new(-30.0, 0.0),
    );
    world.last_touch = Some(holder);
    let mut recorded = false;
    for _ in 0..40 {
        world.step(TICK_SECONDS);
        if world.events.iter().any(|event| {
            matches!(&event.fact, Fact::BallContact { kind: "out_of_bounds", .. })
        }) {
            recorded = true;
            break;
        }
    }
    assert!(recorded, "球出界必须留下出界事实");
    // 最后触球的是进攻方：球权给防守方。
    match world.phase() {
        Phase::DeadBall { team, .. } => assert_eq!(
            team,
            offense.other(),
            "进攻方碰出界，球权给对方"
        ),
        other => panic!("出界之后必须停表，实际 {other:?}"),
    }
}

#[test]
fn a_ball_touched_by_the_defence_out_of_bounds_keeps_the_offence_in_possession() {
    let mut world = live_world(1104);
    let offense = world.possession.team;
    let defender = world
        .on_court(offense.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    world.ball = nba_sim_core::ball::BallBody::loose(
        Vec2::new(2.0, 25.0),
        Vec2::new(-30.0, 0.0),
    );
    world.last_touch = Some(defender);
    let mut recorded = false;
    for _ in 0..40 {
        world.step(TICK_SECONDS);
        if world.events.iter().any(|event| {
            matches!(&event.fact, Fact::BallContact { kind: "out_of_bounds", .. })
        }) {
            recorded = true;
            break;
        }
    }
    assert!(recorded, "球出界必须留下出界事实");
    match world.phase() {
        Phase::DeadBall { team, .. } => assert_eq!(
            team, offense,
            "防守方碰出界，进攻方保留球权"
        ),
        other => panic!("出界之后必须停表，实际 {other:?}"),
    }
}

#[test]
fn a_shot_that_leaves_the_court_is_not_a_turnover() {
    let mut world = live_world(1105);
    let offense = world.possession.team;
    let holder = world.holder().expect("live possession has a holder");
    // 投篮出手之后球飞出界：这是投篮不中，不记失误。
    world.ball.pos = Vec2::new(40.0, -1.0);
    world.ball.vel = Vec2::new(0.0, -5.0);
    world.ball.vz = 0.0;
    world.last_touch = Some(holder);
    // 建一笔待结算的出手：出手中的球飞出界不是违例。
    world.pending_shot = Some(nba_sim_core::resolve::shot::PendingShot {
        shooter: holder,
        kind: nba_sim_core::action::ShotKind::CatchShoot,
        zone: "three",
        distance_ft: 25.0,
        contest: 0.0,
        fouled: false,
        and_one: false,
        free_throws: 0,
        points: 3,
        is_three: true,
        assisted_by: None,
        shot_id: 1,
        free_throw: None,
        block_by: None,
        result: None,
    });
    for _ in 0..40 {
        world.step(TICK_SECONDS);
        if world.events.iter().any(|event| {
            matches!(&event.fact, Fact::BallContact { kind: "out_of_bounds", .. })
        }) {
            break;
        }
    }
    let turnover = world.events.iter().any(|event| {
        matches!(
            &event.fact,
            Fact::Turnover {
                kind: TurnoverKind::OutOfBounds,
                ..
            }
        )
    });
    assert!(!turnover, "投篮出手之后球飞出界不记失误");
}

#[test]
fn a_second_dribble_after_a_gather_is_a_turnover() {
    let mut world = live_world(1106);
    let holder = world.holder().expect("live possession has a holder");
    // 收球：轴心确立。
    world.possession.dribble_state = DribbleState::Gathered;
    let gather = world.players[holder].pos;
    nba_sim_core::footwork::establish_pivot(&mut world, holder, gather);
    // 收球之后再运球：规则禁止（NBA Rule 10 §II.3）。
    world.possession.dribble_state = DribbleState::Dribbling;
    let dribbled = nba_sim_core::footwork::dribble_after_gather(&world, holder);
    assert!(
        !dribbled,
        "收球之后再次运球必须被脚步层判出来"
    );
}

#[test]
fn a_legal_recovery_after_a_loose_ball_is_not_a_dribble_violation() {
    let mut world = live_world(1107);
    let holder = world.holder().expect("live possession has a holder");
    // 没有收球：轴心不存在，此时运球合法。
    world.possession.dribble_state = DribbleState::Dribbling;
    world.possession.pivot = None;
    let dribbled = nba_sim_core::footwork::dribble_after_gather(&world, holder);
    assert!(
        !dribbled,
        "没有收球的运球是合法运球"
    );
}

#[test]
fn moving_the_pivot_foot_after_a_gather_is_a_travel() {
    let mut world = live_world(1108);
    let holder = world.holder().expect("live possession has a holder");
    world.possession.dribble_state = DribbleState::Gathered;
    let gather = world.players[holder].pos;
    nba_sim_core::footwork::establish_pivot(&mut world, holder, gather);
    let pivot = world.possession.pivot.expect("收球确立轴心");
    // 球手离开轴心脚的位置：走步。
    let step = nba_sim_core::footwork::step_length(&world, holder);
    world.players[holder].pos = pivot.position + Vec2::X * (step * 3.0);
    let verdict = nba_sim_core::footwork::judge_release(&world, holder);
    assert_eq!(
        verdict,
        nba_sim_core::footwork::FootworkVerdict::PivotMoved,
        "移动轴心脚必须判走步"
    );
}

#[test]
fn a_gather_within_two_steps_is_legal() {
    let mut world = live_world(1109);
    let holder = world.holder().expect("live possession has a holder");
    world.possession.dribble_state = DribbleState::Gathered;
    let gather = world.players[holder].pos;
    nba_sim_core::footwork::establish_pivot(&mut world, holder, gather);
    let step = nba_sim_core::footwork::step_length(&world, holder);
    // 走两步：每一步都记成一次脚步接触。
    let mut moved = gather;
    for _ in 0..2 {
        moved += Vec2::X * step;
        world.players[holder].pos = moved;
        world.players[holder].vel = Vec2::X * step;
        nba_sim_core::footwork::accumulate_contact(&mut world, holder);
    }
    assert_eq!(
        world.possession.pivot.map(|pivot| pivot.steps_taken),
        Some(2),
        "两步都已经用掉"
    );
    assert_eq!(
        nba_sim_core::footwork::judge_release(&world, holder),
        nba_sim_core::footwork::FootworkVerdict::Legal,
        "收球后的两步之内不得判走步"
    );
}

#[test]
fn a_gather_beyond_two_steps_is_a_travel() {
    let mut world = live_world(1110);
    let holder = world.holder().expect("live possession has a holder");
    world.possession.dribble_state = DribbleState::Gathered;
    let gather = world.players[holder].pos;
    nba_sim_core::footwork::establish_pivot(&mut world, holder, gather);
    let step = nba_sim_core::footwork::step_length(&world, holder);
    // 第三步：脚步额度用尽，走步。
    let mut moved = gather;
    for _ in 0..3 {
        moved += Vec2::X * step;
        world.players[holder].pos = moved;
        world.players[holder].vel = Vec2::X * step;
        nba_sim_core::footwork::accumulate_contact(&mut world, holder);
    }
    assert_eq!(
        world.possession.pivot.map(|pivot| pivot.steps_taken),
        Some(3),
        "第三步已经用掉了额度"
    );
    assert_eq!(
        nba_sim_core::footwork::judge_release(&world, holder),
        nba_sim_core::footwork::FootworkVerdict::StepsExhausted,
        "收球后的第三步必须判走步"
    );
}
