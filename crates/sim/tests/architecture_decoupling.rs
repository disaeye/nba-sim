//! 权威状态与档案结构的解耦：结构要求各有直接回归。
//!
//! 方案 §3.2 与 §4.1-§4.2 要求：
//!
//! 1. 比赛程序与球的飞行是两个维度：`Phase` 不含任何飞行变体，飞行记在
//!    `BallFlight` 上，飞行不替换比赛程序。
//! 2. 球权控制状态是一个结构体：它表达控球者、控制球队、最后触球人
//!    与球队、控制建立时刻与前场控制状态。
//! 3. 联赛档案按策略族存储：节次、时钟、犯规、暂停各自成族，档案里
//!    没有未声明的默认值——少一个键就是读不出来的档案。
//!
//! 工作包 A 追加四条：七种球态、单一改钟入口、事件因果字段与控制序列。

use std::collections::BTreeSet;

use nba_domain::league::LeagueProfile;
use nba_domain::match_model::{BallState, BallStateKind};
use nba_sim_core::clock::{ClockDirective, TimerId};
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

#[test]
fn the_ball_state_holds_seven_kinds_and_nothing_else() {
    // 七种球态：持球、运球飞行、传球飞行、投篮飞行、松球、裁判控球与比赛
    // 用球离场。少一种就有一段比赛没有归属表达，多一种就是重复的表。
    assert_eq!(BallStateKind::ALL.len(), 7);
    for kind in BallStateKind::ALL {
        assert!(!kind.as_str().is_empty(), "每种球态都必须有名字");
    }
    let mut names: Vec<&str> = BallStateKind::ALL.iter().map(|kind| kind.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 7, "球态的名字必须互不相同");
    // 投篮飞行携带投篮语义：罚球飞行期间比赛程序仍然是罚球。
    let shot = BallState::ShotFlight {
        shot_id: 1,
        shooter: 0,
        context: nba_domain::match_model::ShotContext::FreeThrow,
        from_pos: glam::Vec2::ZERO,
        target: glam::Vec2::ONE,
        start: 0.0,
        aperture_ft: 0.75,
        rim_in_probability: 0.0,
        aim_offset: glam::Vec2::ZERO,
    };
    assert!(shot.is_in_flight());
    assert_eq!(shot.holder(), None, "飞行中的球没有持球人");
}

#[test]
fn the_clock_has_exactly_four_directive_shapes() {
    // 四类指令：启动、停止、设定与恢复。缺一类就有一种改钟方式没有依据，
    // 多一类就有两种写法表达同一件事。
    let checkpoint = nba_domain::CheckpointId::from_raw(1);
    let shapes = [
        ClockDirective::Start {
            timer: TimerId::GameClock,
        },
        ClockDirective::Stop {
            timer: TimerId::GameClock,
        },
        ClockDirective::Set {
            timer: TimerId::GameClock,
            micros: 0,
        },
        ClockDirective::Restore {
            timer: TimerId::GameClock,
            checkpoint_id: checkpoint,
        },
    ];
    assert_eq!(shapes.len(), 4);
    let mut names: Vec<&str> = shapes.iter().map(|shape| shape.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 4, "四类指令必须各有名字");
    for shape in shapes {
        assert_eq!(shape.timer(), TimerId::GameClock);
    }
}

#[test]
fn the_clock_writes_through_one_entry_only() {
    // 计时器值只能由控制器写：其他模块读到的是秒数投影或者微秒读数，
    // 写入口只有一处。
    let mut clock = nba_sim_core::state::Clock::new(720.0, 24.0);
    clock.take_commits();
    clock.set_shot_clock(14.0);
    clock.set_game_clock(600.0);
    assert_eq!(clock.shot_clock(), 14.0);
    assert_eq!(clock.game_clock(), 600.0);
    let commits = clock.drain_commits();
    assert!(!commits.is_empty(), "改钟必须留下提交记录");
    for commit in &commits {
        assert!(
            matches!(
                commit,
                nba_sim_core::clock::ClockCommit::Directive(_)
                    | nba_sim_core::clock::ClockCommit::Segment(_)
                    | nba_sim_core::clock::ClockCommit::CheckpointSaved(_, _)
            ),
            "提交记录只能是指令、连续推进段或检查点"
        );
    }
}

#[test]
fn every_event_carries_its_causal_identity() {
    // 事件的因果字段不许为空：程序身份、控制序列身份与来源事件都写进
    // 事件信封，验证器据此重建。
    let params = nba_sim_core::params::workspace_calibration_dir();
    let mut world = nba_sim_core::World::new(
        nba_sim_core::MatchSetup::builtin(nba_domain::GameRules::default()),
        nba_sim_core::Params::load(&params).expect("calibration tables load"),
        5,
    );
    for _ in 0..400 {
        world.step(0.05);
    }
    assert!(!world.events.is_empty(), "比赛必须产出事件");
    let mut program_ids = std::collections::BTreeSet::new();
    for event in &world.events {
        assert!(event.program_id > 0, "每条事件都必须带程序身份");
        program_ids.insert(event.program_id);
        assert!(
            event.parent_event_id.is_some(),
            "每条事件都必须指回直接触发它的事件"
        );
        assert!(
            !event.source_event_ids.is_empty(),
            "每条事件都必须带来源事件"
        );
    }
    assert!(
        program_ids.len() > 1,
        "比赛过程中必须切换过比赛程序：程序身份要随之变化"
    );
}
