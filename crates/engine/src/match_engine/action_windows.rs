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
    /// 注册并启动一个动作时间窗口。
    ///
    /// charter 与 basketball.md 规格保证：
    /// 1. 同一球员在同一 tick 至多保留一个活动动作窗口；如果该球员已有窗口，
    ///    旧窗口以 `Superseded` 原因取消，并释放物理运动学锁定；
    /// 2. 动作开始时发布 `ActionStarted` 事实；
    /// 3. 若新窗口声明 `lock_kinematics`，立即同步物理层运动学锁定。
    pub(crate) fn start_action_window(
        &mut self,
        player_id: &str,
        window: nba_domain::action_window::ActionTimeWindow,
        target_id: Option<String>,
        target_pos: Option<(f32, f32)>,
    ) {
        if let Some(prev) = self.observations.active_windows.remove(player_id) {
            self.systems
                .physics
                .set_player_locked(player_id, false, None);
            self.journal
                .pending_events
                .push(GameEvent::ActionCancelled {
                    player_id: player_id.to_string(),
                    action_type: prev.action_type,
                    reason: nba_domain::event::ActionCancellationReason::Superseded,
                });
        }
        let lock_now = window.lock_kinematics && !window.is_finished(self.current_time());
        self.systems
            .physics
            .set_player_locked(player_id, lock_now, None);
        self.journal.pending_events.push(GameEvent::ActionStarted {
            player_id: player_id.to_string(),
            action_type: window.action_type,
            target_id,
            target_pos,
        });
        self.observations
            .active_windows
            .insert(player_id.to_string(), window);
    }

    /// 取消球员当前的动作时间窗口，并立即解除运动学锁定。
    pub(crate) fn cancel_action_window(
        &mut self,
        player_id: &str,
        reason: nba_domain::event::ActionCancellationReason,
    ) -> bool {
        if let Some(window) = self.observations.active_windows.remove(player_id) {
            self.systems
                .physics
                .set_player_locked(player_id, false, None);
            self.journal
                .pending_events
                .push(GameEvent::ActionCancelled {
                    player_id: player_id.to_string(),
                    action_type: window.action_type,
                    reason,
                });
            true
        } else {
            false
        }
    }

    /// 标记球员当前的动作时间窗口失败，并立即解除运动学锁定。
    pub(crate) fn fail_action_window(
        &mut self,
        player_id: &str,
        reason: nba_domain::event::ActionFailureReason,
    ) -> bool {
        if let Some(window) = self.observations.active_windows.remove(player_id) {
            self.systems
                .physics
                .set_player_locked(player_id, false, None);
            self.journal.pending_events.push(GameEvent::ActionFailed {
                player_id: player_id.to_string(),
                action_type: window.action_type,
                reason,
            });
            true
        } else {
            false
        }
    }

    /// 取消全部活动动作窗口（死球、犯规、违例、节末等边界）。
    pub(crate) fn cancel_all_active_windows(
        &mut self,
        reason: nba_domain::event::ActionCancellationReason,
    ) {
        let pids: Vec<String> = self.observations.active_windows.keys().cloned().collect();
        for pid in pids {
            if let Some(pending) = &self.observations.pending_shot_release {
                if pending.shooter_id == pid {
                    continue;
                }
            }
            self.cancel_action_window(&pid, reason);
        }
    }

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
                    window_events.push(GameEvent::ActionPhaseChanged {
                        player_id: window.player_id.clone(),
                        action_type: window.action_type,
                        old_phase: previous,
                        new_phase: current,
                    });
                    // plan.md §6.2：JumpShot 窗口跨过 Execution→FollowThrough
                    // 边界即真正的 Release（合球起跳完成、球脱手）。此刻
                    // 消费挂起的冻结裁定，把球态转为 Shot 并发布事件。
                    let release_ready = previous
                        == nba_domain::action_window::ActionPhase::Execution
                        && current == nba_domain::action_window::ActionPhase::FollowThrough
                        && (window.action_type == nba_domain::action_window::ActionType::JumpShot
                            || window.action_type
                                == nba_domain::action_window::ActionType::Putback);
                    if release_ready {
                        self.consume_pending_shot_release(current_t);
                    }
                }
            }
            let mut completed_events = Vec::new();
            self.observations.active_windows.retain(|pid, window| {
                let finished = window.is_finished(current_t);
                if finished {
                    self.systems.physics.set_player_locked(pid, false, None);
                    completed_events.push(GameEvent::ActionCompleted {
                        player_id: pid.clone(),
                        action_type: window.action_type,
                    });
                    if let Some(p) = self.systems.physics.get_player_mut(pid) {
                        if p.action.ends_with("Shot")
                            || p.action == "Layup"
                            || p.action == "Dunk"
                            || p.action == "Floater"
                            || p.action == "ScreenSet"
                            || p.action == "ScreenRoll"
                            || p.action == "ScreenPop"
                            || p.action == "Cut"
                            || p.action == "CutBackdoor"
                            || p.action == "BoxOut"
                            || p.action == "Putback"
                        {
                            p.action = "Recover".to_string();
                        }
                    }
                }
                !finished
            });
            window_events.extend(completed_events);
            self.journal.pending_events.extend(window_events);
        }
    }
}
