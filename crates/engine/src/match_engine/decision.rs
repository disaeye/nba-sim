//! 决策阶段：按子阶段决定是否产生一次运球决策。
//!
//! 依据 `docs/architecture.md` §5.1：决策管线是「感知 → 候选生成 → 硬约束过滤
//! → 物理可行性 → 软约束惩罚 → 效用评分 → softmax 采样」。本模块只负责**触发
//! 条件与上下文装配**：判断本 tick 是否到达决策间隔、把持球人的体力与士气偏置
//! 与当前约束上下文交给 `DecisionSystem`，产出意图供执行阶段实施。
//!
//! 意图不是命令：执行阶段会基于**当前**世界重新过一遍硬约束（`architecture.md`
//! §5.2 执行点重校验），因此这里产出的结果可能被降级或作废。
//!
//! `rng` 在调用前后经 `std::mem::replace` 换出再换回，避免把 `&mut self` 的
//! 其余字段借给决策系统时产生借用冲突；种子重放保证同种子逐 tick 一致（C4）。

use nba_decision::pipeline::DecisionOutput;
use nba_domain::{GameFlowState, Possession, SubPhase};
use nba_physics::ballistics::BallTrajectoryKind;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use super::MatchEngine;

impl MatchEngine {
    pub(crate) fn decision_phase(&mut self, current_t: f32) -> Option<DecisionOutput> {
        // Phase state machine: inbound decisions are evaluated only during the
        // inbound phase; live-ball decisions use the same registry pipeline.
        let mut decision_output: Option<DecisionOutput> = None;
        match self.clock.sub_phase {
            SubPhase::Initiation => {
                if self.flow.game_flow == GameFlowState::DeadBall {
                    // 发球阶段使用专用（更短）决策间隔：发球受 5 秒规则约束，
                    // 套用阵地节奏会与之竞速（实测 37% 发球被判五秒违例）。
                    if matches!(
                        self.ball.ball_state,
                        BallTrajectoryKind::InboundReady { .. }
                    ) && current_t - self.clock.last_decision_time
                        >= self.config.rules.inbound_decision_interval_seconds
                    {
                        let carrier = self.carrier_id();
                        let stamina = self
                            .systems
                            .physics
                            .get_player(&carrier)
                            .map(|p| (p.stamina / p.max_stamina.max(f32::EPSILON)).clamp(0.0, 1.0))
                            .unwrap_or(1.0);
                        let morale_bias = self.morale_bias_for(&carrier);
                        let mut rng =
                            std::mem::replace(&mut self.systems.rng, ChaCha8Rng::seed_from_u64(0));
                        let ctx = self.constraint_ctx();
                        decision_output = self.systems.decision.decide_on_ball(
                            &ctx,
                            &carrier,
                            stamina,
                            morale_bias,
                            &self.systems.coach,
                            &mut rng,
                        );
                        self.systems.rng = rng;
                    }
                } else {
                    // 第一性原理：后场推进不受「战术发起」延迟约束。
                    //
                    // 8 秒规则要求进攻方在 8 秒内把球推过中线，而
                    // `tactical_initiation_seconds = 6.5s` 会让球队在后场
                    // 干等到 6.5s 才首次决策，只剩 1.5s 窗口（决策间隔
                    // 2.4s）—— 实测因此产生 31 次/场 8 秒违例，球 x 在
                    // 8 秒内只从 11.4 移到 12.8 ft（需越过 47）。
                    //
                    // 半场阵地进攻才需要「战术发起」等待；后场是转换推进，
                    // 必须立即允许决策（`Advance` 候选随即可用）。
                    let in_backcourt = {
                        let midcourt = self.config.rules.court.width_ft / 2.0;
                        match self.flow.possession {
                            Possession::Home => self.ball.ball_pos_3d.0.x < midcourt,
                            Possession::Away => self.ball.ball_pos_3d.0.x > midcourt,
                        }
                    };
                    if in_backcourt
                        || self.clock.sub_phase_timer
                            >= self.config.rules.tactical_initiation_seconds
                    {
                        self.transition_phase(SubPhase::ActionExecution);
                        self.set_game_flow(GameFlowState::LiveBall);
                        self.journal.current_event = Some("TACTICAL_EXECUTION".to_string());
                        self.journal.current_callout =
                            Some(format!("战术发起：{}", self.config.tactical_set.name_zh()));
                    }
                }
            }
            SubPhase::ActionExecution => {
                if current_t - self.clock.last_decision_time
                    >= self.config.rules.decision_interval_seconds
                {
                    let carrier = self.carrier_id();
                    let stamina = self
                        .systems
                        .physics
                        .get_player(&carrier)
                        .map(|p| (p.stamina / p.max_stamina.max(f32::EPSILON)).clamp(0.0, 1.0))
                        .unwrap_or(1.0);
                    let morale_bias = self.morale_bias_for(&carrier);
                    let mut rng =
                        std::mem::replace(&mut self.systems.rng, ChaCha8Rng::seed_from_u64(0));
                    let ctx = self.constraint_ctx();
                    decision_output = self.systems.decision.decide_on_ball(
                        &ctx,
                        &carrier,
                        stamina,
                        morale_bias,
                        &self.systems.coach,
                        &mut rng,
                    );
                    self.systems.rng = rng;
                }
            }
            SubPhase::ShotAttempt | SubPhase::FlightAndRebound | SubPhase::DeadBallReset => {}
        }
        decision_output
    }
}
