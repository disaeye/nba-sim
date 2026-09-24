//! 赛前身份字段投影（attributes.md §2.7a/§2.7b、tactics.md TA9）。
//!
//! 守卫对象：六类位置与赛前攻防角色必须出现在每一帧的球员投影里；
//! **整场比赛保持不变**——换人、换防、突破分球或回合结束都不改写它们
//! （TA9：角色是赛前配置，不是回合状态的函数）。

use nba_engine::MatchEngine;
use std::collections::HashMap;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Identity {
    position: String,
    offensive_role: String,
    defensive_role: String,
}

/// 全场模拟（full scope，4 节）下逐帧采集身份投影，断言：
/// 1. 每名登场球员的身份字段非空；
/// 2. 同一球员的身份在整个比赛中逐帧一致（含换人后登场的替补）；
/// 3. 位置值落在六类枚举内（投影不做自由字符串）。
#[test]
fn match_identity_projection_is_stable_across_full_game() {
    const VALID_POSITIONS: [&str; 6] = ["Point", "Combo", "Wing", "Forward", "Big", "Center"];
    const VALID_OFF_ROLES: [&str; 11] = [
        "PrimaryHandler",
        "SecondaryHandler",
        "ShotCreator",
        "Slasher",
        "AthleticFinisher",
        "OffScreenShooter",
        "StationaryShooter",
        "VersatileBig",
        "PostScorer",
        "StretchBig",
        "RollCutBig",
    ];
    const VALID_DEF_ROLES: [&str; 7] = [
        "PointOfAttack",
        "Chaser",
        "Helper",
        "WingStopper",
        "MobileBig",
        "AnchorBig",
        "LowActivity",
    ];

    let mut engine = MatchEngine::new(42);
    engine.set_scope("full").expect("full scope is valid");
    let mut identities: HashMap<String, Identity> = HashMap::new();
    let mut frames_with_players = 0usize;
    let mut substitutions_seen = 0usize;
    let mut ticks = 0usize;

    while !engine.is_finished() && ticks < 400_000 {
        let tick = engine.step();
        ticks += 1;
        substitutions_seen += tick
            .frame
            .event_log
            .iter()
            .filter(|event| event.kind == "SUBSTITUTION")
            .count();
        if tick.frame.players.is_empty() {
            continue;
        }
        frames_with_players += 1;
        for player in &tick.frame.players {
            let identity = Identity {
                position: player.position.clone(),
                offensive_role: player.offensive_role.clone(),
                defensive_role: player.defensive_role.clone(),
            };
            assert!(
                !identity.position.is_empty()
                    && !identity.offensive_role.is_empty()
                    && !identity.defensive_role.is_empty(),
                "player {} projected empty identity at tick {}",
                player.id,
                tick.frame.t
            );
            assert!(
                VALID_POSITIONS.contains(&identity.position.as_str()),
                "player {} projected unknown position `{}`",
                player.id,
                identity.position
            );
            assert!(
                VALID_OFF_ROLES.contains(&identity.offensive_role.as_str()),
                "player {} projected unknown offensive role `{}`",
                player.id,
                identity.offensive_role
            );
            assert!(
                VALID_DEF_ROLES.contains(&identity.defensive_role.as_str()),
                "player {} projected unknown defensive role `{}`",
                player.id,
                identity.defensive_role
            );
            let previous = identities.insert(player.id.clone(), identity.clone());
            if let Some(prev) = previous {
                assert_eq!(
                    prev,
                    identity,
                    "player {} identity drifted mid-game: {:?} -> {:?} (tick {})",
                    player.id,
                    (
                        prev.position.clone(),
                        prev.offensive_role.clone(),
                        prev.defensive_role.clone()
                    ),
                    (
                        identity.position.clone(),
                        identity.offensive_role.clone(),
                        identity.defensive_role.clone()
                    ),
                    tick.frame.t
                );
            }
        }
    }

    assert!(
        engine.is_finished(),
        "full scope must finish within the tick budget"
    );
    assert!(frames_with_players > 1000, "projection must be observed");
    assert!(
        substitutions_seen > 0,
        "full game must contain substitutions for the stability guard to be meaningful"
    );
    assert!(
        identities.len() >= 8,
        "both rosters must be projected, got {} players",
        identities.len()
    );
}
