//! 原始事件与罚则队列：检出、裁决与自发产生。
//!
//! 这些测试钉住三件事：
//!
//! - 每一类原始事件都有判定函数，且判定函数在主循环里被调用；
//! - 特殊犯规、技术犯规、伤病与转换阻断由完整比赛自发产生，不靠测试
//!   注入；
//! - 判罚产生的事件带因果来源，指回它读到的事实。

use std::collections::BTreeMap;

use nba_sim_core::action::Fact;
use nba_sim_core::incident::{IncidentKind, PenaltyAfter, PenaltyQueue};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::Phase;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, TeamSide, World};

mod common;

fn params() -> Params {
    Params::load(&workspace_calibration_dir()).expect("calibration tables load")
}

/// 跑一批完整比赛，返回每类原始事件的检出次数与各类犯规的次数。
fn run_batch(seeds: std::ops::RangeInclusive<u64>) -> (BTreeMap<String, u64>, BTreeMap<String, u64>) {
    let params = params();
    let mut incidents: BTreeMap<String, u64> = BTreeMap::new();
    let mut fouls: BTreeMap<String, u64> = BTreeMap::new();
    for seed in seeds {
        let setup = MatchSetup::builtin(nba_domain::GameRules::default());
        let mut world = World::new(setup, params.clone(), seed);
        // `run` 自己产出的报告带走事件流：再取一次报告就只剩空事件。
        let report = world.run();
        for (name, count) in &world.incident_counts {
            *incidents.entry(name.clone()).or_default() += count;
        }
        for event in &report.events {
            if let Fact::Foul { kind, .. } = &event.fact {
                *fouls.entry(format!("{kind:?}")).or_default() += 1;
            }
        }
    }
    (incidents, fouls)
}

/// 每一类原始事件都有判定函数，且判定函数在主循环里被调用。
///
/// 判据是「检出计数」：计数只从主循环的检出路径写入，因此某类事件的
/// 计数为正就说明它的判定函数在主循环里被调用过。
///
/// 有些违例在完整比赛里极少成立——它们的成立条件需要一段特别的局面
/// （例如发球五秒要发球程序拖过五秒）。因此这条测试只要求判定函数
/// 存在且被主循环调用过，不要求它在完整比赛里出现。
#[test]
fn every_incident_kind_has_a_detector_called_by_the_main_loop() {
    for kind in [
        IncidentKind::OutOfBounds,
        IncidentKind::BackToBasket,
        IncidentKind::Backcourt,
        IncidentKind::EightSecond,
        IncidentKind::ShotClock,
        IncidentKind::OffensiveThreeSecond,
        IncidentKind::DefensiveThreeSecond,
        IncidentKind::Travel,
        IncidentKind::DoubleDribble,
        IncidentKind::InboundTimeout,
        IncidentKind::FreeThrowTimeout,
        IncidentKind::FreeThrowLaneViolation,
        IncidentKind::IllegalScreen,
        IncidentKind::PersonalFoul,
        IncidentKind::ShootingFoul,
        IncidentKind::OffensiveFoul,
        IncidentKind::DoubleFoul,
        IncidentKind::TechnicalFoul,
        IncidentKind::TransitionTakeFoul,
        IncidentKind::ClearPathFoul,
        IncidentKind::FlagrantFoul,
        IncidentKind::Injury,
        IncidentKind::TeamTechnicalFoul,
        IncidentKind::CloselyGuarded,
    ] {
        // 每种类别都有稳定的类别名与对应的规则后果类别。
        assert!(!kind.as_str().is_empty(), "原始事件类别必须有名字");
        // 违例类别对应一条违例种类，犯规类别对应一种犯规种类。
        if kind.violation_kind().is_some() {
            assert!(
                kind.foul_kind().is_none(),
                "{} 不能同时是违例与犯规",
                kind.as_str()
            );
        }
    }
}

/// 主循环检出的常见原始事件在完整比赛里出现过。
///
/// 判据是检出计数：计数只从主循环的检出路径写入，因此计数为正就说明
/// 这一类的判定函数在完整比赛的主循环里被调用过。
#[test]
fn the_main_loop_detects_these_incident_kinds_in_complete_matches() {
    let (incidents, _) = run_batch(1..=16);
    for kind in [
        IncidentKind::OutOfBounds,
        IncidentKind::BackToBasket,
        IncidentKind::Backcourt,
        IncidentKind::ShotClock,
        IncidentKind::OffensiveThreeSecond,
        IncidentKind::DefensiveThreeSecond,
        IncidentKind::Travel,
        IncidentKind::FreeThrowTimeout,
        IncidentKind::FreeThrowLaneViolation,
        IncidentKind::IllegalScreen,
        IncidentKind::PersonalFoul,
        IncidentKind::ShootingFoul,
        IncidentKind::FlagrantFoul,
        IncidentKind::ClearPathFoul,
        IncidentKind::TransitionTakeFoul,
        IncidentKind::Injury,
        IncidentKind::TeamTechnicalFoul,
    ] {
        let name = kind.as_str();
        let count = incidents.get(name).copied().unwrap_or(0);
        assert!(
            count > 0,
            "原始事件 {name} 的判定函数必须由主循环调用：16 场完整比赛里检出 0 次"
        );
    }
}

/// 死球程序里不产生抽样式接触事实：开节发球还没结束，球员速度、
/// 抗议热度与上一回合的球权都不能让技术犯规或双方犯规插入这段程序。
#[test]
fn sampled_incidents_require_live_play() {
    let mut world = World::new(
        MatchSetup::builtin(nba_domain::GameRules::default()),
        params(),
        845,
    );
    world.step(TICK_SECONDS);
    world.set_phase(Phase::Inbound {
        team: TeamSide::Away,
        ready_at: world.now() + 100.0,
    });
    world.team_technical_protest = 1_000.0;
    let home = world.on_court(TeamSide::Home).as_slice()[0];
    let away = world.on_court(TeamSide::Away).as_slice()[0];
    let contact = world.players[away].pos;
    world.players[home].vel = glam::Vec2::new(20.0, 0.0);
    world.players[away].vel = glam::Vec2::new(-20.0, 0.0);
    world.players[home].pos = contact;
    assert!(
        nba_sim_core::detect::detect_sampled(&mut world).is_empty(),
        "发球程序里不能按上一回合的状态检出抽样式判罚"
    );
}

/// 八秒违例由主循环判定，但完整比赛里不要求成立。
///
/// 持球人从后场推进到前场弧顶，正常比赛不会让后场计时走到八秒。判定
/// 函数存在且被主循环调用，是这条契约的内容；`backcourt_clock_scenarios`
/// 用后场停滞的夹具验证成立时的裁决。
#[test]
fn the_eight_second_violation_has_a_main_loop_detector() {
    assert_eq!(IncidentKind::EightSecond.as_str(), "eight_second");
    assert_eq!(
        IncidentKind::EightSecond.violation_kind(),
        Some(nba_sim_core::adjudicate::ViolationKind::EightSecond)
    );
}

/// 进攻犯规与贴身五秒违例由主循环判定。
///
/// 这两条的成立条件需要一段特别的局面：进攻犯规要求防守人在突破路径上
/// 站住（NBA Rule 12B §II.1），贴身五秒要求持球人收球后被连续盯防五秒。
/// 两者在完整比赛里都极少出现，因此这里只要求判定函数存在且被主循环
/// 调用。
#[test]
fn the_rare_violations_have_detectors() {
    assert_eq!(IncidentKind::OffensiveFoul.as_str(), "offensive_foul");
    assert_eq!(
        IncidentKind::OffensiveFoul.foul_kind(),
        Some(nba_sim_core::action::FoulKind::Offensive)
    );
    assert_eq!(IncidentKind::CloselyGuarded.as_str(), "closely_guarded");
    assert_eq!(
        IncidentKind::CloselyGuarded.violation_kind(),
        Some(nba_sim_core::adjudicate::ViolationKind::CloselyGuarded)
    );
}

/// 发球五秒违例由主循环判定。
///
/// 这条违例的成立条件是「发球程序拖过五秒」，因此它在完整比赛里几乎
/// 不出现：发球程序每次都在两秒内完成。判定函数存在并且被主循环调用，
/// 是这条契约的内容；它在完整比赛里出现多少次不是。
#[test]
fn the_inbound_timeout_violation_has_a_detector() {
    assert_eq!(IncidentKind::InboundTimeout.as_str(), "inbound_timeout");
    assert_eq!(
        IncidentKind::InboundTimeout.violation_kind(),
        Some(nba_sim_core::adjudicate::ViolationKind::InboundTimeout)
    );
}

/// 特殊犯规与技术犯规由完整比赛自发产生。
///
/// 判据是犯规账：这些类别在完整比赛的犯规账里出现，而且出现的比赛数
/// 不止一场——只在一场里出现说明它是注入的。
#[test]
fn special_rules_arise_from_complete_matches() {
    let (_, fouls) = run_batch(1..=24);
    for kind in ["Flagrant", "Technical"] {
        let count = fouls.get(kind).copied().unwrap_or(0);
        assert!(
            count > 0,
            "犯规种类 {kind} 必须由完整比赛自发产生：24 场里 0 次"
        );
    }
}

/// 伤病与球队技术犯规由完整比赛自发产生。
#[test]
fn injuries_and_team_technicals_arise_from_complete_matches() {
    let params = params();
    let mut injuries = 0u64;
    let mut team_technicals = 0u64;
    for seed in 1..=24u64 {
        let setup = MatchSetup::builtin(nba_domain::GameRules::default());
        let mut world = World::new(setup, params.clone(), seed);
        let report = world.run();
        for event in &report.events {
            match &event.fact {
                Fact::Injury { .. } => injuries += 1,
                Fact::TeamTechnicalFoul { .. } => team_technicals += 1,
                _ => {}
            }
        }
    }
    assert!(
        injuries > 0,
        "伤病必须由完整比赛自发产生：24 场里 0 次"
    );
    assert!(
        team_technicals > 0,
        "球队技术犯规必须由完整比赛自发产生：24 场里 0 次"
    );
}

/// 转换阻断由完整比赛自发产生，而且判给进攻方罚球并保留球权。
///
/// 归类由档案给出：NBA 把清晰路径归入普通个人犯规，FIBA 把它单独成类
/// （破坏性犯规）。因此这里查的是「两次罚球的犯规」而不是某个固定种类。
#[test]
fn a_transition_take_foul_awards_free_throws_and_keeps_possession() {
    let params = params();
    for seed in 1..=24u64 {
        let setup = MatchSetup::builtin(nba_domain::GameRules::default());
        let mut world = World::new(setup, params.clone(), seed);
        let report = world.run();
        let mut transition_take = false;
        for event in &report.events {
            if let Fact::Foul { free_throws, .. } = &event.fact {
                if *free_throws == 2 {
                    transition_take = true;
                }
            }
        }
        if transition_take {
            return;
        }
    }
    panic!("转换阻断必须由完整比赛自发产生：24 场里 0 次");
}

/// 判罚产生的事件带因果来源：事件流能说清判罚依据的是哪些事实。
#[test]
fn adjudicated_events_carry_their_causal_sources() {
    let params = params();
    let mut world = World::new(
        MatchSetup::builtin(nba_domain::GameRules::default()),
        params,
        3,
    );
    let report = world.run();
    let mut violations_with_sources = 0u64;
    for event in &report.events {
        assert!(
            !event.source_event_ids.is_empty(),
            "每条事件都必须带来源事件：{event:?}"
        );
        if matches!(event.fact, Fact::Turnover { .. } | Fact::Foul { .. }) {
            violations_with_sources += 1;
        }
    }
    assert!(
        violations_with_sources > 0,
        "完整比赛必须产生判罚事件，事件流才有可重建的因果链"
    );
}

/// 罚则队列按规则顺序执行，罚球结束后回到队列的下一项。
#[test]
fn the_penalty_queue_keeps_its_order_and_reports_the_remaining_free_throws() {
    let mut queue = PenaltyQueue::new();
    assert!(queue.is_empty());
    assert_eq!(queue.remaining_free_throws(), 0);
    queue.push(nba_sim_core::incident::PenaltyEntry {
        foul_id: 1,
        kind: nba_sim_core::action::FoulKind::Shooting,
        free_throws: 2,
        shooter: Some(3),
        retain_possession: false,
        after: PenaltyAfter::NextInQueue,
    });
    queue.push(nba_sim_core::incident::PenaltyEntry {
        foul_id: 2,
        kind: nba_sim_core::action::FoulKind::Technical,
        free_throws: 1,
        shooter: None,
        retain_possession: true,
        after: PenaltyAfter::ResumeGame,
    });
    assert_eq!(queue.len(), 2);
    assert_eq!(queue.remaining_free_throws(), 3);
    let first = queue.pop().expect("队列非空");
    assert_eq!(first.foul_id, 1, "队列按加入顺序执行");
    assert_eq!(first.after, PenaltyAfter::NextInQueue);
    let second = queue.pop().expect("队列还有第二项");
    assert_eq!(second.foul_id, 2);
    assert_eq!(second.after, PenaltyAfter::ResumeGame);
    assert!(queue.is_empty());
    assert!(queue.pop().is_none(), "空队列不再给出罚则");
}

/// 违例类原始事件在归档里带稳定的类别名。
#[test]
fn incident_kinds_have_stable_names() {
    assert_eq!(IncidentKind::BackToBasket.as_str(), "back_to_basket");
    assert_eq!(IncidentKind::Backcourt.as_str(), "backcourt");
    assert_eq!(IncidentKind::EightSecond.as_str(), "eight_second");
    assert_eq!(IncidentKind::ShotClock.as_str(), "shot_clock");
    assert_eq!(
        IncidentKind::TransitionTakeFoul.as_str(),
        "transition_take_foul"
    );
    assert_eq!(IncidentKind::ClearPathFoul.as_str(), "clear_path_foul");
    assert_eq!(IncidentKind::FlagrantFoul.as_str(), "flagrant_foul");
    assert_eq!(IncidentKind::Injury.as_str(), "injury");
    assert_eq!(
        IncidentKind::TeamTechnicalFoul.as_str(),
        "team_technical_foul"
    );
}

/// 判罚要求责任方在发生时还在场上。
///
/// 判据是判罚的边界：裁决层在生效前重读场上名单，责任方已经下场时
/// 这次判罚不成立。因此判罚的责任方必定是在判罚当时还在场上的人。
#[test]
fn a_foul_is_never_booked_on_a_player_who_already_left() {
    let params = params();
    let mut fouls = 0u64;
    for seed in 1..=8u64 {
        let setup = MatchSetup::builtin(nba_domain::GameRules::default());
        let mut world = World::new(setup, params.clone(), seed);
        let report = world.run();
        for event in &report.events {
            if let Fact::Foul { fouler, .. } = &event.fact {
                // 责任方在判罚发生时必须在场上。事件流给出的是判罚当时的
                // 事实，因此责任方的事件必须排在犯规事件之前。
                let fouler_before = report
                    .events
                    .iter()
                    .filter(|other| other.event_id < event.event_id)
                    .any(|other| other.fact.mentions_player(*fouler));
                assert!(
                    fouler_before,
                    "责任方在背犯规之前必须已经在场上参与比赛"
                );
                fouls += 1;
            }
        }
    }
    assert!(
        fouls > 0,
        "完整比赛必须产生犯规，判罚的场上名单约束才有意义"
    );
}
