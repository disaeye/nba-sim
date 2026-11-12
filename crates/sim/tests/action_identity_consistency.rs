//! 动作身份在事件流与协议投影之间的一致性。
//!
//! 方案 §10.2：事件流保留同一动作身份及其参与者，支持动作开始、阶段变化
//! 与唯一终态的因果关联。渲染帧中的球员动作状态必须来自权威运行时，
//! 包含稳定动作身份、动作族、技术变体、参与者角色、阶段与动作时间区间。
//!
//! 这条测试钉住三件事：
//!
//! 1. 事件流里的动作身份与协议投影里的动作身份指向同一个动作；
//! 2. 十名球员并行动作按 InteractionId 结算，同一次交互的参与者共享
//!    一个身份；
//! 3. 动作的唯一终态（完成、失败、取消）只出现一次。

mod common;

use std::collections::{BTreeMap, BTreeSet};

use nba_sim_core::action::Fact;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::World;

/// 事件流里的动作身份：数字身份与交互身份。
#[derive(Default)]
struct EventIdentities {
    /// 动作身份到交互身份的映射。
    action_to_interaction: BTreeMap<u64, u64>,
    /// 每个动作的终态计数。
    terminals: BTreeMap<u64, u32>,
    /// 已经读过的最后一个事件身份：事件身份全场单调递增。
    last_event_id: u64,
}

fn collect_events(world: &World, out: &mut EventIdentities) {
    // 事件身份全场单调递增：只读新事件，重复读全量会把同一个动作数很多次。
    for event in &world.events {
        if event.event_id <= out.last_event_id {
            continue;
        }
        out.last_event_id = event.event_id;
        match &event.fact {
            Fact::ActionStarted {
                action_id,
                interaction_id,
                ..
            } => {
                out.action_to_interaction.insert(*action_id, *interaction_id);
            }
            Fact::ActionCompleted { action_id, .. }
            | Fact::ActionFailed { action_id, .. }
            | Fact::ActionCancelled { action_id, .. } => {
                *out.terminals.entry(*action_id).or_insert(0) += 1;
            }
            _ => {}
        }
    }
}

#[test]
fn every_action_has_exactly_one_terminal_state() {
    let mut world = common::short_world(1);
    let mut identities = EventIdentities::default();
    for _ in 0..12_000 {
        world.step(TICK_SECONDS);
        collect_events(&world, &mut identities);
    }
    assert!(
        identities.action_to_interaction.len() > 100,
        "一场比赛必须产生大量动作：{}",
        identities.action_to_interaction.len()
    );
    for (action_id, count) in &identities.terminals {
        assert_eq!(
            *count, 1,
            "动作 {action_id} 出现了 {count} 个终态，终态必须唯一"
        );
    }
}

#[test]
fn a_parallel_interaction_is_settled_once_for_all_its_participants() {
    // 掩护是多人交互：掩护人与持球人共享一个 interaction_id，动作各���
    // 有自己的身份。
    let mut world = common::short_world(2);
    let mut identities = EventIdentities::default();
    let mut parallel_found = false;
    for _ in 0..12_000 {
        world.step(TICK_SECONDS);
        collect_events(&world, &mut identities);
        // 同一个交互里有两个以上的参与者：交互是并行结算的单位。
        let mut per_interaction: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
        for (action_id, interaction_id) in &identities.action_to_interaction {
            per_interaction
                .entry(*interaction_id)
                .or_default()
                .insert(*action_id);
        }
        if per_interaction.values().any(|actions| actions.len() > 1) {
            parallel_found = true;
            break;
        }
    }
    assert!(
        parallel_found,
        "一场比赛必须出现多人交互：同一个 interaction_id 下有多个动作身份"
    );
    // 每个动作只有一个终态，因此并行交互不会被结算两次。
    for (action_id, count) in &identities.terminals {
        assert_eq!(*count, 1, "动作 {action_id} 出现了 {count} 个终态");
    }
}

#[test]
fn the_frame_event_action_id_matches_the_render_action() {
    // 事件流里的动作身份与协议投影里的动作身份必须指向同一个动作。
    // 事件的日志窗口是最近五十毫秒，因此里面既有本 tick 的动作，也有
    // 刚刚结束的动作。核对的办法是：投影里的每个动作身份都能在事件流里
    // 找到，事件流里的动作身份格式也必须是引擎发出的那一种。
    let mut world = common::short_world(3);
    let mut projected_ids: BTreeSet<u64> = BTreeSet::new();
    let mut checked = 0usize;
    for _ in 0..8_000 {
        world.step(TICK_SECONDS);
        let tick = world.stream_tick();
        for player in &tick.frame.players {
            for action in &player.active_actions {
                projected_ids.insert(action.action_id);
            }
        }
        for event in &tick.frame.event_log {
            let Some(action_id) = event.action_id.as_deref() else {
                continue;
            };
            assert!(
                action_id
                    .strip_prefix("action-")
                    .and_then(|digits| digits.parse::<u64>().ok())
                    .is_some(),
                "事件流里的动作身份格式不对：{action_id}"
            );
            checked += 1;
        }
    }
    // 整场事件流里的动作身份。防守动作也有动作身份：它与进攻动作共用
    // 同一套身份空间。
    let mut event_ids: BTreeSet<u64> = BTreeSet::new();
    for event in &world.events {
        match &event.fact {
            Fact::ActionStarted { action_id, .. }
            | Fact::ActionPhaseChanged { action_id, .. }
            | Fact::ActionCompleted { action_id, .. }
            | Fact::ActionFailed { action_id, .. }
            | Fact::ActionCancelled { action_id, .. }
            | Fact::DefensiveActionStarted { action_id, .. }
            | Fact::DefensiveActionPhaseChanged { action_id, .. }
            | Fact::DefensiveActionCompleted { action_id, .. }
            | Fact::DefensiveActionCancelled { action_id, .. }
            // 持续姿态登记自己的身份与时间边界：投影里的姿态身份因此
            // 也能在事件流里找到对应。无球职责与持球站位的身份就在这两条
            // 事实里。
            | Fact::PostureStarted { action_id, .. }
            | Fact::PostureEnded { action_id, .. }
            | Fact::DefenseResponsibilityChanged { action_id, .. }
            | Fact::OnBallPositioningChanged { action_id, .. } => {
                event_ids.insert(*action_id);
            }
            _ => {}
        }
    }
    assert!(!projected_ids.is_empty(), "协议投影里必须出现动作");
    for id in &projected_ids {
        assert!(
            event_ids.contains(id),
            "协议投影里的动作 {id} 在事件流里没有对应的身份"
        );
    }
    assert!(checked > 50, "核对的动作身份太少：{checked}");
}

#[test]
fn the_action_window_reports_its_participants() {
    // 掩护交互里，参与者的身份与角色都在协议投影里给出。
    let mut world = common::short_world(4);
    for _ in 0..8_000 {
        world.step(TICK_SECONDS);
        let tick = world.stream_tick();
        for player in &tick.frame.players {
            for action in &player.active_actions {
                if action.family != "ACTION-SCREEN" {
                    continue;
                }
                assert!(
                    !action.role.is_empty(),
                    "掩护动作必须给出参与者角色"
                );
                assert!(
                    ["screener", "cut_partner", "ball_handler"].contains(&action.role.as_str()),
                    "掩护动作的角色不对：{}",
                    action.role
                );
            }
        }
    }
}
