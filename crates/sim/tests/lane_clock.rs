//! 限制区计时：进攻三秒与防守三秒。
//!
//! 规则读 `GameRules::three_second_lane_seconds` 与
//! `defensive_three_second_seconds`：连续停留超过时限即违例。
//!
//! 这些测试直接推进限制区计时层，不推进整场比赛：无球清空限制区的落点
//! 修正会把禁区里的球员推到区外，他因此不会被判三秒。规则本身的判定
//! ——谁在计时、计时何时归零、违例何时成立——由这里逐项验证。

use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_sim_core::action::{Fact, FoulKind, TurnoverKind};
use nba_sim_core::params::{workspace_calibration_dir, Params};
use nba_sim_core::state::{Phase, PossessionKind, Slot};
use nba_sim_core::world::{World, TICK_SECONDS};
use nba_sim_core::{MatchSetup, TeamSide};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

/// Home 向右进攻时的限制区：底线在 x=94，罚球线在 x=75，禁区宽到 y=17..33。
fn in_lane(pos: Vec2) -> bool {
    (pos.x - 94.0).abs() <= 19.0 && (pos.y - 25.0).abs() <= 8.0
}

/// 一个待测的进攻人待在限制区里，持球人在前场深处。
///
/// 进攻方控制前场的球，两条三秒规则的计时因此都开始。持球人自己不算：
/// 进攻三秒罚的是无球人，防守三秒罚的是不盯人的防守人。
fn offense_fixture(seed: u64) -> (World, usize, usize, usize) {
    let mut world = world(seed);
    let stuck = world.on_court(TeamSide::Home).as_slice()[0];
    let handler = world.on_court(TeamSide::Home).as_slice()[1];
    let defender = world.on_court(TeamSide::Away).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == stuck || index == handler || index == defender;
        player.vel = Vec2::ZERO;
    }
    // 待测的进攻人占一个外线槽位：低位与顺下两个槽位本身就要求人在
    // 禁区里，规则对它们另有豁免。
    world.players[stuck].slot = Slot::Wing;
    world.players[handler].slot = Slot::Handler;
    world.players[defender].slot = Slot::Corner;
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(handler);
    // 持球人在前场，但在禁区之外：球在禁区里时两条规则的计时都暂停
    // （球在禁区里说明有人正要接球或出手）。
    world.players[handler].pos = Vec2::new(70.0, 40.0);
    world.players[handler].target = world.players[handler].pos;
    world.ball.pos = world.players[handler].pos;
    world.ball.z = world.rules.ball_radius_ft;
    // 待在禁区里的进攻人：他的站位不动，计时因此一直走。
    world.players[stuck].pos = Vec2::new(80.0, 25.0);
    world.players[stuck].target = world.players[stuck].pos;
    // 防守人站在禁区外：他不在防守三秒的计时条件里。
    world.players[defender].pos = Vec2::new(60.0, 10.0);
    world.players[defender].target = world.players[defender].pos;
    (world, stuck, handler, defender)
}

#[test]
fn an_offensive_player_who_stays_in_the_lane_for_three_seconds_is_a_violation() {
    let (mut world, stuck, _, _) = offense_fixture(311);
    assert!(
        in_lane(world.players[stuck].pos),
        "夹具必须把进攻人放在限制区里"
    );
    assert_eq!(
        nba_sim_core::lane::offense_timer_holder(&world),
        Some(stuck),
        "进攻方控制前场时，禁区里的进攻人开始计时"
    );
    let limit = world.rules.three_second_lane_seconds;
    let mut violation = false;
    for _ in 0..(limit / TICK_SECONDS as f32) as usize + 10 {
        nba_sim_core::lane::step(&mut world, TICK_SECONDS);
        for event in &world.events {
            if matches!(
                &event.fact,
                Fact::Turnover {
                    kind: TurnoverKind::LaneViolation,
                    ..
                }
            ) {
                violation = true;
            }
        }
        if violation {
            break;
        }
    }
    assert!(violation, "进攻人在禁区里待满三秒必须判违例");
}

#[test]
fn an_offensive_player_who_leaves_the_lane_in_time_is_not_a_violation() {
    let (mut world, stuck, _, _) = offense_fixture(311);
    // 计时开始之后他立刻走出去：连续停留的时间不够三秒。
    for _ in 0..20 {
        nba_sim_core::lane::step(&mut world, TICK_SECONDS);
    }
    world.players[stuck].pos = Vec2::new(60.0, 25.0);
    world.players[stuck].target = Vec2::new(60.0, 25.0);
    let limit = world.rules.three_second_lane_seconds;
    let mut violation = false;
    for _ in 0..(limit / TICK_SECONDS as f32) as usize + 20 {
        nba_sim_core::lane::step(&mut world, TICK_SECONDS);
        for event in &world.events {
            if matches!(
                &event.fact,
                Fact::Turnover {
                    kind: TurnoverKind::LaneViolation,
                    ..
                }
            ) {
                violation = true;
            }
        }
    }
    assert!(!violation, "及时走出限制区的进攻人不得被判三秒违例");
    assert_eq!(
        world.players[stuck].lane_seconds, 0.0,
        "走出限制区之后计时归零"
    );
}

#[test]
fn a_defender_who_stays_in_the_lane_without_guarding_is_a_technical_foul() {
    let (mut world, stuck, handler, defender) = offense_fixture(311);
    // 进攻三秒不参与这一条：待测的进攻人退出禁区，进攻三秒因此不计时。
    world.players[stuck].pos = Vec2::new(60.0, 25.0);
    world.players[stuck].target = world.players[stuck].pos;
    // 禁区里有一个进攻人可盯：没有可盯的人时裁判不吹这个。
    let inside = world.on_court(TeamSide::Home).as_slice()[1];
    world.players[inside].pos = Vec2::new(80.0, 30.0);
    world.players[inside].target = world.players[inside].pos;
    // 持球人退出禁区：球在禁区里时两条规则的计时都暂停。
    world.players[handler].pos = Vec2::new(70.0, 40.0);
    world.players[handler].target = world.players[handler].pos;
    world.ball.pos = world.players[handler].pos;
    // 防守人待在禁区里，并且没有在盯防任何人：他离两个进攻人都超过
    // 一臂的距离。
    world.players[defender].pos = Vec2::new(80.0, 18.0);
    world.players[defender].target = world.players[defender].pos;
    let _ = handler;
    assert!(
        in_lane(world.players[defender].pos),
        "夹具必须把防守人放在限制区里"
    );
    assert_eq!(
        nba_sim_core::lane::defense_timer_holder(&world),
        Some(defender),
        "进攻方控制前场时，不盯人的防守人开始计时"
    );
    let limit = world.rules.defensive_three_second_seconds;
    let mut foul = false;
    for _ in 0..(limit / TICK_SECONDS as f32) as usize + 10 {
        nba_sim_core::lane::step(&mut world, TICK_SECONDS);
        for event in &world.events {
            if let Fact::Foul {
                kind: FoulKind::Technical,
                ..
            } = &event.fact
            {
                foul = true;
            }
        }
        if foul {
            break;
        }
    }
    assert!(foul, "防守人在禁区里待满三秒且不盯防必须判技术犯规");
}

#[test]
fn a_defender_who_is_actively_guarding_is_not_called_for_three_seconds() {
    let (mut world, _, handler, defender) = offense_fixture(311);
    // 禁区里有一个进攻人可盯。
    let inside = world.on_court(TeamSide::Home).as_slice()[1];
    world.players[inside].pos = Vec2::new(80.0, 30.0);
    world.players[inside].target = world.players[inside].pos;
    // 防守人就在持球人身边：他在积极盯防（NBA Rule 10 §VII.2）。
    world.players[defender].pos = Vec2::new(70.0, 25.0);
    world.players[defender].target = world.players[defender].pos;
    world.players[handler].pos = Vec2::new(70.0, 25.0);
    world.players[handler].target = world.players[handler].pos;
    world.ball.pos = world.players[handler].pos;
    let _ = inside;
    assert_eq!(
        nba_sim_core::lane::defense_timer_holder(&world),
        None,
        "正在积极盯防的防守人不进入计时"
    );
    let limit = world.rules.defensive_three_second_seconds;
    let mut foul = false;
    for _ in 0..(limit / TICK_SECONDS as f32) as usize + 10 {
        nba_sim_core::lane::step(&mut world, TICK_SECONDS);
        for event in &world.events {
            if let Fact::Foul {
                kind: FoulKind::Technical,
                ..
            } = &event.fact
            {
                foul = true;
            }
        }
    }
    assert!(!foul, "正在积极盯防的防守人不得被判防守三秒违例");
}

#[test]
fn the_three_second_count_does_not_start_before_the_offense_reaches_the_frontcourt() {
    let (mut world, stuck, handler, _) = offense_fixture(311);
    // 持球人退到后场：进攻方还没有控制前场的球，计时不开始。
    world.players[handler].pos = Vec2::new(30.0, 25.0);
    world.players[handler].target = world.players[handler].pos;
    world.ball.pos = world.players[handler].pos;
    assert!(
        !nba_sim_core::lane::offense_controls_frontcourt(&world),
        "球在后场时进攻方没有控制前场"
    );
    assert_eq!(
        nba_sim_core::lane::offense_timer_holder(&world),
        None,
        "球在后场时进攻三秒不计时"
    );
    let limit = world.rules.three_second_lane_seconds;
    for _ in 0..(limit / TICK_SECONDS as f32) as usize + 10 {
        nba_sim_core::lane::step(&mut world, TICK_SECONDS);
    }
    assert!(
        !world.events.iter().any(|event| matches!(
            &event.fact,
            Fact::Turnover {
                kind: TurnoverKind::LaneViolation,
                ..
            }
        )),
        "球在后场时不得判进攻三秒违例"
    );
    assert_eq!(
        world.players[stuck].lane_seconds, 0.0,
        "球在后场时计时不累积"
    );
}
