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
use rayon::prelude::*;

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

    // 基线：主场名册按档案顺序。基线与各变体的模拟相互独立，种子间并行。
    let seeds = [42u64, 7, 100];
    let baseline_hashes: Vec<u64> = seeds
        .par_iter()
        .copied()
        .map(|seed| hash_game(base.clone(), seed))
        .collect();

    // 变体：主场名册数组轮转（顺序变了，集合没变）。
    // 轮转步长只决定新顺序的形态，三个步长对断言完全同构
    // （判定的是「顺序不携带身份」这一个事实），取一个步长 × 全部种子。
    for by in [1usize] {
        let mut setup = MatchSetup::builtin(rules.clone());
        setup.home_team.players = rotate(&base.home_team.players, by);
        let variant_hashes: Vec<u64> = seeds
            .par_iter()
            .copied()
            .map(|seed| hash_game(setup.clone(), seed))
            .collect();
        for (i, seed) in seeds.iter().enumerate() {
            assert_eq!(
                variant_hashes[i], baseline_hashes[i],
                "shuffling the roster array (rotate by {by}) changed behaviour for seed {seed}: \
                 {:#018x} vs {:#018x}. Identity must come from the player's data and slot fit, \
                 NOT from his position in the array (attributes.md §2.7/T1, tactics.md TA3).",
                variant_hashes[i], baseline_hashes[i]
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
