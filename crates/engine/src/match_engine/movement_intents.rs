// 运动意图的登记与到期清算（validation.md §4.2）。
//
// 划分依据（D29 行数门）：本文件只回答「意图是否已达成」，不回答
// 「目标点是什么」——目标点由 `tactics_phase.rs` 每 tick 从战术档案重算。
// 二者的耦合点只有 `observations.movement_intents`：那边追加与修订，
// 这边在决策间隔到期时清算终态。放在一起会让目标生成与终态判定
// 互相嵌套，故按写入时机切成两半。
//
// 到位判定在意图到期时统一清算而非每结算点立即结案：驻留型任务
// （球员已在目标点执行任务）会在登记后立刻满足几何，形成
// 「登记→完成→再登记」的意图流淹没（实测 98% completed 寿命为 0 tick）。
// 任务寿命与决策间隔一致，到期清算即真实终态。

use glam::Vec2;
use nba_domain::GameEvent;

use super::state::MovementIntentRecord;
use super::MatchEngine;

impl MatchEngine {
    /// 登记（或修订）一条运动意图：同一球员的任务类型与目标不变时
    /// 续用现有意图，变化时发布替代意图（revision + 1）。
    /// 意图期限用本引擎的决策间隔折算（任务在下一次目标重算前有效）。
    pub(crate) fn record_movement_intent(
        &mut self,
        player_id: &str,
        task: &str,
        target_region: &str,
        target_pos: Vec2,
    ) {
        let key = player_id.to_string();
        let current_position = self
            .systems
            .physics
            .get_player(player_id)
            .map(|player| player.pos_ft)
            .unwrap_or(target_pos);
        let deadline_ticks = (self.config.rules.decision_interval_seconds
            / self.config.rules.tick_seconds)
            .ceil()
            .max(1.0) as u32;
        if let Some(existing) = self.observations.movement_intents.get(&key) {
            if existing.task == task && (existing.target_pos - target_pos).length() < 1.0 {
                return;
            }
            // 任务或目标变化：先登记替代事实，再发布新意图。
            let revision = existing.revision + 1;
            self.journal
                .pending_events
                .push(GameEvent::MovementOutcome {
                    player_id: key.clone(),
                    task: existing.task.clone(),
                    outcome: "superseded".to_string(),
                    path_length_ft: 0.0,
                    evidence_event_id: None,
                });
            self.journal.pending_events.push(GameEvent::MovementIntent {
                player_id: key,
                cause_event_id: None,
                task: task.to_string(),
                target: target_region.to_string(),
                target_pos: (target_pos.x, target_pos.y),
                deadline_ticks,
                revision,
            });
            self.observations.movement_intents.insert(
                player_id.to_string(),
                MovementIntentRecord {
                    task: task.to_string(),
                    target_pos,
                    started_tick: self.clock.tick_index,
                    deadline_ticks,
                    revision,
                    last_position: current_position,
                    path_length_ft: 0.0,
                },
            );
            return;
        }
        self.journal.pending_events.push(GameEvent::MovementIntent {
            player_id: key,
            cause_event_id: None,
            task: task.to_string(),
            target: target_region.to_string(),
            target_pos: (target_pos.x, target_pos.y),
            deadline_ticks,
            revision: 0,
        });
        self.observations.movement_intents.insert(
            player_id.to_string(),
            MovementIntentRecord {
                task: task.to_string(),
                target_pos,
                started_tick: self.clock.tick_index,
                deadline_ticks,
                revision: 0,
                last_position: current_position,
                path_length_ft: 0.0,
            },
        );
    }

    /// 到位与回位判定（validation.md §4.2）：实际位置进入目标区域
    /// （半径 `arrival_radius_ft`）即登记完成；任务到期未达成登记
    /// 超时事实。声明完成却不满足几何是 Integrity 缺陷，超时但如实
    /// 登记属于行为样本。
    ///
    /// 到位判定在意图到期时统一清算：意图到期时刻若球员在目标区域内
    /// 则登记完成，否则登记超时。不在每个结算点立即结案的原因：驻留
    /// 型任务（球员已在目标点执行任务）会在登记后立刻满足几何，形成
    /// 「登记→完成→再登记」的意图流淹没（实测 98% completed 寿命为
    /// 0 tick）。任务寿命与决策间隔一致，到期清算即真实终态。
    pub(crate) fn settle_movement_intents(&mut self) {
        if self.observations.movement_intents.is_empty() {
            return;
        }
        let arrival_radius_ft = self.config.rules.tactics.arrival_radius_ft;
        let current_tick = self.clock.tick_index;
        let mut finished: Vec<(String, &'static str)> = Vec::new();
        for (player_id, intent) in &mut self.observations.movement_intents {
            let expired = current_tick >= intent.started_tick + intent.deadline_ticks as u64;
            let Some(player) = self.systems.physics.get_player(player_id) else {
                finished.push((player_id.clone(), "interrupted"));
                continue;
            };
            intent.path_length_ft += (player.pos_ft - intent.last_position).length();
            intent.last_position = player.pos_ft;
            if expired {
                let arrived = (player.pos_ft - intent.target_pos).length() <= arrival_radius_ft;
                finished.push((
                    player_id.clone(),
                    if arrived { "completed" } else { "timed_out" },
                ));
            }
        }
        for (player_id, outcome) in finished {
            let Some(intent) = self.observations.movement_intents.remove(&player_id) else {
                continue;
            };
            let path_length_ft = intent.path_length_ft;
            self.journal
                .pending_events
                .push(GameEvent::MovementOutcome {
                    player_id,
                    task: intent.task,
                    outcome: outcome.to_string(),
                    path_length_ft,
                    evidence_event_id: None,
                });
        }
    }
}
