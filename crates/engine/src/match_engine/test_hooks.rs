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

    #[doc(hidden)]
    pub fn execute_drive_for_test(
        &mut self,
        driver_id: &str,
        from_pos: Vec2,
        target_pos: Vec2,
        move_kind: Option<nba_domain::action_window::DribbleMoveKind>,
    ) {
        self.execute_drive(
            driver_id,
            from_pos,
            target_pos,
            move_kind,
            self.clock.current_time,
        );
    }

    /// 实证探针用：突破犯规（DRIVE_FOUL）累计计数。
    pub fn drive_foul_count(&self) -> usize {
        self.observations.drive_foul_counter
    }

    /// 实证探针用：突破发起计数。
    pub fn drive_initiated_count(&self) -> usize {
        self.observations.drive_initiated_counter
    }

    /// 实证探针用：指定球员的 openness 采样（最近防守距离、干扰强度）。
    pub fn openness_probe(&self, player_id: &str) -> (f32, f32) {
        let o = self.systems.physics.openness(player_id);
        (o.closest_defender_dist, o.contest_intensity)
    }

    /// 实证探针用：只读快照挂起的投篮释放（shooter_id, release_time）。
    pub fn peek_pending_shot(&self) -> Option<(String, f32)> {
        self.observations
            .pending_shot_release
            .as_ref()
            .map(|p| (p.shooter_id.clone(), p.release_time))
    }

    /// 实证探针用：指定球员的动作窗口相位名（无窗口时 None）。
    pub fn window_phase(&self, player_id: &str) -> Option<String> {
        self.observations
            .active_windows
            .get(player_id)
            .map(|w| format!("{:?}", w.phase))
    }

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
            // 构造新的传球飞行即视为新的一次出手，清空连续接触状态。
            self.ball.pass_contact_states.clear();
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

    /// 当前激活的进攻 Play（只读，仅测试）：集成测试用它断言 Play 生命周期。
    #[doc(hidden)]
    pub fn active_play_id_for_test(&self, possession: nba_domain::Possession) -> Option<String> {
        let active = match possession {
            nba_domain::Possession::Home => &self.observations.home_active_play,
            nba_domain::Possession::Away => &self.observations.away_active_play,
        };
        active.as_ref().map(|play| play.spec.id.clone())
    }

    /// 主队激活簿记条目（只读，仅测试）：冷却/激活阶段的断言入口。
    #[doc(hidden)]
    pub fn home_play_book_entries_for_test(&self) -> Vec<nba_decision::PlayBookEntry> {
        self.observations.home_play_activation_book.entries()
    }

    /// 最近一次决策调试投影（只读，仅测试）：同 tick Play 效用断言入口。
    #[doc(hidden)]
    pub fn last_decision_trace_for_test(&self) -> Option<&nba_protocol::DecisionDebug> {
        self.observations.last_decision_trace.as_deref()
    }

    /// 当前球权（只读，仅测试）：集成测试驱动回合翻转断言。
    #[doc(hidden)]
    pub fn possession_for_test(&self) -> nba_domain::Possession {
        self.possession()
    }

    /// 当前持球人（只读，仅测试）：测试用它把球态装配为 Held。
    #[doc(hidden)]
    pub fn carrier_id_for_test(&self) -> String {
        self.carrier_id()
    }

    /// 挂起投篮的出手者（只读，仅测试）：Release 时序闭环的泄漏探针。
    #[doc(hidden)]
    pub fn pending_release_shooter_for_test(&self) -> Option<String> {
        self.observations
            .pending_shot_release
            .as_ref()
            .map(|pending| pending.shooter_id.clone())
    }
}
