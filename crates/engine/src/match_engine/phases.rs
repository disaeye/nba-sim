//! 生命周期阶段：跳球、节间、暂停与终场的阶段函数。
//!
//! 依据 `docs/architecture.md` §4.1：阶段函数返回 [`PhaseOutcome`] 表达
//! 「本 tick 是否在此短路」，由调度器决定后续动作，而不是就地 `return`。
//! 这样「顺序」与「短路」都成为调度器可见的事实。

use glam::Vec2;
use nba_domain::{GameFlowState, Possession, SubPhase};
use nba_physics::ballistics::BallTrajectoryKind;

use super::MatchEngine;

/// 阶段执行结果：本 tick 是否应在此结束。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PhaseOutcome {
    /// 进入下一阶段。
    Continue,
    /// 本 tick 提前结束，交由调度器发布事件并输出帧。
    ShortCircuit,
}

impl MatchEngine {
    /// 时钟推进：比赛时间、子阶段计时、后场计时、进攻钟与节间计时。
    /// 写入状态组：clock。
    ///
    /// 依据 `gap.md` §4.1：八秒违例是「连续停留在后场」的计时，进攻方一旦
    /// 持球越过中线，后场计时必须清零并从 0 重新计；前场判定复用领域层
    /// `is_backcourt` 的同一几何定义。
    pub(crate) fn clock_advance_phase(&mut self, dt: f32, was_tip_off: bool) {
        // 节间休息（architecture.md §2：停表期间比赛/进攻钟不推进）：
        // 本阶段只累计节间计时，由 `dead_flow_phase` 的 sync 判定节间结束。
        if matches!(
            self.flow.game_flow,
            GameFlowState::QuarterEnd | GameFlowState::Halftime
        ) {
            self.clock.period_break_elapsed += dt;
            return;
        }
        if !was_tip_off {
            self.clock.current_time += dt;
            self.clock.sub_phase_timer += dt;
        }
        if self.clock.sub_phase == SubPhase::Initiation
            && self.flow.game_flow == GameFlowState::DeadBall
            && matches!(
                self.ball.ball_state,
                BallTrajectoryKind::InboundReady { .. }
            )
        {
            self.clock.inbound_elapsed += dt;
        } else if self.clock.sub_phase != SubPhase::DeadBallReset {
            self.clock.inbound_elapsed = 0.0;
        }
        if self.flow.game_flow.allows_live_ball_actions() {
            let attacking_right = self.flow.possession == Possession::Home;
            // 控球谓词与决策侧 ConstraintContext::offense_has_possession 同一口径：
            // 球无主（松球/篮板/停球）或已出手时不累加——否则会对无主球计时，
            // 第 8 秒判出没有控球方的伪违例（seed 14 第 4 节开场实测：
            // 201 tick 内球不可取，计时照走）。
            // 发球例外：发球飞行（`pending_pass_inbound`）期间无球队控制，
            // 回场/前场建立与三秒计时都不适用（charter §6.1/§6.2：发球
            // 可以传向任何方向）。
            let inbounds_pass_in_flight =
                matches!(self.ball.ball_state, BallTrajectoryKind::Pass { .. })
                    && self.ball.pending_pass_inbound;
            let offense_in_control = !inbounds_pass_in_flight
                && matches!(
                    self.ball_phase(),
                    nba_domain::BallPhase::Held
                        | nba_domain::BallPhase::Drive
                        | nba_domain::BallPhase::ControlTransfer
                        | nba_domain::BallPhase::PassFlight
                );
            let in_backcourt = self
                .config
                .rules
                .court
                .is_backcourt(self.ball.ball_pos_3d.0, attacking_right);
            if in_backcourt && offense_in_control {
                self.clock.backcourt_elapsed += dt;
            } else if !in_backcourt {
                // 越过中线：推进义务完成，恢复常规战术站位。
                self.clock.backcourt_elapsed = 0.0;
                self.observations.advancing_player = None;
            }
            // 球在后场但无控球方：计时保持原值（规则语义：控球中断暂停计数，
            // 重新建立控制后继续），不清零也不累加。

            // 回场违例（charter §6.2）：前场控制建立后球回到后场。
            // 判据与后场计时同一口径：控球延续（持球/交接/传球飞行）期间
            // 球在前场 → 建立；建立后球回后场 → 由约束层判违例。
            if !in_backcourt && offense_in_control {
                if !self.clock.frontcourt_established {
                    self.clock.frontcourt_established = true;
                    self.clock.frontcourt_seconds = f32::from(0u8);
                }
                self.clock.frontcourt_seconds += dt;
            } else if !offense_in_control {
                self.clock.frontcourt_established = false;
                self.clock.frontcourt_seconds = f32::from(0u8);
            } else if in_backcourt {
                self.clock.frontcourt_seconds = f32::from(0u8);
            }

            // 攻方三秒（charter §6.2）：前场控制时累加进攻方场上球员在
            // 限制区的连续停留。控球结束（出手/失球）或球回后场时清零；
            // 正在运球突破的持球人（Drive 态）不计数（规则语义：正运球
            // 或立即攻框的宽容条款）。
            let frontcourt_control =
                !in_backcourt && offense_in_control && self.clock.frontcourt_established;
            if frontcourt_control {
                let carrier_id: Option<String> = match &self.ball.ball_state {
                    BallTrajectoryKind::Held { carrier_id }
                    | BallTrajectoryKind::Drive {
                        driver_id: carrier_id,
                        ..
                    }
                    | BallTrajectoryKind::ControlTransfer { carrier_id, .. } => {
                        Some(carrier_id.clone())
                    }
                    _ => None,
                };
                // 宽容条款（charter §6.2）：正运球或立即攻框的进攻人不
                // 计数（含无球）——掩护顺下、背切、运球攻框与上篮扣篮
                // 抛投都是攻框移动；原地停车类动作不在此列。动作窗口
                // （顺下/背切/补篮/上篮等）执行期同样豁免：窗口锁定期
                // 内玩家运动学被锁定，不是「停留」。
                let rim_attack_exempt = |id: &str, is_carrier: bool| -> bool {
                    if let Some(p) = self.systems.physics.get_player(id) {
                        if !is_carrier
                            && !matches!(
                                p.action.as_str(),
                                "ROLL_TO_RIM"
                                    | "PLAY_ScreenRoll"
                                    | "PLAY_CutBackdoor"
                                    | "BACKDOOR_CUT"
                                    | "DIP_TO_RIM"
                                    | "RECEIVE_CUT"
                            )
                            && !self.observations.active_windows.contains_key(id)
                        {
                            return false;
                        }
                        matches!(
                            p.action.as_str(),
                            "ROLL_TO_RIM"
                                | "PLAY_ScreenRoll"
                                | "PLAY_CutBackdoor"
                                | "BACKDOOR_CUT"
                                | "DIP_TO_RIM"
                                | "DriveToBasket"
                                | "Layup"
                                | "Dunk"
                                | "Floater"
                                | "RECEIVE_CUT"
                        ) || self.observations.active_windows.contains_key(id)
                    } else {
                        false
                    }
                };
                let carrier_attacking_rim = carrier_id
                    .as_deref()
                    .is_some_and(|id| rim_attack_exempt(id, true));
                let attacking_team = if attacking_right { "home" } else { "away" };
                let on_court: Vec<(String, bool)> = self
                    .systems
                    .physics
                    .get_players()
                    .values()
                    .filter(|p| p.on_court && p.team == attacking_team)
                    .map(|p| {
                        (
                            p.id.clone(),
                            self.config
                                .rules
                                .court
                                .is_in_lane(p.pos_ft, attacking_right),
                        )
                    })
                    .collect();
                let on_court_ids: std::collections::HashSet<String> =
                    on_court.iter().map(|(id, _)| id.clone()).collect();
                for (id, in_lane) in on_court {
                    let exempt = (carrier_attacking_rim && Some(&id) == carrier_id.as_ref())
                        || rim_attack_exempt(&id, false);
                    if in_lane && !exempt {
                        let dwell = self.clock.lane_dwell.entry(id).or_insert(f32::from(0u8));
                        *dwell += dt;
                    } else {
                        self.clock.lane_dwell.remove(&id);
                    }
                }
                // 被换下的进攻球员不参与计数（换下即中断连续停留）。
                self.clock
                    .lane_dwell
                    .retain(|id, _| on_court_ids.contains(id));
            } else {
                self.clock.lane_dwell.clear();
            }
            if self.clock.game_clock > 0.0 {
                self.clock.game_clock = (self.clock.game_clock - dt).max(0.0);
            }
            if !matches!(
                self.ball.ball_state,
                BallTrajectoryKind::Shot { .. }
                    | BallTrajectoryKind::FreeThrow { .. }
                    | BallTrajectoryKind::RimRebound { .. }
            ) && self.clock.shot_clock > 0.0
            {
                self.clock.shot_clock = (self.clock.shot_clock - dt).max(0.0);
            }
        }
        self.clock.period_break_elapsed = 0.0;
    }

    /// 跳球阶段：可配置时长的死球表现阶段，零时长在同一固定步内直接转换。
    /// 写入状态组：clock, flow, ball, journal, observations, systems。
    pub(crate) fn tip_off_phase(&mut self, dt: f32) -> PhaseOutcome {
        // Tip-off is a configurable dead-ball presentation phase. A zero
        // duration transitions in this same fixed step so the default policy
        // retains the historical first-tick behavior.
        self.clock.current_time += dt;
        self.clock.sub_phase_timer += dt;
        if self.config.rules.tip_off_duration_seconds > 0.0 {
            if self.clock.sub_phase_timer + f32::EPSILON
                < self.config.rules.tip_off_duration_seconds
            {
                self.publish_events();
                self.systems.physics.reset_motion();
                let center_x = self.config.rules.court.width_ft * 0.5;
                let center_y = self.config.rules.court.height_ft * 0.5;
                let total_time = self.config.rules.tip_off_duration_seconds;
                let progress = (self.clock.sub_phase_timer / total_time).clamp(0.0, 1.0);
                let peak_z = 11.5;
                let base_z = 4.0;
                let z = base_z + 4.0 * (peak_z - base_z) * progress * (1.0 - progress);
                self.ball.ball_pos_3d = (Vec2::new(center_x, center_y), z);
                if self.clock.sub_phase_timer <= dt + f32::EPSILON {
                    self.journal.current_event = Some("TIPOFF".to_string());
                    self.journal.current_event_types = vec!["TIPOFF".to_string()];
                } else {
                    self.journal.current_event = None;
                    self.journal.current_event_types.clear();
                }
                self.journal.current_callout =
                    Some("裁判中圈垂直抛球，双方中锋起跳争顶！".to_string());
                self.journal.current_enforcements.clear();
                self.observations.last_decision_trace = None;
                return PhaseOutcome::ShortCircuit;
            }
            // 严格遵循宪章 Positionless 原则：跳球代表绝无固定位置硬编码，
            // 由场上摸高上限最高的球员（身高 height_cm + 垂直弹跳 vertical）纯函数涌现产生
            let home_jumper_id = self.select_jumper_id(Possession::Home);
            let away_jumper_id = self.select_jumper_id(Possession::Away);
            let winner_is_home = self.flow.possession == Possession::Home;
            let tapping_player = if winner_is_home {
                &home_jumper_id
            } else {
                &away_jumper_id
            };
            let center_x = self.config.rules.court.width_ft * 0.5;
            let center_y = self.config.rules.court.height_ft * 0.5;
            let tap_target = if winner_is_home {
                Vec2::new(center_x - 14.0, center_y)
            } else {
                Vec2::new(center_x + 14.0, center_y)
            };
            self.set_game_flow(GameFlowState::LiveBall);
            if let Some(p) = self.systems.physics.get_player_mut(&home_jumper_id) {
                p.target_pos_ft = Vec2::new(center_x - 3.0, center_y);
            }
            if let Some(p) = self.systems.physics.get_player_mut(&away_jumper_id) {
                p.target_pos_ft = Vec2::new(center_x + 3.0, center_y);
            }
            let tap_dir = (tap_target - Vec2::new(center_x, center_y)).normalize();
            self.transition_ball_state(BallTrajectoryKind::LooseBall {
                pos: Vec2::new(center_x, center_y),
                vel: tap_dir * 28.0,
                z: 5.5,
                vel_z: 6.0,
                last_touch_team: if winner_is_home {
                    Possession::Home
                } else {
                    Possession::Away
                },
                // 物理最后触球人是点拍赢球方的跳球员。
                last_touch_player: Some(tapping_player.clone()),
            });
            self.journal.current_event_types = vec!["TIPOFF_SECURED".to_string()];
            self.journal.current_callout = Some(format!(
                "{} 起跳率先触球，将球点拍向后场！第一攻展开！",
                tapping_player
            ));
            return PhaseOutcome::ShortCircuit;
        }
        self.set_game_flow(GameFlowState::LiveBall);
        self.transition_phase(SubPhase::Initiation);
        self.journal.current_callout = Some("比赛开始，跳球后主队获得第一攻球权。".to_string());
        PhaseOutcome::Continue
    }

    /// 节间、暂停与终场阶段。
    /// 写入状态组：clock, flow, ball, ledger, journal, observations
    /// （节间结束经 sync_game_flow 触发跨组转场）。
    ///
    /// `was_period_break` 表示本 tick 起点处于节间；节间结束当 tick 需要
    /// 发布积压事件后输出帧。暂停与终场为纯展示帧，不推进任何状态。
    pub(crate) fn dead_flow_phase(&mut self, was_period_break: bool) -> PhaseOutcome {
        if was_period_break
            && !matches!(
                self.flow.game_flow,
                GameFlowState::QuarterEnd | GameFlowState::Halftime
            )
        {
            self.journal.current_event_types.clear();
            self.journal.current_enforcements.clear();
            self.observations.last_decision_trace = None;
            self.publish_events();
            return PhaseOutcome::ShortCircuit;
        }
        if matches!(
            self.flow.game_flow,
            GameFlowState::Timeout | GameFlowState::GameEnd
        ) {
            self.journal.current_event = None;
            self.journal.current_event_types.clear();
            self.journal.current_enforcements.clear();
            self.observations.last_decision_trace = None;
            return PhaseOutcome::ShortCircuit;
        }
        if matches!(
            self.flow.game_flow,
            GameFlowState::QuarterEnd | GameFlowState::Halftime
        ) {
            // 节间计时已在 clock_advance_phase 累计，此处只负责判定与转场。
            self.sync_game_flow();
            if matches!(
                self.flow.game_flow,
                GameFlowState::QuarterEnd | GameFlowState::Halftime
            ) {
                self.journal.current_event = None;
                self.journal.current_event_types.clear();
                self.journal.current_enforcements.clear();
                self.observations.last_decision_trace = None;
                return PhaseOutcome::ShortCircuit;
            }
            self.journal.current_event_types.clear();
            self.journal.current_enforcements.clear();
            self.observations.last_decision_trace = None;
            self.publish_events();
            return PhaseOutcome::ShortCircuit;
        }
        PhaseOutcome::Continue
    }
}
