//! 赛前角色与行为倾向的一致性守卫。
//!
//! ADR-020 要求引擎对角色字段零读取：角色的行为含义由倾向字段承载，
//! 代码不按角色枚举分支；角色与行为的一致性由产出判据保证
//! （`attributes.md` §2.7b、`decisions.md`）。这里把两层判据都写死：
//! 名册里每一类角色的最低行为要求，以及 ADR-020 给出的投射区域判据，
//! 让「角色写了什么，倾向就必须支持什么」成为一条可自动核对的约束。

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
struct Roster {
    team: String,
    players: Vec<Player>,
}

#[derive(Debug, Deserialize)]
struct Player {
    name: String,
    offensive_role: String,
    defensive_role: String,
    tendencies: Tendencies,
}

#[derive(Debug, Deserialize)]
struct Tendencies {
    shoot_frequency: f32,
    three_point_preference: f32,
    drive_frequency: f32,
    pass_frequency: f32,
    cut_frequency: f32,
    screen_frequency: f32,
    offensive_rebound_frequency: f32,
    gamble_steal: f32,
    block_aggressiveness: f32,
    help_aggressiveness: f32,
    physicality: f32,
}

/// 一侧倾向字段的判据：字段名、最低值或最高值。
enum Requirement {
    AtLeast(&'static str, f32),
    AtMost(&'static str, f32),
}

/// 每一类进攻角色必须满足的行为要求。
fn offensive_requirements(role: &str) -> Vec<Requirement> {
    match role {
        "PrimaryHandler" => vec![
            Requirement::AtLeast("pass_frequency", 0.70),
            Requirement::AtLeast("drive_frequency", 0.70),
        ],
        "SecondaryHandler" => vec![
            Requirement::AtLeast("pass_frequency", 0.60),
            Requirement::AtMost("shoot_frequency", 0.70),
        ],
        "ShotCreator" => vec![
            Requirement::AtLeast("shoot_frequency", 0.70),
            Requirement::AtLeast("drive_frequency", 0.60),
        ],
        "Slasher" => vec![Requirement::AtLeast("drive_frequency", 0.70)],
        "AthleticFinisher" => vec![
            Requirement::AtLeast("cut_frequency", 0.75),
            Requirement::AtMost("three_point_preference", 0.50),
        ],
        "OffScreenShooter" => vec![
            Requirement::AtLeast("three_point_preference", 0.80),
            Requirement::AtLeast("shoot_frequency", 0.60),
        ],
        "StationaryShooter" => vec![
            Requirement::AtLeast("three_point_preference", 0.90),
            Requirement::AtLeast("shoot_frequency", 0.80),
            Requirement::AtMost("pass_frequency", 0.50),
        ],
        "VersatileBig" => vec![
            Requirement::AtLeast("screen_frequency", 0.70),
            Requirement::AtLeast("three_point_preference", 0.40),
        ],
        "PostScorer" => vec![
            Requirement::AtMost("three_point_preference", 0.10),
            Requirement::AtLeast("offensive_rebound_frequency", 0.75),
        ],
        "StretchBig" => vec![
            Requirement::AtLeast("three_point_preference", 0.80),
            Requirement::AtLeast("screen_frequency", 0.70),
        ],
        "RollCutBig" => vec![
            Requirement::AtLeast("screen_frequency", 0.75),
            Requirement::AtMost("three_point_preference", 0.40),
        ],
        other => panic!("进攻角色 {other} 没有行为判据，名单新增角色时必须同时补上判据"),
    }
}

/// 每一类防守角色必须满足的行为要求。职责由这四项倾向承载，不进入
/// 效用、概率或行为分支。
fn defensive_requirements(role: &str) -> Vec<Requirement> {
    match role {
        "PointOfAttack" => vec![
            Requirement::AtLeast("gamble_steal", 0.60),
            Requirement::AtMost("help_aggressiveness", 0.56),
        ],
        "Chaser" => vec![
            Requirement::AtMost("help_aggressiveness", 0.56),
            Requirement::AtLeast("gamble_steal", 0.60),
        ],
        "Helper" => vec![Requirement::AtLeast("help_aggressiveness", 0.65)],
        "WingStopper" => vec![
            Requirement::AtLeast("physicality", 0.42),
            Requirement::AtLeast("help_aggressiveness", 0.34),
        ],
        "MobileBig" => vec![
            Requirement::AtLeast("block_aggressiveness", 0.45),
            Requirement::AtLeast("help_aggressiveness", 0.55),
        ],
        "AnchorBig" => vec![
            Requirement::AtLeast("block_aggressiveness", 0.45),
            Requirement::AtLeast("physicality", 0.60),
        ],
        "LowActivity" => vec![
            Requirement::AtMost("help_aggressiveness", 0.45),
            Requirement::AtMost("gamble_steal", 0.45),
        ],
        other => panic!("防守角色 {other} 没有行为判据，名单新增角色时必须同时补上判据"),
    }
}

fn tendency(player: &Player, field: &str) -> f32 {
    match field {
        "shoot_frequency" => player.tendencies.shoot_frequency,
        "three_point_preference" => player.tendencies.three_point_preference,
        "drive_frequency" => player.tendencies.drive_frequency,
        "pass_frequency" => player.tendencies.pass_frequency,
        "cut_frequency" => player.tendencies.cut_frequency,
        "screen_frequency" => player.tendencies.screen_frequency,
        "offensive_rebound_frequency" => player.tendencies.offensive_rebound_frequency,
        "gamble_steal" => player.tendencies.gamble_steal,
        "block_aggressiveness" => player.tendencies.block_aggressiveness,
        "help_aggressiveness" => player.tendencies.help_aggressiveness,
        "physicality" => player.tendencies.physicality,
        other => panic!("未知倾向字段 {other}"),
    }
}

fn roster(text: &str) -> Roster {
    serde_json::from_str(text).expect("roster parses")
}

fn check_requirements(player: &Player, team: &str, role: &str, requirements: Vec<Requirement>) {
    for requirement in requirements {
        match requirement {
            Requirement::AtLeast(field, floor) => {
                let value = tendency(player, field);
                assert!(
                    value >= floor,
                    "{}（{}）的 {field} 是 {value:.2}，低于角色 {role} 的下限 {floor:.2}：角色与倾向不一致",
                    player.name,
                    team
                );
            }
            Requirement::AtMost(field, ceiling) => {
                let value = tendency(player, field);
                assert!(
                    value <= ceiling,
                    "{}（{}）的 {field} 是 {value:.2}，高于角色 {role} 的上限 {ceiling:.2}：角色与倾向不一致",
                    player.name,
                    team
                );
            }
        }
    }
}

#[test]
fn every_declared_role_is_supported_by_its_tendencies() {
    let rosters = [
        roster(include_str!("../../../data/roster/home.json")),
        roster(include_str!("../../../data/roster/away.json")),
    ];
    let mut offensive: BTreeMap<&str, usize> = BTreeMap::new();
    let mut defensive: BTreeMap<&str, usize> = BTreeMap::new();
    for side in &rosters {
        for player in &side.players {
            *offensive
                .entry(player.offensive_role.as_str())
                .or_default() += 1;
            *defensive
                .entry(player.defensive_role.as_str())
                .or_default() += 1;
            check_requirements(
                player,
                &side.team,
                &player.offensive_role,
                offensive_requirements(&player.offensive_role),
            );
            check_requirements(
                player,
                &side.team,
                &player.defensive_role,
                defensive_requirements(&player.defensive_role),
            );
        }
    }
    assert!(
        !offensive.is_empty() && !defensive.is_empty(),
        "名单至少要有一名球员，否则这条一致性守卫没有覆盖对象"
    );
}

#[test]
fn adr_020_shot_zone_criteria_are_supported_by_tendencies() {
    let rosters = [
        roster(include_str!("../../../data/roster/home.json")),
        roster(include_str!("../../../data/roster/away.json")),
    ];
    let post_ceiling = 0.10;
    let shooter_floor = 0.90;
    for side in &rosters {
        for player in &side.players {
            if player.offensive_role == "PostScorer" {
                assert!(
                    player.tendencies.three_point_preference <= post_ceiling,
                    "{} 的三分倾向高于背身中锋的判据阈值",
                    player.name
                );
            }
            if player.offensive_role == "StationaryShooter" {
                assert!(
                    player.tendencies.three_point_preference >= shooter_floor,
                    "{} 的三分倾向低于定点射手的判据阈值",
                    player.name
                );
            }
        }
    }
}
