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
    // 规则语义：8 秒违例的主体是「控球队」，球无主（松球/篮板/停球）或
    // 已出手（投篮飞行）时不存在控球队，计时不得产生违例（seed 14 实测：
    // 停球在第 4 节开场被伪判 8 秒且无责任人）。计时器本身仍在时钟层按
    // 同一口径累加（phases.rs），两层用同一谓词，不另立第二口径。
    if ctx.backcourt_elapsed >= ctx.rules.backcourt_seconds
        && !ctx.is_in_frontcourt()
        && ctx.is_live_ball()
        && ctx.offense_has_possession()
    {
        ConstraintResult::violate("EIGHT_SECOND_BACKCOURT")
    } else {
        ConstraintResult::pass()
    }
}

pub(super) fn eval_three_second_lane_world(ctx: &ConstraintContext) -> ConstraintResult {
    // 规则语义（charter §6.2）：前场控制时进攻人在限制区连续停留超过
    // 时限。计时器在时钟层按同一控球谓词累加，控球结束自动清零——
    // 此处只需验证判据与责任人在场。
    if ctx.lane_dwell_seconds >= ctx.rules.three_second_lane_seconds
        && ctx.lane_dwell_player_id.is_some()
        && ctx.is_in_frontcourt()
        && ctx.is_live_ball()
        && ctx.offense_has_possession()
    {
        ConstraintResult::violate("THREE_SECOND_LANE")
    } else {
        ConstraintResult::pass()
    }
}

pub(super) fn eval_over_and_back_world(ctx: &ConstraintContext) -> ConstraintResult {
    // 规则语义（charter §6.2）：前场控制建立后，球回到后场即违例。
    // 判据与后场计时同一口径：控球延续（持球/运球/交接/传球飞行）期间
    // 球在后场且前场已建立 → 违例，责任人是最近触球者。
    //
    // 传球飞行中必须再看**目标点**：向前场推进的传球在飞行前半段球
    // 仍采样在中线后（合法推进），只有目标点本身在后场的传球才是
    // 「把球带回」（实测：过场传球曾被误判为回场，30 tick 内无人接到球）。
    let pass_returns_to_backcourt = match ctx.pass_target_pos {
        Some(target) => {
            let midcourt = ctx.rules.court.width_ft / 2.0;
            match ctx.possession_team {
                "home" => target.x < midcourt,
                "away" => target.x > midcourt,
                _ => false,
            }
        }
        // 非传球飞行（或无目标点）：球的位置就是事实。
        None => true,
    };
    if ctx.frontcourt_established
        && !ctx.is_in_frontcourt()
        && pass_returns_to_backcourt
        && ctx.is_live_ball()
        && ctx.offense_has_possession()
        && ctx.last_touch_player_id.is_some()
    {
        ConstraintResult::violate("OVER_AND_BACK")
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

/// 前场建立后，进攻方不得主动传球回后场（charter §6.2 回场条款的
/// 事前阻断）：这类传球在真实比赛里不存在，放行只会让运行时违例
/// 把回合变成白给球权。发球（`InboundPass`）不适用回场条款——
/// 发球期间无球队控制，球可以传向任何方向。
pub(super) fn eval_pass_backcourt_action(
    ctx: &ConstraintContext,
    action: &CandidateAction,
) -> ConstraintResult {
    if let CandidateAction::Pass { to_pos, .. } = action {
        let attacking_right = ctx.possession_team == "home";
        if ctx.frontcourt_established
            && ctx.is_live_ball()
            && ctx.rules.court.is_backcourt(*to_pos, attacking_right)
        {
            return ConstraintResult::violate("PASS_TO_BACKCOURT");
        }
    }
    ConstraintResult::pass()
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
            let has_ball = ctx.ball_holder_id == Some(player_id.as_str());
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
