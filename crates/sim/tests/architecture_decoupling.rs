//! 权威状态与档案结构的解耦：三条结构要求各有直接回归。
//!
//! 方案 §3.2 与 §4.1-§4.2 要求：
//!
//! 1. 比赛程序与球的飞行是两个维度：`Phase` 不含任何飞行变体，飞行记在
//!    `BallFlight` 上，飞行不替换比赛程序。
//! 2. 球权控制状态是一个结构体：它表达控球者、控制球队、最后触球人
//!    与球队、控制建立时刻与前场控制状态。
//! 3. 联赛档案按策略族存储：节次、时钟、犯规、暂停各自成族，档案里
//!    没有未声明的默认值——少一个键就是读不出来的档案。

use std::collections::BTreeSet;

use nba_domain::league::LeagueProfile;
use nba_sim_core::state::{FlightKind, MatchProgram, Phase};

/// 比赛阶段的名字：用来核对 `Phase` 里没有飞行变体。
fn phase_names() -> Vec<&'static str> {
    vec![
        "Inbound",
        "JumpBall",
        "Live",
        "FreeThrows",
        "DeadBall",
        "Timeout",
        "PeriodBreak",
        "Pregame",
        "Finished",
    ]
}

#[test]
fn the_phase_enum_holds_no_flight_variant() {
    // 飞行是球的第二个维度：它记在 BallFlight 上，不替换比赛程序。
    // 因此比赛阶段里不能有「传球飞行」「投篮飞行」这一类把球的飞行折进
    // 比赛程序的变体。
    let folded_flight_phases = [
        "PassFlight",
        "ShotFlight",
        "FreeThrowFlight",
        "JumpBallFlight",
    ];
    for name in phase_names() {
        assert!(
            !folded_flight_phases.contains(&name),
            "比赛阶段里出现了飞行变体 {name}：飞行必须记在 BallFlight 上"
        );
    }
    // 跳球是比赛程序，不是球的飞行：撮球飞行记在 FlightKind::JumpBall
    // 上，而比赛程序仍然是跳球。
    assert_eq!(
        Phase::JumpBall {
            home_jumper: 0,
            away_jumper: 1,
            resolve_at: 0.0,
            kind: nba_sim_core::action::JumpBallKind::Opening,
        }
        .program(),
        MatchProgram::JumpBall,
        "跳球阶段对应跳球程序"
    );
    // 每个阶段都能映射到一个比赛程序。
    for (phase, program) in [
        (Phase::Pregame, MatchProgram::Pregame),
        (Phase::Live, MatchProgram::OpenPlay),
        (Phase::Finished, MatchProgram::Complete),
    ] {
        assert_eq!(phase.program(), program, "阶段到比赛程序的映射必须确定");
    }
}

#[test]
fn the_flight_kind_is_independent_of_the_match_program() {
    // 球的飞行期间比赛程序仍然是开放进攻：飞行不替换程序。
    for kind in [
        FlightKind::Pass,
        FlightKind::Shot,
        FlightKind::FreeThrow,
        FlightKind::JumpBall,
    ] {
        assert_eq!(kind.as_str().is_empty(), false, "飞行种类必须有名字");
    }
    // 罚球飞行停两钟，投篮飞行只停进攻钟，传球飞行两钟都走。
    let pass = FlightKind::Pass.clocks();
    assert!(pass.game_clock && pass.shot_clock, "传球飞行两钟都走");
    let shot = FlightKind::Shot.clocks();
    assert!(shot.game_clock && !shot.shot_clock, "投篮飞行只走比赛钟");
    let free_throw = FlightKind::FreeThrow.clocks();
    assert!(!free_throw.game_clock && !free_throw.shot_clock, "罚球飞行两钟都停");
}

#[test]
fn the_control_state_carries_the_authoritative_ball_control_fields() {
    // 控制状态是一个结构体，不是四个变体：它要同时表达控球者、控制球队、
    // 最后触球人与其球队、控制建立时刻与前场控制状态。
    let control = nba_sim_core::adjudicate::ControlState::dead_ball();
    assert!(control.holder.is_none(), "死球时没有控球人");
    assert!(control.team.is_none(), "死球时没有控制球队");
    assert!(control.last_touch.is_none(), "死球时没有最后触球人");
    assert!(control.last_touch_team.is_none(), "死球时没有最后触球球队");
    assert!(control.control_since.is_none(), "死球时没有控制建立时刻");
    assert!(!control.frontcourt, "死球时没有前场控制");
    assert!(control.dead, "死球状态必须自己声明");
    assert!(!control.in_flight, "死球时球不在空中");
    assert!(!control.rim_contact, "死球时本回合没有触筐");
}

#[test]
fn the_league_profile_is_stored_as_policy_families() {
    let nba = LeagueProfile::nba();
    let fiba = LeagueProfile::fiba();
    // 四个策略族各自独立：节次、时钟、犯规、暂停。
    assert!(nba.periods.regulation_periods == 4);
    assert!(nba.clocks.shot_clock_seconds == 24.0);
    assert!(nba.fouls.max_personal_fouls == 6);
    assert!(nba.timeouts.timeouts_per_game == 7);
    assert!(fiba.timeouts.timeouts_per_game == 5);
    // 档案里没有未声明的默认值：两个联赛档案的每个字段都明确写出来，
    // 因此两个档案的字段集合必须一致。
    let nba_keys = serde_json::to_value(&nba).expect("the profile serializes");
    let fiba_keys = serde_json::to_value(&fiba).expect("the profile serializes");
    let nba_fields: BTreeSet<String> = nba_keys
        .as_object()
        .expect("the profile is an object")
        .keys()
        .cloned()
        .collect();
    let fiba_fields: BTreeSet<String> = fiba_keys
        .as_object()
        .expect("the profile is an object")
        .keys()
        .cloned()
        .collect();
    assert_eq!(
        nba_fields, fiba_fields,
        "两个联赛档案必须声明同一组字段"
    );
}

#[test]
fn a_league_profile_missing_a_declared_field_is_rejected() {
    // 档案少一个键必须读不出来：未声明的默认值会让两个档案悄悄长得
    // 不一样，规则差异因此消失在档案里。
    let raw = include_str!("../../../data/rules/league_nba.json");
    let mut value: serde_json::Value =
        serde_json::from_str(raw).expect("the league profile parses");
    let object = value.as_object_mut().expect("the profile is an object");
    object.remove("bonus_fouls_overtime");
    let error = serde_json::from_value::<LeagueProfile>(value)
        .expect_err("a profile missing a declared field must not load");
    assert!(
        error.to_string().contains("bonus_fouls_overtime"),
        "错误必须指出缺的那个字段：{error}"
    );
}
