//! 每 tick 收尾簿记：体力推进、语义接触归集与事实抽取。
//!
//! 依据 `docs/architecture.md` §2 的数据流：物理步进产出 `RawContact` 与
//! `PhysicsFact`，语义层把它们解释为 `SemanticContact` 与配位评估，最后统一
//! 汇入 `pending_events`。本模块是这条链路的收口，顺序不可调换：
//!
//! 1. 按球态同步持球姿态生命周期（失去球权的球员回到面框基准）；
//! 2. 按 player id 排序推进体力（含规则通道的消耗曲线）；
//! 3. 抽取接触事实与配位评估，映射为待发布事件；
//! 4. 把体力与士气写回物理层球员，供渲染与下一 tick 的决策读取。

use glam::Vec2;
use nba_semantics::SemanticEvaluator;

use super::projection::{physics_fact_to_event, semantic_contact_to_event};
use super::MatchEngine;

impl MatchEngine {
    /// 收球持球人的接地点裁定：收球后允许两步，第三步落地即走步；
    /// 尚未迈步时轴心脚在容差外重新落地也是走步（轴心脚滑动）。
    /// 迈步后的轴心脚再落地由步数预算约束（真实收球后两步内轴心脚
    /// 可离地再落地，第二落点是上篮节奏的常态）。
    ///
    /// 只裁定时正在持球且处于收球态的球员；事实事件同时入流，供回放与
    /// 评判重建收球后的脚步。
    fn observe_gathered_foot_contact(
        &mut self,
        entity_id: &str,
        foot: nba_domain::Foot,
        position: Vec2,
    ) {
        let Some(nba_domain::DribbleControlState::Gathered {
            pivot_foot,
            pivot_position,
            steps_after_gather,
        }) = self.clock.dribble_control.get(entity_id).copied()
        else {
            return;
        };
        if self.ball.holder_id() != Some(entity_id) {
            return;
        }
        self.journal
            .pending_events
            .push(nba_domain::GameEvent::FootContact {
                player_id: entity_id.to_string(),
                foot,
                position: (position.x, position.y),
            });
        if self.clock.dribble_violation.is_some() {
            return;
        }
        let pivot_dragged = pivot_foot == Some(foot)
            && steps_after_gather == 0
            && (position - pivot_position).length() > self.config.rules.pivot_foot_tolerance_ft;
        let steps = steps_after_gather.saturating_add(1);
        if pivot_dragged || steps > 2 {
            self.clock.dribble_violation = Some(nba_decision::constraint::ViolationKind::Travel);
            self.clock.current_possession_turnover_player = Some(entity_id.to_string());
        } else {
            self.clock.dribble_control.insert(
                entity_id.to_string(),
                nba_domain::DribbleControlState::Gathered {
                    pivot_foot,
                    pivot_position,
                    steps_after_gather: steps,
                },
            );
        }
    }

    pub(crate) fn collect_tick_facts(&mut self, dt: f32) {
        // 球态是持球人的唯一事实源，物理层只接收显式输入：
        // 持球权变更即姿态生命周期边界。
        self.systems.physics.set_ball_holder(self.ball.holder_id());
        let player_ids: Vec<String> = {
            let mut ids: Vec<String> = self.systems.physics.get_players().keys().cloned().collect();
            ids.sort();
            ids
        };
        let modulation = &mut self.observations.modulation;
        for pid in &player_ids {
            let speed = self
                .systems
                .physics
                .get_player(pid)
                .map(|p| p.vel_ft.length())
                .unwrap_or(0.0);
            modulation
                .entry(pid.clone())
                .or_default()
                .update_stamina_with_rules(speed, dt, &self.config.rules);
        }
        // modulation 状态在物理推进后保留为决策反馈，渲染读取当前值。

        let raw_contacts = self.systems.physics.drain_contacts();
        self.observations.latest_contacts = SemanticEvaluator::event_facts(
            raw_contacts.clone(),
            &self.systems.physics,
            self.flow.possession,
            self.clock.sub_phase,
            self.observations
                .pending_shot_release
                .as_ref()
                .map(|pending| pending.shooter_id.as_str())
                .or(match &self.ball.ball_state {
                    nba_physics::ballistics::BallTrajectoryKind::Shot { shooter_id, .. }
                    | nba_physics::ballistics::BallTrajectoryKind::FreeThrowSetup {
                        shooter_id,
                        ..
                    }
                    | nba_physics::ballistics::BallTrajectoryKind::FreeThrow {
                        shooter_id, ..
                    } => Some(shooter_id.as_str()),
                    _ => None,
                }),
            self.ball_holder_id(),
            self.clock.tick_index,
            &self.config.rules,
        );
        self.observations.latest_spacing = Some(SemanticEvaluator::spacing(
            self.flow.possession,
            self.ball.ball_pos_3d.0,
            &self.systems.physics,
            &self.config.rules,
        ));
        self.journal.pending_events.extend(
            self.observations
                .latest_contacts
                .iter()
                .map(semantic_contact_to_event),
        );
        for fact in self.systems.physics.drain_facts() {
            match fact {
                nba_physics::PhysicsFact::FootContact {
                    entity_id,
                    foot,
                    position,
                } => self.observe_gathered_foot_contact(&entity_id, foot, position),
                other => {
                    if let Some(event) = physics_fact_to_event(other) {
                        self.journal.pending_events.push(event);
                    }
                }
            }
        }
        for player_id in &player_ids {
            if let Some(state) = self.observations.modulation.get(player_id) {
                if let Some(player) = self.systems.physics.get_player_mut(player_id) {
                    player.stamina = state.stamina * player.max_stamina;
                    player.morale = format!("{:?}", state.morale);
                }
            }
        }
    }
}
