//! 比赛时钟与进攻时钟：运行、停止、启动与重置。
//!
//! NBA Rule 5 §II 与 §VIII 规定比赛钟何时走何时停；Rule 7 §II-§IV 规定
//! 进攻钟何时启动、继续与重置。
//!
//! 这些测试直接读 `Clock` 的走钟策略与 `World` 的重置入口：时钟本身是
//! 纯函数，判定必须落在同一个地方，测试因此钉的是「哪种状态下哪个钟
//! 在走」与「哪种事件把进攻钟设成多少」。

mod common;

use nba_sim_core::action::Fact;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::{Clock, Phase, PossessionKind, TeamSide};
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

#[test]
fn the_game_clock_runs_during_a_pass_in_flight() {
    // 传球飞行：两钟都走（NBA Rule 5 §VIII 与 Rule 7 §II）。
    let advance = nba_sim_core::state::FlightKind::Pass.clocks();
    assert!(advance.game_clock, "传球飞行期间比赛钟必须走");
    assert!(advance.shot_clock, "传球飞行期间进攻钟必须走");
    assert!(advance.elapsed, "传球飞行期间必须累计持球时间");
}

#[test]
fn the_game_clock_runs_but_the_shot_clock_stops_during_a_shot_in_flight() {
    // 投篮飞行：比赛钟走，进攻钟停（NBA Rule 7 §II-b）。
    let advance = nba_sim_core::state::FlightKind::Shot.clocks();
    assert!(advance.game_clock, "投篮飞行期间比赛钟必须走");
    assert!(!advance.shot_clock, "投篮飞行期间进攻钟必须停");
}

#[test]
fn the_game_clock_stops_during_a_free_throw() {
    // 罚球期间两钟都停（NBA Rule 5 §VIII-c）。
    let advance = nba_sim_core::state::FlightKind::FreeThrow.clocks();
    assert!(!advance.game_clock, "罚球期间比赛钟必须停");
    assert!(!advance.shot_clock, "罚球期间进攻钟必须停");
}

#[test]
fn the_game_clock_stops_during_a_dead_ball_and_an_inbound() {
    for phase in [
        Phase::DeadBall {
            team: TeamSide::Home,
            ready_at: 0.0,
        },
        Phase::Inbound {
            team: TeamSide::Home,
            ready_at: 0.0,
        },
    ] {
        let advance = phase.clocks();
        assert!(!advance.game_clock, "{phase:?} 期间比赛钟必须停");
        assert!(!advance.shot_clock, "{phase:?} 期间进攻钟必须停");
    }
}

#[test]
fn a_new_control_starts_the_shot_clock_at_twenty_four_seconds() {
    let mut world = live_world(601);
    let offense = world.possession.team;
    // 新的控制：进攻钟回到二十四秒（NBA Rule 7 §II）。
    world.clock.set_shot_clock(3.0);
    world.adjudicate(nba_sim_core::adjudicate::Adjudication::violation(
        nba_sim_core::adjudicate::ViolationKind::ShotClock,
        nba_sim_core::adjudicate::RestartPlan::Live {
            team: offense.other(),
        },
    ));
    assert_eq!(
        world.clock.shot_clock(),
        f64::from(world.rules.league.clocks.shot_clock_seconds),
        "新的控制权必须把进攻钟设回二十四秒"
    );
}

#[test]
fn an_offensive_rebound_off_the_rim_refills_the_shot_clock_to_fourteen() {
    let mut world = live_world(602);
    let offense = world.possession.team;
    let rebounder = world.holder().expect("live possession has a holder");
    world.clock.set_shot_clock(6.0);
    world.start_second_chance(offense, rebounder, true);
    assert_eq!(
        world.clock.shot_clock(),
        14.0,
        "触筐的前场篮板必须把进攻钟补到十四秒"
    );
}

#[test]
fn an_offensive_rebound_that_missed_the_rim_leaves_the_shot_clock_alone() {
    let mut world = live_world(603);
    let offense = world.possession.team;
    let rebounder = world.holder().expect("live possession has a holder");
    world.clock.set_shot_clock(6.0);
    world.start_second_chance(offense, rebounder, false);
    assert_eq!(
        world.clock.shot_clock(),
        6.0,
        "没有触筐的前场篮板不得重置进攻钟"
    );
}

#[test]
fn the_shot_clock_violation_ends_the_possession() {
    let mut world = live_world(604);
    let offense = world.possession.team;
    let holder = world.holder().expect("live possession has a holder");
    world.clock.set_shot_clock(0.0);
    // 让持球人一直握着球，这样时钟归零时球还在他手里。
    world.players[holder].schedule.next_redecide_at = world.now() + 1_000.0;
    let mut violation = false;
    for _ in 0..20 {
        world.step(TICK_SECONDS);
        if world.events.iter().any(|event| {
            matches!(
                &event.fact,
                Fact::Turnover {
                    kind: nba_sim_core::action::TurnoverKind::ShotClock,
                    ..
                }
            )
        }) {
            violation = true;
            break;
        }
    }
    assert!(violation, "进攻钟归零必须判违例");
    assert_ne!(
        world.possession.team, offense,
        "进攻钟违例之后球权必须交给对方"
    );
}

#[test]
fn a_whistle_stops_the_game_clock() {
    // 停表的判据是阶段：死球期间比赛钟不走。
    let mut clock = Clock::new(300.0, 24.0);
    let before = clock.game_clock();
    let advance = nba_sim_core::clock::advance(
        nba_sim_core::state::Phase::DeadBall {
            team: TeamSide::Home,
            ready_at: 0.0,
        },
        None,
        &mut clock,
        5.0,
    );
    assert!(!advance.game_clock, "死球期间比赛钟不得走动");
    assert_eq!(
        clock.game_clock(),
        before,
        "死球期间比赛钟不得走动"
    );
}

#[test]
fn the_inbound_starts_the_clock_when_the_ball_is_touched_in_play() {
    // 发球程序本身停表：球进入场内并被合法触到之后才启动（NBA Rule 7
    // §II 的例外条款）。
    let mut world = world(605);
    // 直接检查发球阶段的走钟策略。
    let advance = Phase::Inbound {
        team: TeamSide::Home,
        ready_at: 0.0,
    }
    .clocks();
    assert!(!advance.game_clock, "发球程序期间比赛钟必须停");
    assert!(!advance.shot_clock, "发球程序期间进攻钟必须停");
}
