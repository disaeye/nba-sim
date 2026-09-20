//! 约束求值：每条约束的纯函数实现。
//!
//! 这些函数是 `Constraint` 上三个函数指针字段（`evaluate` / `evaluate_world` /
//! `evaluate_event`）的实现体，全部是 `(context, action|event) -> ConstraintResult`
//! 的纯函数：不修改上下文、不产生副作用、不依赖注册表。
//!
//! 与 `table.rs` 的分工：本模块提供「怎么判」，`table.rs` 声明「有哪些约束、
//! 何时激活、违规后果是什么」。分开是因为前者需要逐个单测求值逻辑，
//! 后者是声明式数据，改动频率与关注点都不同。

use super::{CandidateAction, ConstraintContext, ConstraintResult};
use nba_domain::flow::PhaseType;

pub(super) fn pass_action(_ctx: &ConstraintContext, _action: &CandidateAction) -> ConstraintResult {
    ConstraintResult::pass()
}
pub(super) fn pass_world(_ctx: &ConstraintContext) -> ConstraintResult {
    ConstraintResult::pass()
}
pub(super) fn pass_event(
    _ctx: &ConstraintContext,
    _event: &nba_domain::GameEvent,
) -> ConstraintResult {
    ConstraintResult::pass()
}
pub(super) fn eval_shot_clock_action(
    _ctx: &ConstraintContext,
    _action: &CandidateAction,
) -> ConstraintResult {
    ConstraintResult::pass()
}
pub(super) fn eval_shot_clock_world(ctx: &ConstraintContext) -> ConstraintResult {
    if ctx.shot_clock <= 0.0 && !ctx.is_ball_in_flight() && ctx.is_live_ball() {
        ConstraintResult::violate("SHOT_CLOCK_VIOLATION")
    } else {
        ConstraintResult::pass()
    }
}
pub(super) fn eval_inbound_clock_world(ctx: &ConstraintContext) -> ConstraintResult {
    if ctx.inbound_elapsed >= ctx.rules.inbound_seconds
        && ctx.phase == PhaseType::Inbound
        && ctx.is_dead_ball()
    {
        ConstraintResult::violate("FIVE_SECOND_INBOUND")
    } else {
        ConstraintResult::pass()
    }
}

pub(super) fn eval_backcourt_clock_world(ctx: &ConstraintContext) -> ConstraintResult {
    if ctx.backcourt_elapsed >= ctx.rules.backcourt_seconds
        && !ctx.is_in_frontcourt()
        && ctx.is_live_ball()
    {
        ConstraintResult::violate("EIGHT_SECOND_BACKCOURT")
    } else {
        ConstraintResult::pass()
    }
}

pub(super) fn eval_game_clock_world(ctx: &ConstraintContext) -> ConstraintResult {
    if ctx.game_clock <= 0.0 && !ctx.is_ball_in_flight() && ctx.is_live_ball() {
        ConstraintResult::violate("GAME_CLOCK_EXPIRED")
    } else {
        ConstraintResult::pass()
    }
}
pub(super) fn eval_out_of_bounds_action(
    ctx: &ConstraintContext,
    action: &CandidateAction,
) -> ConstraintResult {
    let in_bounds = match action {
        CandidateAction::Shoot { from_pos, .. } => ctx.rules.court.contains(*from_pos, 0.0),
        CandidateAction::Drive { target_pos, .. }
        | CandidateAction::PostUp { target_pos, .. }
        | CandidateAction::Advance { target_pos, .. } => ctx.rules.court.contains(*target_pos, 0.0),
        CandidateAction::Pass { to_pos, .. } => ctx.rules.court.contains(*to_pos, 0.0),
        CandidateAction::InboundPass {
            passer_id, to_pos, ..
        } => {
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
                    let near_ball = (p.pos_ft - ctx.ball_pos).length()
                        <= ctx.rules.inbound_boundary_tolerance_ft;
                    is_legal_oob && near_ball
                })
                .unwrap_or(false);
            to_in_bounds && passer_ready
        }
        CandidateAction::TripleThreatJab { pivot_pos, .. } => {
            ctx.rules.court.contains(*pivot_pos, 0.0)
        }
        CandidateAction::Dwell { .. } => true,
    };
    if in_bounds {
        ConstraintResult::pass()
    } else {
        ConstraintResult::violate("OUT_OF_BOUNDS")
    }
}

pub(super) fn eval_out_of_bounds_event(
    ctx: &ConstraintContext,
    event: &nba_domain::GameEvent,
) -> ConstraintResult {
    let is_boundary_cross = matches!(event, nba_domain::GameEvent::BoundaryCross { .. });
    if !is_boundary_cross {
        return ConstraintResult::pass();
    }
    let (is_ball_carrier, is_inbounding) = match event {
        nba_domain::GameEvent::BoundaryCross { player_id, pos, .. } => {
            let player = ctx.physics.get_player(player_id);
            // 第一性原理：判定「球员越过边界是否构成出界」必须依据
            // **球的权威归属**，而不是 physics 逐球员缓存 `has_ball`。
            //
            // `has_ball` 由 `sync_ball_holder` 在球态变更后同步；而
            // `BoundaryCross` 是 physics 在球态变更**之前**产生的物理事实，
            // 于是该 tick 上的 `has_ball` 可能仍是上一 tick 的旧值——实测
            // 因此把「已进入发球程序、球已离手」的球员误判为持球出界，
            // 每场产生 42 次虚假 `TURNOVER:OUT_OF_BOUNDS`。
            //
            // 权威来源是 `ctx.ball_phase`（由领域层 `BallState` 派生）：
            // 只有球处于「有明确持球人」的相位时，该球员才可能是出界的
            // 持球人。飞行/松球/发球/死球相位下，球员越界不构成球权违例。
            let ball_held = matches!(
                ctx.ball_phase,
                nba_domain::BallPhase::Held | nba_domain::BallPhase::Drive
            );
            let has_ball = player.map(|p| p.has_ball).unwrap_or(false) && ball_held;
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

pub(super) fn eval_dead_ball_action(
    ctx: &ConstraintContext,
    action: &CandidateAction,
) -> ConstraintResult {
    if ctx.is_dead_ball()
        && !action.is_inbound()
        && !matches!(
            action,
            CandidateAction::Dwell { .. } | CandidateAction::TripleThreatJab { .. }
        )
    {
        ConstraintResult::violate("DEAD_BALL_ACTION")
    } else if action.is_inbound() && ctx.phase != PhaseType::Inbound && !ctx.is_dead_ball() {
        ConstraintResult::violate("INBOUND_NOT_ALLOWED_IN_PHASE")
    } else {
        ConstraintResult::pass()
    }
}

pub(super) fn eval_game_clock_action(
    ctx: &ConstraintContext,
    action: &CandidateAction,
) -> ConstraintResult {
    if ctx.game_clock <= 0.0 && action.is_ball_action() {
        ConstraintResult::violate("GAME_CLOCK_EXPIRED")
    } else {
        ConstraintResult::pass()
    }
}

pub(super) fn eval_action_eligibility(
    ctx: &ConstraintContext,
    action: &CandidateAction,
) -> ConstraintResult {
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

pub(super) fn eval_drive(ctx: &ConstraintContext, action: &CandidateAction) -> ConstraintResult {
    let CandidateAction::Drive {
        driver_id,
        from_pos,
        target_pos,
        ..
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
    let target_depth = (start_distance - target_distance) / start_distance.max(f32::EPSILON);
    let depth_penalty = if target_depth > ctx.rules.tactics.drive_distance_ratio {
        (target_depth - ctx.rules.tactics.drive_distance_ratio)
            * ctx.rules.tactics.drive_distance_ratio
    } else {
        0.0
    };
    let penalty = -(density * ctx.rules.tactics.drive_distance_ratio
        + openness.contest_intensity * ctx.rules.shot_contest_sensitivity
        + depth_penalty);
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

pub(super) fn eval_risky_pass(
    ctx: &ConstraintContext,
    action: &CandidateAction,
) -> ConstraintResult {
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

pub(super) fn eval_crowded_receiver(
    ctx: &ConstraintContext,
    action: &CandidateAction,
) -> ConstraintResult {
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

pub(super) fn eval_contested_shot(
    ctx: &ConstraintContext,
    action: &CandidateAction,
) -> ConstraintResult {
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

pub(super) fn eval_shot_clock_urgency(
    ctx: &ConstraintContext,
    action: &CandidateAction,
) -> ConstraintResult {
    let urgency_window = ctx.rules.shot_clock_urgency_seconds.max(f32::EPSILON);
    let urgency = (1.0 - ctx.shot_clock / urgency_window).clamp(0.0, 1.0);
    match action {
        CandidateAction::Shoot { .. } => ConstraintResult::flagged(
            "CLOCK_URGENCY_BOOST",
            ctx.rules.decision.urgency_shoot_boost * urgency,
            0.0,
        ),
        CandidateAction::Drive { .. }
        | CandidateAction::PostUp { .. }
        | CandidateAction::Advance { .. } => ConstraintResult::flagged(
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
        CandidateAction::Dwell { .. } | CandidateAction::TripleThreatJab { .. } => {
            ConstraintResult::flagged(
                "CLOCK_URGENCY_NO_HESITATE",
                -ctx.rules.decision.urgency_dwell_penalty * urgency,
                0.0,
            )
        }
    }
}

pub(super) fn eval_contact_event(
    _ctx: &ConstraintContext,
    event: &nba_domain::GameEvent,
) -> ConstraintResult {
    if matches!(event, nba_domain::GameEvent::Contact { .. }) {
        ConstraintResult::flagged("CONTACT_FACT", 0.0, 0.0)
    } else {
        ConstraintResult::pass()
    }
}
