//! 动作窗口推进：推进每个在场动作的时间窗口，并同步物理层的运动学锁。
//!
//! 依据 `docs/architecture.md` §2 [3] 与 `docs/domain` 的 `ActionTimeWindow`：
//! 动作窗口是「一个动作从发起到结束的时间窗口」，它决定球员的运动学是否被
//! 锁定（`lock_kinematics`）以及动作阶段（Preparation/Execution/FollowThrough）。
//! 阶段变化必须发布为 `WindowTransition` 事实，而不是让下游自行推断。
//!
//! 遍历顺序按 player id 排序，保证同一 tick 的处理顺序与 `HashMap` 迭代序无关
//! （charter C4：相同种子逐 tick 完全一致）。

use nba_domain::GameEvent;

use super::MatchEngine;

impl MatchEngine {
    /// 推进全部动作窗口，并返回本 tick 产生的阶段迁移事实。
    pub(crate) fn advance_action_windows(&mut self, current_t: f32) {
        // Advance action windows in stable player-id order.
        if !self.observations.active_windows.is_empty() {
            let mut window_ids: Vec<String> =
                self.observations.active_windows.keys().cloned().collect();
            window_ids.sort();
            let mut window_events = Vec::new();
            for window_id in window_ids {
                let Some(window) = self.observations.active_windows.get_mut(&window_id) else {
                    continue;
                };
                let previous = window.phase;
                let current = window.update(current_t);
                self.systems.physics.set_player_locked(
                    &window.player_id,
                    window.lock_kinematics && !window.is_finished(current_t),
                    None,
                );
                if current != previous {
                    window_events.push(GameEvent::WindowTransition {
                        player_id: window.player_id.clone(),
                        action_type: window.action_type,
                        new_phase: current,
                    });
                    // plan.md §6.2：JumpShot 窗口跨过 Execution→FollowThrough
                    // 边界即真正的 Release（合球起跳完成、球脱手）。此刻
                    // 消费挂起的冻结裁定，把球态转为 Shot 并发布事件。
                    let release_ready = previous == nba_domain::action_window::ActionPhase::Execution
                        && current == nba_domain::action_window::ActionPhase::FollowThrough
                        && window.action_type == nba_domain::action_window::ActionType::JumpShot;
                    if release_ready {
                        self.consume_pending_shot_release(current_t);
                    }
                }
            }
            self.observations.active_windows.retain(|pid, window| {
                let finished = window.is_finished(current_t);
                if finished {
                    if let Some(p) = self.systems.physics.get_player_mut(pid) {
                        if p.action.ends_with("Shot")
                            || p.action == "Layup"
                            || p.action == "Dunk"
                            || p.action == "Floater"
                            || p.action == "ScreenSet"
                        {
                            p.action = "Recover".to_string();
                        }
                    }
                }
                !finished
            });
            self.journal.pending_events.extend(window_events);
        }
    }
}
