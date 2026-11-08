//! 暂停：配额、申请门控与恢复。
//!
//! NBA Rule 8 §III 与 Rule 10 §XX；FIBA Article 18 与 Article 29 的
//! 暂停条款。
//!
//! 暂停是受配额约束的资源：整场额度、第四节额度、最后时段额度三层分开
//! 记账。判定落在 `nba_sim_core::timeouts`。

mod common;

use nba_domain::league::LeagueProfile;
use nba_domain::rules::GameRules;
use nba_sim_core::action::Fact;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::Phase;
use nba_sim_core::timeouts::{
    can_request, duration_for, request, request_with_origin, TimeoutOrigin, TimeoutRejection,
};
use nba_sim_core::world::{World, TICK_SECONDS};
use nba_sim_core::{MatchSetup, Params, TeamSide};

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
    world
        .clock
        .set_regulation_clock(4, 600.0, world.clock.shot_clock());
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
    assert_eq!(used, 4, "NBA 第四节最多四次暂停（Rule 8 §III 与联赛档案）");
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
    world
        .clock
        .set_regulation_clock(4, 170.0, world.clock.shot_clock());
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
    assert_eq!(used, 2, "NBA 第四节最后三分钟最多两次暂停");
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
    world
        .clock
        .set_regulation_clock(4, 600.0, world.clock.shot_clock());
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
    assert_eq!(used, 3, "FIBA 第四节最多三次暂停（Article 29.2.4）");
    assert_eq!(
        can_request(&world, team),
        Err(TimeoutRejection::FourthQuarterExhausted),
        "第四节的额度用尽之后必须拒给"
    );
}

#[test]
fn fiba_has_no_final_window_timeout_limit() {
    // FIBA Article 18 没有末段暂停限制：末段窗口里照常可以叫暂停。
    let mut world = fiba_world(819);
    let team = TeamSide::Home;
    assert_eq!(
        world.rules.league.timeouts.timeouts_final_period, 0,
        "FIBA 档案不启用末段暂停限制"
    );
    world
        .clock
        .set_regulation_clock(4, 60.0, world.clock.shot_clock());
    let budget = &mut world.timeouts[World::team_index(team)];
    budget.remaining = 10;
    budget.used_in_fourth = 0;
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
        "FIBA 第四节最后两分钟里暂停不受末段限制，只有第四节的额度在约束"
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

#[test]
fn the_timeout_duration_follows_the_league_profile_and_the_request_origin() {
    // 三种来源分别取联赛档案里的三档时长；NBA 75 / 20 / 60。
    let rules = GameRules::default();
    assert_eq!(duration_for(&rules, 2, TimeoutOrigin::Full), 75.0);
    assert_eq!(duration_for(&rules, 4, TimeoutOrigin::Short), 20.0);
    assert_eq!(duration_for(&rules, 5, TimeoutOrigin::Overtime), 60.0);

    let mut world = live_world(820);
    let team = world.possession.team;
    let before = world.now();
    assert!(
        request_with_origin(&mut world, team, TimeoutOrigin::Short),
        "末段来源的暂停必须获批"
    );
    let short_resume = match world.phase() {
        Phase::Timeout { resume_at, .. } => resume_at,
        other => panic!("暂停之后必须进入暂停程序，实际 {other:?}"),
    };
    assert_eq!(
        short_resume - before,
        duration_for(&world.rules, world.clock.period(), TimeoutOrigin::Short),
        "请求入口必须用档案里的短暂停时长"
    );

    // FIBA 只有完整档，短暂停来源回落到 60 秒。
    let fiba = GameRules::with_league(LeagueProfile::fiba());
    assert_eq!(duration_for(&fiba, 4, TimeoutOrigin::Short), 60.0);
    assert_eq!(duration_for(&fiba, 5, TimeoutOrigin::Overtime), 60.0);
}

#[test]
fn a_live_ball_timeout_uses_the_full_duration() {
    let mut world = live_world(821);
    let team = world.possession.team;
    let before = world.now();
    assert!(request(&mut world, team), "活球持球方可以叫完整暂停");
    let resume = match world.phase() {
        Phase::Timeout { resume_at, .. } => resume_at,
        other => panic!("暂停之后必须进入暂停程序，实际 {other:?}"),
    };
    assert_eq!(
        resume - before,
        duration_for(&world.rules, world.clock.period(), TimeoutOrigin::Full),
        "活球暂停必须占用完整档时长"
    );
}

#[test]
fn the_nba_mandatory_timeout_fires_at_the_first_dead_ball_under_six_fifty_nine() {
    let mut world = nba_world(811);
    let triggers = world.rules.league.timeouts.mandatory_timeouts.clone();
    assert_eq!(triggers.len(), 2, "NBA 档案必须登记两个强制暂停触发点");
    assert_eq!(triggers[0].period, 2);
    assert_eq!(triggers[0].remaining_seconds, 419.0);
    assert_eq!(triggers[0].charge, 2);
    assert!(triggers[0].duration_seconds > 0.0);
    let before = [world.timeouts[0].remaining, world.timeouts[1].remaining];
    let mut charged = None;
    for _ in 0..200_000 {
        world.step(TICK_SECONDS);
        if world.mandatory_timeouts_fired & 1 != 0 {
            charged = world
                .events
                .iter()
                .rev()
                .find_map(|event| match &event.fact {
                    Fact::Timeout {
                        mandatory: true,
                        remaining,
                        ..
                    } => Some(*remaining),
                    _ => None,
                });
            break;
        }
    }
    let remaining = charged.expect("强制暂停必须发出一条带 mandatory 标记的暂停事实");
    assert_eq!(world.clock.period(), 2, "强制暂停发生在第二节");
    assert!(
        world.clock.game_clock() <= 419.0,
        "强制暂停不晚于本节剩 6:59 后的第一次死球"
    );
    assert_eq!(
        remaining[0],
        before[0].saturating_sub(triggers[0].charge),
        "主队暂停额度按触发点记录的次数扣除"
    );
    assert!(
        remaining[1] <= before[1].saturating_sub(triggers[0].charge),
        "客队同样被扣掉触发点记录的次数：{} 从 {} 出发",
        remaining[1],
        before[1]
    );
}

#[test]
fn fiba_has_no_mandatory_timeout() {
    let world = fiba_world(812);
    assert!(
        world.rules.league.timeouts.mandatory_timeouts.is_empty(),
        "FIBA 2026 Article 18 没有强制暂停"
    );
    assert_eq!(
        world.rules.league.timeouts.timeout_durations.full_seconds,
        60.0
    );
    assert_eq!(
        world.rules.league.timeouts.timeout_durations.short_seconds,
        0.0
    );
}
