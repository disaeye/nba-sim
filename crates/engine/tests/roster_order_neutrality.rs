//! P-2 守卫：**名册数组顺序不得携带身份**（round-10 Step4a）。
//!
//! ## 原则
//!
//! `attributes.md §2.7/§2.9/T1` 要求 `roles` 字段移除、角色降级为派生视图；
//! `tactics.md TA3` 要求「角色是槽位不是身份」。此前名册由
//! `builtin_*(index)` 按下标分派、且 `id` 编码下标 —— **顺序即身份**。
//!
//! ## 本门的判据
//!
//! 打乱 `data/roster/*.json` 的球员数组顺序，**行为必须完全不变**。
//! 这是决定性的：只要还有任何逻辑依赖下标，逐 tick 轨迹就会漂移。
//!
//! 实现方式：直接构造两个 `TeamData`，其球员集合相同、仅数组顺序不同，
//! 比较两场比赛的逐 tick 行为哈希。

use nba_domain::GameRules;
use nba_domain::PlayerData;
use nba_engine::MatchEngine;
use nba_engine::MatchSetup;

fn hash_game(setup: MatchSetup, seed: u64) -> u64 {
    let mut e = MatchEngine::with_setup(setup, seed);
    e.set_scope("1q").expect("1q scope is valid");
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut ticks = 0usize;
    while !e.is_finished() && ticks < 60_000 {
        let t = e.step();
        let mut buf = String::new();
        use std::fmt::Write;
        write!(
            buf,
            "{}|{:.4}|{:.4}|",
            t.frame.t, t.frame.t_game, t.frame.shot_clock
        )
        .unwrap();
        for ev in &t.frame.event_log {
            write!(buf, "{};", ev.kind).unwrap();
        }
        for p in &t.frame.players {
            write!(buf, "{}:{:.5},{:.5};", p.id, p.x, p.y).unwrap();
        }
        for b in buf.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
        ticks += 1;
    }
    h
}

/// 打乱一个球员数组的顺序（确定性轮转，不用随机数）。
fn rotate(players: &[PlayerData], by: usize) -> Vec<PlayerData> {
    let n = players.len();
    (0..n).map(|i| players[(i + by) % n].clone()).collect()
}

#[test]
fn roster_array_order_does_not_change_behaviour() {
    let rules = GameRules::default();
    let base = MatchSetup::builtin(rules.clone());

    // 基线：主场名册按档案顺序
    let mut baseline_hashes = Vec::new();
    for seed in [42u64, 7, 100] {
        baseline_hashes.push(hash_game(base.clone(), seed));
    }

    // 变体：主场名册数组轮转（顺序变了，集合没变）
    for by in [1usize, 3, 5] {
        let mut setup = MatchSetup::builtin(rules.clone());
        setup.home_team.players = rotate(&base.home_team.players, by);
        for (i, seed) in [42u64, 7, 100].iter().enumerate() {
            let h = hash_game(setup.clone(), *seed);
            assert_eq!(
                h, baseline_hashes[i],
                "shuffling the roster array (rotate by {by}) changed behaviour for seed {seed}: \
                 {h:#018x} vs {:#018x}. Identity must come from the player's data and slot fit, \
                 NOT from his position in the array (attributes.md §2.7/T1, tactics.md TA3).",
                baseline_hashes[i]
            );
        }
    }
}

/// `roles` 字段必须已从名册档案移除（文档要求）。
#[test]
fn roster_assets_carry_no_roles_field() {
    for (name, json) in [
        ("home", include_str!("../../../data/roster/home.json")),
        ("away", include_str!("../../../data/roster/away.json")),
    ] {
        assert!(
            !json.contains("\"roles\""),
            "data/roster/{name}.json must not carry a `roles` field: \
             attributes.md §2.9 requires roles to be removed from player data \
             and re-derived as a presentation projection."
        );
    }
}
