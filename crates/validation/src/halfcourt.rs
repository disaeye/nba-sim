//! 按公开动作身份核对掩护片段。
//!
//! 这一层只读帧与事件：动作身份来自 `ACTION_STARTED`，机会几何来自帧
//! 里的球员位置与速度，感知证据来自决策记录的读取集合。执行层给出的
//! 接触、使用与路线由本层独立测量后再核对，引擎自己的结论不作为预期
//! 值。

use nba_protocol::{FrameEvent, StreamTick};
use serde::Serialize;

use crate::fragment::Finding;

#[derive(Debug, Serialize)]
pub struct ScreenSequence {
    pub action_id: String,
    pub handler_id: String,
    pub screener_id: String,
    pub defender_id: Option<String>,
    pub started_at: u64,
    pub start_event: u64,
    pub resolved_at: u64,
    pub terminal_event: u64,
    pub screen_event: Option<u64>,
    pub used: Option<bool>,
    pub route: Option<String>,
    pub disposition: String,
    pub decision: ScreenDecisionEvidence,
    pub geometry: ScreenGeometry,
}

/// 掩护开始时的几何与到达时间。
#[derive(Debug, Serialize)]
pub struct ScreenGeometry {
    /// 持球人与对位防守人的距离。
    pub handler_defender_gap_ft: f32,
    /// 掩护人与对位防守人的距离。
    pub screener_defender_gap_ft: f32,
    /// 掩护人与持球人的距离。
    pub screener_handler_gap_ft: f32,
    /// 掩护落点与对位防守人的距离：落点由防守人朝持球人的方向推出，
    /// 因此这个距离是掩护人真正要挤进去的位置。
    pub planned_spot_defender_gap_ft: f32,
    /// 掩护人走到落点所需的时间。
    pub screener_arrival_seconds: f32,
    /// 对位防守人走到落点所需的时间。
    pub defender_arrival_seconds: f32,
    /// 防守人比掩护人晚到多少秒：正数表示掩护先到。
    pub screen_window_margin_seconds: f32,
    /// 结算帧上掩护人与对位防守人的实测距离。
    pub resolved_screener_defender_gap_ft: f32,
    /// 结算帧上掩护人与落点的实测距离。
    pub resolved_screener_spot_gap_ft: f32,
    /// 结算帧上持球人与掩护人的实测距离。
    pub resolved_handler_screener_gap_ft: f32,
    /// 由实测距离判定的接触：与执行层给出的接触事实核对。
    pub contact_expected: bool,
    /// 掩护人是否到达决策时刻算出的落点：与执行层给出的使用事实核对。
    pub reached_spot_expected: bool,
    /// 由实测距离判定的持球人接近：与执行层给出的接近事实核对。
    pub handler_approached_expected: bool,
}

/// 决策证据：谁在什么时候看见了谁、选了什么、是否被重新校验。
#[derive(Debug, Serialize)]
pub struct ScreenDecisionEvidence {
    pub decision_id: u64,
    pub actor_id: String,
    pub observation_digest: u64,
    /// 决策者是否读到了对位防守人。
    pub defender_seen: Option<bool>,
    /// 决策者是否读到了掩护搭档。
    pub partner_seen: bool,
    /// 决策者没有读到的在场球员。
    pub unknown_players: Vec<String>,
    pub chosen_action: String,
    pub chosen_probability: f32,
    pub candidate_count: usize,
    /// 执行前的重校验结果：接受或被取消。
    pub revalidated: bool,
    pub revalidation: String,
}

pub fn screen_sequence(
    frames: &[StreamTick],
    events: &[FrameEvent],
    decisions: &[nba_sim_core::state::DecisionRecord],
) -> Result<ScreenSequence, Finding> {
    let finding = |tick, check, ids: Vec<u64>, detail: &str| Finding {
        check,
        tick,
        events: ids,
        detail: detail.to_string(),
    };
    let first_tick = frames
        .first()
        .ok_or_else(|| finding(0, "screen_frame", vec![], "缺少片段帧"))?;
    let start = screen_start_event(first_tick).ok_or_else(|| {
        finding(
            first_tick.frame.tick_index,
            "screen_start",
            vec![],
            "缺少掩护动作的开始事实",
        )
    })?;
    let (Some(id), Some(actor), Some(target), Some(holder)) = (
        start.action_id.as_deref(),
        field(start, "player_id"),
        field(start, "target_id"),
        first_tick.frame.ball.holder_id.as_deref(),
    ) else {
        return Err(finding(
            start.tick_index,
            "screen_identity",
            vec![start.event_id],
            "缺少掩护动作身份或参与人",
        ));
    };
    // 掩护有两个发起方向：持球人点名（发起者是持球人）与掩护人自己
    // 发起（发起者是掩护人）。两者都由持球人是谁决定谁是持球人、谁是
    // 掩护人。
    let (handler, screener) = if actor == holder {
        (actor, target)
    } else {
        (holder, actor)
    };
    let terminal = screen_terminal_event(events, id, actor).ok_or_else(|| {
        finding(
            frames.last().map(|tick| tick.frame.tick_index).unwrap_or(0),
            "screen_terminal",
            vec![start.event_id],
            "缺少同一动作的终态事实",
        )
    })?;
    let screen = events
        .iter()
        .find(|event| event.action_id.as_deref() == Some(id) && event.kind == "SCREEN");
    let defender = screen.and_then(|event| field(event, "defender_id"));
    let decision = screen_decision_evidence(first_tick, start, actor, target, defender, decisions)
        .map_err(|(check, detail)| finding(start.tick_index, check, vec![start.event_id], detail))?;
    let resolve_tick = frames
        .iter()
        .find(|tick| tick.frame.tick_index == terminal.tick_index)
        .ok_or_else(|| {
            finding(
                terminal.tick_index,
                "screen_frame",
                vec![start.event_id, terminal.event_id],
                "缺少掩护结算帧",
            )
        })?;
    let geometry = screen_geometry(
        first_tick,
        resolve_tick,
        handler,
        screener,
        defender.unwrap_or_default(),
        decision.planned_spot_ft,
    );
    let mut result = ScreenSequence {
        action_id: id.to_string(),
        handler_id: handler.to_string(),
        screener_id: screener.to_string(),
        defender_id: defender.map(str::to_string),
        started_at: start.tick_index,
        start_event: start.event_id,
        resolved_at: terminal.tick_index,
        terminal_event: terminal.event_id,
        screen_event: None,
        used: None,
        route: None,
        disposition: terminal.kind.clone(),
        decision: decision.evidence,
        geometry,
    };
    if let Some(screen) = screen {
        let ids = || vec![start.event_id, screen.event_id, terminal.event_id];
        let facts = screen_facts(screen).ok_or_else(|| {
            finding(
                screen.tick_index,
                "screen_execution",
                ids(),
                "掩护结算缺少路线、防守人或使用状态",
            )
        })?;
        if screen.tick_index != terminal.tick_index || screen.event_id >= terminal.event_id
            || field(screen, "handler_id") != Some(handler)
            || field(screen, "screener_id") != Some(screener)
            || (facts.used && terminal.kind != "ACTION_COMPLETED")
        {
            return Err(finding(
                screen.tick_index,
                "screen_execution",
                ids(),
                "掩护使用事实与动作终态不一致",
            ));
        }
        check_screen_gaps(resolve_tick, screen, handler, screener, &facts)
            .map_err(|(check, detail)| finding(screen.tick_index, check, ids(), detail))?;
        check_screen_geometry(resolve_tick, &facts, &result.geometry)
            .map_err(|(check, detail)| finding(screen.tick_index, check, ids(), detail))?;
        result.screen_event = Some(screen.event_id);
        result.used = Some(facts.used);
        result.route = Some(facts.route.to_string());
    } else if terminal.kind == "ACTION_COMPLETED" {
        return Err(finding(
            terminal.tick_index,
            "screen_execution",
            vec![start.event_id, terminal.event_id],
            "掩护动作完成时缺少结算事实",
        ));
    }
    Ok(result)
}

struct ScreenFacts {
    route: String,
    contact: bool,
    used: bool,
    approached: bool,
    /// 持球人与掩护人的距离。
    handler_screener_gap_ft: Option<f32>,
    /// 掩护人与防守人的距离。
    separation_ft: Option<f32>,
    displacement_ft: Option<f32>,
}

fn screen_facts(event: &FrameEvent) -> Option<ScreenFacts> {
    let route = field(event, "route")?;
    let data = event.data.as_ref()?;
    Some(ScreenFacts {
        route: route.to_string(),
        contact: data["contact"].as_bool()?,
        used: data["used"].as_bool()?,
        approached: data["handler_approached"].as_bool()?,
        handler_screener_gap_ft: data["handler_screener_gap_ft"].as_f64().map(|value| value as f32),
        separation_ft: data["separation_ft"].as_f64().map(|value| value as f32),
        displacement_ft: data["displacement_ft"].as_f64().map(|value| value as f32),
    })
}

/// 接触距离与接近距离的核对：执行层报出的距离必须与帧上的实测距离
/// 一致，实测距离又必须落在规则允许的接触范围内。
fn check_screen_gaps(
    resolve_tick: &StreamTick,
    screen: &FrameEvent,
    handler: &str,
    screener: &str,
    facts: &ScreenFacts,
) -> Result<(), (&'static str, &'static str)> {
    let rules = &resolve_tick.frame.rules;
    let threshold = rules.min_player_separation_ft + rules.contact_margin_ft;
    let tolerance = rules.max_player_speed_ftps * rules.tick_seconds;
    if facts.approached {
        let recorded = facts.handler_screener_gap_ft.ok_or((
            "screen_use_geometry",
            "缺少持球人与掩护人的距离",
        ))?;
        let measured = gap(resolve_tick, handler, screener).ok_or((
            "screen_use_geometry",
            "缺少持球人或掩护人的位置",
        ))?;
        if !recorded.is_finite() || recorded > threshold || measured > threshold + tolerance {
            return Err((
                "screen_use_geometry",
                "持球人与掩护人的距离超出掩护范围",
            ));
        }
    }
    if facts.contact {
        let recorded = facts.separation_ft.ok_or((
            "screen_contact_geometry",
            "缺少掩护接触距离",
        ))?;
        let measured = gap(resolve_tick, screener, field(screen, "defender_id").unwrap_or_default())
            .ok_or(("screen_contact_geometry", "缺少掩护接触参与人的位置"))?;
        if !recorded.is_finite() || recorded > threshold || measured > threshold + tolerance {
            return Err((
                "screen_contact_geometry",
                "掩护接触距离超出球员可接触范围",
            ));
        }
    }
    Ok(())
}

/// 几何模型的核对：执行层给出的接触与使用结果必须能由独立测量复现。
///
/// 接触看的是顶开之前防守人与掩护人的距离；使用还要加上掩护人是否真的
/// 到了决策时刻算出的落点，以及持球人是否真的靠了上来。
fn check_screen_geometry(
    resolve_tick: &StreamTick,
    facts: &ScreenFacts,
    geometry: &ScreenGeometry,
) -> Result<(), (&'static str, &'static str)> {
    let rules = &resolve_tick.frame.rules;
    let threshold = rules.min_player_separation_ft + rules.contact_margin_ft;
    // 顶开防守人的位移会把实测距离拉大：接触判定用的是顶开之前的距离。
    let pushed = facts.displacement_ft.unwrap_or_default().max(0.0);
    let pre_push_gap = geometry.resolved_screener_defender_gap_ft - pushed;
    if facts.contact != (pre_push_gap <= threshold) {
        return Err((
            "screen_geometry_model",
            "执行层的接触判定与独立几何测量不一致",
        ));
    }
    // 是否使用只由两件可测的事决定：掩护人是否真的碰到防守人（接触事实
    // 已由顶开前距离独立核对过），以及持球人是否真的靠了上来。掩护人站
    // 在哪个点不是引擎的判据，因此不拿它反推使用结果。
    let expected_used = facts.contact && geometry.handler_approached_expected;
    if facts.used != expected_used {
        return Err((
            "screen_geometry_model",
            "执行层给出的掩护使用结果与独立几何测量不一致",
        ));
    }
    Ok(())
}

fn screen_start_event(frame: &StreamTick) -> Option<&FrameEvent> {
    frame
        .frame
        .event_log
        .iter()
        .filter(|event| event.tick_index == frame.frame.tick_index)
        .find(|event| {
            event.kind == "ACTION_STARTED"
                && field(event, "action") == Some("Screen")
                && field(event, "role") == Some("Primary")
                && (field(event, "player_id") == frame.frame.ball.holder_id.as_deref()
                    || field(event, "target_id") == frame.frame.ball.holder_id.as_deref())
        })
}

fn screen_terminal_event<'a>(
    events: &'a [FrameEvent],
    action_id: &str,
    actor: &str,
) -> Option<&'a FrameEvent> {
    events.iter().find(|event| {
        event.action_id.as_deref() == Some(action_id)
            && field(event, "player_id") == Some(actor)
            && matches!(
                event.kind.as_str(),
                "ACTION_COMPLETED" | "ACTION_FAILED" | "ACTION_CANCELLED"
            )
    })
}

struct DecisionEvidence {
    evidence: ScreenDecisionEvidence,
    planned_spot_ft: glam::Vec2,
}

fn screen_decision_evidence(
    frame: &StreamTick,
    start: &FrameEvent,
    actor: &str,
    target: &str,
    defender: Option<&str>,
    decisions: &[nba_sim_core::state::DecisionRecord],
) -> Result<DecisionEvidence, (&'static str, &'static str)> {
    // 决策记录里的参与人是引擎句柄，帧里的球员带同一个句柄：两者因此
    // 可以直接对上，不靠名单顺序推断。
    let index = |id: &str| {
        frame
            .frame
            .players
            .iter()
            .find(|player| player.id == id && player.on_court)
            .map(|player| player.handle)
    };
    let actor_index = index(actor).ok_or(("screen_identity", "掩护发起人缺少球员索引"))?;
    let target_index = index(target).ok_or(("screen_identity", "掩护搭档缺少球员索引"))?;
    // 决策记录由做出选择的那名球员写下：持球人点名时是持球人，掩护人
    // 自己发起时是掩护人。两者都落在动作发起人身上。
    let decision = decisions
        .iter()
        .find(|decision| decision.tick == start.tick_index && decision.actor == actor_index)
        .ok_or(("screen_decision", "缺少对应发起人和动作时刻的决策记录"))?;
    let candidate = decision
        .candidates
        .get(decision.chosen)
        .ok_or(("screen_decision", "决策记录的所选候选索引无效"))?;
    if !matches!(candidate.kind, nba_sim_core::action::ActionKind::Screen(_))
        || candidate.target != Some(target_index)
        || field(start, "target_id") != Some(target)
    {
        return Err((
            "screen_decision",
            "决策记录的所选候选与已开始的掩护身份不一致",
        ));
    }
    let read = |player: usize| decision.player_reads.iter().any(|known| known.player == player);
    let partner_seen = read(target_index);
    let defender_seen = defender.and_then(index).map(read);
    let unknown_players = decision
        .unknown_players
        .iter()
        .filter_map(|player| {
            frame
                .frame
                .players
                .iter()
                .find(|candidate| candidate.handle == *player)
        })
        .map(|player| player.id.clone())
        .collect();
    let revalidation = match &decision.revalidation {
        nba_sim_core::state::Revalidation::Accepted => "accepted".to_string(),
        nba_sim_core::state::Revalidation::Cancelled(reason) => format!("cancelled:{reason}"),
    };
    Ok(DecisionEvidence {
        evidence: ScreenDecisionEvidence {
            decision_id: decision.decision_id,
            actor_id: actor.to_string(),
            observation_digest: decision.observation_digest,
            defender_seen,
            partner_seen,
            unknown_players,
            chosen_action: candidate.kind.as_str().to_string(),
            chosen_probability: candidate.probability,
            candidate_count: decision.candidates.len(),
            revalidated: matches!(
                decision.revalidation,
                nba_sim_core::state::Revalidation::Accepted
            ),
            revalidation,
        },
        planned_spot_ft: candidate.to_ft,
    })
}

/// 独立测量的掩护几何：起点距离、落点、到达时间与结算帧距离。
fn screen_geometry(
    start_frame: &StreamTick,
    resolve_frame: &StreamTick,
    handler: &str,
    screener: &str,
    defender: &str,
    planned_spot: glam::Vec2,
) -> ScreenGeometry {
    let handler_defender_gap_ft = gap(start_frame, handler, defender).unwrap_or_default();
    let screener_defender_gap_ft = gap(start_frame, screener, defender).unwrap_or_default();
    let screener_handler_gap_ft = gap(start_frame, screener, handler).unwrap_or_default();
    let defender_position = position(start_frame, defender).unwrap_or_default();
    let planned_spot_defender_gap_ft = (planned_spot - defender_position).length();
    let screener_arrival_seconds = arrival(start_frame, screener, planned_spot);
    let defender_arrival_seconds = arrival(start_frame, defender, planned_spot);
    let resolved_screener_defender_gap_ft = gap(resolve_frame, screener, defender).unwrap_or_default();
    let resolved_screener_spot_gap_ft = position(resolve_frame, screener)
        .map(|spot| (spot - planned_spot).length())
        .unwrap_or_default();
    let resolved_handler_screener_gap_ft = gap(resolve_frame, handler, screener).unwrap_or_default();
    let rules = &resolve_frame.frame.rules;
    let contact_threshold = rules.min_player_separation_ft + rules.contact_margin_ft;
    ScreenGeometry {
        handler_defender_gap_ft,
        screener_defender_gap_ft,
        screener_handler_gap_ft,
        planned_spot_defender_gap_ft,
        screener_arrival_seconds,
        defender_arrival_seconds,
        screen_window_margin_seconds: defender_arrival_seconds - screener_arrival_seconds,
        resolved_screener_defender_gap_ft,
        resolved_screener_spot_gap_ft,
        resolved_handler_screener_gap_ft,
        contact_expected: resolved_screener_defender_gap_ft <= contact_threshold,
        reached_spot_expected: resolved_screener_spot_gap_ft <= rules.body_contact_radius_ft,
        handler_approached_expected: resolved_handler_screener_gap_ft <= contact_threshold,
    }
}

fn field<'a>(event: &'a FrameEvent, key: &str) -> Option<&'a str> {
    event.data.as_ref()?.get(key)?.as_str()
}

fn player<'a>(frame: &'a StreamTick, id: &str) -> Option<&'a nba_protocol::RenderPlayer> {
    frame
        .frame
        .players
        .iter()
        .find(|player| player.id == id && player.on_court)
}

fn position(frame: &StreamTick, id: &str) -> Option<glam::Vec2> {
    let player = player(frame, id)?;
    Some(glam::Vec2::new(
        player.x * frame.frame.rules.court_width_ft,
        player.y * frame.frame.rules.court_height_ft,
    ))
}

fn velocity(frame: &StreamTick, id: &str) -> glam::Vec2 {
    let Some(player) = player(frame, id) else {
        return glam::Vec2::ZERO;
    };
    glam::Vec2::new(
        player.velocity_x * frame.frame.rules.court_width_ft,
        player.velocity_y * frame.frame.rules.court_height_ft,
    )
}

fn gap(frame: &StreamTick, left: &str, right: &str) -> Option<f32> {
    let left = position(frame, left)?;
    let right = position(frame, right)?;
    Some((left - right).length())
}

/// 一名球员走到目标点所需的时间：按当前速度方向加速到最高速度。
fn arrival(frame: &StreamTick, id: &str, target: glam::Vec2) -> f32 {
    let Some(position) = position(frame, id) else {
        return f32::INFINITY;
    };
    let rules = &frame.frame.rules;
    travel_seconds(
        target,
        position,
        velocity(frame, id),
        rules.max_player_speed_ftps,
        rules.max_player_accel_ftps2,
    )
}

fn travel_seconds(
    target: glam::Vec2,
    position: glam::Vec2,
    velocity: glam::Vec2,
    speed: f32,
    accel: f32,
) -> f32 {
    let offset = target - position;
    let distance = offset.length();
    if distance <= f32::EPSILON {
        return 0.0;
    }
    let speed = speed.max(f32::EPSILON);
    let accel = accel.max(f32::EPSILON);
    let along = velocity.dot(offset / distance).clamp(-speed, speed);
    let acceleration_time = (speed - along) / accel;
    let acceleration_distance = (speed + along) * acceleration_time * 0.5;
    if distance > acceleration_distance {
        acceleration_time + (distance - acceleration_distance) / speed
    } else {
        ((along * along + 2.0 * accel * distance).sqrt() - along) / accel
    }
}
