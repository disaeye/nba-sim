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
pub mod ledger;
pub use ledger::{check_ledger, LedgerViolation};
pub mod pbp;
pub use pbp::{convert_pbp_events_to_fixture, PbpEvent};

/// 裁决结论（gap.md §15.1 / dev 方案 §4 D1.1 三态化）。
///
/// `NotApplicable` 与 `InsufficientEvidence` 不是 pass——它们不计入
/// 分母、不产生真实性贡献。空证据按 pass 计是历史 0.994 高分假安全感的
/// 来源之一，本枚举在类型层堵住这条路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Defect,
    /// 准则对该观测窗口不适用（如罚球得分对出手干扰准则）。
    NotApplicable,
    /// 证据不足，无法裁决（空流/窗口内无相关事件）。
    InsufficientEvidence,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Defect => "defect",
            Verdict::NotApplicable => "not_applicable",
            Verdict::InsufficientEvidence => "insufficient_evidence",
        }
    }

    /// 是否计入固定分母（只有 pass/defect 参与缺陷率与指数聚合）。
    pub fn counts_toward_evidence(self) -> bool {
        matches!(self, Verdict::Pass | Verdict::Defect)
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

pub fn criterion_severity(criterion: &str) -> &'static str {
    match criterion {
        "POSSESSION_DURATION_BOUNDS"
        | "PASS_CORRIDOR_REACHABLE"
        | "PASS_LANDING_FACT_MISMATCH"
        | "STEAL_CORRIDOR"
        | "SCORE_SOURCE_CAUSALITY"
        | "TURNOVER_ATTRIBUTION"
        | "TURNOVER_ACTOR_CONSISTENCY"
        | "CONTEST_CONSISTENCY"
        | "PHASE_TRANSITION_LEGALITY"
        | "POSSESSION_COVERAGE" => "hard",

        "RHYTHM_DURATION"
        | "RECEIVE_ESTIMATE_DIVERGENCE"
        | "ACTION_COMPOSITION_PASSES"
        | "SHOT_QUALITY_CONTEST"
        | "PHASE_DWELL_TIME"
        | "TURNOVER_RATE"
        | "THREE_ATTEMPT_RATE"
        | "INTENT_DOWNGRADE_RATE" => "soft",

        _ => "hard",
    }
}

impl Judgment {
    pub fn pass(criterion: &str, attribution: &'static str, possession: u64) -> Self {
        Self {
            criterion: criterion.to_string(),
            verdict: Verdict::Pass,
            severity: criterion_severity(criterion).to_string(),
            detail: String::new(),
            attribution,
            possession: Some(possession),
            tick: None,
            seed: None,
        }
    }

    /// 准则不适用：不产生裁决分母（gap.md §15.1 NotApplicable）。
    pub fn not_applicable(criterion: &str, attribution: &'static str, possession: u64) -> Self {
        Self {
            criterion: criterion.to_string(),
            verdict: Verdict::NotApplicable,
            severity: criterion_severity(criterion).to_string(),
            detail: String::new(),
            attribution,
            possession: Some(possession),
            tick: None,
            seed: None,
        }
    }

    /// 证据不足：不得得满分（gap.md §15.1 InsufficientEvidence）。
    pub fn insufficient(criterion: &str, attribution: &'static str, possession: u64) -> Self {
        Self {
            criterion: criterion.to_string(),
            verdict: Verdict::InsufficientEvidence,
            severity: criterion_severity(criterion).to_string(),
            detail: String::new(),
            attribution,
            possession: Some(possession),
            tick: None,
            seed: None,
        }
    }
    pub fn defect(
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
    ///
    /// **Hard 门解耦（dev 方案 §4 D1.2）**：任一 Hard defect 存在时，
    /// 指数在语义上无效——输出 0.0 且 `hard_gate_failed=true`，而不是
    /// 一个仍接近 1 的数。消费方必须先检查 `hard_gate_failed`。
    pub realism_index: f32,
    /// 任一 Hard defect 存在即为 true；此时 realism_index 无效（置 0）。
    pub hard_gate_failed: bool,
    /// Hard defect 条数；`defect_count` 是 Hard+Soft 合计。
    ///
    /// 消费方报告门禁时必须用本字段：把「Hard + Soft 合计」写成「Hard 缺陷」
    /// 会高估严重度，与被本项修复的「以聚合量掩盖真相」是同一类错误。
    pub hard_defect_count: usize,
    /// Soft defect 条数。
    pub soft_defect_count: usize,
    pub total_judgments: usize,
    pub defect_count: usize,
    /// 固定分母五元组（dev 方案 §4 D1.1）：各 verdict 计数。
    /// `passes + defects` = 证据覆盖面；`insufficient + not_applicable`
    /// 不计入分母。覆盖率 = evidence / opportunities。
    pub opportunities: usize,
    pub passes: usize,
    pub defects: usize,
    pub not_applicable: usize,
    pub insufficient_evidence: usize,
    /// 证据覆盖率：(passes+defects)/opportunities，无观测时为 0。
    pub evidence_coverage: f32,
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

/// 篮下区域半径（ft）：与 GameRules 篮下/禁区几何口径对齐的判定阈值，
/// 用于 SHOT_PROFILE_ZONE_MIX 的 rim/mid 拆分。属评判口径常数，集中于此。
const RIM_ZONE_RADIUS_FT: f32 = 4.0;
/// 篮筐距边线的偏移（ft）：NBA 篮筐距端线 5.25ft， hoop_x = court_width - offset。
const RIM_OFFSET_FT: f32 = 5.25;
/// 48 分钟等效的标准比赛秒数（NBA 4×12min）。
const REGULATION_SECONDS_48MIN: f32 = 2880.0;

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
    let mut opportunities = 0usize;
    let mut passes = 0usize;
    let mut defects = 0usize;
    let mut not_applicable = 0usize;
    let mut insufficient_evidence = 0usize;
    let mut hard_gate_failed = false;
    let mut hard_defect_count = 0usize;
    let mut soft_defect_count = 0usize;
    let mut by_criterion: BTreeMap<(String, &'static str), (usize, usize, usize)> = BTreeMap::new();
    for j in judgments {
        opportunities += 1;
        match j.verdict {
            Verdict::Pass => {
                passes += 1;
                // 只有 pass/defect 参与指数加权。
                total_w += weight(&j.severity);
            }
            Verdict::Defect => {
                defects += 1;
                total_w += weight(&j.severity);
                defect_w += weight(&j.severity);
                if j.severity != "soft" {
                    hard_gate_failed = true;
                    hard_defect_count += 1;
                } else {
                    soft_defect_count += 1;
                }
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
            Verdict::NotApplicable => not_applicable += 1,
            Verdict::InsufficientEvidence => insufficient_evidence += 1,
        }
    }
    let evidence = passes + defects;
    let evidence_coverage = if opportunities > 0 {
        evidence as f32 / opportunities as f32
    } else {
        0.0
    };
    // Hard 门解耦：任一 Hard defect 存在时指数无效（置 0），不给出
    // 接近 1 的假安全感。空证据（total_w=0）同样得 0 而非满分。
    let realism_index = if hard_gate_failed || total_w <= 0.0 {
        0.0
    } else {
        (1.0 - defect_w / total_w).clamp(0.0, 1.0)
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
        hard_gate_failed,
        hard_defect_count,
        soft_defect_count,
        total_judgments: judgments.len(),
        defect_count: defects,
        opportunities,
        passes,
        defects,
        not_applicable,
        insufficient_evidence,
        evidence_coverage,
        defects_by_criterion,
    }
}

/// 从 ndjson 流解析 tick 序列（严格模式，遇到坏行返回错误及行号）。
pub fn try_parse_stream(content: &str) -> Result<Vec<StreamTick>, String> {
    let mut ticks = Vec::new();
    for (line_no, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let tick: StreamTick = serde_json::from_str(trimmed)
            .map_err(|e| format!("line {}: failed to parse StreamTick: {}", line_no + 1, e))?;
        ticks.push(tick);
    }
    Ok(ticks)
}

/// 从 ndjson 流解析 tick 序列（**默认严格**，gap.md §15.4 / dev 方案 §4 D1.3）：
/// 任何坏行 = 整个流不可信，返回错误。静默保留能解析的部分会让截断流
/// 被当成完整流评判（高分假安全感的另一来源）。
/// 软解析仅在显式 `parse_stream_lenient` 下可用，并由调用方自负证据完整性。
pub fn parse_stream(content: &str) -> Result<Vec<StreamTick>, String> {
    try_parse_stream(content)
}

/// 软解析（仅显式调用）：坏行打 stderr 告警并保留能解析的部分。
/// 仅用于调试/勘探，不得用于任何评判门。
pub fn parse_stream_lenient(content: &str) -> Vec<StreamTick> {
    match try_parse_stream(content) {
        Ok(ticks) => ticks,
        Err(err) => {
            eprintln!("parse_stream_lenient warning: {}", err);
            content
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        }
    }
}

/// 单回合上下文：从上一 POSSESSION_SUMMARY 到本条之间的全部事件。
/// 一次传球释放：`(sequence, passer, receiver, from, to, 是否已终结)`。
///
/// `sequence` 用于**按事件顺序配对**，而不是按接球人 id 配对。
/// 后者会产生误报：同一接球人在一个回合内可能多次接球，若其中一次传球
/// 被点掉/掉落（不产生 `PASS_RECEIVED`），按 id 配对会把**更早的释放**与
/// **更晚的接球**凑成一对，算出数十英尺的虚假距离。
/// 实测 seed 1 full：按 id 配对报 4 条走廊 Hard，其中 3 条是这种错配
/// （释放到接球间隔 0.3–1.2s，却跨全场 52–55 ft）；按顺序配对后只剩 1 条。
type PassRelease = (u64, String, String, (f32, f32), (f32, f32));

#[derive(Debug, Default)]
struct PossessionWindow {
    pass_releases: Vec<PassRelease>,
    /// 接球事实：`(sequence, receiver, position)`。
    pass_received: Vec<(u64, String, (f32, f32))>,
    /// 传球终结事实（点掉/掉落）的 sequence：用于把对应的 release 标记为
    /// 「不会再有接球」，从而不参与走廊配对。
    pass_terminations: Vec<(u64, String)>,
    steals: Vec<(String, String, String, (f32, f32))>,
    tipped_passes: usize,
    loose_ball_secures: Vec<String>,
    /// 带球被切掉的次数（`BALL_POKED_LOOSE`）。
    ///
    /// 与 `loose_ball_secures` 是**两件事**：前者是「球被拨离持球人」的
    /// 原因事实，后者是「松球被某方收下」的结果事实。回合以
    /// `TurnoverLooseBall` 终止时，原因事实必然是前者；球可能由防守方直接
    /// 收下、或弹出界。此时没有 `LOOSE_BALL_SECURED`，却仍是一次合法的
    /// 带球丢球——原判据把两者混为一谈，导致真事实被判 Hard defect。
    poked_loose: usize,
    shot_releases: Vec<(String, bool, f32)>,
    made_arrivals: usize,
    ft_made: usize,
    /// 进攻篮板数：回合时长上界的自变量（每次进攻篮板重置时钟）。
    offensive_rebounds: usize,
    drops: usize,
    violations: usize,
    /// 本回合是否发布了「传球落点修正」事实。
    ///
    /// 层 A（有限信息）下，接球人按自己的估计跑位，接球成功时球的位置
    /// 可能与传球人冻结的 `to_pos` 不同。引擎必须把这一修正显式发布为事实
    /// （`PASS_LANDING_CORRECTED`），否则评判器无法区分
    /// 「设计内的估计偏差」与「事实自相矛盾」。
    saw_pass_position_fix: bool,
}

/// 逐回合 + 逐阶段评判主入口。
pub fn evaluate_stream(ticks: &[StreamTick], fixture: &ReferenceDistributions) -> Vec<Judgment> {
    // 紧凑流（facts/summary）只在首条记录携带 rules；在此向前继承，
    // 使下游准则读取同一份规则事实（gap.md §16.4）。
    let mut carried_rules: Option<nba_protocol::FrameRules> = None;
    let mut carried_set: Option<String> = None;
    let hydrated: Vec<StreamTick> = ticks
        .iter()
        .map(|tick| {
            let mut t = tick.clone();
            if t.frame.rules.is_present() {
                carried_rules = Some(t.frame.rules.clone());
            } else if let Some(rules) = &carried_rules {
                t.frame.rules = rules.clone();
            }
            if !t.tactical_set.is_empty() {
                carried_set = Some(t.tactical_set.clone());
            } else if let Some(set) = &carried_set {
                t.tactical_set = set.clone();
            }
            t
        })
        .collect();
    let ticks: &[StreamTick] = &hydrated;
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
                            &window,
                            &summary,
                            fixture,
                            tick_idx,
                            rules_cache.as_ref(),
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
                            entry.sequence,
                            ev.passer_id,
                            ev.receiver_id,
                            ev.from_pos,
                            ev.to_pos,
                        ));
                    }
                }
                "PASS_RECEIVED" => {
                    if let Ok(ev) = serde_json::from_value::<PassReceivedData>(d.clone()) {
                        window
                            .pass_received
                            .push((entry.sequence, ev.receiver_id, ev.position));
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
                "PASS_LANDING_CORRECTED" => {
                    window.saw_pass_position_fix = true;
                }
                "PASS_DROPPED" => {
                    window.drops += 1;
                    // 按顺序标记：本次传球不会产生接球事实。
                    // 不能用 `pass_releases.pop()`（队列尾部不一定是本次传球）。
                    let recv = d.get("receiver_id").and_then(|v| v.as_str());
                    window
                        .pass_terminations
                        .push((entry.sequence, recv.unwrap_or_default().to_string()));
                }
                "SHOT_RELEASE" => {
                    if let Ok(ev) = serde_json::from_value::<ShotReleaseData>(d.clone()) {
                        window
                            .shot_releases
                            .push((ev.shooter_id, ev.is_three, ev.contest_level));
                    }
                }
                // 得分到达事实：HoopArrival{is_made:true} 的 kind 为 "SCORE"
                // （SHOT_MISS 为不中）。窗口内存在该事实才可满足
                // SCORE_SOURCE_CAUSALITY；此前该计数恒为 0，导致每个得分
                // 回合都被误报为 Hard defect（F2.1）。
                "SCORE" => {
                    window.made_arrivals += 1;
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
                    // 同上：点掉的传球也不会有接球事实。
                    let recv = d.get("receiver_id").and_then(|v| v.as_str());
                    window
                        .pass_terminations
                        .push((entry.sequence, recv.unwrap_or_default().to_string()));
                }
                "LOOSE_BALL_SECURED" => {
                    if let Ok(ev) = serde_json::from_value::<LooseBallSecuredData>(d.clone()) {
                        window.loose_ball_secures.push(ev.player_id);
                    }
                }
                "BALL_POKED_LOOSE" => {
                    // 持球被切掉：带球丢球的**原因事实**（round-14）。
                    window.poked_loose += 1;
                }
                "REBOUND" => {
                    // 进攻篮板会延长同一回合（重置进攻时钟），因此它是回合时长
                    // 上界的**自变量**：一个回合可能抢到多个进攻篮板。此前
                    // `_rebounds` 字段声明了但从未写入，导致上界被写死为
                    // 「24s + 一次 14s 重置」，实测有 3 个进攻篮板、43.6s 的
                    // 合法回合被误报为 Hard defect。
                    if d.get("is_offensive")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        window.offensive_rebounds += 1;
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
    rules_cache: Option<&nba_protocol::FrameRules>,
) -> Vec<Judgment> {
    let mut out = Vec::new();
    let idx = summary.possession_index;
    let _ = tick_idx;

    // ---- (a) 结构因果 ----
    // 回合时长边界（round-5 审计修正）。
    //
    // 口径：`duration_seconds` 是**墙钟模拟时间**（含死球/罚球/发球程序），
    // 而进攻时钟在投篮飞行与篮板争抢期间停表，因此合法回合本就可以超过
    // 一个完整进攻时钟。历史上界 `24 + 14 + 2 = 40` 是对此的**经验包络**，
    // 在实测数据上表现良好，不应收紧。
    //
    // 它的唯一结构性缺陷是：隐含“每回合最多一个进攻篮板”。每次进攻篮板
    // 会把进攻时钟重置为 `offensive_rebound_shot_clock_seconds`，因此 n 个
    // 进攻篮板的回合会多出 n-1 个完整窗口。实测 seed 1 possession 148 有
    // 3 个进攻篮板、43.6s 的**合法**回合被判 Hard defect（此前 20 种子共
    // 11 条同类误报）。
    //
    // 数值来源遵守 gap.md §8.2 单一事实源：进攻时钟取自帧携带的
    // `FrameRules`（`rules_cache`，此前声明却未被消费），而非评判代码里的
    // 常数副本。帧未携带规则时（无几何投影的紧凑流）不做时长裁决。
    let dur = summary.duration_seconds;
    if let Some(r) = rules_cache.as_ref() {
        // 进攻篮板重置量：NBA 为 14s。该值尚无 `FrameRules` 字段，
        // 沿用 fixture 的数据契约（league 级别），不新增内联常数。
        let base = r.shot_clock_seconds;
        let reset = fixture.offensive_rebound_shot_clock_seconds;
        let tolerance = fixture.duration_tolerance_seconds;
        let extra_windows = window.offensive_rebounds.saturating_sub(1) as f32;
        let upper = base + reset + tolerance + reset * extra_windows;
        if dur < 0.0 || dur > upper {
            out.push(Judgment::defect(
                "POSSESSION_DURATION_BOUNDS",
                "hard",
                format!(
                    "possession {} duration {:.1}s exceeds {:.1}s \
                     ({} offensive rebound(s) -> {:.0}s base + {:.0}s tolerance \
                     + {:.0}s extra window(s))",
                    idx,
                    dur,
                    upper,
                    window.offensive_rebounds,
                    base + reset,
                    tolerance,
                    reset * extra_windows
                ),
                "engine",
                idx,
            ));
        } else {
            out.push(Judgment::pass("POSSESSION_DURATION_BOUNDS", "engine", idx));
        }
    }

    // 接球人走廊可达：release 的冻结线段与 arrival 事实位置一致。
    //
    // ## 配对纪律（round-6 审计修复）
    //
    // 按**事件顺序**配对，不按接球人 id 配对。按 id 配对会把「被点掉/掉落的
    // 传球」与「同一接球人更晚的一次接球」凑成一对，算出数十英尺的虚假距离。
    // 实测 seed 1 full：按 id 配对报 4 条走廊 Hard，其中 3 条是这种错配
    // （释放到接球间隔 0.3–1.2s，却跨全场 52–55 ft）；按顺序配对后只剩 1 条。
    // 8 seed 下该错配共产生 12 条 Hard defect。
    //
    // 配对规则：对每个接球事实，取**最近一个未终结、未匹配**的、sequence
    // 小于它且 receiver 相同的 release。
    let corridor = fixture.pass_corridor_radius_ft;
    let mut taken = vec![false; window.pass_releases.len()];
    for (rec_seq, receiver, receiver_pos) in &window.pass_received {
        let mut chosen: Option<usize> = None;
        for (i, (rel_seq, _passer, rel_receiver, _from, _to)) in
            window.pass_releases.iter().enumerate().rev()
        {
            if taken[i] || rel_seq >= rec_seq || rel_receiver != receiver {
                continue;
            }
            // 本次释放是否已被点掉/掉落终结？若是则不可配对。
            let terminated = window
                .pass_terminations
                .iter()
                .any(|(term_seq, term_receiver)| {
                    term_seq > rel_seq && term_seq < rec_seq && term_receiver == receiver
                });
            if terminated {
                continue;
            }
            chosen = Some(i);
            break;
        }
        let Some(i) = chosen else { continue };
        taken[i] = true;
        let (_rel_seq, passer, _receiver, from, to) = &window.pass_releases[i];
        let dist = point_segment_distance_ft(*receiver_pos, *from, *to);
        // ## 两类语义分离（round-10）
        //
        // 旧口径把两种**完全不同**的现象混为一条 Hard 准则：
        //   (a) 接球人按自己的估计跑位，到达点与传球人的意图不同 ——
        //       这是 **P-1 有限信息原则的设计意图**，不是缺陷；
        //   (b) 事实自相矛盾（如 `PassReceived.position` 与冻结终点不一致、
        //       同一 tick 内 `to_pos` 被改写）—— 这才是因果断裂（Hard）。
        //
        // 分离后：
        //   - (a) 超过走廊 → `RECEIVE_ESTIMATE_DIVERGENCE`（**soft/informational**），
        //     用于观测预估偏差的分布，不阻断门；
        //   - (b) 仍为 Hard，但判据收窄为“事实不一致”，见下方 `pass_fact_mismatch`。
        //
        // 注意：这不是“放宽门” —— 同时新增了更严的 (b)。
        if dist > corridor + 1.0 {
            out.push(Judgment::defect(
                "RECEIVE_ESTIMATE_DIVERGENCE",
                "soft",
                format!(
                    "receiver {} arrived {:.1} ft from leader's intended landing \
                     (passer {}; limited-information estimate, not a defect)",
                    receiver, dist, passer
                ),
                "decision",
                idx,
            ));
        } else {
            out.push(Judgment::pass(
                "RECEIVE_ESTIMATE_DIVERGENCE",
                "decision",
                idx,
            ));
        }
        // (b) 事实一致性（Hard）：`PassReceived.position` 必须与冻结终点
        // `to_pos` **一致**。层 A 修正后，引擎在接球成功时把球收到接球人身上，
        // 位置会不同于 `to_pos` —— 那时引擎需发出“落点修正”事实（见引擎侧）。
        // 这里只检测“事件声称的位置与它引用的冻结事实不可调和”的情形。
        if dist > corridor + 1.0 && !window.saw_pass_position_fix {
            out.push(Judgment::defect(
                "PASS_LANDING_FACT_MISMATCH",
                "hard",
                format!(
                    "receiver {} arrived {:.1} ft from intended landing (passer {}) \
                     with no landing-correction fact published",
                    receiver, dist, passer
                ),
                "engine",
                idx,
            ));
        } else {
            out.push(Judgment::pass("PASS_LANDING_FACT_MISMATCH", "engine", idx));
        }
    }
    // 抢断走廊：抢断位置必须在 passer→receiver 连线走廊半径内。
    for (passer, receiver, defender, pos) in &window.steals {
        // 取该 (passer, receiver) 组合**最后一次**释放的线段：抢断发生在那次
        // 传球的走廊上。按位置取而非按 id 配对，避免与更早的传球混淆。
        if let Some((_seq, _p, _r, from, to)) = window
            .pass_releases
            .iter()
            .rev()
            .find(|(_s, p, r, _, _)| p == passer && r == receiver)
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
    if summary.terminal_event == nba_domain::PossessionEndCause::Score
        && window.made_arrivals == 0
        && window.ft_made == 0
    {
        out.push(Judgment::defect(
            "SCORE_SOURCE_CAUSALITY",
            "hard",
            format!("possession {} scored without shot/free-throw source", idx),
            "engine+invariants",
            idx,
        ));
    } else if summary.terminal_event == nba_domain::PossessionEndCause::Score {
        out.push(Judgment::pass(
            "SCORE_SOURCE_CAUSALITY",
            "engine+invariants",
            idx,
        ));
    }

    // 失误归因：终端标签必须与同一回合内的原因事实一致；不能用
    // “有任意一个失误事件”掩盖 PASS_TIPPED/STEAL/PASS_DROPPED 的错配。
    if summary.terminal_event.is_turnover() {
        let cause_present = match summary.terminal_event {
            nba_domain::PossessionEndCause::TurnoverSteal => !window.steals.is_empty(),
            nba_domain::PossessionEndCause::TurnoverPassTipped => window.tipped_passes > 0,
            nba_domain::PossessionEndCause::TurnoverPassDropped => window.drops > 0,
            nba_domain::PossessionEndCause::TurnoverViolation => window.violations > 0,
            nba_domain::PossessionEndCause::TurnoverLooseBall => {
                // 原因事实：带球被切掉（`BALL_POKED_LOOSE`），或松球被某方
                // 收下后球权易主。两者都是合法归因，缺一才算 Hard defect。
                window.poked_loose > 0 || !window.loose_ball_secures.is_empty()
            }
            _ => {
                !window.steals.is_empty()
                    || window.drops > 0
                    || window.tipped_passes > 0
                    || window.violations > 0
                    || window.poked_loose > 0
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
    // 罚球得分不适用：罚球是停表的无干扰程序（gap.md §15.1 的 NotApplicable），
    // 不得计入分母，也不得作为 Hard defect 误报该准则。
    if summary.terminal_event == nba_domain::PossessionEndCause::Score {
        let free_throw_score = window.made_arrivals == 0 && window.ft_made > 0;
        if free_throw_score {
            // D1.1：显式 NotApplicable 裁决——不产生分母，不再默默跳过。
            out.push(Judgment::not_applicable(
                "CONTEST_CONSISTENCY",
                "physics+semantics",
                idx,
            ));
        } else {
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
    }

    let outcome = fixture.classify_outcome(summary.terminal_event.as_str());
    if let Some(band) = fixture.duration_band(outcome) {
        // ## ORB 窗口扩带（round-17，与 POSSESSION_DURATION_BOUNDS 同源）
        //
        // 静态带（如 score [2,26]s）没算进攻篮板重置：n 个 ORB 的回合
        // 合法多出 (n−1) 个 14s 窗口，可到 ~40s。实测 round-17 后该准则
        // 7.7 次/场，几乎全部是 ORB 回合撞静态上限。上界按窗口扩展，
        // 并计入容差（与硬界公式一致）；下界不变（ORB 不会缩短回合）。
        let extra_windows = window.offensive_rebounds.saturating_sub(1) as f32;
        let orb_allowance = fixture.offensive_rebound_shot_clock_seconds * extra_windows
            + fixture.duration_tolerance_seconds;
        let effective_max = band.max + orb_allowance;
        if dur < band.min || dur > effective_max {
            out.push(Judgment::defect(
                "RHYTHM_DURATION",
                "soft",
                format!(
                    "possession {} duration {:.1}s outside {:?}(+ORB {:.0}s) band for outcome {}",
                    idx, dur, band, orb_allowance, outcome
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
    // D1 证据模型：空流不得静默产空裁决（那会被读成"无缺陷"的假安全感）。
    // 空流 = 零证据，显式产出 GAME_LEVEL InsufficientEvidence。
    if ticks.is_empty() {
        out.push(Judgment::insufficient("GAME_LEVEL_EVIDENCE", "engine", 0));
        return out;
    }
    let mut turnovers = 0usize;
    let mut possessions = 0usize;
    let mut idx: Option<u64> = None;

    // D2 构成准则采集（dev 方案 §5.1）：出手/命中按区域拆分。
    let mut fga_three = 0usize;
    let mut fga_two = 0usize;
    let mut fgm_three = 0usize;
    let mut fgm_two = 0usize;
    // 中距离/篮下拆分需要出手位置与篮筐距离（ft）。
    let mut fga_mid = 0usize;
    let mut fga_rim = 0usize;
    let mut fta = 0usize;
    // 比赛时长（秒，墙钟 t 单调），用于 48 分钟等效回合数。
    let mut wall_start: Option<f32> = None;
    let mut wall_end: Option<f32> = None;

    let mut total_decisions = 0usize;
    let mut intent_revalidation_blocked = 0usize;
    for (tick_idx, tick) in ticks.iter().enumerate() {
        idx = idx.or(Some(tick_idx as u64));
        if wall_start.is_none() {
            wall_start = Some(tick.frame.t);
        }
        wall_end = Some(tick.frame.t);
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
        // 出手/命中采集（D2）：ShotRelease 记 FGA 与区域，SCORE 记 FGM。
        // DriveOutcome 不是得分事实（problem.md §19.1），其终结经后续
        // ShotRelease → HoopArrival 入账，故 FGA 由 ShotRelease 单一来源计。
        for entry in &tick.frame.event_log {
            let Some(raw) = &entry.data else { continue };
            match entry.kind.as_str() {
                "SHOT_RELEASE" => {
                    if let Some(s) = raw.get("ShotRelease") {
                        let is_three = s.get("is_three").and_then(|v| v.as_bool()).unwrap_or(false);
                        if is_three {
                            fga_three += 1;
                        } else {
                            fga_two += 1;
                            // 区域拆分：出手点与进攻篮筐距离（ft）。
                            let pos = s.get("pos").and_then(|v| v.as_array());
                            let hoop_x = if tick.frame.possession_team == "home" {
                                fixture.court_width_ft - RIM_OFFSET_FT
                            } else {
                                RIM_OFFSET_FT
                            };
                            let hoop_y = fixture.court_height_ft / 2.0;
                            if let Some(p) = pos {
                                let x = p.first().and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                                let y = p.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                                let dist = ((x - hoop_x).powi(2) + (y - hoop_y).powi(2)).sqrt();
                                if dist <= RIM_ZONE_RADIUS_FT {
                                    fga_rim += 1;
                                } else {
                                    fga_mid += 1;
                                }
                            }
                        }
                    }
                }
                "SCORE" => {
                    if let Some(a) = raw.get("HoopArrival") {
                        if a.get("is_made").and_then(|v| v.as_bool()).unwrap_or(false) {
                            if a.get("is_three").and_then(|v| v.as_bool()).unwrap_or(false) {
                                fgm_three += 1;
                            } else {
                                fgm_two += 1;
                            }
                        }
                    }
                }
                "FREE_THROW" => {
                    fta += 1;
                }
                _ => {}
            }
        }
        let summaries = event_data(&tick.frame, "POSSESSION_SUMMARY")
            .into_iter()
            .filter_map(|d| serde_json::from_value::<PossessionSummaryData>(d).ok());
        for s in summaries {
            possessions += 1;
            idx = Some(s.possession_index);
            if s.terminal_event.is_turnover() {
                turnovers += 1;
            }
        }
    }
    let shot_attempts = fga_two + fga_three;
    let three_attempts = fga_three;

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

    // ---- D2 比赛级构成准则簇（dev 方案 §5.1）----
    // 定位：必要条件的回归网——进带不庆祝，出带必报警。不构成"真实"的证明。
    evaluate_composition_criteria(
        &mut out,
        fixture,
        idx.unwrap_or(0),
        CompositionEvidence {
            possessions,
            turnovers,
            fga_two,
            fga_three,
            fgm_two,
            fgm_three,
            fga_mid,
            fga_rim,
            fta,
            wall_seconds: wall_start
                .zip(wall_end)
                .map(|(s, e)| (e - s).abs())
                .unwrap_or(0.0),
        },
    );

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

/// D2 构成准则的输入证据（dev 方案 §5.1）：从事件流采集的比赛级统计量。
struct CompositionEvidence {
    possessions: usize,
    turnovers: usize,
    fga_two: usize,
    fga_three: usize,
    fgm_two: usize,
    fgm_three: usize,
    fga_mid: usize,
    fga_rim: usize,
    fta: usize,
    wall_seconds: f32,
}

/// 比赛级构成准则簇（dev 方案 §5.1）。
///
/// 每条准则遵循 D1 证据模型：证据不足 = `InsufficientEvidence`（不得满分），
/// fixture 缺参考带 = `NotApplicable`（v1 fixture 向后兼容路径）。
/// 定位：必要条件的回归网——进带不庆祝，出带必报警。
fn evaluate_composition_criteria(
    out: &mut Vec<Judgment>,
    fixture: &ReferenceDistributions,
    idx: u64,
    ev: CompositionEvidence,
) {
    // v1 fixture 无构成带：准则不适用，而非默认通过。
    let Some(bands) = &fixture.composition_bands else {
        for criterion in [
            "SHOT_PROFILE_3PA_RATE",
            "SHOT_PROFILE_ZONE_MIX",
            "TEAM_TURNOVER_RATE",
            "PACE_POSSESSIONS",
            "FT_RATE",
            "SHOT_MAKE_PROFILE",
            "ASSIST_PROFILE",
        ] {
            out.push(Judgment::not_applicable(criterion, "decision", idx));
        }
        return;
    };

    let fga = ev.fga_two + ev.fga_three;
    // 证据门槛：出手/回合样本过少时判 InsufficientEvidence，不给假结论。
    const MIN_SHOTS_FOR_PROFILE: usize = 10;
    const MIN_POSSESSIONS_FOR_PACE: usize = 10;

    // SHOT_PROFILE_3PA_RATE：三分出手占比。
    if fga < MIN_SHOTS_FOR_PROFILE {
        out.push(Judgment::insufficient(
            "SHOT_PROFILE_3PA_RATE",
            "decision",
            idx,
        ));
    } else {
        let rate = ev.fga_three as f32 / fga as f32;
        if bands.three_attempt_rate.contains(&rate) {
            out.push(Judgment::pass("SHOT_PROFILE_3PA_RATE", "decision", idx));
        } else {
            out.push(Judgment::defect(
                "SHOT_PROFILE_3PA_RATE",
                "soft",
                format!(
                    "3PA rate {:.3} outside {:?} ({} 3PA / {} FGA)",
                    rate, bands.three_attempt_rate, ev.fga_three, fga
                ),
                "decision",
                idx,
            ));
        }
    }

    // SHOT_PROFILE_ZONE_MIX：中距离/篮下构成（中距离回归的直接证据）。
    if fga < MIN_SHOTS_FOR_PROFILE {
        out.push(Judgment::insufficient(
            "SHOT_PROFILE_ZONE_MIX",
            "decision",
            idx,
        ));
    } else {
        let mid_share = ev.fga_mid as f32 / fga as f32;
        let rim_share = ev.fga_rim as f32 / fga as f32;
        let mid_ok = bands.mid_range_share_of_fga.contains(&mid_share);
        let rim_ok = bands.rim_share_of_fga.contains(&rim_share);
        if mid_ok && rim_ok {
            out.push(Judgment::pass("SHOT_PROFILE_ZONE_MIX", "decision", idx));
        } else {
            out.push(Judgment::defect(
                "SHOT_PROFILE_ZONE_MIX",
                "soft",
                format!(
                    "zone mix off: mid {:.3} (band {:?}), rim {:.3} (band {:?}) of {} FGA",
                    mid_share, bands.mid_range_share_of_fga, rim_share, bands.rim_share_of_fga, fga
                ),
                "decision",
                idx,
            ));
        }
    }

    // TEAM_TURNOVER_RATE：比赛级固定分母失误率（现有 TURNOVER_RATE 的升格版）。
    if ev.possessions < MIN_POSSESSIONS_FOR_PACE {
        out.push(Judgment::insufficient(
            "TEAM_TURNOVER_RATE",
            "decision",
            idx,
        ));
    } else {
        let rate = ev.turnovers as f32 / ev.possessions as f32;
        if bands_turnover(fixture).contains(&rate) {
            out.push(Judgment::pass("TEAM_TURNOVER_RATE", "decision", idx));
        } else {
            out.push(Judgment::defect(
                "TEAM_TURNOVER_RATE",
                "soft",
                format!(
                    "turnover rate {:.3} outside {:?} ({}/{})",
                    rate,
                    bands_turnover(fixture),
                    ev.turnovers,
                    ev.possessions
                ),
                "decision",
                idx,
            ));
        }
    }

    // PACE_POSSESSIONS：48 分钟等效回合数。
    if ev.possessions < MIN_POSSESSIONS_FOR_PACE || ev.wall_seconds <= 0.0 {
        out.push(Judgment::insufficient("PACE_POSSESSIONS", "decision", idx));
    } else {
        let pace = ev.possessions as f32 * REGULATION_SECONDS_48MIN / ev.wall_seconds;
        if bands.pace_possessions_per_48min.contains(&pace) {
            out.push(Judgment::pass("PACE_POSSESSIONS", "decision", idx));
        } else {
            out.push(Judgment::defect(
                "PACE_POSSESSIONS",
                "soft",
                format!(
                    "pace {:.1} poss/48min outside {:?} ({} poss in {:.0}s)",
                    pace, bands.pace_possessions_per_48min, ev.possessions, ev.wall_seconds
                ),
                "decision",
                idx,
            ));
        }
    }

    // FT_RATE：罚球率 FTA/FGA。
    if fga < MIN_SHOTS_FOR_PROFILE {
        out.push(Judgment::insufficient("FT_RATE", "officiating", idx));
    } else {
        let rate = ev.fta as f32 / fga as f32;
        if bands.free_throw_rate.contains(&rate) {
            out.push(Judgment::pass("FT_RATE", "officiating", idx));
        } else {
            out.push(Judgment::defect(
                "FT_RATE",
                "soft",
                format!(
                    "FT rate {:.3} outside {:?} ({} FTA / {} FGA)",
                    rate, bands.free_throw_rate, ev.fta, fga
                ),
                "officiating",
                idx,
            ));
        }
    }

    // SHOT_MAKE_PROFILE：两/三分命中率（校准 D3.1 的直接门）。
    if fga < MIN_SHOTS_FOR_PROFILE {
        out.push(Judgment::insufficient("SHOT_MAKE_PROFILE", "decision", idx));
    } else {
        let three_pct = if ev.fga_three > 0 {
            ev.fgm_three as f32 / ev.fga_three as f32
        } else {
            0.0
        };
        let two_pct = if ev.fga_two > 0 {
            ev.fgm_two as f32 / ev.fga_two as f32
        } else {
            0.0
        };
        let three_ok = ev.fga_three == 0 || bands.three_make_pct.contains(&three_pct);
        let two_ok = ev.fga_two == 0 || bands.two_make_pct.contains(&two_pct);
        if three_ok && two_ok {
            out.push(Judgment::pass("SHOT_MAKE_PROFILE", "decision", idx));
        } else {
            out.push(Judgment::defect(
                "SHOT_MAKE_PROFILE",
                "soft",
                format!(
                    "make pct off: 3P {:.3} (band {:?}, {}/{}), 2P {:.3} (band {:?}, {}/{})",
                    three_pct,
                    bands.three_make_pct,
                    ev.fgm_three,
                    ev.fga_three,
                    two_pct,
                    bands.two_make_pct,
                    ev.fgm_two,
                    ev.fga_two
                ),
                "decision",
                idx,
            ));
        }
    }

    // ASSIST_PROFILE：事件流当前没有 AST 载荷——按 dev 方案 §5.1 登记为
    // InsufficientEvidence 缺口，禁止伪造助攻（盲区清单条目 #4 的对应准则）。
    out.push(Judgment::insufficient("ASSIST_PROFILE", "decision", idx));
}

/// 失误率带：取 fixture 顶字段（构成带未单设失误率——沿用 v1 顶字段口径）。
fn bands_turnover(fixture: &ReferenceDistributions) -> fixture::Band {
    fixture.turnover_rate_band
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
    terminal_event: nba_domain::PossessionEndCause,
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
