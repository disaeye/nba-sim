//! 决策审计追踪的流级守卫：每条动作事件都必须带着产生它的那次决策。
//!
//! 取样口径按「回合」计：跑满 1000 个以上动作事件为止。审计要回答的问题
//! 是「这次出手当时还考虑过什么、每一项期望得分怎么来的」，所以缺一条
//! 追踪就等于多一个无法解释的动作。

use nba_domain::rules::GameRules;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};

const ACTION_KINDS: [&str; 4] = ["SHOT", "PASS", "DRIVE", "POST_UP"];

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

fn finite(value: f32) -> bool {
    value.is_finite()
}

#[test]
fn every_action_event_carries_a_complete_decision_trace() {
    let mut actions = 0usize;
    let mut degenerate = 0usize;
    let mut multi_candidate = 0usize;
    let mut single_candidate = 0usize;
    let mut seeds = 1u64;
    while actions < 1000 {
        let mut world = world(seeds);
        let mut seen_events = 0usize;
        while !world.finished {
            world.step(TICK_SECONDS);
            if world.events.len() == seen_events {
                continue;
            }
            seen_events = world.events.len();
            let tick = world.stream_tick();
            for event in &tick.frame.event_log {
                if !ACTION_KINDS.contains(&event.kind.as_str()) {
                    continue;
                }
                actions += 1;
                let trace = event
                    .decision_trace
                    .as_ref()
                    .unwrap_or_else(|| panic!("{} 事件没有决策追踪", event.kind));
                assert!(
                    !trace.actor_id.is_empty(),
                    "决策追踪没有持球人：{trace:?}"
                );
                assert!(
                    !trace.chosen.is_empty(),
                    "决策追踪没有记录选中的动作：{trace:?}"
                );
                assert!(finite(trace.beta) && trace.beta > 0.0, "β 非法：{trace:?}");
                assert!(finite(trace.gamma), "γ 非法：{trace:?}");
                assert!(
                    !trace.candidates.is_empty(),
                    "候选集为空：{trace:?}"
                );
                // 战术硬抑制可以把候选压到只剩一条路；这种决策没有可比较的
                // 候选，单独计数并在整体上限制占比。
                if trace.candidates.len() >= 2 {
                    multi_candidate += 1;
                } else {
                    single_candidate += 1;
                }
                assert!(
                    trace.candidates.iter().any(|c| c.action == trace.chosen),
                    "选中的动作不在候选集里：{trace:?}"
                );
                let top = trace.top(3);
                assert!(!top.is_empty(), "Top-3 候选为空：{trace:?}");
                let mut total = 0.0f32;
                for candidate in &trace.candidates {
                    for value in [
                        candidate.total,
                        candidate.probability,
                        candidate.variance,
                        candidate.clock_cost,
                        candidate.foul_cost,
                        candidate.tactical_bonus,
                        candidate.epv.ability,
                        candidate.epv.contest,
                        candidate.epv.lane,
                        candidate.epv.help,
                        candidate.epv.tendency,
                        candidate.epv.context,
                        candidate.epv.turnover,
                    ] {
                        assert!(
                            finite(value),
                            "候选 {} 的分解项非有限值：{candidate:?}",
                            candidate.action
                        );
                    }
                    assert!(
                        candidate.probability >= 0.0,
                        "候选概率为负：{candidate:?}"
                    );
                    total += candidate.probability;
                }
                assert!(
                    (total - 1.0).abs() < 1e-3,
                    "候选概率和不为 1（{total}）：{trace:?}"
                );
                assert!(
                    (top.iter().map(|c| c.probability).sum::<f32>() - 1.0).abs()
                        < 1e-3
                        || top.len() < trace.candidates.len(),
                    "Top-3 截断后仍有全部候选"
                );
                if trace.candidates.len() >= 2 && trace.probability_spread() <= 1.0 + 1e-6 {
                    degenerate += 1;
                }
            }
        }
        seeds += 1;
        assert!(seeds < 40, "取样回合数不足：只有 {actions}");
    }
    assert!(actions >= 1000, "取样回合数不足：{actions}");
    assert!(multi_candidate > 0, "所有决策都只有一个候选");
    assert!(
        single_candidate * 20 < actions,
        "{single_candidate}/{actions} 条决策只有一个候选"
    );
    // 退化均匀分布只允许零星出现（两个候选恰好等价时是合理的）。
    assert!(
        degenerate * 100 < actions,
        "{degenerate}/{actions} 条决策追踪是退化的均匀分布"
    );
}
