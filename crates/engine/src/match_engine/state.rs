//! `MatchEngine` 的具名状态组。
//!
//! 依据 `docs/architecture.md` §4.3：82 个字段按**写入点的同现关系**（同一函数同时
//! 写入的字段归入同组）收敛为十个具名组。组的字段用 `pub(crate)` 暴露给同一 crate
//! 的兄弟模块（Rust 的字段可见性以模块为界，`state.rs` 的私有字段对它以外的模块不可见），
//! 组结构体自身不 `pub`；`MatchEngine` 仍保持零 `pub` 字段（`scripts/check_world_privacy.py`
//! 的判据）。
//!
//! 实测边界：现有阶段函数仍接收 `&mut self`（见 ADR-014 的取舍记录），状态组
//! 先给出「哪个函数写了哪些字段」的可读归属，窄签名由签名中列出的组名表达。
//! 状态组由 `scripts/check_engine_state_groups.py` 守卫（零裸字段、组不泄出 crate、
//! `mod.rs` ≤ 400 行）。

use nba_invariants::{InvariantChecker, Violation};

use nba_decision::modulation::{CoachStrategy, PlayerModulationState};
use nba_decision::pipeline::DecisionSystem;
use nba_decision::tactics::{DefensiveTactic, TacticalSet};
use nba_domain::action_window::ActionTimeWindow;
use nba_domain::{GameFlowState, Possession, SubPhase};
use nba_protocol::DecisionDebug;
use nba_semantics::{SemanticContact, SpacingEvaluation};
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;

use nba_physics::ballistics::BallTrajectoryKind;
use nba_physics::movement::PhysicsWorld;

use super::types::MatchBoxScore;

/// 内部子系统与外部依赖：物理后端、决策管线、教练策略与随机源。
///
/// 这四个字段都是构造时确定、运行期不改绑定的对象；它们的方法会被调用，但字段
/// 本身不重新赋值（`rng` 与 `physics` 例外：前者在决策时换出再换回以避开借用
/// 冲突，后者内部可变）。放在一起使「引擎依赖了哪些子系统」一眼可见。
pub(crate) struct Systems {
    pub(crate) physics: PhysicsWorld,
    pub(crate) decision: DecisionSystem,
    pub(crate) coach: CoachStrategy,
    pub(crate) rng: ChaCha8Rng,
}

/// 运行期观测：本 tick 的动作窗口、决策追踪与空间/接触评估。
///
/// 这些字段是「引擎看过什么」的缓存：它们不决定比赛真相，但决定下一 tick 的
/// 决策与渲染。放在一起使「观测值的生命周期是本 tick 还是跨 tick」成为局部问题
/// （`active_windows` 与 `beaten_recovery_until` 跨 tick，其余逐 tick 重算）。
pub(crate) struct RuntimeObservations {
    pub(crate) active_windows: HashMap<String, ActionTimeWindow>,
    pub(crate) last_decision_trace: Option<Box<DecisionDebug>>,
    pub(crate) latest_spacing: Option<SpacingEvaluation>,
    pub(crate) latest_contacts: Vec<SemanticContact>,
    /// 被过防守人的恢复窗口截止时刻（round-19）：窗口内战术层不得重派。
    pub(crate) beaten_recovery_until: HashMap<String, f32>,
    pub(crate) advancing_player: Option<String>,
    pub(crate) modulation: HashMap<String, PlayerModulationState>,
    /// 每名球员最近的出场/离场时刻（比赛时钟秒）：换人体息时间判据
    /// （gap.md G6）。在场者存出场时刻，替补存离场时刻。
    pub(crate) rotation_clock: HashMap<String, RotationClock>,
    /// 当前死球窗口内已执行的换人次数（每队）：受
    /// `RotationRules::max_substitutions_per_window` 约束，窗口结束时清零。
    pub(crate) substitutions_this_window: (u32, u32),
    /// 当前死球窗口是否已评估过轮换：同一窗口只评估一次。
    pub(crate) rotation_window_done: bool,
}

/// 一名球员的轮换时刻记录。
#[derive(Debug, Clone, Copy)]
pub(crate) struct RotationClock {
    /// 上次状态翻转（入场或离场）发生时的比赛时钟秒。
    pub(crate) since: f32,
    /// 当前是否在场。
    pub(crate) on_court: bool,
}

impl RuntimeObservations {
    pub(crate) fn new(
        active_windows: HashMap<String, ActionTimeWindow>,
        modulation: HashMap<String, PlayerModulationState>,
    ) -> Self {
        Self {
            active_windows,
            last_decision_trace: None,
            latest_spacing: None,
            latest_contacts: Vec::new(),
            beaten_recovery_until: HashMap::new(),
            advancing_player: None,
            modulation,
            rotation_clock: HashMap::new(),
            substitutions_this_window: (0, 0),
            rotation_window_done: false,
        }
    }
}

/// 比分与犯规账本：得分、团队犯规、罚球程序与箱体统计。
///
/// 这些字段共同构成「比赛记分」这一事实：它们只在得分、犯规与罚球结算时写入，
/// 并被帧投影与回合总结读取。放在一起使「改了记分就必须同步罚球程序」成为类型上
/// 可见的耦合，并集中在一组声明里。
pub(crate) struct ScoreLedger {
    pub(crate) home_score: u32,
    pub(crate) away_score: u32,
    pub(crate) team_fouls_home: u32,
    pub(crate) team_fouls_away: u32,
    pub(crate) free_throws_remaining: u8,
    pub(crate) free_throw_attempt: u8,
    pub(crate) free_throw_shooter: Option<String>,
    /// 投篮与比赛统计分解（2P/3P/FT 命中率与出手数、失误、犯规）。
    pub(crate) box_score: MatchBoxScore,
}

impl ScoreLedger {
    pub(crate) fn new() -> Self {
        Self {
            home_score: 0,
            away_score: 0,
            team_fouls_home: 0,
            team_fouls_away: 0,
            free_throws_remaining: 0,
            free_throw_attempt: 0,
            free_throw_shooter: None,
            box_score: MatchBoxScore::default(),
        }
    }
}

/// 比赛流程：球权、宏观生命周期、作用域边界与发球基线。
///
/// 这些字段回答「现在是谁的球、比赛处在哪个生命周期阶段、本次运行的要求完成了吗」：
/// 它们只在球权转移、阶段迁移、作用域设置与得分/失误结算时写入。放在一起使
/// 「球权一条事实」与「生命周期一条事实」的写入边界在类型上可见。
pub(crate) struct GameFlow {
    /// 当前宏观生命周期（TipOff/LiveBall/DeadBall/FreeThrow/节末/终场）。
    pub(crate) game_flow: GameFlowState,
    /// 球权归属（由 `ball_state` 派生维护，不再有独立 `carrier_idx`）。
    pub(crate) possession: Possession,
    pub(crate) possession_id: u32,
    /// FIBA 交替拥有球权指示箭头（D20）。
    pub(crate) possession_arrow: Option<Possession>,
    /// Raw `step()` runs continuously; scoped exports enable this boundary.
    pub(crate) scope_active: bool,
    pub(crate) scope_boundary: super::ScopeBoundary,
    pub(crate) target_possessions: usize,
    pub(crate) completed_possessions: usize,
    /// Requested-scope completion is separate from the real game's lifecycle.
    pub(crate) simulation_complete: bool,
    /// 发球基线位置（界外球的放置基准）。
    pub(crate) inbound_baseline: glam::Vec2,
}

impl GameFlow {
    pub(crate) fn new(game_flow: GameFlowState, inbound_baseline: glam::Vec2) -> Self {
        Self {
            game_flow,
            possession: Possession::Home,
            possession_id: 1,
            possession_arrow: None,
            scope_active: false,
            scope_boundary: super::ScopeBoundary::Possessions,
            target_possessions: 10,
            completed_possessions: 0,
            simulation_complete: false,
            inbound_baseline,
        }
    }
}

/// 事件日志：本 tick 待发布事实、全场事件 ID、因果链与解说文案。
///
/// 依据 `docs/architecture.md` §2 [12] 与 `gap.md` §7.1：事件是引擎对外的**事实**
/// 而非日志，`event_id` 全场单调且因果链只记真实因果关系。解说文案
/// （`current_callout` / `current_intensity`）与事实同处一组，因为它们同属
/// 「本 tick 对外呈现什么」，与比赛真相不同层。
pub(crate) struct EventJournal {
    pub(crate) pending_events: Vec<nba_domain::GameEvent>,
    pub(crate) current_event: Option<String>,
    pub(crate) current_event_types: Vec<String>,
    pub(crate) current_enforcements: Vec<String>,
    /// D4.1 全场单调的事件 ID（跨 tick 唯一，与逐 tick 的 `event_sequence` 区分）。
    pub(crate) event_id_counter: u64,
    /// D4.1 语义因果链注册表：记录"触发事件"的 event_id，按因果槽位索引
    /// （如 `shot_outcome` ← SHOT_RELEASE / `pass_outcome` ← PASS /
    /// `foul_ft` ← FOUL）。后续"结果事件"发布时按同槽位取父。
    ///
    /// 刻意不做"同 tick 首个事件作父"的粗暴串链：同 tick 内的两个独立
    /// 接触事实（CONTACT_BUMP ×2）之间没有因果关系，串链即是伪造因果，
    /// 违反"事件只陈述已发生的事实"（gap.md §7.1）。
    pub(crate) causal_links: HashMap<&'static str, u64>,
    pub(crate) current_event_log: Vec<nba_protocol::FrameEvent>,
    /// 本 tick 的逐事件序（仅用于帧输出，不参与因果）。
    pub(crate) event_sequence: u64,
    /// 解说文案与强度（展示层，随事实同步更新）。
    pub(crate) current_callout: Option<String>,
    pub(crate) current_intensity: Option<String>,
}

impl EventJournal {
    pub(crate) fn new() -> Self {
        Self {
            pending_events: Vec::new(),
            current_event: None,
            current_event_types: Vec::new(),
            current_enforcements: Vec::new(),
            event_id_counter: 0,
            causal_links: HashMap::new(),
            current_event_log: Vec::new(),
            event_sequence: 0,
            current_callout: None,
            current_intensity: None,
        }
    }

    /// 清空本 tick 的逐 tick 账目（因果槽位与全场 ID 跨 tick 存活）。
    pub(crate) fn begin_tick(&mut self) {
        self.current_event = None;
        self.current_event_types.clear();
        self.current_enforcements.clear();
    }
}

/// 队伍配置：规则、两队资料、战术档案与当前生效方案。
///
/// 这一组同时包含**构造时固定的配置**（规则、名册、档案）与**每 tick 同步的
/// 派生量**（`tactical_set` 由球权决定，`rules.tactics.defense` 由防守方案决定）。
/// 放在一起使「哪些配置是只读的」与「哪些是运行期派生到规则里的」显式可见：
/// 后者是 charter C1 的例外路径，由 ADR-004 允许（防守方案必须成为因果输入），
/// 其余字段在构造后只读。
pub(crate) struct TeamConfig {
    pub(crate) rules: nba_domain::GameRules,
    /// 当前球权对应的进攻战术集（由 `sync_team_tactics` 每 tick 派生）。
    pub(crate) tactical_set: TacticalSet,
    pub(crate) home_team: nba_domain::TeamData,
    pub(crate) away_team: nba_domain::TeamData,
    pub(crate) team_traits: HashMap<String, nba_domain::TeamTraits>,
    pub(crate) home_roster_order: Vec<String>,
    pub(crate) away_roster_order: Vec<String>,
    pub(crate) home_offense_tactic: TacticalSet,
    pub(crate) away_offense_tactic: TacticalSet,
    /// D5.1b：生效的进攻战术档案（槽位元数据来源）。此前档案只被用于
    /// 校验 id 合法，从未参与目标生成，导致所有槽位由全局 ratio 推得、
    /// 全队挤在弧顶三分线外。
    pub(crate) home_offense_spec: nba_domain::TacticalSetSpec,
    pub(crate) away_offense_spec: nba_domain::TacticalSetSpec,
    pub(crate) home_defensive_tactic: DefensiveTactic,
    pub(crate) away_defensive_tactic: DefensiveTactic,
}

/// 比赛时钟：时间轴、节次、子阶段与各类计时器。
///
/// 这些字段共同决定「现在是什么时间、处于哪一节、哪个子阶段」：它们只在时钟
/// 推进、阶段迁移与节间休息时写入，并被几乎所有阶段读取（136 处引用）。
/// 放在一起使「推进时间」与「切换阶段」的职责边界在类型上可见。
pub(crate) struct MatchClock {
    /// Monotonic fixed-step index used by semantic facts and replay consumers.
    pub(crate) tick_index: u64,
    /// 比赛时钟（秒，节内倒计时）。
    pub(crate) game_clock: f32,
    /// 进攻时钟（秒）。
    pub(crate) shot_clock: f32,
    /// 单调仿真时间（秒）。
    pub(crate) current_time: f32,
    /// 当前节次。
    pub(crate) period: u32,
    /// 回合子阶段。
    pub(crate) sub_phase: SubPhase,
    pub(crate) sub_phase_timer: f32,
    /// 发球程序已用时间（5 秒规则）。
    pub(crate) inbound_elapsed: f32,
    /// 后场连续持球时间（秒，8 秒违例判据）。
    pub(crate) backcourt_elapsed: f32,
    /// 节间休息已用时间（秒）。
    pub(crate) period_break_elapsed: f32,
    /// 上一次决策的时刻（用于决策间隔）。
    pub(crate) last_decision_time: f32,
}

impl MatchClock {
    pub(crate) fn new(
        game_clock: f32,
        shot_clock: f32,
        sub_phase: SubPhase,
        last_decision_time: f32,
    ) -> Self {
        Self {
            tick_index: 0,
            game_clock,
            shot_clock,
            current_time: 0.0,
            period: 1,
            sub_phase,
            sub_phase_timer: 0.0,
            inbound_elapsed: 0.0,
            backcourt_elapsed: 0.0,
            period_break_elapsed: 0.0,
            last_decision_time,
        }
    }
}

/// 当前回合上下文：本回合的起点、传球数、出手人与干扰度。
///
/// 这些字段只在回合开始时初始化、在回合进行中更新、在回合结束时重置。
/// 放在一起使「一个回合的统计口径」集中在一处可读的事实里。
pub(crate) struct PossessionContext {
    /// 当前回合开始时的游戏时钟（用于计算回合时长）。
    pub(crate) current_possession_start_clock: f32,
    /// Monotonic simulation time at the start of the active possession.
    pub(crate) current_possession_start_time: f32,
    /// 当前回合内的连续传球次数。
    pub(crate) current_possession_passes: u32,
    /// 当前回合内的出手球员 ID。
    pub(crate) current_possession_shooter: Option<String>,
    /// 当前回合内的出手干扰度。
    pub(crate) current_possession_contest: Option<f32>,
    /// Offensive player responsible for the active possession's last action.
    pub(crate) current_possession_turnover_player: Option<String>,
    /// 最近一次回合总结的 index（`complete_possession` 兜底发射的判据，
    /// M8 验收"回合零遗漏"：任何结束路径都必须有总结）。
    pub(crate) last_possession_summary_index: Option<u64>,
}

impl PossessionContext {
    pub(crate) fn new(start_clock: f32) -> Self {
        Self {
            current_possession_start_clock: start_clock,
            current_possession_start_time: 0.0,
            current_possession_passes: 0,
            current_possession_shooter: None,
            current_possession_contest: None,
            current_possession_turnover_player: None,
            last_possession_summary_index: None,
        }
    }

    /// 开启新回合：重置上下文，把起点设为当前时钟与时间。
    pub(crate) fn begin(&mut self, game_clock: f32, current_time: f32) {
        self.current_possession_start_clock = game_clock;
        self.current_possession_start_time = current_time;
        self.current_possession_passes = 0;
        self.current_possession_shooter = None;
        self.current_possession_contest = None;
        self.current_possession_turnover_player = None;
    }
}

/// 球运行态：球位、球态、传球链路与接球人预判。
///
/// 依据 `docs/architecture.md` §3：`ball_state` 是球权归属的**唯一事实源**，
/// 位置（`ball_pos_3d`）是它的派生量；其余字段描述「球在人与人间传递时的中间状态」
/// （上一传球人、待定的接球人与其自己的预判、松球终结原因）。放在一起使
/// 「球的归属与传递」成为一个内聚状态，而非散在八个字段上。
#[derive(Debug, Clone, Copy)]
pub(crate) struct PassContactState {
    pub(crate) duration_seconds: f32,
}

pub(crate) struct BallRuntime {
    /// 球的三维位置（ft）。位置的唯一写入点是弹道采样与状态转移。
    pub(crate) ball_pos_3d: (glam::Vec2, f32),
    /// 权威球态（球权真相的唯一载体，只能经 `transition_ball_state` 改写）。
    pub(crate) ball_state: BallTrajectoryKind,
    pub(crate) last_passer_id: Option<String>,
    /// Receiver awaiting physical convergence to a frozen pass endpoint.
    pub(crate) pending_pass_receiver: Option<String>,
    /// 接球人**自己的**接球点估计（层 A，P-1），跨 tick 保留。
    ///
    /// ## 为什么必须跨 tick 保留（round-10 修正）
    ///
    /// 第一版每 tick 从当前 `ball_pos` 重算估计点。实测后果：飞行末期球
    /// 逼近接球人时 `dist` 变小，估计点**向接球人塌缩**，随后方向翻转——
    /// 接球人目标点大幅抖动（实测 (19.8,36.5) → 自身位置 → (4.9,46.1)），
    /// 他来回跑，距球从 6.8 ft 单调恶化到 12.8 ft。
    ///
    /// 真实球员不会每帧重估：他在球出手后形成一个**稳定的预判**，之后
    /// 只做小幅修正。因此估计点必须作为状态保存，并在观察到新证据时
    /// **按观察力加权地向新信息靠拢**，而不是整体重算。
    pub(crate) receiver_estimate: Option<(String, glam::Vec2)>,
    /// Cause carried by a loose ball until a player secures it.
    pub(crate) pending_loose_ball_terminal: Option<nba_domain::PossessionEndCause>,
    /// Whether the pending control transfer is the delayed arrival of an inbound pass.
    pub(crate) pending_pass_inbound: bool,
    /// 上一 tick 的球位置（P-1 观测差分用，感知延迟一步）。
    pub(crate) prev_observed_ball_pos: Option<glam::Vec2>,
    /// 最近一次被过掉（drive successful）的对位防守人（round-18）。
    pub(crate) beaten_defender_id: Option<String>,
    /// 当前自由球身体接触区间内已经处理过的球员 id（ADR-017 第三步）。
    ///
    /// 球离开该球员的身体接触范围后，调用方清除此记录；重新进入时可以
    /// 发生新的身体碰撞。这里保存的是引擎内部的接触状态，不属于领域球态。
    pub(crate) loose_contact_resolved: Vec<String>,
    /// 当前传球飞行中每名防守者的连续接触段状态。
    ///
    /// 接触状态在球进入可及范围时建立，在球离开范围时移除。状态保存
    /// 接触段累计时间，概率裁定使用 `hazard × dt`，结果不会因提高 tick
    /// 频率而重复放大。
    pub(crate) pass_contact_states: HashMap<String, PassContactState>,
}

impl BallRuntime {
    pub(crate) fn new(ball_pos_3d: (glam::Vec2, f32), ball_state: BallTrajectoryKind) -> Self {
        Self {
            ball_pos_3d,
            ball_state,
            last_passer_id: None,
            pending_pass_receiver: None,
            receiver_estimate: None,
            pending_loose_ball_terminal: None,
            pending_pass_inbound: false,
            prev_observed_ball_pos: None,
            beaten_defender_id: None,
            loose_contact_resolved: Vec::new(),
            pass_contact_states: HashMap::new(),
        }
    }
}

/// 校验账本：每 tick 的不变量校验器与最近一次运行产生的违反。
pub(crate) struct AuditTrail {
    /// Per-tick invariant checker; validates every emitted frame against the
    /// physical/basketball rules that must always hold, regardless of tactics.
    pub(crate) invariant_checker: InvariantChecker,
    /// Violations produced by the most recent `step()`; exported for callers
    /// that want a single aggregated report rather than per-tick stderr.
    pub(crate) last_tick_violations: Vec<Violation>,
}

impl AuditTrail {
    pub(crate) fn new() -> Self {
        Self {
            invariant_checker: InvariantChecker::new(),
            last_tick_violations: Vec::new(),
        }
    }
}
