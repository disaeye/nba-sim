//! 暂停：配额、申请门控与恢复。
//!
//! NBA Rule 8 §III 与 Rule 10 §XX；FIBA Article 18 与 Article 29 的
//! 暂停条款。
//!
//! 暂停是受配额约束的资源：整场额度、第四节额度、最后时段额度三层分开
//! 记账。判定落在 `nba_sim_core::timeouts`。

mod common;

use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::Phase;
use nba_sim_core::timeouts::{can_request, request, TimeoutRejection};
use nba_sim_core::world::{World, TICK_SECONDS};
use nba_sim_core::{MatchSetup, Params, TeamSide};
use nba_domain::league::LeagueProfile;
use nba_domain::rules::GameRules;

fn world_with_profile(seed: u64, league: LeagueProfile) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut rules = GameRules::default();
    rules.league = league;
    World::new(MatchSetup::builtin(rules), params, seed)
}

fn nba_world(seed: u64) -> World {
    world_with_profile(seed, LeagueProfile::nba())
}

fn fiba_world(seed: u64) -> World {
    world_with_profile(seed, LeagueProfile::fiba())
}

/// 一个活球有人持球的世界。
fn live_world(seed: u64) -> World {
    let mut world = nba_world(seed);
    for _ in 0..40_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::Live) && world.holder().is_some() {
            return world;
        }
    }
    panic!("the engine never reached a live ball state");
}

#[test]
fn a_timeout_costs_one_from_the_game_budget() {
    let mut world = live_world(801);
    let team = world.possession.team;
    let remaining_before = world.timeouts[World::team_index(team)].remaining;
    assert!(request(&mut world, team), "持球方可以叫暂停");
    assert_eq!(
        world.timeouts[World::team_index(team)].remaining,
        remaining_before - 1,
        "暂停必须从整场额度里扣一次"
    );
    assert!(
        matches!(world.phase(), Phase::Timeout { .. }),
        "暂停之后比赛进入暂停程序"
    );
}

#[test]
fn the_opponent_cannot_call_a_timeout_while_the_ball_is_live() {
    let mut world = live_world(802);
    let holder_team = world.possession.team;
    let other = holder_team.other();
    assert_eq!(
        can_request(&world, other),
        Err(TimeoutRejection::NoPossession),
        "活球时对方不能叫暂停"
    );
    assert!(!request(&mut world, other), "对方叫暂停必须被拒");
}

#[test]
fn either_team_can_call_a_timeout_during_a_dead_ball() {
    let mut world = nba_world(803);
    // 制造一个死球：停在发球程序。
    world.set_phase(Phase::DeadBall {
        team: TeamSide::Home,
        ready_at: world.now() + 60.0,
    });
    assert!(
        can_request(&world, TeamSide::Home).is_ok(),
        "死球时主队可以叫暂停"
    );
    assert!(
        can_request(&world, TeamSide::Away).is_ok(),
        "死球时客队也可以叫暂停"
    );
}

#[test]
fn the_nba_fourth_quarter_budget_is_capped_at_four() {
    let mut world = live_world(804);
    let team = world.possession.team;
    // 跳到第四节。
    world.clock.set_regulation_clock(4, 600.0, world.clock.shot_clock());
    // 把整场额度用掉，只留第四节的额度。
    let budget = &mut world.timeouts[World::team_index(team)];
    budget.remaining = 10;
    let mut used = 0;
    for _ in 0..6 {
        world.set_phase(Phase::DeadBall {
            team,
            ready_at: world.now() + 60.0,
        });
        if request(&mut world, team) {
            used += 1;
            // 暂停程序期间把阶段放回死球，才能继续叫下一次。
            world.set_phase(Phase::DeadBall {
                team,
                ready_at: world.now() + 60.0,
            });
        } else {
            break;
        }
    }
    assert_eq!(
        used, 4,
        "NBA 第四节最多四次暂停（Rule 8 §III 与联赛档案）"
    );
    assert_eq!(
        can_request(&world, team),
        Err(TimeoutRejection::FourthQuarterExhausted),
        "第四节的额度用尽之后必须拒给"
    );
}

#[test]
fn the_nba_final_window_budget_is_capped_at_two() {
    let mut world = live_world(805);
    let team = world.possession.team;
    // 第四节最后三分钟：末段窗口生效。
    world.clock.set_regulation_clock(4, 170.0, world.clock.shot_clock());
    let budget = &mut world.timeouts[World::team_index(team)];
    budget.remaining = 10;
    budget.used_in_fourth = 0;
    let mut used = 0;
    for _ in 0..5 {
        world.set_phase(Phase::DeadBall {
            team,
            ready_at: world.now() + 60.0,
        });
        if request(&mut world, team) {
            used += 1;
            world.set_phase(Phase::DeadBall {
                team,
                ready_at: world.now() + 60.0,
            });
        } else {
            break;
        }
    }
    assert_eq!(
        used, 2,
        "NBA 第四节最后三分钟最多两次暂停"
    );
    assert_eq!(
        can_request(&world, team),
        Err(TimeoutRejection::FinalWindowExhausted),
        "末段的额度用尽之后必须拒给"
    );
}

#[test]
fn the_fiba_fourth_quarter_budget_is_capped_at_three() {
    let mut world = fiba_world(806);
    let team = TeamSide::Home;
    world.clock.set_regulation_clock(4, 600.0, world.clock.shot_clock());
    let budget = &mut world.timeouts[World::team_index(team)];
    budget.remaining = 10;
    let mut used = 0;
    for _ in 0..6 {
        world.set_phase(Phase::DeadBall {
            team,
            ready_at: world.now() + 60.0,
        });
        if request(&mut world, team) {
            used += 1;
            world.set_phase(Phase::DeadBall {
                team,
                ready_at: world.now() + 60.0,
            });
        } else {
            break;
        }
    }
    assert_eq!(
        used, 3,
        "FIBA 第四节最多三次暂停（Article 29.2.4）"
    );
    assert_eq!(
        can_request(&world, team),
        Err(TimeoutRejection::FourthQuarterExhausted),
        "第四节的额度用尽之后必须拒给"
    );
}

#[test]
fn the_fiba_game_budget_is_five() {
    let mut world = fiba_world(807);
    let team = TeamSide::Home;
    assert_eq!(
        world.rules.league.timeouts.timeouts_per_game, 5,
        "FIBA 每场五次暂停（Article 29.2.4）"
    );
    let budget = &mut world.timeouts[World::team_index(team)];
    budget.remaining = 5;
    let mut used = 0;
    for _ in 0..8 {
        world.set_phase(Phase::DeadBall {
            team,
            ready_at: world.now() + 60.0,
        });
        if request(&mut world, team) {
            used += 1;
            world.set_phase(Phase::DeadBall {
                team,
                ready_at: world.now() + 60.0,
            });
        } else {
            break;
        }
    }
    assert_eq!(used, 5, "FIBA 每场五次暂停");
    assert_eq!(
        can_request(&world, team),
        Err(TimeoutRejection::NoRemaining),
        "整场额度用尽之后必须拒给"
    );
}

#[test]
fn the_overtime_budget_is_separate_from_the_game_budget() {
    let mut world = nba_world(808);
    let team = TeamSide::Home;
    // 加时节的额度写在档案里：NBA 每次加时两次。
    assert_eq!(world.rules.league.timeouts.overtime_timeouts, 2);
    assert_eq!(
        world.rules.league.timeouts.timeouts_per_game, 7,
        "加时额度不改变整场额度"
    );
}

#[test]
fn a_timeout_resumes_play_with_the_ball() {
    let mut world = live_world(809);
    let team = world.possession.team;
    assert!(request(&mut world, team), "持球方可以叫暂停");
    // 暂停程序结束时比赛回到中断时的状态。
    let resume_at = match world.phase() {
        Phase::Timeout { resume_at, .. } => resume_at,
        other => panic!("暂停之后必须进入暂停程序，实际 {other:?}"),
    };
    assert!(resume_at > world.now(), "暂停必须有一个未来的恢复时刻");
    // 推进到恢复时刻之后。
    for _ in 0..2_000 {
        world.step(TICK_SECONDS);
        if !matches!(world.phase(), Phase::Timeout { .. }) {
            break;
        }
    }
    assert!(
        !matches!(world.phase(), Phase::Timeout { .. }),
        "暂停结束之后比赛必须恢复"
    );
}
