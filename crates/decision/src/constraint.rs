//! 约束系统：可声明、可激活、可求值、可执行、可反馈的运行时法则子系统。
//!
//! 一条约束的完整生命周期是：
//! `activation -> pre/runtime/post evaluation -> pure enforcement intent -> feedback`。
//! 约束求值只读世界快照；所有状态变更均由应用层执行，避免把规则散落到
//! 投篮、传球或阶段转换函数中。

use glam::Vec2;
pub use nba_domain::flow::PhaseType;
use nba_domain::{BallPhase, GameEvent, GameFlowState, GameRules};
use nba_physics::SpatialPhysics;
use std::collections::HashMap;

/// 约束来源域。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintDomain {
    Physical,
    Semantic,
}

/// 约束强度：硬约束过滤动作，软约束改变效用，偏好改变分布。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Preference,
    Soft,
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

/// 约束生效时机。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintTiming {
    Pre,
    Runtime,
    Post,
}

/// 约束求值结论。
#[derive(Debug, Clone, PartialEq)]
pub enum ConstraintStatus {
    Pass,
    Flagged { reason: String },
    Violate { reason: String },
}

/// 违反后的执行意图。它是值对象，不直接执行副作用。
#[derive(Debug, Clone, PartialEq)]
pub enum EnforcementAction {
    None,
    BlockAction,
    ModifyAction { reason: String },
    DelayAction { seconds: f32 },
    Violation { kind: ViolationKind },
    TriggerFoul { kind: String },
    ChangePossession { reason: String },
    EndPhase { next: PhaseType },
    ResetPosition,
    Turnover { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationKind {
    ShotClock,
    InboundClock,
    BackcourtClock,
    OutOfBounds,
    GameClockExpired,
    DeadBallAction,
    IllegalAction,
    Contact,
}

impl ViolationKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ShotClock => "SHOT_CLOCK_VIOLATION",
            Self::InboundClock => "FIVE_SECOND_INBOUND",
            Self::BackcourtClock => "EIGHT_SECOND_BACKCOURT",
            Self::OutOfBounds => "OUT_OF_BOUNDS",
            Self::GameClockExpired => "GAME_CLOCK_EXPIRED",
            Self::DeadBallAction => "DEAD_BALL_ACTION",
            Self::IllegalAction => "ILLEGAL_ACTION",
            Self::Contact => "CONTACT_VIOLATION",
        }
    }
}

/// 候选动作统一抽象。动作是意图，执行后的事实由物理/裁决层产生。
#[derive(Debug, Clone, PartialEq)]
pub enum CandidateAction {
    Shoot {
        shooter_id: String,
        from_pos: Vec2,
        is_three: bool,
    },
    Drive {
        driver_id: String,
        from_pos: Vec2,
        target_pos: Vec2,
    },
    Pass {
        passer_id: String,
        receiver_id: String,
        from_pos: Vec2,
        to_pos: Vec2,
    },
    /// 发球是死球期间唯一允许改变球轨迹的普通球权动作。
    InboundPass {
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
            Self::Shoot { shooter_id, .. } => shooter_id,
            Self::Drive { driver_id, .. } => driver_id,
            Self::Pass { passer_id, .. } | Self::InboundPass { passer_id, .. } => passer_id,
            Self::Dwell { player_id } => player_id,
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Shoot { .. } => "SHOOT",
            Self::Drive { .. } => "DRIVE",
            Self::Pass { .. } => "PASS",
            Self::InboundPass { .. } => "INBOUND_PASS",
            Self::Dwell { .. } => "DWELL",
        }
    }

    pub fn is_ball_action(&self) -> bool {
        matches!(
            self,
            Self::Shoot { .. } | Self::Drive { .. } | Self::Pass { .. } | Self::InboundPass { .. }
        )
    }

    pub fn is_inbound(&self) -> bool {
        matches!(self, Self::InboundPass { .. })
    }
}

/// 约束求值上下文：世界状态的只读切面。
///
/// `rules` 是比赛策略的唯一数值来源；`game_flow` 和 `ball_state` 是
/// 生命周期事实，避免用多个布尔值拼出互相矛盾的状态。
pub struct ConstraintContext<'a> {
    pub physics: &'a dyn SpatialPhysics,
    pub ball_pos: Vec2,
    pub possession_team: &'a str,
    pub shot_clock: f32,
    pub game_clock: f32,
    pub phase: PhaseType,
    pub game_flow: GameFlowState,
    pub ball_phase: BallPhase,
    /// Elapsed time on the active semantic clock (inbound/backcourt).
    pub inbound_elapsed: f32,
    pub backcourt_elapsed: f32,
    pub rules: &'a GameRules,
    /// Team-level style traits supplied by the match setup.
    pub team_traits: &'a HashMap<String, nba_domain::TeamTraits>,
}

impl<'a> ConstraintContext<'a> {
    pub fn is_dead_ball(&self) -> bool {
        self.game_flow.is_dead_ball()
    }

    pub fn is_ball_in_flight(&self) -> bool {
        matches!(
            self.ball_phase,
            BallPhase::PassFlight | BallPhase::ShotFlight | BallPhase::Loose | BallPhase::Rebound
        )
    }

    pub fn is_live_ball(&self) -> bool {
        self.game_flow.allows_live_ball_actions()
    }

    pub fn ball_available_for_action(&self) -> bool {
        matches!(self.ball_phase, BallPhase::Held) && self.is_live_ball()
    }

    pub fn is_in_frontcourt(&self) -> bool {
        let midcourt = self.rules.court.width_ft / 2.0;
        match self.possession_team {
            "home" => self.ball_pos.x >= midcourt,
            "away" => self.ball_pos.x <= midcourt,
            _ => false,
        }
    }
}

/// 单条约束对决策的反馈。
#[derive(Debug, Clone)]
pub struct ConstraintResult {
    pub status: ConstraintStatus,
    /// Soft 约束通常为负，Preference 可以为正。
    pub utility_penalty: f32,
    pub risk_delta: f32,
}

impl ConstraintResult {
    pub fn pass() -> Self {
        Self {
            status: ConstraintStatus::Pass,
            utility_penalty: 0.0,
            risk_delta: 0.0,
        }
    }

    pub fn flagged(reason: impl Into<String>, penalty: f32, risk: f32) -> Self {
        Self {
            status: ConstraintStatus::Flagged {
                reason: reason.into(),
            },
            utility_penalty: penalty,
            risk_delta: risk,
        }
    }

    pub fn violate(reason: impl Into<String>) -> Self {
        Self {
            status: ConstraintStatus::Violate {
                reason: reason.into(),
            },
            utility_penalty: 0.0,
            risk_delta: 0.0,
        }
    }
}

/// 统一约束对象。
///
/// `evaluate` 服务事前候选；`evaluate_world` 服务事中快照；
/// `evaluate_event` 服务事后领域事实。三个回调都必须是纯求值函数。
#[derive(Debug)]
pub struct Constraint {
    pub id: &'static str,
    pub domain: ConstraintDomain,
    pub severity: Severity,
    /// Lower values run first when constraints compete for a decision.
    pub priority: u8,
    pub scope: ConstraintScope,
    pub timing: ConstraintTiming,
    pub phases: &'static [PhaseType],
    pub when: fn(&ConstraintContext) -> bool,
    pub evaluate: fn(&ConstraintContext, &CandidateAction) -> ConstraintResult,
    pub evaluate_world: fn(&ConstraintContext) -> ConstraintResult,
    pub evaluate_event: fn(&ConstraintContext, &GameEvent) -> ConstraintResult,
    pub on_violation: fn(&ConstraintContext) -> EnforcementAction,
}

impl Constraint {
    pub fn is_active(&self, ctx: &ConstraintContext) -> bool {
        self.phases.contains(&ctx.phase) && (self.when)(ctx)
    }
}

fn pass_action(_ctx: &ConstraintContext, _action: &CandidateAction) -> ConstraintResult {
    ConstraintResult::pass()
}
fn pass_world(_ctx: &ConstraintContext) -> ConstraintResult {
    ConstraintResult::pass()
}
fn pass_event(_ctx: &ConstraintContext, _event: &GameEvent) -> ConstraintResult {
    ConstraintResult::pass()
}
fn eval_shot_clock_action(_ctx: &ConstraintContext, _action: &CandidateAction) -> ConstraintResult {
    ConstraintResult::pass()
}
fn eval_shot_clock_world(ctx: &ConstraintContext) -> ConstraintResult {
    if ctx.shot_clock <= 0.0 && !ctx.is_ball_in_flight() && ctx.is_live_ball() {
        ConstraintResult::violate("SHOT_CLOCK_VIOLATION")
    } else {
        ConstraintResult::pass()
    }
}
fn eval_inbound_clock_world(ctx: &ConstraintContext) -> ConstraintResult {
    if ctx.inbound_elapsed >= ctx.rules.inbound_seconds
        && ctx.phase == PhaseType::Inbound
        && ctx.is_dead_ball()
    {
        ConstraintResult::violate("FIVE_SECOND_INBOUND")
    } else {
        ConstraintResult::pass()
    }
}

fn eval_backcourt_clock_world(ctx: &ConstraintContext) -> ConstraintResult {
    if ctx.backcourt_elapsed >= ctx.rules.backcourt_seconds
        && !ctx.is_in_frontcourt()
        && ctx.is_live_ball()
    {
        ConstraintResult::violate("EIGHT_SECOND_BACKCOURT")
    } else {
        ConstraintResult::pass()
    }
}

fn eval_game_clock_world(ctx: &ConstraintContext) -> ConstraintResult {
    if ctx.game_clock <= 0.0 && !ctx.is_ball_in_flight() && ctx.is_live_ball() {
        ConstraintResult::violate("GAME_CLOCK_EXPIRED")
    } else {
        ConstraintResult::pass()
    }
}
fn eval_out_of_bounds_action(
    ctx: &ConstraintContext,
    action: &CandidateAction,
) -> ConstraintResult {
    let in_bounds = match action {
        CandidateAction::Shoot { from_pos, .. } => ctx.rules.court.contains(*from_pos, 0.0),
        CandidateAction::Drive { target_pos, .. } => ctx.rules.court.contains(*target_pos, 0.0),
        CandidateAction::Pass { to_pos, .. } => ctx.rules.court.contains(*to_pos, 0.0),
        CandidateAction::InboundPass { passer_id, to_pos, .. } => {
            // 接球点必须在界内
            let to_in_bounds = ctx.rules.court.contains(*to_pos, 0.0);
            // 发球人必须在场且身体在界外底线附近（与发球点/球距离不超过阈值）
            let passer_ready = ctx
                .physics
                .get_player(passer_id)
                .map(|p| {
                    let is_legal_oob = nba_domain::court::Court::is_inbound_release(
                        p.pos_ft,
                        ctx.rules.inbound_boundary_tolerance_ft,
                        ctx.rules.court,
                    );
                    let near_ball = (p.pos_ft - ctx.ball_pos).length() <= ctx.rules.inbound_boundary_tolerance_ft;
                    is_legal_oob && near_ball
                })
                .unwrap_or(false);
            to_in_bounds && passer_ready
        }
        CandidateAction::Dwell { .. } => true,
    };
    if in_bounds {
        ConstraintResult::pass()
    } else {
        ConstraintResult::violate("OUT_OF_BOUNDS")
    }
}

fn eval_out_of_bounds_event(ctx: &ConstraintContext, event: &GameEvent) -> ConstraintResult {
    let is_boundary_cross = matches!(event, GameEvent::BoundaryCross { .. });
    if !is_boundary_cross {
        return ConstraintResult::pass();
    }
    let (is_ball_carrier, is_inbounding) = match event {
        GameEvent::BoundaryCross { player_id, pos, .. } => {
            let player = ctx.physics.get_player(player_id);
            let has_ball = player.map(|p| p.has_ball).unwrap_or(false);
            let is_inbound_action = player
                .map(|p| p.action == "INBOUND_SETUP" || p.action == "InboundPositioning")
                .unwrap_or(false);
            let inbounding = (ctx.is_dead_ball() || ctx.phase == PhaseType::Inbound)
                && (is_inbound_action
                    || nba_domain::court::Court::is_inbound_release(
                        glam::Vec2::new(pos.0, pos.1),
                        ctx.rules.inbound_boundary_tolerance_ft,
                        ctx.rules.court,
                    ));
            (has_ball, inbounding)
        }
        _ => (false, false),
    };
    if is_inbounding {
        ConstraintResult::pass()
    } else if is_ball_carrier {
        ConstraintResult::violate("OUT_OF_BOUNDS")
    } else {
        ConstraintResult::flagged("BOUNDARY_CROSSING", 0.0, 0.0)
    }
}

fn eval_dead_ball_action(ctx: &ConstraintContext, action: &CandidateAction) -> ConstraintResult {
    if ctx.is_dead_ball()
        && !action.is_inbound()
        && !matches!(action, CandidateAction::Dwell { .. })
    {
        ConstraintResult::violate("DEAD_BALL_ACTION")
    } else if action.is_inbound() && ctx.phase != PhaseType::Inbound {
        ConstraintResult::violate("INBOUND_NOT_ALLOWED_IN_PHASE")
    } else {
        ConstraintResult::pass()
    }
}

fn eval_game_clock_action(ctx: &ConstraintContext, action: &CandidateAction) -> ConstraintResult {
    if ctx.game_clock <= 0.0 && action.is_ball_action() {
        ConstraintResult::violate("GAME_CLOCK_EXPIRED")
    } else {
        ConstraintResult::pass()
    }
}

fn eval_action_eligibility(ctx: &ConstraintContext, action: &CandidateAction) -> ConstraintResult {
    let actor = match ctx.physics.get_player(action.actor_id()) {
        Some(player) => player,
        None => return ConstraintResult::violate("ACTOR_NOT_FOUND"),
    };
    if actor.team != ctx.possession_team {
        return ConstraintResult::violate("ACTOR_NOT_IN_POSSESSION");
    }
    if action.is_inbound() {
        if ctx.phase != PhaseType::Inbound || !ctx.is_dead_ball() {
            return ConstraintResult::violate("INBOUND_NOT_ALLOWED");
        }
        return ConstraintResult::pass();
    }
    if action.is_ball_action() && !ctx.ball_available_for_action() {
        return ConstraintResult::violate("BALL_NOT_AVAILABLE");
    }
    if action.is_ball_action() && !matches!(ctx.phase, PhaseType::Transition | PhaseType::SetPlay) {
        return ConstraintResult::violate("ACTION_NOT_ALLOWED_IN_PHASE");
    }
    ConstraintResult::pass()
}
fn eval_drive(ctx: &ConstraintContext, action: &CandidateAction) -> ConstraintResult {
    let CandidateAction::Drive {
        driver_id,
        from_pos,
        target_pos,
    } = action
    else {
        return ConstraintResult::pass();
    };
    let Some(driver) = ctx.physics.get_player(driver_id) else {
        return ConstraintResult::violate("DRIVER_NOT_FOUND");
    };
    let hoop = ctx.rules.court.hoop_pos(driver.team == "home");
    let start_distance = (*from_pos - hoop).length();
    let target_distance = (*target_pos - hoop).length();
    if (*target_pos - *from_pos).length_squared() <= f32::EPSILON
        || target_distance >= start_distance
    {
        return ConstraintResult::violate("DRIVE_NO_RIM_ADVANTAGE");
    }
    let density = ctx.physics.query_nearby(
        *target_pos,
        ctx.rules.teammate_density_radius_ft,
        &nba_physics::EntityFilter::Team(driver.team.clone()),
    );
    let density = density.len() as f32 / ctx.rules.teammate_density_capacity.max(1.0);
    let density = density.clamp(0.0, 1.0);
    let openness = ctx.physics.openness(driver_id);
    let penalty = -(density * ctx.rules.tactics.drive_distance_ratio
        + openness.contest_intensity * ctx.rules.shot_contest_sensitivity);
    ConstraintResult::flagged(
        if density > ctx.rules.teammate_density_capacity.recip().min(1.0) {
            "DRIVE_PAINT_CROWDED"
        } else {
            "DRIVE_LANE_OPEN"
        },
        penalty,
        (density * ctx.rules.tactics.drive_distance_ratio
            + openness.contest_intensity * ctx.rules.shot_contest_sensitivity)
            .clamp(0.0, 1.0),
    )
}

fn eval_risky_pass(ctx: &ConstraintContext, action: &CandidateAction) -> ConstraintResult {
    if let CandidateAction::Pass {
        passer_id,
        receiver_id,
        from_pos,
        to_pos,
    }
    | CandidateAction::InboundPass {
        passer_id,
        receiver_id,
        from_pos,
        to_pos,
    } = action
    {
        let lane = ctx.physics.pass_corridor(
            *from_pos,
            *to_pos,
            ctx.rules.pass_corridor_radius_ft,
            passer_id,
            receiver_id,
        );
        if lane.is_blocked {
            ConstraintResult::flagged(
                "PASS_LANE_CONTESTED",
                -ctx.rules.semantics.pass_lane_risk_weight,
                ctx.rules.semantics.pass_lane_risk_weight,
            )
        } else {
            ConstraintResult::flagged(
                "PASS_LANE_NARROW",
                -(lane
                    .corridor_clearance
                    .clamp(0.0, ctx.rules.pass_lane_clearance_reference_ft)
                    / ctx.rules.pass_lane_clearance_reference_ft)
                    * ctx.rules.semantics.pass_lane_risk_weight,
                ctx.rules.semantics.pass_lane_risk_weight * 0.2,
            )
        }
    } else {
        ConstraintResult::pass()
    }
}

fn eval_crowded_receiver(ctx: &ConstraintContext, action: &CandidateAction) -> ConstraintResult {
    if let CandidateAction::Pass {
        passer_id, to_pos, ..
    }
    | CandidateAction::InboundPass {
        passer_id, to_pos, ..
    } = action
    {
        let Some(passer) = ctx.physics.get_player(passer_id) else {
            return ConstraintResult::pass();
        };
        let density = ctx.physics.query_nearby(
            *to_pos,
            ctx.rules.teammate_density_radius_ft,
            &nba_physics::EntityFilter::Team(passer.team.clone()),
        );
        let density =
            (density.len() as f32 / ctx.rules.teammate_density_capacity.max(1.0)).clamp(0.0, 1.0);
        if density > ctx.rules.semantics.crowded_zone_threshold {
            ConstraintResult::flagged(
                "CROWDED_LANDING_ZONE",
                -ctx.rules.semantics.crowded_zone_utility_penalty,
                ctx.rules.semantics.crowded_zone_risk,
            )
        } else if density > ctx.rules.semantics.dense_zone_threshold {
            ConstraintResult::flagged(
                "DENSE_LANDING_ZONE",
                -ctx.rules.semantics.dense_zone_utility_penalty,
                ctx.rules.semantics.dense_zone_risk,
            )
        } else {
            ConstraintResult::pass()
        }
    } else {
        ConstraintResult::pass()
    }
}

fn eval_contested_shot(ctx: &ConstraintContext, action: &CandidateAction) -> ConstraintResult {
    if let CandidateAction::Shoot { shooter_id, .. } = action {
        let openness = ctx.physics.openness(shooter_id);
        let urgency_window = ctx.rules.shot_clock_urgency_seconds.max(f32::EPSILON);
        let patience_mult = if ctx.shot_clock > urgency_window {
            1.0 + (ctx.shot_clock - urgency_window) / ctx.rules.league.shot_clock_seconds.max(1.0)
        } else {
            (ctx.shot_clock / urgency_window)
                .clamp(ctx.rules.decision.contested_patience_floor, 1.0)
        };
        if openness.contest_intensity > ctx.rules.semantics.heavily_contested_threshold {
            ConstraintResult::flagged(
                "HEAVILY_CONTESTED",
                -ctx.rules.semantics.heavily_contested_utility_penalty * patience_mult,
                ctx.rules.semantics.heavily_contested_risk,
            )
        } else if openness.contest_intensity > ctx.rules.semantics.contested_threshold {
            ConstraintResult::flagged(
                "CONTESTED",
                -ctx.rules.semantics.contested_utility_penalty * patience_mult,
                ctx.rules.semantics.contested_risk,
            )
        } else {
            ConstraintResult::pass()
        }
    } else {
        ConstraintResult::pass()
    }
}

fn eval_shot_clock_urgency(ctx: &ConstraintContext, action: &CandidateAction) -> ConstraintResult {
    let urgency_window = ctx.rules.shot_clock_urgency_seconds.max(f32::EPSILON);
    let urgency = (1.0 - ctx.shot_clock / urgency_window).clamp(0.0, 1.0);
    match action {
        CandidateAction::Shoot { .. } => ConstraintResult::flagged(
            "CLOCK_URGENCY_BOOST",
            ctx.rules.decision.urgency_shoot_boost * urgency,
            0.0,
        ),
        CandidateAction::Drive { .. } => ConstraintResult::flagged(
            "CLOCK_URGENCY_DRIVE",
            ctx.rules.decision.urgency_drive_boost * urgency,
            0.0,
        ),
        CandidateAction::InboundPass { .. } => ConstraintResult::flagged(
            "CLOCK_URGENCY_INBOUND_PASS",
            ctx.rules.decision.urgency_pass_penalty * urgency,
            0.0,
        ),
        CandidateAction::Pass { .. } => ConstraintResult::flagged(
            "CLOCK_URGENCY_PASS_DEPRIORITIZE",
            0.0 - ctx.rules.decision.urgency_pass_penalty * urgency,
            0.0,
        ),
        CandidateAction::Dwell { .. } => ConstraintResult::flagged(
            "CLOCK_URGENCY_NO_HESITATE",
            -ctx.rules.decision.urgency_dwell_penalty * urgency,
            0.0,
        ),
    }
}

fn eval_contact_event(_ctx: &ConstraintContext, event: &GameEvent) -> ConstraintResult {
    if matches!(event, GameEvent::Contact { .. }) {
        ConstraintResult::flagged("CONTACT_FACT", 0.0, 0.0)
    } else {
        ConstraintResult::pass()
    }
}

macro_rules! constraint {
    ($name:ident, $id:literal, $domain:expr, $severity:expr, $priority:expr, $scope:expr, $timing:expr, $phases:expr, $when:expr, $evaluate:expr, $world:expr, $event:expr, $on_violation:expr) => {
        pub static $name: Constraint = Constraint {
            id: $id,
            domain: $domain,
            severity: $severity,
            priority: $priority,
            scope: $scope,
            timing: $timing,
            phases: $phases,
            when: $when,
            evaluate: $evaluate,
            evaluate_world: $world,
            evaluate_event: $event,
            on_violation: $on_violation,
        };
    };
}

constraint!(
    SHOT_CLOCK_CONSTRAINT,
    "shot_clock",
    ConstraintDomain::Semantic,
    Severity::Hard,
    10,
    ConstraintScope::Team,
    ConstraintTiming::Runtime,
    &[
        PhaseType::Transition,
        PhaseType::SetPlay,
        PhaseType::Resolution
    ],
    |ctx| ctx.shot_clock <= 0.0,
    eval_shot_clock_action,
    eval_shot_clock_world,
    pass_event,
    |_ctx| EnforcementAction::Violation {
        kind: ViolationKind::ShotClock
    }
);
constraint!(
    INBOUND_CLOCK_CONSTRAINT,
    "inbound_clock",
    ConstraintDomain::Semantic,
    Severity::Hard,
    15,
    ConstraintScope::Phase,
    ConstraintTiming::Runtime,
    &[PhaseType::Inbound],
    |ctx| ctx.phase == PhaseType::Inbound,
    pass_action,
    eval_inbound_clock_world,
    pass_event,
    |_ctx| EnforcementAction::Violation {
        kind: ViolationKind::InboundClock
    }
);
constraint!(
    BACKCOURT_CLOCK_CONSTRAINT,
    "backcourt_clock",
    ConstraintDomain::Semantic,
    Severity::Hard,
    15,
    ConstraintScope::Team,
    ConstraintTiming::Runtime,
    &[PhaseType::Transition, PhaseType::SetPlay],
    |ctx| ctx.is_live_ball(),
    pass_action,
    eval_backcourt_clock_world,
    pass_event,
    |_ctx| EnforcementAction::Violation {
        kind: ViolationKind::BackcourtClock
    }
);
constraint!(
    GAME_CLOCK_RUNTIME_CONSTRAINT,
    "game_clock_runtime",
    ConstraintDomain::Semantic,
    Severity::Hard,
    0,
    ConstraintScope::Game,
    ConstraintTiming::Runtime,
    &[
        PhaseType::TipOff,
        PhaseType::Transition,
        PhaseType::SetPlay,
        PhaseType::Resolution,
        PhaseType::Rebound
    ],
    |ctx| ctx.game_clock <= 0.0,
    pass_action,
    eval_game_clock_world,
    pass_event,
    |_ctx| EnforcementAction::EndPhase {
        next: PhaseType::DeadBallReset
    }
);
constraint!(
    OUT_OF_BOUNDS_PRE_CONSTRAINT,
    "out_of_bounds",
    ConstraintDomain::Physical,
    Severity::Hard,
    20,
    ConstraintScope::Player,
    ConstraintTiming::Pre,
    &[
        PhaseType::TipOff,
        PhaseType::Inbound,
        PhaseType::Transition,
        PhaseType::SetPlay,
        PhaseType::Resolution,
        PhaseType::Rebound
    ],
    |_ctx| true,
    eval_out_of_bounds_action,
    pass_world,
    pass_event,
    |_ctx| EnforcementAction::Violation {
        kind: ViolationKind::OutOfBounds
    }
);
constraint!(
    OUT_OF_BOUNDS_POST_CONSTRAINT,
    "out_of_bounds_event",
    ConstraintDomain::Physical,
    Severity::Hard,
    20,
    ConstraintScope::Ball,
    ConstraintTiming::Post,
    &[
        PhaseType::TipOff,
        PhaseType::Inbound,
        PhaseType::Transition,
        PhaseType::SetPlay,
        PhaseType::Resolution,
        PhaseType::Rebound
    ],
    |_ctx| true,
    pass_action,
    pass_world,
    eval_out_of_bounds_event,
    |_ctx| EnforcementAction::Turnover {
        reason: "OUT_OF_BOUNDS".to_string(),
    }
);
constraint!(
    DEAD_BALL_ACTION_CONSTRAINT,
    "dead_ball_action",
    ConstraintDomain::Semantic,
    Severity::Hard,
    30,
    ConstraintScope::Phase,
    ConstraintTiming::Pre,
    &[
        PhaseType::Timeout,
        PhaseType::DeadBallReset,
        PhaseType::Inbound
    ],
    |ctx| ctx.is_dead_ball() || ctx.phase == PhaseType::Inbound,
    eval_dead_ball_action,
    pass_world,
    pass_event,
    |_ctx| EnforcementAction::BlockAction
);
constraint!(
    GAME_CLOCK_ACTION_CONSTRAINT,
    "game_clock_expired",
    ConstraintDomain::Semantic,
    Severity::Hard,
    0,
    ConstraintScope::Game,
    ConstraintTiming::Pre,
    &[
        PhaseType::Transition,
        PhaseType::SetPlay,
        PhaseType::Resolution
    ],
    |ctx| ctx.game_clock <= 0.0,
    eval_game_clock_action,
    pass_world,
    pass_event,
    |_ctx| EnforcementAction::EndPhase {
        next: PhaseType::DeadBallReset
    }
);
constraint!(
    ACTION_ELIGIBILITY_CONSTRAINT,
    "action_eligibility",
    ConstraintDomain::Semantic,
    Severity::Hard,
    40,
    ConstraintScope::Game,
    ConstraintTiming::Pre,
    &[
        PhaseType::TipOff,
        PhaseType::Inbound,
        PhaseType::Transition,
        PhaseType::SetPlay,
        PhaseType::Resolution,
        PhaseType::Rebound,
        PhaseType::FreeThrow,
        PhaseType::Timeout,
        PhaseType::DeadBallReset
    ],
    |_ctx| true,
    eval_action_eligibility,
    pass_world,
    pass_event,
    |_ctx| EnforcementAction::BlockAction
);
constraint!(
    DRIVE_PRE_CONSTRAINT,
    "drive_execution",
    ConstraintDomain::Semantic,
    Severity::Soft,
    65,
    ConstraintScope::Player,
    ConstraintTiming::Pre,
    &[PhaseType::Transition, PhaseType::SetPlay],
    |_ctx| true,
    eval_drive,
    pass_world,
    pass_event,
    |_ctx| EnforcementAction::None
);
constraint!(
    RISKY_PASS_CONSTRAINT,
    "risky_pass",
    ConstraintDomain::Physical,
    Severity::Soft,
    60,
    ConstraintScope::Player,
    ConstraintTiming::Pre,
    &[
        PhaseType::Inbound,
        PhaseType::Transition,
        PhaseType::SetPlay
    ],
    |_ctx| true,
    eval_risky_pass,
    pass_world,
    pass_event,
    |_ctx| EnforcementAction::None
);
constraint!(
    CROWDED_RECEIVER_CONSTRAINT,
    "crowded_receiver",
    ConstraintDomain::Physical,
    Severity::Soft,
    70,
    ConstraintScope::Team,
    ConstraintTiming::Pre,
    &[
        PhaseType::Inbound,
        PhaseType::Transition,
        PhaseType::SetPlay
    ],
    |_ctx| true,
    eval_crowded_receiver,
    pass_world,
    pass_event,
    |_ctx| EnforcementAction::None
);
constraint!(
    CONTESTED_SHOT_CONSTRAINT,
    "contested_shot",
    ConstraintDomain::Semantic,
    Severity::Soft,
    80,
    ConstraintScope::Player,
    ConstraintTiming::Pre,
    &[PhaseType::Transition, PhaseType::SetPlay],
    |_ctx| true,
    eval_contested_shot,
    pass_world,
    pass_event,
    |_ctx| EnforcementAction::None
);
constraint!(
    SHOT_CLOCK_URGENCY_PREFERENCE,
    "shot_clock_urgency",
    ConstraintDomain::Semantic,
    Severity::Preference,
    100,
    ConstraintScope::Game,
    ConstraintTiming::Pre,
    &[PhaseType::Transition, PhaseType::SetPlay],
    |ctx| ctx.shot_clock < ctx.rules.shot_clock_urgency_seconds,
    eval_shot_clock_urgency,
    pass_world,
    pass_event,
    |_ctx| EnforcementAction::None
);
constraint!(
    CONTACT_FACT_CONSTRAINT,
    "contact_fact",
    ConstraintDomain::Physical,
    Severity::Soft,
    50,
    ConstraintScope::Player,
    ConstraintTiming::Post,
    &[
        PhaseType::TipOff,
        PhaseType::Inbound,
        PhaseType::Transition,
        PhaseType::SetPlay,
        PhaseType::Resolution,
        PhaseType::Rebound
    ],
    |_ctx| true,
    pass_action,
    pass_world,
    eval_contact_event,
    |_ctx| EnforcementAction::None
);

/// 单个候选动作的完整约束报告。
#[derive(Debug, Clone)]
pub struct ScoredCandidate {
    pub action: CandidateAction,
    pub feasible: bool,
    pub blocked_by: Option<&'static str>,
    pub constraint_penalty: f32,
    pub risk: f32,
    pub feasibility_score: f32,
    pub flags: Vec<(&'static str, String, f32)>,
}

/// 约束求值发现：结果和执行意图仍然是纯数据。
#[derive(Debug, Clone)]
pub struct ConstraintFinding {
    pub constraint: &'static Constraint,
    pub result: ConstraintResult,
    pub enforcement: EnforcementAction,
}

/// 约束注册中心：阶段激活集合从这里派生。
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
                &INBOUND_CLOCK_CONSTRAINT,
                &BACKCOURT_CLOCK_CONSTRAINT,
                &GAME_CLOCK_RUNTIME_CONSTRAINT,
                &OUT_OF_BOUNDS_PRE_CONSTRAINT,
                &OUT_OF_BOUNDS_POST_CONSTRAINT,
                &DEAD_BALL_ACTION_CONSTRAINT,
                &GAME_CLOCK_ACTION_CONSTRAINT,
                &ACTION_ELIGIBILITY_CONSTRAINT,
                &RISKY_PASS_CONSTRAINT,
                &CROWDED_RECEIVER_CONSTRAINT,
                &CONTESTED_SHOT_CONSTRAINT,
                &SHOT_CLOCK_URGENCY_PREFERENCE,
                &CONTACT_FACT_CONSTRAINT,
            ],
        }
    }

    /// 从注册中心增加约束，便于规则包/测试场景扩展，而非修改决策代码。
    pub fn register(&mut self, constraint: &'static Constraint) {
        self.all.push(constraint);
    }

    pub fn active_set(&self, ctx: &ConstraintContext) -> Vec<&'static Constraint> {
        let mut active: Vec<_> = self
            .all
            .iter()
            .copied()
            .filter(|c| c.is_active(ctx))
            .collect();
        active.sort_by_key(|c| (c.priority, c.id));
        active
    }

    /// 事前求值：硬约束在第一条违反处停止，软/偏好反馈全部保留。
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
            feasibility_score: 1.0,
            flags: Vec::new(),
        };
        for constraint in self
            .active_set(ctx)
            .into_iter()
            .filter(|c| c.timing == ConstraintTiming::Pre)
        {
            let result = (constraint.evaluate)(ctx, action);
            match result.status {
                ConstraintStatus::Pass => {}
                ConstraintStatus::Flagged { reason } => {
                    scored.constraint_penalty += result.utility_penalty;
                    scored.risk += result.risk_delta;
                    scored.feasibility_score =
                        (scored.feasibility_score - result.risk_delta * 0.5).clamp(0.0, 1.0);
                    scored
                        .flags
                        .push((constraint.id, reason, result.utility_penalty));
                }
                ConstraintStatus::Violate { reason } => {
                    scored.feasible = false;
                    scored.blocked_by = Some(constraint.id);
                    scored.flags.push((constraint.id, reason, 0.0));
                    break;
                }
            }
        }
        scored
    }
    /// 意图执行重校验（docs/architecture.md §5.2）：
    /// 当决策意图在当前 tick 真正执行时，基于最新物理几何快照重新检验硬约束可行性。
    pub fn revalidate_intent(&self, ctx: &ConstraintContext, action: &CandidateAction) -> Result<(), String> {
        let scored = self.evaluate_candidate(ctx, action);
        if !scored.feasible {
            Err(scored.blocked_by.unwrap_or("BLOCKED_BY_HARD_CONSTRAINT").to_string())
        } else {
            Ok(())
        }
    }

    /// 事中约束：只观察世界状态，不伪造候选动作。
    pub fn evaluate_runtime(&self, ctx: &ConstraintContext) -> Vec<ConstraintFinding> {
        self.active_set(ctx)
            .into_iter()
            .filter(|c| c.timing == ConstraintTiming::Runtime)
            .filter_map(|constraint| {
                let result = (constraint.evaluate_world)(ctx);
                (!matches!(result.status, ConstraintStatus::Pass)).then(|| ConstraintFinding {
                    constraint,
                    result,
                    enforcement: (constraint.on_violation)(ctx),
                })
            })
            .collect()
    }

    /// 事后约束：每个领域事实都进入统一裁决入口。
    pub fn evaluate_events(
        &self,
        ctx: &ConstraintContext,
        events: &[GameEvent],
    ) -> Vec<ConstraintFinding> {
        self.active_set(ctx)
            .into_iter()
            .filter(|c| c.timing == ConstraintTiming::Post)
            .flat_map(|constraint| {
                events.iter().filter_map(move |event| {
                    let result = (constraint.evaluate_event)(ctx, event);
                    (!matches!(result.status, ConstraintStatus::Pass)).then(|| ConstraintFinding {
                        constraint,
                        result,
                        enforcement: (constraint.on_violation)(ctx),
                    })
                })
            })
            .collect()
    }

    /// 兼容既有世界检查调用者：返回首个带执行意图的 Runtime 发现。
    pub fn evaluate_world(
        &self,
        ctx: &ConstraintContext,
    ) -> Option<(&'static Constraint, EnforcementAction)> {
        self.evaluate_runtime(ctx).into_iter().find_map(|finding| {
            (finding.enforcement != EnforcementAction::None)
                .then_some((finding.constraint, finding.enforcement))
        })
    }
}

impl ConstraintFinding {
    /// 稳定的调试/协议表示，不暴露闭包或内部指针。
    pub fn summary(&self) -> String {
        let reason = match &self.result.status {
            ConstraintStatus::Pass => "PASS".to_string(),
            ConstraintStatus::Flagged { reason } => reason.clone(),
            ConstraintStatus::Violate { reason } => reason.clone(),
        };
        format!("{}:{}", self.constraint.id, reason)
    }
}
