//! 约束系统：可声明、可激活、可求值、可执行、可反馈的运行时法则子系统。
//!
//! 设计契约（架构文档 §2/§8）：
//! - 每个约束是独立对象：id / domain / severity / scope / timing / when / evaluate / on_violation
//! - 三级强度：Hard（剔除动作、触发违例）、Soft（效用惩罚+风险）、Preference（权重偏置）
//! - 三段时间维度：Pre（候选过滤）、Runtime（事中检查）、Post（事后裁决）
//! - 约束按阶段激活集合，求值结果统一反馈决策层
//!
//! 引擎公式：FinalAction = Decision.select(Constraint.feasible(candidates, world, phase), ...)

use glam::Vec2;

use nba_domain::court::{COURT_HEIGHT_FT, COURT_WIDTH_FT};
use nba_physics::movement::PlayerPhysicsState;
use nba_physics::spatial::SpatialGeometry;
use std::collections::HashMap;

// ============================================================================
// 约束对象模型
// ============================================================================

/// 约束来源域。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintDomain {
    /// 物理现实：速度/空间/路径/球体状态/接触
    Physical,
    /// 篮球语义：规则/进程/战术/合理性
    Semantic,
}

/// 约束强度分级（文档 §2.2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// 偏好：不判违规，只改变决策分布
    Preference,
    /// 软约束：可违反但不应，降低效用、增加风险
    Soft,
    /// 硬约束：绝对不可违反，剔除/中断/违例
    Hard,
}

/// 约束作用对象。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintScope {
    Player,
    Ball,
    Team,
    Phase,
    Game,
}

/// 约束生效时间段（文档 §2.4）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintTiming {
    /// 事前：候选动作过滤
    Pre,
    /// 事中：执行过程持续检查
    Runtime,
    /// 事后：动作完成后裁决
    Post,
}

/// 阶段类型：约束集合按此激活（文档 §5 阶段约束绑定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseType {
    /// 回合发起（发球/抢断后一传）
    Inbound,
    /// 推进/快攻
    Transition,
    /// 阵地战术执行
    SetPlay,
    /// 出手飞行与裁决
    Resolution,
    /// 篮板争抢
    Rebound,
    /// 死球重置
    DeadBallReset,
}

impl PhaseType {
    pub fn as_str(&self) -> &'static str {
        match self {
            PhaseType::Inbound => "INBOUND",
            PhaseType::Transition => "TRANSITION",
            PhaseType::SetPlay => "SET_PLAY",
            PhaseType::Resolution => "RESOLUTION",
            PhaseType::Rebound => "REBOUND",
            PhaseType::DeadBallReset => "DEAD_BALL_RESET",
        }
    }
}

/// 约束求值结论。
#[derive(Debug, Clone, PartialEq)]
pub enum ConstraintStatus {
    Pass,
    /// 软约束/偏好被触发
    Flagged { reason: String },
    /// 硬约束被触发
    Violate { reason: String },
}

/// 违反后的执行动作（文档 §8.5 EnforcementExecutor 的输出）。
#[derive(Debug, Clone, PartialEq)]
pub enum EnforcementAction {
    None,
    /// 阻止候选动作
    BlockAction,
    /// 触发违例，球权转换
    Violation { kind: ViolationKind },
    /// 球权转换（抢断/出界等非违例路径）
    Turnover { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationKind {
    ShotClock,
    OutOfBounds,
    GameClockExpired,
    DeadBallAction,
}

impl ViolationKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ViolationKind::ShotClock => "SHOT_CLOCK_VIOLATION",
            ViolationKind::OutOfBounds => "OUT_OF_BOUNDS",
            ViolationKind::GameClockExpired => "GAME_CLOCK_EXPIRED",
            ViolationKind::DeadBallAction => "DEAD_BALL_ACTION",
        }
    }
}

/// 候选动作的统一抽象（决策层产出，约束层过滤）。
#[derive(Debug, Clone, PartialEq)]
pub enum CandidateAction {
    Shoot {
        shooter_id: String,
        from_pos: Vec2,
        is_three: bool,
    },
    Pass {
        passer_id: String,
        receiver_id: String,
        from_pos: Vec2,
        to_pos: Vec2,
    },
    Dwell {
        player_id: String,
    },
}

impl CandidateAction {
    pub fn actor_id(&self) -> &str {
        match self {
            CandidateAction::Shoot { shooter_id, .. } => shooter_id,
            CandidateAction::Pass { passer_id, .. } => passer_id,
            CandidateAction::Dwell { player_id } => player_id,
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            CandidateAction::Shoot { .. } => "SHOOT",
            CandidateAction::Pass { .. } => "PASS",
            CandidateAction::Dwell { .. } => "DWELL",
        }
    }
}

/// 约束求值上下文：世界状态的只读切面。
pub struct ConstraintContext<'a> {
    pub players: &'a HashMap<String, PlayerPhysicsState>,
    pub ball_pos: Vec2,
    /// 当前球权方 "home" / "away"
    pub possession_team: &'a str,
    pub shot_clock: f32,
    pub game_clock: f32,
    pub phase: PhaseType,
    /// 球是否处于飞行状态（传/投飞行中）
    pub ball_in_flight: bool,
    /// 死球标志
    pub is_dead_ball: bool,
}

/// 约束求值结果：状态 + 对决策的影响（文档 §2.1 ConstraintResult + decisionEffects）。
#[derive(Debug, Clone)]
pub struct ConstraintResult {
    pub status: ConstraintStatus,
    /// 效用惩罚（soft 违反时为负；preference 可为正）
    pub utility_penalty: f32,
    /// 风险增量
    pub risk_delta: f32,
}

impl ConstraintResult {
    pub fn pass() -> Self {
        Self { status: ConstraintStatus::Pass, utility_penalty: 0.0, risk_delta: 0.0 }
    }

    pub fn flagged(reason: impl Into<String>, penalty: f32, risk: f32) -> Self {
        Self {
            status: ConstraintStatus::Flagged { reason: reason.into() },
            utility_penalty: penalty,
            risk_delta: risk,
        }
    }

    pub fn violate(reason: impl Into<String>) -> Self {
        Self {
            status: ConstraintStatus::Violate { reason: reason.into() },
            utility_penalty: 0.0,
            risk_delta: 0.0,
        }
    }
}

/// 统一约束对象（文档 §2.1 Constraint 接口）。
pub struct Constraint {
    pub id: &'static str,
    pub domain: ConstraintDomain,
    pub severity: Severity,
    pub scope: ConstraintScope,
    pub timing: ConstraintTiming,
    /// 生效阶段集合
    pub phases: &'static [PhaseType],
    /// 激活条件（阶段之外的条件门）
    pub when: fn(&ConstraintContext) -> bool,
    /// 求值逻辑（针对单个候选动作）
    pub evaluate: fn(&ConstraintContext, &CandidateAction) -> ConstraintResult,
    /// 硬约束违反后的处理
    pub on_violation: fn(&ConstraintContext) -> EnforcementAction,
}

impl Constraint {
    /// 阶段 + 条件双门激活判定。
    pub fn is_active(&self, ctx: &ConstraintContext) -> bool {
        self.phases.contains(&ctx.phase) && (self.when)(ctx)
    }
}

// ============================================================================
// 具体约束实现：硬约束（规则违例，文档 §4.2.1）
// ============================================================================

fn eval_shot_clock(_ctx: &ConstraintContext, action: &CandidateAction) -> ConstraintResult {
    // 出手在时钟归零前离手则有效；持球/传/停顿在归零后即违例
    let expired = !matches!(action, CandidateAction::Shoot { .. });
    if expired {
        ConstraintResult::violate("SHOT_CLOCK_VIOLATION")
    } else {
        ConstraintResult::pass()
    }
}

/// 24 秒进攻时钟（语义/硬/team/Runtime；世界级检查由求值器驱动）。
pub static SHOT_CLOCK_CONSTRAINT: Constraint = Constraint {
    id: "shot_clock",
    domain: ConstraintDomain::Semantic,
    severity: Severity::Hard,
    scope: ConstraintScope::Team,
    timing: ConstraintTiming::Runtime,
    phases: &[PhaseType::SetPlay, PhaseType::Transition, PhaseType::Resolution],
    when: |ctx| ctx.shot_clock <= 0.0 && !ctx.ball_in_flight,
    evaluate: eval_shot_clock,
    on_violation: |_ctx| EnforcementAction::Violation { kind: ViolationKind::ShotClock },
};

fn pos_out_of_bounds(pos: Vec2) -> bool {
    pos.x < 0.0 || pos.x > COURT_WIDTH_FT || pos.y < 0.0 || pos.y > COURT_HEIGHT_FT
}

fn eval_out_of_bounds(_ctx: &ConstraintContext, action: &CandidateAction) -> ConstraintResult {
    let target = match action {
        CandidateAction::Shoot { from_pos, .. } => Some(*from_pos),
        CandidateAction::Pass { to_pos, .. } => Some(*to_pos),
        CandidateAction::Dwell { .. } => None,
    };
    match target.filter(|&p| pos_out_of_bounds(p)) {
        Some(_) => ConstraintResult::violate("OUT_OF_BOUNDS"),
        None => ConstraintResult::pass(),
    }
}

/// 出界（物理/硬/player/Pre，文档 §3.3 + §4.2.1）。
pub static OUT_OF_BOUNDS_CONSTRAINT: Constraint = Constraint {
    id: "out_of_bounds",
    domain: ConstraintDomain::Physical,
    severity: Severity::Hard,
    scope: ConstraintScope::Player,
    timing: ConstraintTiming::Pre,
    phases: &[
        PhaseType::Inbound,
        PhaseType::Transition,
        PhaseType::SetPlay,
        PhaseType::Resolution,
    ],
    when: |_ctx| true,
    evaluate: eval_out_of_bounds,
    on_violation: |_ctx| EnforcementAction::Violation { kind: ViolationKind::OutOfBounds },
};

fn eval_dead_ball_action(_ctx: &ConstraintContext, action: &CandidateAction) -> ConstraintResult {
    match action {
        // 死球阶段唯一合法的"传球"是发球本身，由阶段状态机触发，不走候选管线
        CandidateAction::Dwell { .. } => ConstraintResult::pass(),
        _ => ConstraintResult::violate("DEAD_BALL_ACTION"),
    }
}

/// 死球阶段不能投篮/传球（语义/硬，文档 §4.1 比赛进程约束：在错误阶段做正确的事也是非法的）。
pub static DEAD_BALL_ACTION_CONSTRAINT: Constraint = Constraint {
    id: "dead_ball_action",
    domain: ConstraintDomain::Semantic,
    severity: Severity::Hard,
    scope: ConstraintScope::Phase,
    timing: ConstraintTiming::Pre,
    phases: &[PhaseType::DeadBallReset, PhaseType::Inbound],
    when: |ctx| ctx.is_dead_ball,
    evaluate: eval_dead_ball_action,
    on_violation: |_ctx| EnforcementAction::BlockAction,
};

fn eval_game_clock_expired(_ctx: &ConstraintContext, action: &CandidateAction) -> ConstraintResult {
    match action {
        CandidateAction::Shoot { .. } => ConstraintResult::violate("GAME_CLOCK_EXPIRED"),
        _ => ConstraintResult::pass(),
    }
}

/// 节末时间到后不能出手（语义/硬，文档 §4.1.2 QuarterEndConstraint）。
pub static GAME_CLOCK_EXPIRED_CONSTRAINT: Constraint = Constraint {
    id: "game_clock_expired",
    domain: ConstraintDomain::Semantic,
    severity: Severity::Hard,
    scope: ConstraintScope::Game,
    timing: ConstraintTiming::Pre,
    phases: &[PhaseType::SetPlay, PhaseType::Resolution],
    when: |ctx| ctx.game_clock <= 0.0,
    evaluate: eval_game_clock_expired,
    on_violation: |_ctx| EnforcementAction::BlockAction,
};

// ============================================================================
// 软约束：空间与合理性（文档 §3.2 / §4.4）
// ============================================================================

fn eval_risky_pass(ctx: &ConstraintContext, action: &CandidateAction) -> ConstraintResult {
    if let CandidateAction::Pass { passer_id, receiver_id, from_pos, to_pos } = action {
        let lane = SpatialGeometry::check_pass_corridor(*from_pos, *to_pos, 1.0, ctx.players, passer_id, receiver_id);
        if lane.is_blocked {
            ConstraintResult::flagged("PASS_LANE_CONTESTED", -0.24, 0.35)
        } else {
            // 走廊越窄（clearance 越小）惩罚越高
            let narrow = -(lane.corridor_clearance.clamp(0.0, 8.0) / 8.0) * 0.10;
            ConstraintResult::flagged("PASS_LANE_NARROW", narrow, 0.05)
        }
    } else {
        ConstraintResult::pass()
    }
}

/// 传球线路风险（物理/软，复用 spatial 传球走廊检测）。
pub static RISKY_PASS_CONSTRAINT: Constraint = Constraint {
    id: "risky_pass",
    domain: ConstraintDomain::Physical,
    severity: Severity::Soft,
    scope: ConstraintScope::Player,
    timing: ConstraintTiming::Pre,
    phases: &[PhaseType::SetPlay, PhaseType::Transition],
    when: |_ctx| true,
    evaluate: eval_risky_pass,
    on_violation: |_ctx| EnforcementAction::None,
};

fn eval_crowded_receiver(ctx: &ConstraintContext, action: &CandidateAction) -> ConstraintResult {
    if let CandidateAction::Pass { passer_id, to_pos, .. } = action {
        let team = ctx.players.get(passer_id).map(|p| p.team.as_str()).unwrap_or("");
        let density = SpatialGeometry::teammate_density(*to_pos, team, ctx.players, 8.0);
        if density > 0.45 {
            ConstraintResult::flagged("CROWDED_LANDING_ZONE", -0.18, 0.10)
        } else if density > 0.25 {
            ConstraintResult::flagged("DENSE_LANDING_ZONE", -0.08, 0.05)
        } else {
            ConstraintResult::pass()
        }
    } else {
        ConstraintResult::pass()
    }
}

/// 接球落点拥挤度（物理/软）：目标点附近队友密度过高则惩罚（文档 §3.2 空间占用对决策的影响）。
pub static CROWDED_RECEIVER_CONSTRAINT: Constraint = Constraint {
    id: "crowded_receiver",
    domain: ConstraintDomain::Physical,
    severity: Severity::Soft,
    scope: ConstraintScope::Team,
    timing: ConstraintTiming::Pre,
    phases: &[PhaseType::SetPlay, PhaseType::Transition],
    when: |_ctx| true,
    evaluate: eval_crowded_receiver,
    on_violation: |_ctx| EnforcementAction::None,
};

fn eval_contested_shot(ctx: &ConstraintContext, action: &CandidateAction) -> ConstraintResult {
    if let CandidateAction::Shoot { shooter_id, .. } = action {
        let openness = SpatialGeometry::get_openness(shooter_id, ctx.players);
        if openness.contest_intensity > 0.75 {
            ConstraintResult::flagged("HEAVILY_CONTESTED", -0.30, 0.25)
        } else if openness.contest_intensity > 0.5 {
            ConstraintResult::flagged("CONTESTED", -0.15, 0.12)
        } else {
            ConstraintResult::pass()
        }
    } else {
        ConstraintResult::pass()
    }
}

/// 强投惩罚（语义/软）：高对抗强度下出手效用下降（文档 §4.4 篮球合理性约束）。
pub static CONTESTED_SHOT_CONSTRAINT: Constraint = Constraint {
    id: "contested_shot",
    domain: ConstraintDomain::Semantic,
    severity: Severity::Soft,
    scope: ConstraintScope::Player,
    timing: ConstraintTiming::Pre,
    phases: &[PhaseType::SetPlay, PhaseType::Resolution],
    when: |_ctx| true,
    evaluate: eval_contested_shot,
    on_violation: |_ctx| EnforcementAction::None,
};

// ============================================================================
// 偏好约束（文档 §2.2.3：改变决策分布而非判违规）
// ============================================================================

fn eval_shot_clock_urgency(ctx: &ConstraintContext, action: &CandidateAction) -> ConstraintResult {
    let urgency = (1.0 - ctx.shot_clock / 5.0).clamp(0.0, 1.0);
    match action {
        CandidateAction::Shoot { .. } => ConstraintResult::flagged("CLOCK_URGENCY_BOOST", 0.25 * urgency, 0.0),
        CandidateAction::Pass { .. } => ConstraintResult::flagged("CLOCK_URGENCY_PASS_DEPRIORITIZE", -0.10 * urgency, 0.0),
        CandidateAction::Dwell { .. } => ConstraintResult::flagged("CLOCK_URGENCY_NO_HESITATE", -0.20 * urgency, 0.0),
    }
}

/// 时间压力偏置（语义/preference）：时钟紧迫时出手权重上调（文档 §4.3.5 打断约束 / §7.2.3 阶段覆盖）。
pub static SHOT_CLOCK_URGENCY_PREFERENCE: Constraint = Constraint {
    id: "shot_clock_urgency",
    domain: ConstraintDomain::Semantic,
    severity: Severity::Preference,
    scope: ConstraintScope::Game,
    timing: ConstraintTiming::Pre,
    phases: &[PhaseType::SetPlay, PhaseType::Resolution],
    when: |ctx| ctx.shot_clock < 5.0,
    evaluate: eval_shot_clock_urgency,
    on_violation: |_ctx| EnforcementAction::None,
};

// ============================================================================
// 注册中心与求值器（文档 §8.1/§8.3 ConstraintRegistry / ConstraintEvaluator）
// ============================================================================

/// 单个候选动作经过约束管线后的完整评分报告（文档 §6.4 FeasibilityReport 扩展）。
#[derive(Debug, Clone)]
pub struct ScoredCandidate {
    pub action: CandidateAction,
    /// 硬约束是否全部通过
    pub feasible: bool,
    /// 被哪条硬约束剔除
    pub blocked_by: Option<&'static str>,
    /// 约束惩罚合计（soft + preference）
    pub constraint_penalty: f32,
    /// 风险合计
    pub risk: f32,
    /// 触发的约束明细（调试层，文档 Phase 8）
    pub flags: Vec<(&'static str, String, f32)>,
}

/// 约束注册中心：全部约束的唯一登记处。
pub struct ConstraintRegistry {
    pub all: Vec<&'static Constraint>,
}

impl Default for ConstraintRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ConstraintRegistry {
    pub fn new() -> Self {
        Self {
            all: vec![
                &SHOT_CLOCK_CONSTRAINT,
                &OUT_OF_BOUNDS_CONSTRAINT,
                &DEAD_BALL_ACTION_CONSTRAINT,
                &GAME_CLOCK_EXPIRED_CONSTRAINT,
                &RISKY_PASS_CONSTRAINT,
                &CROWDED_RECEIVER_CONSTRAINT,
                &CONTESTED_SHOT_CONSTRAINT,
                &SHOT_CLOCK_URGENCY_PREFERENCE,
            ],
        }
    }

    /// 阶段约束集合（文档 §5.1 ConstraintSet）：按当前阶段抽取激活的约束。
    pub fn active_set(&self, ctx: &ConstraintContext) -> Vec<&'static Constraint> {
        self.all.iter().copied().filter(|c| c.is_active(ctx)).collect()
    }

    /// 约束求值器核心（文档 §8.3）：对候选动作执行 硬过滤 → 软惩罚 → 偏好调整。
    pub fn evaluate_candidate(
        &self,
        ctx: &ConstraintContext,
        action: &CandidateAction,
    ) -> ScoredCandidate {
        let mut scored = ScoredCandidate {
            action: action.clone(),
            feasible: true,
            blocked_by: None,
            constraint_penalty: 0.0,
            risk: 0.0,
            flags: Vec::new(),
        };

        for c in self.active_set(ctx) {
            let result = (c.evaluate)(ctx, action);
            match result.status {
                ConstraintStatus::Pass => {}
                ConstraintStatus::Flagged { reason } => {
                    scored.constraint_penalty += result.utility_penalty;
                    scored.risk += result.risk_delta;
                    scored.flags.push((c.id, reason, result.utility_penalty));
                }
                ConstraintStatus::Violate { reason } => {
                    // 硬约束优先（文档 §7.2.1）：直接剔除
                    scored.feasible = false;
                    scored.blocked_by = Some(c.id);
                    scored.flags.push((c.id, reason, 0.0));
                    return scored;
                }
            }
        }

        scored
    }

    /// 世界级 Runtime 约束检查（不针对候选动作，针对世界状态本身，如 24 秒违例）。
    /// 返回需要执行的首个违反及其执行动作。
    pub fn evaluate_world(&self, ctx: &ConstraintContext) -> Option<(&'static Constraint, EnforcementAction)> {
        for c in self.all.iter() {
            if c.timing == ConstraintTiming::Runtime && c.is_active(ctx) {
                let enforcement = (c.on_violation)(ctx);
                if enforcement != EnforcementAction::None {
                    return Some((c, enforcement));
                }
            }
        }
        None
    }
}
