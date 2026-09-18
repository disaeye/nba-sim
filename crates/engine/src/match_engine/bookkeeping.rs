//! 每 tick 收尾簿记：持球人同步、体力推进、语义接触归集与事实抽取。
//!
//! 依据 `docs/architecture.md` §2 的数据流：物理步进产出 `RawContact` 与
//! `PhysicsFact`，语义层把它们解释为 `SemanticContact` 与配位评估，最后统一
//! 汇入 `pending_events`。本模块是这条链路的收口，顺序不可调换：
//!
//! 1. 按球态同步物理层的持球人（`set_ball_holder`），使「谁在持球」与权威球态一致；
//! 2. 按 player id 排序推进体力（含规则通道的消耗曲线）；
//! 3. 抽取接触事实与配位评估，映射为待发布事件；
//! 4. 把体力与士气写回物理层球员，供渲染与下一 tick 的决策读取。

use nba_physics::ballistics::BallTrajectoryKind;
use nba_semantics::SemanticEvaluator;

use super::projection::{physics_fact_to_event, semantic_contact_to_event};
use super::MatchEngine;

impl MatchEngine {
    pub(crate) fn collect_tick_facts(&mut self, dt: f32) {
        // A control transfer is a flight, not possession. The receiving
        // player becomes the holder only after the frozen trajectory ends.
        let active_carrier = match &self.ball.ball_state {
            BallTrajectoryKind::Held { carrier_id }
            | BallTrajectoryKind::InboundReady {
                inbounder_id: carrier_id,
                ..
            }
            | BallTrajectoryKind::Drive {
                driver_id: carrier_id,
                ..
            } => Some(carrier_id.as_str()),
            _ => None,
        };
        self.systems.physics.set_ball_holder(active_carrier);

        let player_ids: Vec<String> = {
            let mut ids: Vec<String> =
                self.systems.physics.get_players().keys().cloned().collect();
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
        self.journal.pending_events.extend(
            self.systems
                .physics
                .drain_facts()
                .into_iter()
                .map(physics_fact_to_event),
        );
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
