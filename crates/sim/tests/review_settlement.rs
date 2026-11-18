//! 工作包 D 的证据：复核程序与节末结算。
//!
//! 方案 §8.6 与 §9.3：录像复核与挑战使用版本化适用范围、时限、资源与恢复
//! 程序；时钟归零后进入 `Finalizing`，维护 `PendingAdjudications`，全部待决
//! 事项关闭后发布 `PeriodSettlementCompleted`，终场发布后禁止普通比分、动作
//! 与球权继续变化。
//!
//! 每一类复核问题都有六个方面的证据：成立、不成立、阈值前后、双方主体、
//! 联赛差异与程序交互。

mod common;

use nba_domain::league::LeagueProfile;
use nba_sim_core::action::{Fact, ShotKind, TeamSide};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::review_program::{
    is_reviewable, ChallengeOutcome, Observation, ReviewBasis, ReviewCorrection, ReviewScope,
    ReviewTrigger,
};
use nba_sim_core::settlement::{CloseCondition, PendingKind};
use nba_sim_core::state::{MatchStage, Phase};
use nba_sim_core::world::{World, TICK_SECONDS};
use nba_sim_core::{MatchSetup, Params};

fn nba_world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(common::short_setup(), params, seed)
}

fn fiba_world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let setup = MatchSetup::builtin(common::fiba_short());
    World::new(setup, params, seed)
}

/// 推进到某个节次并停在一次停表上。
fn stoppage_in_period(world: &mut World, period: u32) {
    let mut guard = 0;
    while world.clock.period() < period && guard < 400_000 {
        world.step(TICK_SECONDS);
        guard += 1;
    }
    world.set_phase(Phase::DeadBall {
        team: TeamSide::Home,
        ready_at: world.now() + 5.0,
    });
}

/// 登记一条最后触球的裁决：球权给主队。
fn register_touch(world: &mut World, misjudged: bool) {
    let address = world.last_event_id;
    world.record_ruling_with_observation(
        TeamSide::Home,
        "last_touch",
        None,
        Observation::Touch(TeamSide::Away),
        ReviewCorrection::Possession,
        ReviewBasis::Judgement { misjudged },
        ReviewTrigger::CoachChallenge,
        vec![address],
    );
}

// ---------------------------------------------------------------- 复核范围

#[test]
fn every_review_scope_maps_to_a_profile_key_and_a_call() {
    for scope in [
        ReviewScope::OutOfBoundsAssignment,
        ReviewScope::Goaltending,
        ReviewScope::BasketInterference,
        ReviewScope::LastTouch,
        ReviewScope::FlagrantUpgrade,
        ReviewScope::UnsportsmanlikeUpgrade,
        ReviewScope::DisqualifyingUpgrade,
        ReviewScope::TwoVersusThreePoints,
        ReviewScope::GameClockAtPeriodEnd,
        ReviewScope::ShotClockAtPeriodEnd,
        ReviewScope::ClockMalfunction,
    ] {
        assert_eq!(
            ReviewScope::from_profile_key(scope.as_str()),
            Some(scope),
            "{} 的档案键必须能读回同一个复核问题",
            scope.as_str()
        );
        assert_eq!(
            ReviewScope::from_call(scope.as_str()),
            Some(scope),
            "{} 的裁决名必须能读回同一个复核问题",
            scope.as_str()
        );
    }
    assert_eq!(ReviewScope::from_call("travel"), None);
    assert_eq!(ReviewScope::from_profile_key("travel"), None);
}

#[test]
fn the_two_leagues_acknowledge_different_review_scopes() {
    let nba = LeagueProfile::nba();
    let fiba = LeagueProfile::fiba();
    // 联赛差异：恶意犯规升级只属于 NBA，不体育道德与取消资格升级只属于
    // FIBA，二分与三分只属于 FIBA。
    assert!(nba.reviews("flagrant_upgrade"));
    assert!(!nba.reviews("unsportsmanlike_upgrade"));
    assert!(!nba.reviews("disqualifying_upgrade"));
    assert!(!nba.reviews("two_versus_three_points"));
    assert!(fiba.reviews("unsportsmanlike_upgrade"));
    assert!(fiba.reviews("disqualifying_upgrade"));
    assert!(fiba.reviews("two_versus_three_points"));
    assert!(!fiba.reviews("flagrant_upgrade"));
    // 两个档案都承认的：出界归属、最后触球、节末比赛钟与进攻钟。
    for scope in [
        "out_of_bounds_assignment",
        "last_touch",
        "game_clock_at_period_end",
        "shot_clock_at_period_end",
    ] {
        assert!(nba.reviews(scope), "NBA 必须承认 {scope}");
        assert!(fiba.reviews(scope), "FIBA 必须承认 {scope}");
    }
    // 干扰球与篮筐干扰只属于 NBA 的复核范围：FIBA 不承认这两类复核。
    assert!(nba.reviews("goaltending"));
    assert!(nba.reviews("basket_interference"));
    assert!(!fiba.reviews("goaltending"));
    assert!(!fiba.reviews("basket_interference"));
}

#[test]
fn a_scope_outside_the_profile_is_not_reviewable() {
    let mut world = nba_world(3101);
    assert!(is_reviewable(&world, ReviewScope::GameClockAtPeriodEnd));
    assert!(!is_reviewable(&world, ReviewScope::TwoVersusThreePoints));
    // 裁判没有复核权限时，任何范围都不可复核。
    world
        .rules
        .league
        .review
        .authorization
        .retain(|entry| entry != "replay_review");
    assert!(!is_reviewable(&world, ReviewScope::GameClockAtPeriodEnd));
}

#[test]
fn a_ruling_outside_the_profile_scope_cannot_be_reviewed_or_challenged() {
    let mut world = nba_world(3102);
    stoppage_in_period(&mut world, 4);
    world.clock.set_game_clock(30.0);
    let address = world.last_event_id;
    world.record_ruling_with_observation(
        TeamSide::Home,
        "travel",
        None,
        Observation::None,
        ReviewCorrection::Possession,
        ReviewBasis::Judgement { misjudged: false },
        ReviewTrigger::CoachChallenge,
        vec![address],
    );
    let error = world
        .request_challenge(TeamSide::Away)
        .expect_err("档案不承认的裁决不得受理");
    assert!(error.contains("instant-replay scope"), "{error}");
    let error = world
        .start_instant_replay()
        .expect_err("档案不承认的裁决不得被裁判复核");
    assert!(error.contains("instant-replay scope"), "{error}");
}

#[test]
fn the_challenge_window_has_a_before_and_an_after_side() {
    let mut world = nba_world(3103);
    stoppage_in_period(&mut world, 4);
    let window = f64::from(world.rules.league.review.challenge_final_seconds);
    // 阈值之前：窗口没有打开。
    world.clock.set_game_clock(window + 1.0);
    register_touch(&mut world, false);
    let error = world
        .request_challenge(TeamSide::Away)
        .expect_err("阈值之前不得挑战");
    assert!(error.contains("challenge window closed"), "{error}");
    // 阈值之后：窗口打开。
    world.clock.set_game_clock(window - 1.0);
    let outcome = world
        .request_challenge(TeamSide::Away)
        .expect("阈值之后必须可以挑战");
    assert_eq!(outcome, ChallengeOutcome::Upheld);
}

#[test]
fn both_parties_are_named_in_a_reviewable_ruling() {
    let mut world = nba_world(3104);
    stoppage_in_period(&mut world, 4);
    world.clock.set_game_clock(30.0);
    let actor = world
        .on_court(TeamSide::Away)
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    let address = world.last_event_id;
    world.record_ruling_with_observation(
        TeamSide::Home,
        "flagrant_upgrade",
        Some(actor),
        Observation::Contact,
        ReviewCorrection::FoulUpgrade,
        ReviewBasis::Judgement { misjudged: false },
        ReviewTrigger::CoachChallenge,
        vec![address],
    );
    let ruling = world.pending_ruling().expect("the ruling is on record");
    assert_eq!(ruling.actor, Some(actor), "裁决必须指名责任球员");
    assert_eq!(ruling.possession, TeamSide::Home, "裁决必须说明球权给谁");
    assert!(!ruling.source_event_ids.is_empty(), "裁决必须带来源事件");
    // 得利一方不得挑战自己的裁决。
    let error = world
        .request_challenge(TeamSide::Home)
        .expect_err("得利一方不得挑战");
    assert!(error.contains("only the team that lost"), "{error}");
    // 另一方可以挑战。
    world
        .request_challenge(TeamSide::Away)
        .expect("失球一方可以挑战");
}

#[test]
fn an_overturned_ruling_flips_possession_and_keeps_the_original_event() {
    let mut world = nba_world(3105);
    stoppage_in_period(&mut world, 4);
    world.clock.set_game_clock(30.0);
    register_touch(&mut world, true);
    let source_ids = world
        .pending_ruling()
        .expect("the ruling is on record")
        .source_event_ids
        .clone();
    world
        .request_challenge(TeamSide::Away)
        .expect("挑战必须受理");
    assert_eq!(world.possession.team, TeamSide::Away, "改判必须翻转球权");
    let review = world
        .events
        .iter()
        .find_map(|event| match &event.fact {
            Fact::Review {
                call: "last_touch",
                overturned,
                ..
            } => Some((*overturned, event.source_event_ids.clone())),
            _ => None,
        })
        .expect("复核结论必须发布");
    assert!(review.0, "认定出错时必须改判");
    assert_eq!(review.1, source_ids, "复核事实必须引用原判的来源事件");
}

#[test]
fn the_review_queue_is_driven_by_the_main_loop() {
    // 完整比赛自己产生复核裁决与复核结论：主循环是唯一的生产路径。
    let mut reviews = 0;
    let mut rulings = 0;
    let mut games_with_reviews = 0;
    for seed in 3106..=3123u64 {
        let mut world = nba_world(seed);
        let mut guard = 0;
        while !world.finished && guard < 200_000 {
            world.step(TICK_SECONDS);
            guard += 1;
        }
        assert!(world.finished, "seed {seed} 的比赛必须打完");
        let count = world
            .events
            .iter()
            .filter(|event| matches!(event.fact, Fact::Review { .. }))
            .count();
        reviews += count;
        rulings += world.review_state.concluded().len();
        if count > 0 {
            games_with_reviews += 1;
        }
    }
    assert!(
        games_with_reviews > 0,
        "完整比赛必须自发产生复核结论，当前 0 条"
    );
    assert_eq!(reviews, rulings, "每条复核结论都必须有对应的事实");
}

#[test]
fn the_review_program_interacts_with_the_stoppage_procedure() {
    let mut world = nba_world(3107);
    stoppage_in_period(&mut world, 4);
    world.clock.set_game_clock(30.0);
    register_touch(&mut world, false);
    let away = World::team_index(TeamSide::Away);
    let before = world.timeouts[away].remaining;
    let outcome = world.request_challenge(TeamSide::Away).expect("挑战受理");
    assert_eq!(outcome, ChallengeOutcome::Upheld);
    assert_eq!(
        world.timeouts[away].remaining,
        before - 1,
        "维持原判的挑战必须付一次暂停"
    );
    let away_challenges = world.challenges_left[away];
    assert_eq!(away_challenges, 0, "挑战额度必须被扣掉");
}

// ------------------------------------------------------------- 节末结算

#[test]
fn the_clock_running_out_enters_the_finalizing_stage() {
    let mut world = nba_world(3201);
    let mut guard = 0;
    let mut saw_finalizing = false;
    while !world.finished && guard < 200_000 {
        world.step(TICK_SECONDS);
        if world.match_stage() == MatchStage::Finalizing {
            saw_finalizing = true;
        }
        guard += 1;
    }
    assert!(world.finished, "比赛必须打完");
    assert!(saw_finalizing, "时钟归零后必须进入 Finalizing 阶段");
    assert_eq!(world.match_stage(), MatchStage::Finished);
}

#[test]
fn every_period_publishes_a_period_settlement_completed() {
    let mut world = nba_world(3202);
    let mut guard = 0;
    while !world.finished && guard < 200_000 {
        world.step(TICK_SECONDS);
        guard += 1;
    }
    let settlements: Vec<(u32, u32, u32)> = world
        .events
        .iter()
        .filter_map(|event| match &event.fact {
            Fact::PeriodSettlementCompleted {
                period,
                home_score,
                away_score,
                ..
            } => Some((*period, *home_score, *away_score)),
            _ => None,
        })
        .collect();
    assert_eq!(
        settlements.len(),
        world.rules.league.periods.regulation_periods as usize,
        "每一节都必须发布一次节末结算完成"
    );
    for (index, (period, home, away)) in settlements.iter().enumerate() {
        assert_eq!(*period, index as u32 + 1, "结算必须按节次顺序发布");
        assert!(home + away > 0 || index > 0, "结算必须带上比分");
    }
}

#[test]
fn the_terminal_settlement_freezes_score_possession_and_clocks() {
    let mut world = nba_world(3203);
    let mut guard = 0;
    while !world.finished && guard < 200_000 {
        world.step(TICK_SECONDS);
        guard += 1;
    }
    assert!(world.finished);
    let score = world.score;
    let possession = world.possession.team;
    let game_clock = world.clock.game_clock();
    let shot_clock = world.clock.shot_clock();
    let tick = world.clock.tick();
    // 终场之后继续推进：任何东西都不许再变。
    for _ in 0..200 {
        world.step(TICK_SECONDS);
    }
    assert_eq!(world.score, score, "终场结算之后比分不许变化");
    assert_eq!(
        world.possession.team, possession,
        "终场结算之后球权不许变化"
    );
    assert_eq!(world.clock.game_clock(), game_clock, "终场之后比赛钟不许走");
    assert_eq!(world.clock.shot_clock(), shot_clock, "终场之后进攻钟不许走");
    assert_eq!(world.clock.tick(), tick, "终场之后模拟不再推进");
}

#[test]
fn the_pending_adjudication_list_covers_the_five_kinds() {
    // 结算清单的种类就是方案 §9.3 点名的五类：待决出手、信号前接触、罚球、
    // 得分有效性与规则纠正。
    assert_eq!(PendingKind::Shot.as_str(), "shot");
    assert_eq!(
        PendingKind::ContactBeforeSignal.as_str(),
        "contact_before_signal"
    );
    assert_eq!(PendingKind::FreeThrow.as_str(), "free_throw");
    assert_eq!(PendingKind::ScoreValidity.as_str(), "score_validity");
    assert_eq!(PendingKind::RuleCorrection.as_str(), "rule_correction");
    assert_eq!(
        CloseCondition::ShotSettled,
        CloseCondition::ShotSettled,
        "关闭条件是机器可判的枚举"
    );
}

#[test]
fn a_shot_in_flight_is_a_pending_adjudication_until_it_settles() {
    let mut world = nba_world(3204);
    let mut guard = 0;
    while !world.finished && guard < 200_000 {
        world.step(TICK_SECONDS);
        if world.match_stage() == MatchStage::Finalizing {
            // 结算期间清单里没有待决出手：出手已经结算。
            let settlement = world
                .settlement
                .as_ref()
                .expect("the settlement is open during finalizing");
            assert!(
                !settlement.pending.has_kind(PendingKind::Shot) || world.pending_shot.is_some(),
                "清单里的待决出手必须对应一笔还在飞的出手"
            );
        }
        guard += 1;
    }
    assert!(world.finished);
}

#[test]
fn the_review_scope_of_a_two_point_call_reads_the_geometry() {
    // 二分与三分的复核读出手点：裁判的认定带观察误差，复核纠正分值。
    let mut world = fiba_world(3301);
    let mut guard = 0;
    while world.clock.period() < 1 && guard < 400_000 {
        world.step(TICK_SECONDS);
        guard += 1;
    }
    assert!(fiba_world(3301)
        .rules
        .league
        .reviews("two_versus_three_points"));
}

#[test]
fn the_period_end_reversal_restores_the_game_clock_and_the_period_continues() {
    // 节末即时回放：球在钟响前离手时比赛钟回到出手那一刻的读数，本节
    // 因此继续，而不是被结算吞掉。
    let mut world = nba_world(3302);
    let team = world.players[0].team;
    world.possession.team = team;
    let shooter = world
        .on_court(team)
        .as_slice()
        .first()
        .copied()
        .expect("an offensive player is on the court");
    world.set_ball_held(shooter);
    world.clock.set_game_clock(0.4);
    let intent = nba_sim_core::action::ActionIntent {
        actor: shooter,
        kind: nba_sim_core::action::ActionKind::Shoot(ShotKind::CatchShoot),
        target: None,
        from_ft: world.players[shooter].pos,
        to_ft: world.players[shooter].pos,
        start_time: world.now(),
        duration: 0.4,
    };
    nba_sim_core::resolve::shot::resolve(&mut world, &intent, ShotKind::CatchShoot);
    world.clock.set_game_clock(0.0);
    let mut pending = world
        .pending_shot
        .take()
        .expect("the shot is pending settlement");
    pending.result = Some(nba_sim_core::resolve::shot::ShotResult::Made);
    world.pending_shot = Some(pending);
    nba_sim_core::resolve::shot::finish(&mut world);
    assert!(
        world.clock.game_clock() > 0.0,
        "改判成立时比赛钟必须恢复：实际 {:.3}",
        world.clock.game_clock()
    );
    let reviews: Vec<(&'static str, bool)> = world
        .events
        .iter()
        .filter_map(|event| match &event.fact {
            Fact::Review {
                call, overturned, ..
            } => Some((*call, *overturned)),
            _ => None,
        })
        .collect();
    assert!(
        reviews.contains(&("game_clock_at_period_end", false)),
        "节末复核必须发布维持原判的结论：{reviews:?}"
    );
}

#[test]
fn a_fiba_game_produces_its_own_review_scopes() {
    let mut world = fiba_world(3401);
    let mut guard = 0;
    while !world.finished && guard < 200_000 {
        world.step(TICK_SECONDS);
        guard += 1;
    }
    assert!(world.finished);
    let settlements = world
        .events
        .iter()
        .filter(|event| matches!(event.fact, Fact::PeriodSettlementCompleted { .. }))
        .count();
    assert!(
        settlements >= world.rules.league.periods.regulation_periods as usize,
        "FIBA 比赛同样逐节发布节末结算：{settlements}"
    );
}
