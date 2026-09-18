//! 评判结果类型与归因账本聚合。
//!
//! 本模块定义评判对外的输出契约：`Verdict` 三态以上（含「不适用」与
//! 「证据不足」）、`Judgment` 单条裁决、`AttributionReport` 归因账本。
//!
//! 与 `lib.rs` 里六个 `evaluate_*` 准则函数的分工：那些函数**产出**
//! `Judgment`，本模块**聚合**它们。分开是因为聚合逻辑（固定分母、
//! Hard 门解耦、真实度指数）需要单独审阅，而准则函数逐个独立。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
pub(crate) const RIM_ZONE_RADIUS_FT: f32 = 4.0;
/// 篮筐距边线的偏移（ft）：NBA 篮筐距端线 5.25ft， hoop_x = court_width - offset。
pub(crate) const RIM_OFFSET_FT: f32 = 5.25;
/// 48 分钟等效的标准比赛秒数（NBA 4×12min）。
pub(crate) const REGULATION_SECONDS_48MIN: f32 = 2880.0;

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
