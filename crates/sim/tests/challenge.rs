//! 录像复核与教练挑战：窗口、额度与代价。

use nba_domain::rules::GameRules;
use nba_sim_core::action::{Fact, TeamSide};
use nba_sim_core::challenge::ChallengeOutcome;
use nba_sim_core::state::Phase;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, World};

mod common;

fn late_world(seed: u64) -> World {
    let mut world = common::short_world(seed);
    let mut guard = 0;
    while world.clock.period() < 4 && guard < 400_000 {
        world.step(TICK_SECONDS);
        guard += 1;
    }
    world.clock.set_game_clock(30.0);
    world.set_phase(Phase::DeadBall {
        team: TeamSide::Home,
        ready_at: world.now() + 5.0,
    });
    world
}

#[test]
fn a_failed_challenge_keeps_the_ruling_and_costs_a_timeout() {
    let mut world = late_world(401);
    world.record_ruling(TeamSide::Home, "out_of_bounds");
    let away = World::team_index(TeamSide::Away);
    let before = world.timeouts[away].remaining;
    let outcome = world
        .request_challenge(TeamSide::Away)
        .expect("末节最后两分钟必须可以挑战");
    assert_eq!(outcome, ChallengeOutcome::Upheld, "复核结论是维持原判");
    assert_eq!(
        world.timeouts[away].remaining,
        before - 1,
        "挑战失败必须损失一次暂停"
    );
    assert!(
        world.events.iter().any(|event| matches!(
            &event.fact,
            Fact::Review {
                overturned: false,
                cost_timeout: true,
                ..
            }
        )),
        "复核结论必须记录在事件里"
    );
}

#[test]
fn a_second_challenge_is_rejected() {
    let mut world = late_world(403);
    world.record_ruling(TeamSide::Home, "out_of_bounds");
    world
        .request_challenge(TeamSide::Away)
        .expect("第一次挑战成立");
    let error = world
        .request_challenge(TeamSide::Away)
        .expect_err("额度用完之后必须拒绝");
    assert!(error.contains("no challenge remaining"), "{error}");
}

#[test]
fn a_challenge_outside_the_window_is_rejected() {
    let mut world = common::short_world(407);
    world.set_phase(Phase::DeadBall {
        team: TeamSide::Home,
        ready_at: world.now() + 5.0,
    });
    world.record_ruling(TeamSide::Home, "out_of_bounds");
    let error = world
        .request_challenge(TeamSide::Away)
        .expect_err("前段比赛不得挑战");
    assert!(error.contains("challenge window closed"), "{error}");
}

#[test]
fn the_team_that_won_the_ruling_cannot_challenge_it() {
    let mut world = late_world(409);
    world.record_ruling(TeamSide::Home, "out_of_bounds");
    let error = world
        .request_challenge(TeamSide::Home)
        .expect_err("得利一方不得挑战自己的裁决");
    assert!(error.contains("only the team that lost"), "{error}");
}

#[test]
fn a_ruling_free_stoppage_has_nothing_to_review() {
    let mut world = late_world(411);
    // 这个场景只要一条「没有裁决可复核」的停表：把上一回合留下的
    // 可复核裁定清掉。
    world.last_ruling = None;
    let error = world
        .request_challenge(TeamSide::Away)
        .expect_err("没有裁决记录时不得挑战");
    assert!(error.contains("no reviewable ruling"), "{error}");
}

#[test]
fn the_ruleset_defaults_carry_one_challenge() {
    let rules = GameRules::default();
    assert_eq!(rules.league.challenges_per_game, 1);
    assert!(rules.league.challenge_final_seconds > 0.0);
    let setup = MatchSetup::builtin(rules);
    assert!(setup.validate().is_ok());
}

#[test]
fn a_review_leaves_the_possession_and_phase_untouched() {
    let mut world = late_world(413);
    world.record_ruling(TeamSide::Home, "out_of_bounds");
    let team_before = world.possession.team;
    let phase_before = world.phase();
    world
        .request_challenge(TeamSide::Away)
        .expect("挑战必须成立");
    assert_eq!(world.possession.team, team_before, "复核不改变球权");
    assert_eq!(world.phase(), phase_before, "复核不改变阶段");
}

#[test]
fn a_ruling_outside_the_replay_scope_cannot_be_challenged() {
    let mut world = late_world(417);
    world.record_ruling(TeamSide::Home, "back_to_basket");
    let error = world
        .request_challenge(TeamSide::Away)
        .expect_err("复核范围之外的裁决不得受理");
    assert!(error.contains("instant-replay scope"), "{error}");
}

#[test]
fn a_successful_challenge_overturns_ruling_and_retains_timeout() {
    let mut world = late_world(415);
    world.record_ruling_with_evidence(TeamSide::Home, "out_of_bounds", true);
    let away = World::team_index(TeamSide::Away);
    let before_timeouts = world.timeouts[away].remaining;
    let outcome = world
        .request_challenge(TeamSide::Away)
        .expect("末节最后两分钟必须可以挑战");
    assert_eq!(outcome, ChallengeOutcome::Overturned, "复核结论是原判推翻");
    assert_eq!(
        world.timeouts[away].remaining, before_timeouts,
        "挑战成功不得扣除暂停"
    );
    assert_eq!(
        world.possession.team,
        TeamSide::Away,
        "挑战成功必须翻转球权"
    );
    assert!(
        world.events.iter().any(|event| matches!(
            &event.fact,
            Fact::Review {
                overturned: true,
                cost_timeout: false,
                ..
            }
        )),
        "改判事实必须记录在事件里"
    );
}
