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
mod business;
mod construction;
mod contests;
mod decision;
mod events;
mod execution;
mod flow;
mod free_ball;
mod inbound_safety;
mod perturbation;
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
#[derive(Clone)]
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

pub use perturbation::ValidationAttribute;

impl MatchEngine {
    pub fn new(seed: u64) -> Self {
        Self::with_setup(MatchSetup::builtin(GameRules::default()), seed)
    }

    pub fn with_rules(seed: u64, rules: GameRules) -> Self {
        Self::with_setup(MatchSetup::builtin(rules), seed)
    }

    /// 校验专用随机源重播种（validation.md §7.1 具名随机键）。
    ///
    /// 具名随机键 `(experiment_id, checkpoint_id, subsystem, trial_id,
    /// draw_index)` 哈希为 ChaCha8 种子；档位组共享同一键（键里无档位），
    /// 使基线与三档在相同情形上天然配对。`subsystem` 参与哈希——不同
    /// 子系统的抽样相互独立，同键才同随机流。
    pub fn reseed_validation_rng(
        &mut self,
        experiment_id: &str,
        checkpoint_id: u64,
        subsystem: &str,
        trial_id: u64,
        draw_index: u64,
    ) {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        experiment_id.hash(&mut hasher);
        checkpoint_id.hash(&mut hasher);
        subsystem.hash(&mut hasher);
        trial_id.hash(&mut hasher);
        draw_index.hash(&mut hasher);
        let derived = hasher.finish();
        use rand::SeedableRng;
        let new_rng = rand_chacha::ChaCha8Rng::seed_from_u64(derived);
        self.systems.rng = new_rng;
    }

    /// 校验专用扰动入口（validation.md §7.1：恢复后只修改目标输入，
    /// 并通过生产初始化逻辑更新其派生量）。
    ///
    /// 走生产路径的含义：属性写入与 `PlayerAttributes::validate` 同一套
    /// 校验规则；派生量（物理上限）经 `nba_domain::effective_*` 映射层
    /// 重算，与 `construction.rs` 的初始化代码同源。禁止旁路直写内部字段。
    ///
    /// 返回旧值（供实验组恢复或核对档位）。球员不存在或属性越界直接
    /// panic（快崩语义：实验配置错误不应被吞掉）。
    pub fn apply_validation_perturbation(
        &mut self,
        player_id: &str,
        attribute: ValidationAttribute,
        value: f32,
    ) -> f32 {
        let teams = [
            (&mut self.config.home_team, "home"),
            (&mut self.config.away_team, "away"),
        ];
        for (team, _side) in teams {
            if let Some(player) = team.players.iter_mut().find(|p| p.id == player_id) {
                let old = attribute.get(&player.attributes);
                attribute.set(&mut player.attributes, value);
                // 生产校验路径（与 setup.rs 的名册校验同一实现）。
                player
                    .attributes
                    .validate()
                    .unwrap_or_else(|error| panic!("扰动后属性非法: {error}"));
                // 派生量重算：物理后端的运动上限（生产映射层同源）。
                if let Some(state) = self.systems.physics.get_player_mut(player_id) {
                    state.max_speed_ftps =
                        nba_domain::effective_max_speed(&self.config.rules, &player.attributes);
                    state.max_accel_ftps2 =
                        nba_domain::effective_max_accel(&self.config.rules, &player.attributes);
                    // 调小速度类属性后上限随之变小，当前速度若仍按旧上限
                    // 积分会立即击穿物理快崩断言；把当前速度钳回新上限
                    // （实验起点状态约束到新限制内，与「扰动从现在起生效」
                    // 语义一致）。
                    let speed = state.vel_ft.length();
                    if speed > state.max_speed_ftps {
                        state.vel_ft *= state.max_speed_ftps / speed;
                    }
                }
                return old;
            }
        }
        panic!("扰动目标球员不存在: {player_id}");
    }

    /// 校验用检查点分叉（validation.md §7.1 直接机制实验）：
    /// 位精确复制当前比赛状态，返回可独立 `step()` 的副本。
    ///
    /// 与母局的关系：fork 不加任何扰动并继续跑，后续事件流必须与母局
    /// 逐字节一致（位精确续跑检查是 fork 实现的验收闸，见 validation
    /// 集成测试）。外部仍摸不到状态字段——fork 复制整场比赛，符合
    /// D7「外部不改写比赛真相」；对副本的扰动只能经校验专用入口
    /// （`apply_validation_perturbation`，走生产属性路径）。
    pub fn experimental_fork(&self) -> Self {
        self.clone()
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
        self.observations.possession_ticks = self.observations.possession_ticks.saturating_add(1);
        self.advance_active_play();

        let dt = self.config.rules.tick_seconds;
        let was_tip_off = self.flow.game_flow == GameFlowState::TipOff;
        let was_period_break = matches!(
            self.flow.game_flow,
            GameFlowState::QuarterEnd | GameFlowState::Halftime
        );
        self.sync_game_flow();
        self.journal.current_event_log.clear();
        self.sync_team_tactics();

        // 阶段序列依据 docs/architecture.md §4.1：时钟与生命周期先于一切
        // 约束求值；跳球与死球流程是条件激活分支，位于罚球/节末之后。
        // `was_tip_off` 期间时钟不推进（跳球表现阶段自行推进时间，保持
        // 跳球期的时间语义不变）。
        // 写入状态组：clock。
        self.clock_advance_phase(dt, was_tip_off);
        let current_t = self.clock.current_time;
        let is_home = self.flow.possession == Possession::Home;
        self.journal.begin_tick();
        self.journal.current_event_log.clear();
        self.observations.last_decision_trace = None;

        // 贴身切球（on-ball poke check）：为「带球丢球」提供事实路径；
        // 实现见 `runtime_phase.rs`。写入状态组：ball, journal。
        self.on_ball_poke_phase(dt);

        if self.runtime_constraint_phase(dt, is_home) == PhaseOutcome::ShortCircuit {
            return self.build_tick();
        }
        // 动作窗口推进。写入状态组：ball, observations, systems, journal。
        self.advance_action_windows(current_t);
        if self.free_throw_and_period_phase(dt) == PhaseOutcome::ShortCircuit {
            return self.build_tick();
        }

        // Tip-off is a configurable dead-ball presentation phase. A zero
        // duration transitions in this same fixed step so the default policy
        // retains the historical first-tick behavior.
        if was_tip_off && self.tip_off_phase(dt) == PhaseOutcome::ShortCircuit {
            return self.build_tick();
        }
        if self.dead_flow_phase(was_period_break) == PhaseOutcome::ShortCircuit {
            return self.build_tick();
        }

        // 帧锚点：上一 tick 的最终球位（帧间连续性的比对基准）。
        // 必须在决策阶段之前捕获：决策里的球位写入（如突破起手拉回
        // from_pos）与随后的采样写入各自不超限，但同一 tick 内叠加
        // 可击穿帧间包络（实测姿态确立帧 89.6 ft/s）。
        let frame_anchor_ball_pos = self.ball.ball_pos_3d;

        let decision_output = self.decision_phase(current_t);

        // 执行重校验与应用动作（architecture.md §5.2）。
        // 写入状态组：ball, observations, systems, journal。
        if let Some(out) = decision_output {
            self.apply_decision_output(out, current_t);
        }
        // 物理步进与弹道采样。写入状态组：ball, systems。
        let sampled_from_ball_state = self.ball.ball_state.clone();
        let previous_ball_position = frame_anchor_ball_pos;
        // Step the physical world before sampling the ball at this tick.
        self.systems
            .physics
            .step_with_ball_holder(nba_domain::FixedDt(dt), self.ball.holder_id());
        let sample_3d = BallisticsEngine::sample_ball_position(
            &self.ball.ball_state,
            current_t,
            self.systems.physics.get_players(),
            &self.config.rules,
        );
        self.ball.ball_pos_3d = sample_3d;
        // 弹道裁决与拦截检查（阶段实现见 `ball_flight.rs`）。
        // 写入状态组：ball, observations。
        let outcome = self.resolve_ball_flight(current_t, is_home, dt);
        // 持球族连续性（R1）：同一持球人在 Held/Drive 内延续或转换时，
        // 球位相邻帧位移不得超过球速包络。姿态确立便 facing 单帧翻转，
        // 持球偏移参考系换轴（Held→Drive 转换 tick 实测 89.6 ft/s > 86.5），
        // 转换 tick 同样在锥制范围内。
        let ball_carrier_id = |state: &nba_physics::ballistics::BallTrajectoryKind| match state {
            nba_physics::ballistics::BallTrajectoryKind::Held { carrier_id }
            | nba_physics::ballistics::BallTrajectoryKind::Drive {
                driver_id: carrier_id,
                ..
            } => Some(carrier_id.clone()),
            _ => None,
        };
        let held_family_continues = ball_carrier_id(&sampled_from_ball_state).is_some()
            && ball_carrier_id(&sampled_from_ball_state) == ball_carrier_id(&self.ball.ball_state);
        if held_family_continues {
            let previous_3d = glam::Vec3::new(
                previous_ball_position.0.x,
                previous_ball_position.0.y,
                previous_ball_position.1,
            );
            let current_3d = glam::Vec3::new(
                self.ball.ball_pos_3d.0.x,
                self.ball.ball_pos_3d.0.y,
                self.ball.ball_pos_3d.1,
            );
            let displacement = current_3d - previous_3d;
            let max_displacement = self.config.rules.ball_max_speed_ftps * dt;
            if displacement.length() > max_displacement {
                let bounded = previous_3d + displacement.normalize_or_zero() * max_displacement;
                self.ball.ball_pos_3d = (glam::Vec2::new(bounded.x, bounded.y), bounded.z);
            }
        }
        // 弹道结果消费：出界/抢断/松球/新球态。
        // 写入状态组：flow, ball, ledger, possession_ctx, observations, journal, systems。
        if self.apply_ball_flight_outcome(outcome, current_t) == PhaseOutcome::ShortCircuit {
            return self.build_tick();
        }

        // 战术目标生成 & 移动导航（每 tick）。
        // 写入状态组：systems, observations。
        self.plan_tactics_and_navigation(current_t);
        // 到位与回位判定（validation.md §4.2）：意图终态在目标重算后
        // 结算，避免把本 tick 刚下达的任务立即判超时。
        self.settle_movement_intents();
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
