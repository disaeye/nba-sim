//! 并行动作：十名球员各自持有运行时，多个动作在同一 tick 内并存。
//!
//! 这些测试直接读 `World` 的动作运行时与交互表，不依赖日志措辞。

use nba_sim_core::action::{ActionKind, Fact};
use nba_sim_core::state::Phase;
use nba_sim_core::world::TICK_SECONDS;

mod common;

/// 同一 tick 内可以同时存在多个不同球员的动作运行时，且各自属于不同交互。
#[test]
fn several_players_run_actions_concurrently() {
    let mut peak_players = 0usize;
    let mut peak_interactions = 0usize;
    let mut mixed_kinds = false;
    for seed in 1..=3u64 {
        let mut world = common::short_world(seed);
        let mut guard = 0;
        while !world.finished && guard < 200_000 {
            world.step(TICK_SECONDS);
            guard += 1;
            let active: Vec<usize> = world
                .players
                .iter()
                .enumerate()
                .filter(|(_, player)| player.on_court && player.action.is_some())
                .map(|(handle, _)| handle)
                .collect();
            peak_players = peak_players.max(active.len());
            peak_interactions = peak_interactions.max(world.interactions.len());
            if active.len() >= 2 {
                let mut kinds: Vec<ActionKind> = active
                    .iter()
                    .filter_map(|handle| {
                        world.players[*handle]
                            .action
                            .as_ref()
                            .map(|runtime| runtime.intent.kind)
                    })
                    .collect();
                kinds.dedup();
                if kinds.len() >= 2 {
                    mixed_kinds = true;
                }
            }
        }
    }
    assert!(
        peak_players >= 2,
        "同一时刻必须有两名以上球员在跑动作，观测到 {peak_players}"
    );
    assert!(
        peak_interactions >= 2,
        "同一时刻必须有两个以上交互并存，观测到 {peak_interactions}"
    );
    assert!(mixed_kinds, "并发动作必须包含不同种类，而不是同一动作的重复");
}

/// 防守动作与进攻动作并行：出手期间防守人自己也有运行时。
#[test]
fn defensive_actions_run_alongside_the_offensive_action() {
    let mut observed = false;
    for seed in 1..=3u64 {
        let mut world = common::short_world(seed);
        let mut guard = 0;
        while !world.finished && guard < 200_000 && !observed {
            world.step(TICK_SECONDS);
            guard += 1;
            let shooting = world.players.iter().any(|player| {
                player.on_court
                    && player
                        .action
                        .as_ref()
                        .is_some_and(|action| matches!(action.intent.kind, ActionKind::Shoot(_)))
            });
            if !shooting {
                continue;
            }
            observed = world
                .defensive_actions
                .iter()
                .any(|action| action.is_some());
        }
    }
    assert!(observed, "出手期间必须存在并行的防守动作运行时");
}

/// 动作生命周期事实必须成对出现：每个 ActionStarted 都有唯一终态。
#[test]
fn every_action_started_reaches_a_terminal_fact() {
    let report = common::short_world(5).run();
    let mut started = 0u32;
    let mut terminal = 0u32;
    for event in &report.events {
        match &event.fact {
            Fact::ActionStarted { .. } => started += 1,
            Fact::ActionCompleted { .. } | Fact::ActionFailed { .. } | Fact::ActionCancelled { .. } => {
                terminal += 1;
            }
            _ => {}
        }
    }
    assert!(started > 0, "比赛里必须有动作");
    assert_eq!(
        started, terminal,
        "每个动作必须恰好有一个终态：{started} 起，{terminal} 终"
    );
}

/// 并发不破坏阶段：活球阶段之外不会新起进攻动作。
#[test]
fn actions_are_not_started_outside_live_play() {
    let mut world = common::short_world(7);
    let mut guard = 0;
    while !world.finished && guard < 200_000 {
        let before = world.events.len();
        let phase_before = world.phase().clone();
        world.step(TICK_SECONDS);
        guard += 1;
        // 只有真正的死球终态才排除起手：Timeout 等停表阶段会在同一步里恢复比赛。
        if !matches!(phase_before, Phase::Finished | Phase::PeriodBreak { .. }) {
            continue;
        }
        for event in &world.events[before..] {
            if let Fact::ActionStarted { kind, .. } = &event.fact {
                assert!(
                    !matches!(*kind, "Dribble" | "Drive" | "Finish" | "Shoot" | "Pass"),
                    "死球阶段不得新起进攻动作：{kind}（阶段 {phase_before:?}）"
                );
            }
        }
    }
}
