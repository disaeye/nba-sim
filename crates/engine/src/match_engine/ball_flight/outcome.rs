//! 弹道裁决结果的消费：把 `BallFlightOutcome` 变成状态转移。
//!
//! 与 `ball_flight.rs` 的分工：那里只做**裁决与采样**（产出事实），
//! 本模块**消费**事实并触发球权转移、犯规与罚球程序。分开是因为消费路径
//! 需要按优先级短路本 tick（松球出界即结束），而短路的处理权属调度器。

use nba_domain::{court::Court, GameFlowState, SubPhase};
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
            new_ball_position,
            pending_events,
            final_free_throw_resolution,
            free_throw_attempts,
            steal_triggered_defender,
            loose_ball_secured_player,
            live_ball_triggered,
            loose_ball_out_of_bounds,
            drive_kickout_action,
            drive_pullup_action,
            blocked_shot_event,
        } = outcome;
        self.journal.pending_events.extend(pending_events);
        self.journal.pending_events.extend(free_throw_attempts);
        // 封盖事实：先把事件入队，后续平新的球态（松球）照常转移。
        //
        // 与发球次序无关：封盖本身不终结回合（球仍活，双方争夺松球），
        // 因此它不进回合总结也不切球权——那由后续的松球掌控路径决定。
        if let Some(event) = blocked_shot_event {
            self.journal.current_event = Some("BLOCKED_SHOT".to_string());
            if let nba_domain::GameEvent::BlockedShot {
                blocker_id,
                shooter_id,
                ..
            } = &event
            {
                self.fail_action_window(
                    shooter_id,
                    nba_domain::event::ActionFailureReason::Blocked,
                );
                // SHOT_RELEASE 是 FGA 的唯一入账点，封盖分支不重复计数。
                let display = self
                    .systems
                    .physics
                    .get_player(blocker_id)
                    .map(|p| format!("{}号", p.jersey))
                    .unwrap_or_else(|| blocker_id.clone());
                self.journal.current_callout = Some(format!("{} 拍下圆柱体，直接封盖！", display));
                self.journal.current_intensity = Some("Climax".to_string());
                // 被盖的出手同样是一次出手结果：出手者的连中/连铁必须被记录。
                // `record_shot` 是连中/连铁的唯一入口，`MoraleState::HotHand`
                // 的可达性依赖它；若跳过被封的出手，那条路径就会在士气
                // 状态机里凭空消失。
                if let Some(state) = self.observations.modulation.get_mut(shooter_id) {
                    state.record_shot(false);
                }
            }
            self.journal.pending_events.push(event);
        }
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
                nba_domain::ShotCreationSource::DrivePullUp,
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
                // 3D 路径长度决定交接时长：地板球拾起时垂直爬升与水平
                // 位移叠加，只按水平距离计时会让首帧 3D 速度越出包络
                // （实测 96–125 ft/s）。
                let distance = (target_pos - from_pos)
                    .length()
                    .hypot((target_z - from_z).abs());
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
            let is_free_throw = matches!(nbs, BallTrajectoryKind::FreeThrow { .. });
            self.transition_ball_state(nbs);
            if let Some(position) = new_ball_position {
                self.ball.ball_pos_3d = position;
            }
            if let Some((shooter_id, shooter_is_home, made, final_position)) =
                final_free_throw_resolution
            {
                self.possession_ctx.current_possession_shooter = Some(shooter_id);
                if made {
                    self.emit_possession_summary(
                        nba_domain::PossessionEndCause::Score,
                        None,
                        None,
                        None,
                    );
                    let inbound_pos = Court::hoop_pos(shooter_is_home);
                    self.start_inbound_transition(inbound_pos, final_position);
                }
            }
            if live_ball_triggered {
                self.transition_phase(SubPhase::Initiation);
                self.set_game_flow(GameFlowState::LiveBall);
                self.clock.last_decision_time = -self.config.rules.decision_interval_seconds;
            }
            if !is_free_throw && !matches!(self.ball.ball_state, BallTrajectoryKind::Held { .. }) {
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
