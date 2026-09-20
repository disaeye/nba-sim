//! 测试写入钩子：真相字段私有化后，测试构造场景所需的受控写入通道。
//!
//! 依据 `docs/architecture.md` §3.2 与 `current/plan.md` D7：生产路径只能经
//! `step()` 与只读访问器交互，写入意图必须在调用点以 `*_for_test` 命名显式可见。
//! 这些钩子只被 `crates/engine/tests/` 使用，集中在一处便于审计——
//! 它们绕过不变量检查，因此必须一眼可见，不与其他访问器混杂。

use glam::Vec2;
use nba_domain::court::Court;
use nba_domain::{GameEvent, GameFlowState, GameRules, SubPhase};
use nba_physics::ballistics::BallTrajectoryKind;
use nba_physics::movement::PhysicsWorld;
use std::collections::HashMap;

use nba_decision::modulation::PlayerModulationState;

use super::MatchEngine;

impl MatchEngine {
    pub fn new_possession_pg_for_test(&self) -> String {
        self.new_possession_pg()
    }

    #[doc(hidden)]
    pub fn execute_shot_for_test(&mut self, shooter_id: &str, from_pos: Vec2, is_three: bool) {
        self.execute_shot(
            shooter_id,
            from_pos,
            is_three,
            None,
            self.clock.current_time,
        );
    }

    /// 设置交替拥有箭头（仅测试钩子，D20）。
    pub fn set_possession_arrow_for_test(&mut self, arrow: Option<nba_domain::Possession>) {
        self.flow.possession_arrow = arrow;
    }

    /// 物理世界的可变访问（**仅测试**）。
    ///
    /// 生产路径不得直接改物理状态：位置/速度必须经引擎的阶段化推进；
    /// 直接改写会绕过不变量检查（例如把球员瞬移到界外）。属性扰动
    /// 测试需要它来构造对照场景，因此保留为显式的测试后门。
    #[doc(hidden)]
    pub fn physics_mut_for_test(&mut self) -> &mut PhysicsWorld {
        &mut self.systems.physics
    }

    /// 球员调制状态（只读）：士气/手感等跨 tick 状态。
    pub fn modulation_for_test(&self) -> &HashMap<String, PlayerModulationState> {
        &self.observations.modulation
    }

    /// 规则的可变访问（**仅测试**）。
    ///
    /// 生产路径不得在运行中改规则：规则应在构造时经 `with_rules` 固定，
    /// 否则同一场比赛内的行为会依赖“何时改的”而不只依赖输入。
    /// 机制对照测试（如关闭接球噪声）需要它。
    #[doc(hidden)]
    pub fn rules_mut_for_test(&mut self) -> &mut GameRules {
        &mut self.config.rules
    }

    pub fn force_possession_for_test(&mut self, possession: nba_domain::Possession) {
        self.flow.possession = possession;
    }

    // ==================== D4.2 测试写入钩子 ====================
    // 真相字段私有化后，测试的场景构造需要受控写入通道。为了让写入
    // 意图在调用点显式可见（而不是裸露的字段赋值），统一在此集中提供
    // `*_for_test` 设置器，命名即声明"这是测试场景构造，不是生产写入"。
    // 与 `force_possession_for_test` 同类：架构 §3.2 声明的测试豁免通道。

    #[doc(hidden)]
    pub fn set_game_flow_for_test(&mut self, flow: GameFlowState) {
        self.flow.game_flow = flow;
    }

    #[doc(hidden)]
    /// 测试后门：直接设置球态。
    ///
    /// 注意：本后门绕过 `transition_ball_state`（唯一写入口），因此必须
    /// **自行维护派生副作用**。当前需要维护的是接球人标记
    /// （`is_receiving_pass`，round-10）——否则用本后门构造的传球场景里，
    /// 接球人不会获得 APF 豁免与「到位即停」，造成与生产路径不一致的行为
    /// （实测：H_2 带 17.26 ft/s 初速滑离接球点，层 A 误判）。
    pub fn set_ball_state_for_test(&mut self, state: BallTrajectoryKind) {
        if let BallTrajectoryKind::Pass { target_id, .. } = &state {
            let rid = target_id.clone();
            self.mark_receiver(Some(&rid));
        } else {
            self.mark_receiver(None);
        }
        self.ball.ball_state = state;
    }

    #[doc(hidden)]
    pub fn set_ball_pos_for_test(&mut self, pos: Vec2, z: f32) {
        self.ball.ball_pos_3d = (pos, z);
    }

    #[doc(hidden)]
    pub fn push_event_for_test(&mut self, event: GameEvent) {
        self.journal.pending_events.push(event);
    }

    #[doc(hidden)]
    pub fn clear_pending_events_for_test(&mut self) {
        self.journal.pending_events.clear();
    }

    #[doc(hidden)]
    pub fn set_current_time_for_test(&mut self, seconds: f32) {
        self.clock.current_time = seconds;
    }

    #[doc(hidden)]
    pub fn set_game_clock_for_test(&mut self, seconds: f32) {
        self.clock.game_clock = seconds;
    }

    #[doc(hidden)]
    pub fn set_shot_clock_for_test(&mut self, seconds: f32) {
        self.clock.shot_clock = seconds;
    }

    #[doc(hidden)]
    pub fn set_sub_phase_for_test(&mut self, phase: SubPhase) {
        self.clock.sub_phase = phase;
    }

    #[doc(hidden)]
    pub fn set_last_passer_for_test(&mut self, id: Option<String>) {
        self.ball.last_passer_id = id;
    }

    #[doc(hidden)]
    pub fn set_backcourt_elapsed_for_test(&mut self, seconds: f32) {
        self.clock.backcourt_elapsed = seconds;
    }

    #[doc(hidden)]
    pub fn set_period_break_elapsed_for_test(&mut self, seconds: f32) {
        self.clock.period_break_elapsed = seconds;
    }

    #[doc(hidden)]
    pub fn set_scores_for_test(&mut self, home: u32, away: u32) {
        self.ledger.home_score = home;
        self.ledger.away_score = away;
    }

    #[doc(hidden)]
    pub fn set_possession_context_for_test(
        &mut self,
        turnover_player: Option<String>,
        start_clock: f32,
        start_time: f32,
    ) {
        self.possession_ctx.current_possession_turnover_player = turnover_player;
        self.possession_ctx.current_possession_start_clock = start_clock;
        self.possession_ctx.current_possession_start_time = start_time;
    }

    /// Test hook：直接启动一次发球转换，用于验证发球员界外 placement 的事实语义。
    ///
    /// 该后门绕过了正常得分/失误路径，因此必须显式补一条归因总结——
    /// `PossessionEndCause::TurnoverViolation` 是语义上最接近的合法原因
    /// （测试模拟的是“违例后发球”场景）；否则 `complete_possession` 的
    /// 因果 debug_assert 会正确拒绝这条无归因边界（dev 方案 §3.2 D0.1）。
    #[doc(hidden)]
    pub fn start_inbound_transition_for_test(&mut self) {
        let count = self.flow.completed_possessions as u64;
        if self.possession_ctx.last_possession_summary_index != Some(count) {
            self.emit_possession_summary(
                nba_domain::PossessionEndCause::TurnoverViolation,
                None,
                self.current_turnover_player_id(),
                None,
            );
        }
        let pos = self.ball.ball_pos_3d;
        let baseline = Court::nearest_boundary_with_geometry(pos.0, self.config.rules.court);
        self.start_inbound_transition(baseline, pos);
    }
}
