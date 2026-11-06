//! 赛前角色与行为倾向的一致性守卫。
//!
//! 角色的行为含义由倾向字段承载，代码不按角色分支（`data.rs` 的
//! `three_point_preference` 注释）。这条约定只有在「角色写了什么，倾向
//! 就必须支持什么」成立时才安全：否则角色标签会变成装饰，行为与标签
//! 各说各话，赛前部署失去意义。这个测试把每一类角色的最低行为要求写死
//! 成判据，名单里出现不符合角色的倾向就在这里失败。

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
}

/// 一侧倾向字段的判据：字段名、最低值或最高值。
enum Requirement {
    AtLeast(&'static str, f32),
    AtMost(&'static str, f32),
}

/// 每一类进攻角色必须满足的行为要求。
fn requirements(role: &str) -> Vec<Requirement> {
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
            Requirement::AtLeast("three_point_preference", 0.85),
            Requirement::AtLeast("shoot_frequency", 0.80),
            Requirement::AtMost("pass_frequency", 0.50),
        ],
        "VersatileBig" => vec![
            Requirement::AtLeast("screen_frequency", 0.70),
            Requirement::AtLeast("three_point_preference", 0.40),
        ],
        "PostScorer" => vec![
            Requirement::AtMost("three_point_preference", 0.20),
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
        other => panic!("角色 {other} 没有行为判据，名单新增角色时必须同时补上判据"),
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
        other => panic!("未知倾向字段 {other}"),
    }
}

fn roster(text: &str) -> Roster {
    serde_json::from_str(text).expect("roster parses")
}

#[test]
fn every_declared_offensive_role_is_supported_by_its_tendencies() {
    let rosters = [
        roster(include_str!("../../../data/roster/home.json")),
        roster(include_str!("../../../data/roster/away.json")),
    ];
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for side in &rosters {
        for player in &side.players {
            *counts.entry(player.offensive_role.as_str()).or_default() += 1;
            for requirement in requirements(&player.offensive_role) {
                match requirement {
                    Requirement::AtLeast(field, floor) => {
                        let value = tendency(player, field);
                        assert!(
                            value >= floor,
                            "{}（{}）的 {field} 是 {value:.2}，低于 {floor:.2}：角色与倾向不一致",
                            player.name,
                            side.team
                        );
                    }
                    Requirement::AtMost(field, ceiling) => {
                        let value = tendency(player, field);
                        assert!(
                            value <= ceiling,
                            "{}（{}）的 {field} 是 {value:.2}，高于 {ceiling:.2}：角色与倾向不一致",
                            player.name,
                            side.team
                        );
                    }
                }
            }
        }
    }
    assert!(
        !counts.is_empty(),
        "名单至少要有一名球员，否则这条一致性守卫没有覆盖对象"
    );
}
