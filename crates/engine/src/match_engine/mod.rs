//! 比赛模拟编排层：阶段驱动 + 约束过滤 + 效用决策 + 事件化输出。
//!
//! 这是引擎的主循环（架构文档 §11），取代旧的分散 if 决策：
//! - 阶段状态机推进（Inbound → Transition/SetPlay → Resolution → Rebound/DeadBallReset）
//! - 世界级 Runtime 约束检查（24 秒等）
//! - 决策系统按约束管线产出动作意图
//! - 执行系统更新球弹道与物理
//! - 裁决系统处理抢断/得分/篮板与阶段转换

mod accessors;
mod action_windows;
mod ball_flight;
mod block;
mod bookkeeping;
mod construction;
mod contests;
mod decision;
mod events;
mod execution;
mod flow;
mod free_ball;
mod phases;
mod projection;
mod receiver;
mod roster;
mod runtime_phase;
mod state;
mod stream;
mod tactics_phase;
mod test_hooks;
mod transitions;
mod types;

pub use types::{ExportSummary, MatchBoxScore};

use phases::PhaseOutcome;
use stream::StreamPipeline;

pub use stream::{frame_rules_from_game_rules, StreamMode};

use crate::setup::MatchSetup;
use nba_domain::{GameFlowState, GameRules, Possession};
use nba_physics::ballistics::BallisticsEngine;
use nba_protocol::*;

#[derive(Debug, Clone, Copy)]
enum ScopeBoundary {
    Possessions,
    Period { last_period: u32 },
    Game,
}

/// 主模拟状态。每个 possession 是阶段事件流的最小产出单位。
///
/// ## 字段可见性（`current/plan.md` D7 / `gap.md` §20.1）
///
/// 除展示/审计所需的只读访问器外，所有状态字段均**私有**：外部只能经
/// `step()` / `snapshot()` / 显式命令推进与观察比赛，不能直接改写真相。
/// 需变可写的测试场景集中在显式命名的 `*_for_test` 钩子里。
pub struct MatchEngine {
    /// 比赛时钟（时间轴、节次、子阶段与各类计时器）。
    clock: state::MatchClock,
    /// 比赛流程（球权、宏观生命周期、作用域边界、发球基线）。
    flow: state::GameFlow,
    /// 球运行态（球位、球态、传球链路、接球人预判）。
    ball: state::BallRuntime,
    /// 队伍配置（规则、两队资料、战术档案与当前生效方案）。
    config: state::TeamConfig,

    /// 内部子系统与外部依赖（物理、决策、教练、随机源、镜像世界）。
    systems: state::Systems,
    /// 运行期观测（动作窗口、决策追踪、空间与接触、士气调制）。
    observations: state::RuntimeObservations,
    /// 事件日志（待发布事实、全场事件 ID、因果链、解说文案）。
    journal: state::EventJournal,
    /// 比分与犯规账本（得分、团队犯规、罚球程序、箱体统计）。
    ledger: state::ScoreLedger,
    /// 当前回合上下文（起点、传球数、出手人与干扰度、总结 index）。
    possession_ctx: state::PossessionContext,
    /// 校验账本：每 tick 的不变量校验器与最近一次运行的违反。
    audit: state::AuditTrail,
}

pub type Simulation = MatchEngine;

impl MatchEngine {
    pub fn new(seed: u64) -> Self {
        Self::with_setup(MatchSetup::builtin(GameRules::default()), seed)
    }

    pub fn with_rules(seed: u64, rules: GameRules) -> Self {
        Self::with_setup(MatchSetup::builtin(rules), seed)
    }

    pub fn is_finished(&self) -> bool {
        self.flow.game_flow == GameFlowState::GameEnd
            || (self.flow.scope_active && self.flow.simulation_complete)
    }

    fn break_finished(&self) -> bool {
        self.clock.period_break_elapsed + f32::EPSILON >= self.config.rules.period_break_seconds
    }

    /// Result of a scoped export: tick count, scope label, and any invariant
    /// violations detected across the run. Callers should treat a non-empty
    /// violation list as a failed simulation, not merely a warning.
    /// 按指定流模式导出一场比赛（gap.md §16.4 资源治理）。
    ///
    /// - [`StreamMode::Frames`]：逐 tick 完整帧（展示/回放，大）；
    /// - [`StreamMode::FramesGzip`]：同一完整帧协议的 gzip 存储格式；
    /// - [`StreamMode::Facts`]（默认）：只写因果事实、事件日志、阶段/生命周期
    ///   变化、回合总结与周期性检查点。引擎每 tick 自检 L1，流不必携带
    ///   每 tick 的球员投影，因此体积从数百 MB 降到个位数 MB；
    /// - [`StreamMode::Summary`]：只写回合总结与比赛级元数据，体积极小。
    ///
    /// 所有模式都在写入前检查磁盘空间与文件大小预算，超限即报错而不是
    /// 静默写满磁盘。
    pub fn simulate_scope_and_export_with_mode(
        &mut self,
        scope: &str,
        out_path: &str,
        mode: StreamMode,
    ) -> std::io::Result<ExportSummary> {
        StreamPipeline::new(mode).run(self, scope, out_path)
    }

    /// Advance one fixed step and validate the emitted frame.
    ///
    /// The invariant checker runs after every tick (including every early
    /// return inside `step_inner`) so that a physics/basketball violation is
    /// reported with the exact tick that produced it, instead of being
    /// discovered only by post-hoc log analysis.
    pub fn step(&mut self) -> StreamTick {
        let tick = self.step_inner();
        // P-1 观测差分：记录本 tick 的球位置，供接球人下一 tick 估计速度
        // （感知延迟一步，见 ball_velocity_estimate）。
        self.ball.prev_observed_ball_pos = Some(self.ball.ball_pos_3d.0);
        let violations = self.audit.invariant_checker.check_tick(&tick);
        // InvariantChecker owns the protocol-level causal graph; do not run a
        // second independent graph here, which would duplicate findings and
        // make the violation ledger depend on call-site history.
        self.audit.last_tick_violations = violations;
        for v in &self.audit.last_tick_violations {
            eprintln!("[INVARIANT] {}", v);
        }
        tick
    }

    fn check_early_tick_exit(&mut self) -> Option<StreamTick> {
        if self.flow.scope_active && self.flow.simulation_complete {
            self.journal.current_event = None;
            self.journal.current_event_types.clear();
            self.journal.current_enforcements.clear();
            self.journal.current_event_log.clear();
            self.observations.last_decision_trace = None;
            return Some(self.build_tick());
        }
        None
    }

    fn step_inner(&mut self) -> StreamTick {
        if let Some(tick) = self.check_early_tick_exit() {
            return tick;
        }
        self.clock.tick_index = self.clock.tick_index.saturating_add(1);
        self.observations.potential_field.clear();

        let dt = self.config.rules.tick_seconds;
        let was_tip_off = self.flow.game_flow == GameFlowState::TipOff;
        let was_period_break = matches!(
            self.flow.game_flow,
            GameFlowState::QuarterEnd | GameFlowState::Halftime
        );
        self.sync_game_flow();
        self.journal.current_event_log.clear();
        // D4.1：因果槽位跨 tick 存活（动作释放与结果发生常不同 tick：
        // 实测 PASS@t24 → PASS_RECEIVED@t29、FOUL@t84 → FREE_THROW@t86），
        // 因此不在每 tick 清空；槽位在对应动作窗口关闭/被新触发覆盖时
        // 自然失效，保证父指向最近的同槽位触发事件。
        self.sync_team_tactics();

        // Tip-off is a configurable dead-ball presentation phase. A zero
        // duration transitions in this same fixed step so the default policy
        // retains the historical first-tick behavior.
        if was_tip_off && self.tip_off_phase(dt) == PhaseOutcome::ShortCircuit {
            return self.build_tick();
        }
        if self.dead_flow_phase(dt, was_period_break) == PhaseOutcome::ShortCircuit {
            return self.build_tick();
        }
        self.clock_advance_phase(dt, was_tip_off);
        let current_t = self.clock.current_time;
        let is_home = self.flow.possession == Possession::Home;
        self.journal.begin_tick();
        self.journal.current_event_log.clear();
        self.observations.last_decision_trace = None;

        // 贴身切球（on-ball poke check）：为「带球丢球」提供事实路径；
        // 实现见 `runtime_phase.rs`。
        self.on_ball_poke_phase(dt);

        if self.runtime_constraint_phase(dt, is_home) == PhaseOutcome::ShortCircuit {
            return self.build_tick();
        }
        // Advance action windows in stable player-id order.
        self.advance_action_windows(current_t);
        if self.free_throw_and_period_phase(dt) == PhaseOutcome::ShortCircuit {
            return self.build_tick();
        }

        let decision_output = self.decision_phase(current_t);

        // ============================================================
        // 2. 执行决策输出（意图执行重校验，architecture.md §5.2）
        if let Some(out) = decision_output {
            self.apply_decision_output(out, current_t);
        }
        // Step the physical world before sampling the ball at this tick.
        self.systems.physics.step(nba_domain::FixedDt(dt));
        let sample_3d = BallisticsEngine::sample_ball_position(
            &self.ball.ball_state,
            current_t,
            self.systems.physics.get_players(),
            &self.config.rules,
        );
        self.ball.ball_pos_3d = sample_3d;

        // ============================================================
        // 3. 球弹道状态更新与拦截检查（阶段实现见 `ball_flight.rs`）
        let outcome = self.resolve_ball_flight(current_t, is_home, dt);
        if self.apply_ball_flight_outcome(outcome, current_t) == PhaseOutcome::ShortCircuit {
            return self.build_tick();
        }

        // ============================================================
        // 4. 战术目标生成 & 移动导航（每 tick）
        // ============================================================
        self.plan_tactics_and_navigation(current_t);
        self.collect_tick_facts(dt);

        self.publish_events();

        self.build_tick()
    }
}

fn team_name_zh(is_home: bool) -> &'static str {
    if is_home {
        "主队"
    } else {
        "客队"
    }
}
