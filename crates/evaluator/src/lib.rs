//! 真实度评判器（M8 · quality 评判体系规范）。
//!
//! 独立 analyzer：读 ndjson tick 流（含 FrameEvent 载荷与
//! PossessionSummary），对照版本化参考分布 fixture，产出
//! `judgments.ndjson`（逐回合/逐阶段准则裁决）与
//! `attribution_report.json`（准则 × 子系统归因账本 + 真实度指数）。
//!
//! 分层纪律：本 crate 只依赖 protocol（+ fixture），不接触引擎内部；
//! 引擎不为本评判器改变任何行为（宪章 C2：评判是观测，不是干预）。

use nba_protocol::{FrameEvent, RenderFrame, StreamTick};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

pub mod fixture;
pub use fixture::ReferenceDistributions;
pub mod pbp;
pub use pbp::{convert_pbp_events_to_fixture, PbpEvent};

/// 裁决结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Defect,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Defect => "defect",
        }
    }
}

/// 单条准则裁决（quality 评判输出契约）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Judgment {
    pub criterion: String,
    pub verdict: Verdict,
    /// Hard = 结构因果断裂；Soft = 真实度偏差。
    pub severity: String,
    pub detail: String,
    /// 责任子系统（quality 归因表）。
    pub attribution: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub possession: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tick: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

impl Judgment {
    fn pass(criterion: &str, attribution: &'static str, possession: u64) -> Self {
        Self {
            criterion: criterion.to_string(),
            verdict: Verdict::Pass,
            severity: "hard".to_string(),
            detail: String::new(),
            attribution,
            possession: Some(possession),
            tick: None,
            seed: None,
        }
    }
    fn defect(
        criterion: &str,
        severity: &str,
        detail: String,
        attribution: &'static str,
        possession: u64,
    ) -> Self {
        Self {
            criterion: criterion.to_string(),
            verdict: Verdict::Defect,
            severity: severity.to_string(),
            detail,
            attribution,
            possession: Some(possession),
            tick: None,
            seed: None,
        }
    }
}

/// 归因账本（quality 归因规范）：准则 × 子系统聚合 + 真实度指数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributionReport {
    pub fixture_version: String,
    /// 真实度指数：裁决结果的加权聚合（Hard=1.0 / Soft=0.25），
    /// 聚合对象是裁决，不是原始结果（宪章 C2）。
    pub realism_index: f32,
    pub total_judgments: usize,
    pub defect_count: usize,
    /// (准则 → {子系统 → 违反次数})，按频次降序输出。
    pub defects_by_criterion: Vec<CriterionRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriterionRow {
    pub criterion: String,
    pub attribution: String,
    pub count: usize,
    pub hard: usize,
    pub soft: usize,
}

const W_HARD: f32 = 1.0;
const W_SOFT: f32 = 0.25;

fn weight(severity: &str) -> f32 {
    if severity == "soft" {
        W_SOFT
    } else {
        W_HARD
    }
}

pub fn attribution_report(judgments: &[Judgment], fixture_version: &str) -> AttributionReport {
    let mut total_w = 0.0f32;
    let mut defect_w = 0.0f32;
    let mut defect_count = 0usize;
    let mut by_criterion: BTreeMap<(String, &'static str), (usize, usize, usize)> = BTreeMap::new();
    for j in judgments {
        let w = weight(&j.severity);
        total_w += w;
        if j.verdict == Verdict::Defect {
            defect_w += w;
            defect_count += 1;
            let e = by_criterion
                .entry((j.criterion.clone(), j.attribution))
                .or_insert((0, 0, 0));
            e.0 += 1;
            if j.severity == "soft" {
                e.2 += 1;
            } else {
                e.1 += 1;
            }
        }
    }
    let realism_index = if total_w > 0.0 {
        1.0 - defect_w / total_w
    } else {
        1.0
    };
    let mut defects_by_criterion: Vec<CriterionRow> = by_criterion
        .into_iter()
        .map(
            |((criterion, attribution), (count, hard, soft))| CriterionRow {
                criterion,
                attribution: attribution.to_string(),
                count,
                hard,
                soft,
            },
        )
        .collect();
    defects_by_criterion.sort_by(|a, b| b.count.cmp(&a.count).then(a.criterion.cmp(&b.criterion)));
    AttributionReport {
        fixture_version: fixture_version.to_string(),
        realism_index,
        total_judgments: judgments.len(),
        defect_count,
        defects_by_criterion,
    }
}

/// 从 ndjson 流解析 tick 序列。
pub fn parse_stream(content: &str) -> Vec<StreamTick> {
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// 单回合上下文：从上一 POSSESSION_SUMMARY 到本条之间的全部事件。
type PassRelease = (String, String, (f32, f32), (f32, f32));

#[derive(Debug, Default)]
struct PossessionWindow {
    pass_releases: Vec<PassRelease>,
    pass_received: Vec<(String, (f32, f32))>,
    steals: Vec<(String, String, String, (f32, f32))>,
    tipped_passes: usize,
    loose_ball_secures: Vec<String>,
    shot_releases: Vec<(String, bool, f32)>,
    made_arrivals: usize,
    ft_made: usize,
    _rebounds: usize,
    drops: usize,
    violations: usize,
}

/// 逐回合 + 逐阶段评判主入口。
pub fn evaluate_stream(ticks: &[StreamTick], fixture: &ReferenceDistributions) -> Vec<Judgment> {
    let mut judgments = Vec::new();
    judgments.extend(evaluate_possessions(ticks, fixture));
    judgments.extend(evaluate_phases(ticks, fixture));
    judgments.extend(evaluate_game_level(ticks, fixture));
    judgments
}

fn event_data(frame: &RenderFrame, kind: &str) -> Vec<serde_json::Value> {
    frame
        .event_log
        .iter()
        .filter(|e: &&FrameEvent| e.kind == kind)
        .filter_map(|e| e.data.clone())
        // GameEvent 是外挂标签枚举（{"PassRelease": {...}}），剥掉标签层。
        .filter_map(|d| {
            d.as_object()
                .and_then(|o| o.values().next().cloned().filter(|_| o.len() == 1))
        })
        .collect()
}

fn evaluate_possessions(ticks: &[StreamTick], fixture: &ReferenceDistributions) -> Vec<Judgment> {
    let mut judgments = Vec::new();
    let mut window = PossessionWindow::default();
    // 首个回合（index 0）从流开始就已开窗：第一条 summary 关闭并裁决它。
    let mut window_open = true;
    let mut rules_cache: Option<nba_protocol::FrameRules> = None;

    for (tick_idx, tick) in ticks.iter().enumerate() {
        let frame = &tick.frame;
        if frame.rules.tick_seconds > 0.0 {
            rules_cache = Some(frame.rules.clone());
        }
        // POSSESSION_SUMMARY = 回合边界：在此事件之前发生的归前一回合，之后的归新回合
        let _summary_seq = frame
            .event_log
            .iter()
            .find(|e| e.kind == "POSSESSION_SUMMARY")
            .map(|e| e.sequence);

        for entry in &frame.event_log {
            let raw_d = match &entry.data {
                Some(v) => v,
                None => continue,
            };
            let d = raw_d
                .as_object()
                .and_then(|o| {
                    if o.len() == 1 {
                        o.values().next()
                    } else {
                        None
                    }
                })
                .unwrap_or(raw_d);

            if entry.kind == "POSSESSION_SUMMARY" {
                if let Ok(summary) = serde_json::from_value::<PossessionSummaryData>(d.clone()) {
                    if window_open {
                        judgments.extend(evaluate_possession_window(
                            &window, &summary, fixture, tick_idx,
                        ));
                    }
                    window = PossessionWindow::default();
                    window_open = true;
                }
                continue;
            }

            let raw_d = match &entry.data {
                Some(v) => v,
                None => continue,
            };
            let d = raw_d
                .as_object()
                .and_then(|o| {
                    if o.len() == 1 {
                        o.values().next()
                    } else {
                        None
                    }
                })
                .unwrap_or(raw_d);
            match entry.kind.as_str() {
                "PASS" => {
                    if let Ok(ev) = serde_json::from_value::<PassReleaseData>(d.clone()) {
                        window.pass_releases.push((
                            ev.passer_id,
                            ev.receiver_id,
                            ev.from_pos,
                            ev.to_pos,
                        ));
                    }
                }
                "PASS_RECEIVED" => {
                    if let Ok(ev) = serde_json::from_value::<PassReceivedData>(d.clone()) {
                        window.pass_received.push((ev.receiver_id, ev.position));
                    }
                }
                "STEAL" => {
                    if let Ok(ev) = serde_json::from_value::<PassInterceptedData>(d.clone()) {
                        window.steals.push((
                            ev.passer_id,
                            ev.receiver_id,
                            ev.defender_id,
                            ev.position,
                        ));
                    }
                }
                "PASS_DROPPED" => {
                    window.drops += 1;
                    window.pass_releases.pop();
                }
                "SHOT_RELEASE" => {
                    if let Ok(ev) = serde_json::from_value::<ShotReleaseData>(d.clone()) {
                        window
                            .shot_releases
                            .push((ev.shooter_id, ev.is_three, ev.contest_level));
                    }
                }
                "FREE_THROW" => {
                    if let Ok(ev) = serde_json::from_value::<FreeThrowData>(d.clone()) {
                        if ev.made {
                            window.ft_made += 1;
                        }
                    }
                }
                "PASS_TIPPED" => {
                    window.tipped_passes += 1;
                }
                "LOOSE_BALL_SECURED" => {
                    if let Ok(ev) = serde_json::from_value::<LooseBallSecuredData>(d.clone()) {
                        window.loose_ball_secures.push(ev.player_id);
                    }
                }
                "VIOLATION" | "RULE_VIOLATION" | "ENFORCEMENT_APPLIED" => {
                    window.violations += 1;
                }
                _ => {}
            }
        }

        let _ = rules_cache;
    }
    judgments
}

fn evaluate_possession_window(
    window: &PossessionWindow,
    summary: &PossessionSummaryData,
    fixture: &ReferenceDistributions,
    tick_idx: usize,
) -> Vec<Judgment> {
    let mut out = Vec::new();
    let idx = summary.possession_index;
    let _ = tick_idx;

    // ---- (a) 结构因果 ----
    // 回合时长边界：下界 0（发球即断可 <1s），上界 24s + 进攻篮板延长。
    let dur = summary.duration_seconds;
    if dur < 0.0 || dur > 24.0 + 14.0 + 2.0 {
        out.push(Judgment::defect(
            "POSSESSION_DURATION_BOUNDS",
            "hard",
            format!("possession {} duration {}s outside [0, 40]", idx, dur),
            "engine",
            idx,
        ));
    } else {
        out.push(Judgment::pass("POSSESSION_DURATION_BOUNDS", "engine", idx));
    }

    // 接球人走廊可达：release 的冻结线段与 arrival 事实位置一致。
    let corridor = fixture.pass_corridor_radius_ft;
    let mut remaining_received = window.pass_received.clone();
    for (passer, receiver, from, to) in &window.pass_releases {
        if let Some(pos_idx) = remaining_received.iter().position(|(id, _)| id == receiver) {
            let (_, receiver_pos) = remaining_received.remove(pos_idx);
            let dist = point_segment_distance_ft(receiver_pos, *from, *to);
            if dist > corridor + 1.0 {
                out.push(Judgment::defect(
                    "PASS_CORRIDOR_REACHABLE",
                    "hard",
                    format!(
                        "receiver {} arrived {:.1} ft from pass target (passer {})",
                        receiver, dist, passer
                    ),
                    "decision",
                    idx,
                ));
            } else {
                out.push(Judgment::pass("PASS_CORRIDOR_REACHABLE", "decision", idx));
            }
        }
    }
    // 抢断走廊：抢断位置必须在 passer→receiver 连线走廊半径内。
    for (passer, receiver, defender, pos) in &window.steals {
        if let Some((_p, _r, from, to)) = window
            .pass_releases
            .iter()
            .find(|(p, r, _, _)| p == passer && r == receiver)
        {
            let dist = point_segment_distance_ft(*pos, *from, *to);
            if dist > corridor + 1.5 {
                out.push(Judgment::defect(
                    "STEAL_CORRIDOR",
                    "hard",
                    format!(
                        "steal by {} {:.1} ft off the {}->{} pass corridor",
                        defender, dist, passer, receiver
                    ),
                    "physics+semantics",
                    idx,
                ));
            } else {
                out.push(Judgment::pass("STEAL_CORRIDOR", "physics+semantics", idx));
            }
        }
    }

    // 得分因果：得分回合必须有出手或罚球命中来源。
    if summary.terminal_event == "SCORE" && window.made_arrivals == 0 && window.ft_made == 0 {
        out.push(Judgment::defect(
            "SCORE_SOURCE_CAUSALITY",
            "hard",
            format!("possession {} scored without shot/free-throw source", idx),
            "engine+invariants",
            idx,
        ));
    } else if summary.terminal_event == "SCORE" {
        out.push(Judgment::pass(
            "SCORE_SOURCE_CAUSALITY",
            "engine+invariants",
            idx,
        ));
    }

    // 失误归因：终端标签必须与同一回合内的原因事实一致；不能用
    // “有任意一个失误事件”掩盖 PASS_TIPPED/STEAL/PASS_DROPPED 的错配。
    if summary.terminal_event.starts_with("TURNOVER") {
        let cause_present = match summary.terminal_event.as_str() {
            "TURNOVER_STEAL" => !window.steals.is_empty(),
            "TURNOVER_PASS_TIPPED" => window.tipped_passes > 0,
            "TURNOVER_PASS_DROPPED" => window.drops > 0,
            "TURNOVER_VIOLATION" => window.violations > 0,
            "TURNOVER_LOOSE_BALL" => !window.loose_ball_secures.is_empty(),
            _ => {
                !window.steals.is_empty()
                    || window.drops > 0
                    || window.tipped_passes > 0
                    || window.violations > 0
                    || !window.loose_ball_secures.is_empty()
            }
        };
        if cause_present {
            out.push(Judgment::pass(
                "TURNOVER_ATTRIBUTION",
                "engine+invariants",
                idx,
            ));
        } else {
            out.push(Judgment::defect(
                "TURNOVER_ATTRIBUTION",
                "hard",
                format!(
                    "possession {} terminal {} has no matching cause fact",
                    idx, summary.terminal_event
                ),
                "engine+invariants",
                idx,
            ));
        }

        if summary.turnover_player_id.is_some() {
            out.push(Judgment::pass(
                "TURNOVER_ACTOR_CONSISTENCY",
                "engine+invariants",
                idx,
            ));
        } else {
            out.push(Judgment::defect(
                "TURNOVER_ACTOR_CONSISTENCY",
                "hard",
                format!("possession {} turnover has no turnover_player_id", idx),
                "engine+invariants",
                idx,
            ));
        }
    }

    // 出手质量记录一致性：得分回合必须带出手者与 contest 记录（quality 防守干扰规范）。
    if summary.terminal_event == "SCORE" {
        match (&summary.shooter_id, summary.shot_contest_intensity) {
            (Some(_), Some(c)) if (0.0..=1.0).contains(&c) => {
                out.push(Judgment::pass(
                    "CONTEST_CONSISTENCY",
                    "physics+semantics",
                    idx,
                ));
            }
            (Some(_), None) => out.push(Judgment::defect(
                "CONTEST_CONSISTENCY",
                "hard",
                format!("possession {} scored without contest intensity record", idx),
                "physics+semantics",
                idx,
            )),
            _ => out.push(Judgment::defect(
                "CONTEST_CONSISTENCY",
                "hard",
                format!("possession {} scored without shooter record", idx),
                "physics+semantics",
                idx,
            )),
        }
    }

    let outcome = fixture.classify_outcome(&summary.terminal_event);
    if let Some(band) = fixture.duration_band(outcome) {
        if !band.contains(&dur) {
            out.push(Judgment::defect(
                "RHYTHM_DURATION",
                "soft",
                format!(
                    "possession {} duration {:.1}s outside {:?} band for outcome {}",
                    idx, dur, band, outcome
                ),
                "decision",
                idx,
            ));
        } else {
            out.push(Judgment::pass("RHYTHM_DURATION", "decision", idx));
        }
    }
    if let Some(band) = fixture.passes_band(outcome) {
        let passes = summary.passes_count as f32;
        if !band.contains(&passes) {
            out.push(Judgment::defect(
                "ACTION_COMPOSITION_PASSES",
                "soft",
                format!(
                    "possession {} pass count {} outside {:?} band for outcome {}",
                    idx, passes, band, outcome
                ),
                "decision",
                idx,
            ));
        } else {
            out.push(Judgment::pass("ACTION_COMPOSITION_PASSES", "decision", idx));
        }
    }

    // 出手质量：重兵盯防下的强投占比（contest > 阈值）。
    let mut heavy_contests = 0;
    for (shooter, _is_three, contest) in &window.shot_releases {
        if *contest > fixture.heavy_contest_threshold {
            heavy_contests += 1;
            out.push(Judgment::defect(
                "SHOT_QUALITY_CONTEST",
                "soft",
                format!(
                    "shot by {} under heavy contest ({:.2}) in possession {}",
                    shooter, contest, idx
                ),
                "decision",
                idx,
            ));
        }
    }
    if !window.shot_releases.is_empty() && heavy_contests == 0 {
        out.push(Judgment::pass("SHOT_QUALITY_CONTEST", "decision", idx));
    }
    out
}

fn point_segment_distance_ft(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let abx = b.0 - a.0;
    let aby = b.1 - a.1;
    let apx = p.0 - a.0;
    let apy = p.1 - a.1;
    let len2 = abx * abx + aby * aby;
    let t = if len2 <= f32::EPSILON {
        0.0
    } else {
        ((apx * abx + apy * aby) / len2).clamp(0.0, 1.0)
    };
    let cx = a.0 + t * abx;
    let cy = a.1 + t * aby;
    ((p.0 - cx) * (p.0 - cx) + (p.1 - cy) * (p.1 - cy)).sqrt()
}

/// 阶段裁决（quality 阶段转换规范）：转换合法性 + 停留时长。
fn evaluate_phases(ticks: &[StreamTick], fixture: &ReferenceDistributions) -> Vec<Judgment> {
    let mut out = Vec::new();
    let mut prev_phase: Option<String> = None;
    let mut phase_start_t: f32 = 0.0;

    for tick in ticks {
        let frame = &tick.frame;
        let phase = frame.phase.clone();
        if Some(&phase) != prev_phase.as_ref() {
            // 出上一阶段的停留裁决
            if let (Some(prev), Some(band)) = (prev_phase.clone(), fixture.phase_dwell_band(&phase))
            {
                let dwell = frame.t_game - phase_start_t;
                if dwell > band.1 {
                    out.push(Judgment::defect(
                        "PHASE_DWELL_TIME",
                        "soft",
                        format!(
                            "phase {} dwelled {:.1}s (band max {:.1})",
                            prev, dwell, band.1
                        ),
                        "engine",
                        frame.completed_possessions as u64,
                    ));
                }
            }
            // 转换合法性
            if let Some(prev) = &prev_phase {
                if !fixture.phase_transition_allowed(prev, &phase) {
                    out.push(Judgment::defect(
                        "PHASE_TRANSITION_LEGALITY",
                        "hard",
                        format!("illegal phase transition {} -> {}", prev, phase),
                        "engine",
                        frame.completed_possessions as u64,
                    ));
                }
            }
            prev_phase = Some(phase);
            phase_start_t = frame.t_game;
        }
    }
    out
}

/// 比赛级聚合准则（回合裁决的分布对照，宪章 C2 允许的聚合形式）。
fn evaluate_game_level(ticks: &[StreamTick], fixture: &ReferenceDistributions) -> Vec<Judgment> {
    let mut out = Vec::new();
    let mut turnovers = 0usize;
    let mut possessions = 0usize;
    let three_attempts = 0usize;
    let shot_attempts = 0usize;
    let mut idx: Option<u64> = None;

    let mut total_decisions = 0usize;
    let mut intent_revalidation_blocked = 0usize;
    for (tick_idx, tick) in ticks.iter().enumerate() {
        idx = idx.or(Some(tick_idx as u64));
        if let Some(debug) = &tick.frame.debug {
            total_decisions += 1;
            if debug
                .enforcement
                .iter()
                .any(|e| e.starts_with("INTENT_REVALIDATION_BLOCKED"))
            {
                intent_revalidation_blocked += 1;
            }
        }
        let summaries = event_data(&tick.frame, "POSSESSION_SUMMARY")
            .into_iter()
            .filter_map(|d| serde_json::from_value::<PossessionSummaryData>(d).ok());
        for s in summaries {
            possessions += 1;
            idx = Some(s.possession_index);
            if s.terminal_event.starts_with("TURNOVER") {
                turnovers += 1;
            }
        }
    }

    if possessions >= 10 {
        let to_rate = turnovers as f32 / possessions as f32;
        if !fixture.turnover_rate_band.contains(&to_rate) {
            out.push(Judgment::defect(
                "TURNOVER_RATE",
                "soft",
                format!(
                    "game turnover rate {:.2} outside {:?} over {} possessions",
                    to_rate, fixture.turnover_rate_band, possessions
                ),
                "decision",
                idx.unwrap_or(0),
            ));
        } else {
            out.push(Judgment::pass(
                "TURNOVER_RATE",
                "decision",
                idx.unwrap_or(0),
            ));
        }
        if shot_attempts >= 10 {
            let three_rate = three_attempts as f32 / shot_attempts as f32;
            if !fixture.three_attempt_rate_band.contains(&three_rate) {
                out.push(Judgment::defect(
                    "THREE_ATTEMPT_RATE",
                    "soft",
                    format!(
                        "three-point attempt share {:.2} outside {:?}",
                        three_rate, fixture.three_attempt_rate_band
                    ),
                    "decision",
                    idx.unwrap_or(0),
                ));
            } else {
                out.push(Judgment::pass(
                    "THREE_ATTEMPT_RATE",
                    "decision",
                    idx.unwrap_or(0),
                ));
            }
        }
    }
    if total_decisions >= 50 {
        let downgrade_rate = intent_revalidation_blocked as f32 / total_decisions as f32;
        let max_downgrade_rate = 0.20;
        if downgrade_rate > max_downgrade_rate {
            out.push(Judgment::defect(
                "INTENT_DOWNGRADE_RATE",
                "soft",
                format!(
                    "intent revalidation blocked rate {:.2}% ({}/{}) exceeds threshold {:.2}%",
                    downgrade_rate * 100.0,
                    intent_revalidation_blocked,
                    total_decisions,
                    max_downgrade_rate * 100.0
                ),
                "decision",
                idx.unwrap_or(0),
            ));
        } else {
            out.push(Judgment::pass(
                "INTENT_DOWNGRADE_RATE",
                "decision",
                idx.unwrap_or(0),
            ));
        }
    }

    // ---- 覆盖性（M8 验收：回合零遗漏）----
    // possession_index 必须从 0 连续；缺口 = 某条结束路径没有产出回合总结。
    let seen = observed_possession_indices(ticks);
    if let Some(&max) = seen.iter().max() {
        let missing: Vec<u64> = (0..=max).filter(|i| !seen.contains(i)).collect();
        if !missing.is_empty() {
            out.push(Judgment::defect(
                "POSSESSION_COVERAGE",
                "hard",
                format!(
                    "{} possession summary missing: {:?} (engine emit-path gap)",
                    missing.len(),
                    &missing[..missing.len().min(8)]
                ),
                "engine",
                max,
            ));
        } else {
            out.push(Judgment::pass("POSSESSION_COVERAGE", "engine", max));
        }
    }
    out
}

// ---- 流内事件载荷的松散镜像（只取评判所需字段）----
#[derive(Deserialize)]
struct PassReleaseData {
    passer_id: String,
    receiver_id: String,
    from_pos: (f32, f32),
    to_pos: (f32, f32),
}
#[derive(Deserialize)]
struct PassReceivedData {
    receiver_id: String,
    position: (f32, f32),
}
#[derive(Deserialize)]
struct LooseBallSecuredData {
    player_id: String,
}
#[derive(Deserialize)]
struct PassInterceptedData {
    passer_id: String,
    receiver_id: String,
    defender_id: String,
    position: (f32, f32),
}
#[derive(Deserialize)]
struct ShotReleaseData {
    shooter_id: String,
    is_three: bool,
    contest_level: f32,
}
#[derive(Deserialize)]
struct FreeThrowData {
    made: bool,
}
#[derive(Deserialize)]
struct PossessionSummaryData {
    possession_index: u64,
    duration_seconds: f32,
    passes_count: u32,
    terminal_event: String,
    #[serde(default)]
    shooter_id: Option<String>,
    #[serde(default)]
    shot_contest_intensity: Option<f32>,
    #[serde(default)]
    turnover_player_id: Option<String>,
}

/// 判定已见事件集合（用于覆盖性检查的测试辅助）。
pub fn observed_possession_indices(ticks: &[StreamTick]) -> HashSet<u64> {
    let mut seen = HashSet::new();
    for tick in ticks {
        for d in event_data(&tick.frame, "POSSESSION_SUMMARY") {
            if let Ok(s) = serde_json::from_value::<PossessionSummaryData>(d) {
                seen.insert(s.possession_index);
            }
        }
    }
    seen
}

/// 便于外部（CLI）把 HashMap 聚合用于展示的辅助。
pub type JudgmentMap = HashMap<String, usize>;
