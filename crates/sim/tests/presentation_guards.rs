//! 批6表现层与账本守卫。
//!
//! 从权威帧流与账本测量：持续动作窗口、BLOCK 独立事件身份。
//! 剪辑映射与起跳时长由目录守卫和渲染器静态检查覆盖。

mod common;

use nba_domain::rules::GameRules;
use nba_sim_core::action::Fact;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};
use std::collections::{BTreeMap, BTreeSet};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(
        MatchSetup::builtin(GameRules::default().shortened(120.0, 60.0)),
        params,
        seed,
    )
}

#[derive(Default)]
struct PresentationStats {
    /// action_id -> (family/kind, first, last) 的持续姿态寿命。
    sustained: BTreeMap<u64, (String, f64, f64)>,
    /// 独立的 BLOCK 事件身份。
    block_ids: BTreeSet<u64>,
    /// 被盖出手的事件身份（与被盖动作一一对应，但不是同一个身份）。
    blocked_shot_ids: BTreeSet<u64>,
}

fn measure(seed: u64) -> PresentationStats {
    let mut world = world(seed);
    let mut stats = PresentationStats::default();
    let mut guard = 0;
    while !world.finished && guard < 200_000 {
        world.step(TICK_SECONDS);
        guard += 1;
        let tick = world.stream_tick();
        for player in &tick.frame.players {
            for action in &player.active_actions {
                if action.phase == "sustained" {
                    let now = tick.frame.t as f64;
                    let entry = stats
                        .sustained
                        .entry(action.action_id)
                        .or_insert((format!("{}/{}", action.family, action.kind), now, now));
                    entry.2 = now;
                }
            }
        }
    }
    let mut ids = BTreeSet::new();
    for event in &world.events {
        if !ids.insert(event.event_id) {
            continue;
        }
        match &event.fact {
            Fact::Block { .. } => {
                stats.block_ids.insert(event.event_id);
            }
            Fact::Shot { blocked_by: Some(_), .. } => {
                stats.blocked_shot_ids.insert(event.event_id);
            }
            _ => {}
        }
    }
    stats
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted.get((sorted.len() as f64 * p) as usize).copied().unwrap_or(0.0)
}

#[test]
fn sustained_action_windows_are_bounded() {
    let mut durations = Vec::new();
    for seed in 1..=4u64 {
        let s = measure(seed);
        for (_, (_, first, last)) in &s.sustained {
            durations.push(last - first);
        }
    }
    durations.sort_by(|a, b| a.total_cmp(b));
    let p50 = percentile(&durations, 0.5);
    let max = durations.last().copied().unwrap_or(0.0);
    eprintln!("sustained n={} p50={p50:.1}s max={max:.1}s", durations.len());
    assert!(p50 < 8.0, "持续动作窗口 p50={p50:.1}s，超过 8s");
    assert!(max < 30.0, "持续动作窗口 max={max:.1}s，超过 30s");
}

#[test]
fn blocks_have_independent_event_identity() {
    let mut blocks = BTreeSet::new();
    let mut blocked_shots = BTreeSet::new();
    for seed in 1..=4u64 {
        let s = measure(seed);
        blocks.extend(s.block_ids);
        blocked_shots.extend(s.blocked_shot_ids);
    }
    assert!(!blocks.is_empty(), "完整比赛必须产生独立的 BLOCK 事件");
    let overlap: Vec<_> = blocks.intersection(&blocked_shots).collect();
    assert!(
        overlap.is_empty(),
        "BLOCK 与 Shot 不能共用同一个事件身份：重叠 {} 个",
        overlap.len()
    );
}
