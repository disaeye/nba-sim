//! 运行时约束与死球流程阶段：贴身切球请求、运行时约束求值、罚球执行与节末判定。
//!
//! 依据 `docs/architecture.md` §4.1 的阶段序列，本模块承载 `RuntimeConstraintPhase`
//! 与它之后的条件激活分支（`FreeThrowPhase` 的结算、节末与终场短路）。
//! 每个分支在本 tick 需要提前结束时返回 [`PhaseOutcome::ShortCircuit`]，
//! 由调度器统一发布事件与输出帧——阶段函数自身不做帧输出，也不吞掉事件。

use nba_domain::{GameEvent, GameFlowState};
use nba_decision::constraint::EnforcementAction;
use nba_physics::ballistics::BallTrajectoryKind;

use super::projection::physics_fact_to_event;
use super::{MatchEngine, PhaseOutcome};

impl MatchEngine {
    /// 运行时约束求值：违例即终结本 tick 的推进，交由调度器收尾。
    pub(crate) fn runtime_constraint_phase(&mut self, dt: f32, is_home: bool) -> PhaseOutcome {
        // Runtime constraints observe the current snapshot before execution.
        let ctx = self.constraint_ctx();
        let runtime_findings = self.systems.decision.registry.evaluate_runtime(&ctx);
        let runtime_violation = if matches!(
            self.flow.game_flow,
            GameFlowState::QuarterEnd | GameFlowState::Halftime | GameFlowState::GameEnd
        ) {
            None
        } else {
            runtime_findings.iter().find_map(|finding| {
                if let EnforcementAction::Violation { kind } = finding.enforcement {
                    Some((finding.constraint.id, kind))
                } else {
                    None
                }
            })
        };
        if let Some((constraint_id, kind)) = runtime_violation {
            self.journal
                .current_enforcements
                .push(format!("{}:{}", constraint_id, kind.as_str()));
            self.journal.pending_events.push(GameEvent::RuleViolation {
                constraint_id: constraint_id.to_string(),
                reason: kind.as_str().to_string(),
            });
            self.journal.current_event = Some("VIOLATION".to_string());
            let violation_callout =
                format!("{}！{} 失去球权", constraint_id, super::team_name_zh(is_home));
            self.start_violation_turnover(kind);
            // 确保违例判定帧忠实呈现哨响违例事实，不被随后的发球准备覆写
            self.journal.current_callout = Some(violation_callout);
            self.journal.current_event = Some("VIOLATION".to_string());
            self.systems.physics.step(nba_domain::FixedDt(dt));
            self.publish_events();
            return PhaseOutcome::ShortCircuit;
        }
        PhaseOutcome::Continue
    }

    /// 罚球结算与节末判定；两者都可能终结本 tick。
    pub(crate) fn free_throw_and_period_phase(&mut self, dt: f32) -> PhaseOutcome {
        let facts = self.systems.physics.drain_facts();
        if !facts.is_empty() {
            self.journal
                .pending_events
                .extend(facts.into_iter().map(physics_fact_to_event));
        }
        if self.ledger.free_throws_remaining > 0
            && self.flow.game_flow == GameFlowState::FreeThrow
            && self.clock.sub_phase_timer >= self.config.rules.free_throw_interval_seconds
        {
            self.resolve_free_throw();
            self.systems.physics.step(nba_domain::FixedDt(dt));
            self.publish_events();
            return PhaseOutcome::ShortCircuit;
        }
        if self.config.rules.period_expired(self.clock.game_clock)
            && matches!(
                self.flow.game_flow,
                GameFlowState::LiveBall | GameFlowState::Overtime | GameFlowState::TipOff
            )
        {
            let ball_live = matches!(
                self.ball.ball_state,
                BallTrajectoryKind::Shot { .. } | BallTrajectoryKind::RimRebound { .. }
            );
            if !ball_live {
                self.finish_period();
                self.systems.physics.step(nba_domain::FixedDt(dt));
                self.publish_events();
                return PhaseOutcome::ShortCircuit;
            }
        }
        if self.flow.game_flow == GameFlowState::GameEnd {
            self.publish_events();
            return PhaseOutcome::ShortCircuit;
        }
        PhaseOutcome::Continue
    }

    /// 贴身切球（on-ball poke check）：为「带球丢球」提供事实路径。
    ///
    /// 真实 NBA 失误构成中带球丢球占 53.6%（82games 2024-25 IND），
    /// 是占比最大的一类；此前引擎只有传球失败一条失误路径。
    ///
    /// 只在活球且球确实被持有时评估；`resolve_on_ball_poke` 内部按
    /// `rate × dt` 做时间积分，单 tick 概率不随时长累加。
    pub(crate) fn on_ball_poke_phase(&mut self, dt: f32) {
        if self.flow.game_flow.allows_live_ball_actions() {
            let carrier = self.carrier_id();
            let locked = self
                .systems
                .physics
                .get_player(&carrier)
                .map(|p| p.is_locked_kinematics)
                .unwrap_or(false);
            if let Some(defender_id) = self.resolve_on_ball_poke(&carrier, dt, locked) {
                self.apply_on_ball_poke(&carrier, &defender_id);
            }
        }
    }
}
