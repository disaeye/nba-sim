//! 弹道裁决结果的消费：把 `BallFlightOutcome` 变成状态转移。
//!
//! 与 `ball_flight.rs` 的分工：那里只做**裁决与采样**（产出事实），
//! 本模块**消费**事实并触发球权转移、犯规与罚球程序。分开是因为消费路径
//! 需要按优先级短路本 tick（松球出界即结束），而短路的处理权属调度器。

use nba_domain::{GameFlowState, SubPhase};
use nba_physics::ballistics::{BallTrajectoryKind, BallisticsEngine};

use super::BallFlightOutcome;
use crate::match_engine::{MatchEngine, PhaseOutcome};

impl MatchEngine {
    /// 消费一次弹道裁决的结果（`architecture.md` §3.3 的状态转换表入口）。
    ///
    /// 按优先级依次处理：松球出界 → 突破分球 → 急停跳投 → 抢断 → 松球掌控 → 新球态。
    /// 只有出界会短路本 tick（它已经完成回合终结与发球程序，需立即出帧），
    /// 其余分支继续本轮调度。
    pub(crate) fn apply_ball_flight_outcome(
        &mut self,
        outcome: BallFlightOutcome,
        current_t: f32,
    ) -> PhaseOutcome {
        let BallFlightOutcome {
            new_ball_state,
            steal_triggered_defender,
            loose_ball_secured_player,
            live_ball_triggered,
            loose_ball_out_of_bounds,
            drive_kickout_action,
            drive_pullup_action,
        } = outcome;
        // 松球出界：显式状态转移（球权交给对方并发球）。
        //
        // 注意：这里**不能**合成 `GameEvent::BoundaryCross`。该事件的语义是
        // 「**球员**越过边界」，约束层 `out_of_bounds_event` 会据此判定
        // 球权违例；用一个空的 player_id 冒充球员边界事实会被误判成
        // `TURNOVER:OUT_OF_BOUNDS`（实测每场 42 次虚假失误）。
        // 球的出界是**球的**事实，由下面的回合终结 + 发球程序表达，
        // 不需要借用球员边界事件。
        if let Some((boundary, ball_3d)) = loose_ball_out_of_bounds {
            self.journal.current_event = Some("OUT_OF_BOUNDS".to_string());
            self.start_out_of_bounds_transition(boundary, ball_3d);
            self.publish_events();
            return PhaseOutcome::ShortCircuit;
        }
        if let Some((driver_id, target_id, driver_pos, target_spot, start_t)) = drive_kickout_action
        {
            self.journal.current_event = Some("DRIVE_KICKOUT".to_string());
            let driver_display = self
                .systems
                .physics
                .get_player(&driver_id)
                .map(|p| format!("{}号", p.jersey))
                .unwrap_or_else(|| driver_id.clone());
            let target_display = self
                .systems
                .physics
                .get_player(&target_id)
                .map(|p| format!("{}号", p.jersey))
                .unwrap_or_else(|| target_id.clone());
            self.journal.current_callout = Some(format!(
                "{} 吸引内线包夹，行进间妙手突分外线 {}！",
                driver_display, target_display
            ));
            self.execute_pass(
                &driver_id,
                &target_id,
                driver_pos,
                target_spot,
                start_t,
                false,
            );
        } else if let Some((driver_id, driver_pos, start_t)) = drive_pullup_action {
            self.journal.current_event = Some("DRIVE_PULLUP".to_string());
            let driver_display = self
                .systems
                .physics
                .get_player(&driver_id)
                .map(|p| format!("{}号", p.jersey))
                .unwrap_or_else(|| driver_id.clone());
            self.journal.current_callout =
                Some(format!("{} 禁区受阻，急停漂移干拔跳投！", driver_display));
            self.execute_shot(
                &driver_id,
                driver_pos,
                false,
                Some(nba_domain::action_window::JumperKind::PullUp),
                start_t,
            );
        } else if let Some(def_id) = steal_triggered_defender {
            self.journal.current_event = Some("STEAL".to_string());
            let def_display = self
                .systems
                .physics
                .get_player(&def_id)
                .map(|p| format!("{}号", p.jersey))
                .unwrap_or_else(|| def_id.clone());
            self.journal.current_callout =
                Some(format!("传球路线被识破！{} 飞身抢断！", def_display));
            let ball_intercept_pos = self.ball.ball_pos_3d.0;
            self.start_steal_transition(def_id, ball_intercept_pos);
        } else if let Some((player_id, from_pos, from_z)) = loose_ball_secured_player {
            // The security fact is sampled at `next_pos`/`next_z`; use the same
            // point as the control-transfer origin instead of rewinding to the
            // pre-integration ball sample.
            let target_pos = self
                .systems
                .physics
                .get_player(&player_id)
                .map(|player| player.pos_ft)
                .unwrap_or(from_pos);
            self.start_loose_ball_transition(player_id.clone(), from_pos);
            if !self.flow.simulation_complete {
                let target_z = self.config.rules.ball_holder_height_ft;
                let distance = (target_pos - from_pos).length();
                let speed_budget = (self.config.rules.ball_max_speed_ftps
                    - self.config.rules.invariant_speed_tolerance_ftps)
                    .max(self.config.rules.invariant_speed_tolerance_ftps);
                let duration = (distance / speed_budget)
                    .max(self.config.rules.min_pass_duration_seconds)
                    .max(self.config.rules.tick_seconds);
                self.transition_ball_state(BallTrajectoryKind::ControlTransfer {
                    from_pos,
                    from_z,
                    target_pos,
                    target_z,
                    carrier_id: player_id,
                    start_time: current_t,
                    duration,
                });
                self.ball.ball_pos_3d = (from_pos, from_z);
            }
        } else if let Some(nbs) = new_ball_state {
            self.transition_ball_state(nbs);
            if live_ball_triggered {
                self.transition_phase(SubPhase::Initiation);
                self.set_game_flow(GameFlowState::LiveBall);
                self.clock.last_decision_time = -self.config.rules.decision_interval_seconds;
            }
            if !matches!(self.ball.ball_state, BallTrajectoryKind::Held { .. }) {
                self.ball.ball_pos_3d = BallisticsEngine::sample_ball_position(
                    &self.ball.ball_state,
                    current_t,
                    self.systems.physics.get_players(),
                    &self.config.rules,
                );
            }
        }
        PhaseOutcome::Continue
    }
}
