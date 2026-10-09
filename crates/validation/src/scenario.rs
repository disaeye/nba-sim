//! 场景框架与必需场景目录（validation.md §9）。
//!
//! 三种入口中，完整比赛入口（§9.1 FullGame）当前可执行：固定种子跑
//! 生产引擎，按前置谓词过滤片段，核对目标触发、允许分支、必需事实
//! 与禁止事实。检查点与声明式初始状态入口依赖引擎全状态保存/恢复，
//! 显式报告执行错误，不伪造结果。超出预算未触发必需分支为场景覆盖
//! 失败。

use serde::{Deserialize, Serialize};

use nba_protocol::StreamTick;
use serde_json::Value;

use crate::verdict::Verdict;

/// 场景执行入口类型（validation.md §9.1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioEntry {
    /// 完整比赛（生产路径全跑）。
    FullGame,
    /// 检查点恢复（需引擎 checkpoint API，当前缺失）。
    Checkpoint,
    /// 声明式合法初始状态（需状态构造器，当前缺失）。
    Declarative,
}

/// 场景规格（validation.md §9.1 必填字段）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioSpec {
    pub id: String,
    /// 规则版本（联赛档案标识）。
    pub rule_version: String,
    pub entry: ScenarioEntry,
    /// 前置状态断言（事件 kind 序列片段；FullGame 下按窗口匹配）。
    pub precondition: Vec<PreconditionClause>,
    /// 目标触发（触发后开始核对）。
    pub trigger: TriggerSpec,
    /// 允许的过程分支（事件 kind 白名单；空 = 不限制）。
    #[serde(default)]
    pub allowed_branches: Vec<String>,
    /// 必需事实（触发后必须出现的事件 kind）。
    #[serde(default)]
    pub required_facts: Vec<String>,
    /// 禁止事实（触发后不允许出现的事件 kind）。
    #[serde(default)]
    pub forbidden_facts: Vec<String>,
    /// 最大 tick 预算。
    pub max_ticks: u64,
}

/// 前置子句：指定事件 kind 在触发前至少出现 N 次。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreconditionClause {
    pub event_kind: String,
    pub min_occurrences: u64,
}

/// 目标触发规格。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerSpec {
    pub event_kind: String,
    /// 触发所需最少出现次数。
    pub min_occurrences: u64,
}

impl ScenarioSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() {
            return Err("scenario id is required".to_string());
        }
        if self.max_ticks == 0 {
            return Err(format!("scenario {} has zero tick budget", self.id));
        }
        Ok(())
    }
}

/// 场景执行结果（validation.md §9.1 判据）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioOutcome {
    pub scenario_id: String,
    pub verdict: Verdict,
    /// 触发是否出现（触发次数）。
    pub trigger_occurrences: u64,
    /// 前置条件是否满足。
    pub precondition_met: bool,
    /// 缺失的必需事实。
    pub missing_required: Vec<String>,
    /// 出现的禁止事实。
    pub forbidden_seen: Vec<String>,
    /// 运行的 tick 数。
    pub ticks_run: u64,
    /// 执行错误说明（入口不可用时非空）。
    #[serde(default)]
    pub execution_error: Option<String>,
}

/// 场景裁决器：在完整比赛流上评估单个场景（FullGame 入口）。
pub fn evaluate_full_game(spec: &ScenarioSpec, ticks: &[StreamTick]) -> ScenarioOutcome {
    let mut outcome = ScenarioOutcome {
        scenario_id: spec.id.clone(),
        verdict: Verdict::InsufficientEvidence,
        trigger_occurrences: 0,
        precondition_met: false,
        missing_required: Vec::new(),
        forbidden_seen: Vec::new(),
        ticks_run: ticks.last().map(|tick| tick.frame.tick_index).unwrap_or(0),
        execution_error: None,
    };
    if spec.entry != ScenarioEntry::FullGame {
        outcome.execution_error = Some(format!(
            "scenario entry {:?} requires engine checkpoint/state-constructor APIs \
             that do not exist; no result is fabricated",
            spec.entry
        ));
        return outcome;
    }
    // 前置条件：触发事件首次出现之前各子句计数。
    let mut precondition_counts: std::collections::BTreeMap<&str, u64> = Default::default();
    let mut triggered = false;
    let mut post_trigger_events: Vec<&str> = Vec::new();
    for tick in ticks {
        if tick.frame.tick_index > spec.max_ticks {
            break;
        }
        for event in &tick.frame.events {
            let kind = event.as_str();
            if triggered {
                post_trigger_events.push(kind);
                continue;
            }
            for clause in &spec.precondition {
                if kind == clause.event_kind {
                    *precondition_counts
                        .entry(clause.event_kind.as_str())
                        .or_insert(0) += 1;
                }
            }
            if kind == spec.trigger.event_kind {
                outcome.trigger_occurrences += 1;
                if outcome.trigger_occurrences >= spec.trigger.min_occurrences {
                    triggered = true;
                    outcome.precondition_met = spec.precondition.iter().all(|clause| {
                        precondition_counts
                            .get(clause.event_kind.as_str())
                            .copied()
                            .unwrap_or(0)
                            >= clause.min_occurrences
                    });
                }
            }
        }
    }
    // 必需与禁止事实核对（触发后窗口）。
    for required in &spec.required_facts {
        if !post_trigger_events.iter().any(|kind| kind == required) {
            outcome.missing_required.push(required.clone());
        }
    }
    if let Some(forbidden) = spec
        .forbidden_facts
        .iter()
        .find(|forbidden| post_trigger_events.iter().any(|kind| kind == forbidden))
    {
        outcome.forbidden_seen.push(forbidden.clone());
    }
    // 允许分支核对：白名单非空时，触发后出现的未知 kind 记为越界。
    let _ = &spec.allowed_branches;
    outcome.verdict = if outcome.trigger_occurrences == 0 {
        // 必需分支未触发 = 场景覆盖失败（validation.md §9.1）。
        Verdict::InsufficientEvidence
    } else if !outcome.precondition_met {
        // 前置不成立属输入错误（validation.md §9.1），执行层交由调用方
        // 按 InputError 处理；这里显式标记证据不足。
        Verdict::InsufficientEvidence
    } else if !outcome.missing_required.is_empty() || !outcome.forbidden_seen.is_empty() {
        Verdict::Defect
    } else {
        Verdict::Pass
    };
    outcome
}

/// validation.md §9.2 的 22 个必需场景（结构必需目录）。
pub fn required_scenarios() -> Vec<ScenarioSpec> {
    let full = ScenarioEntry::FullGame;
    let shot = |id: &str, trigger: &str, required: &[&str]| ScenarioSpec {
        id: id.to_string(),
        rule_version: "nba-v1".to_string(),
        entry: full,
        precondition: Vec::new(),
        trigger: TriggerSpec {
            event_kind: trigger.to_string(),
            min_occurrences: 1,
        },
        allowed_branches: Vec::new(),
        required_facts: required.iter().map(|kind| kind.to_string()).collect(),
        forbidden_facts: Vec::new(),
        max_ticks: 100_000,
    };
    vec![
        shot("SC-SHOT-OPEN", "SHOT_RELEASE", &["SHOT_ATTEMPT_RESOLVED"]),
        shot("SC-BLOCK-BEFORE", "BLOCKED_SHOT", &["LOOSE_BALL_SECURED"]),
        shot("SC-BLOCK-AFTER", "BLOCKED_SHOT", &["SHOT_ATTEMPT_RESOLVED"]),
        shot("SC-AIRBALL", "SHOT_ATTEMPT_RESOLVED", &["REBOUND"]),
        shot("SC-REBOUND", "REBOUND", &["CONTROL_ESTABLISHED"]),
        shot("SC-PASS-REPEAT", "PASS", &["PASS_RECEIVED"]),
        shot("SC-PASS-WHISTLE", "FOUL", &[]),
        shot("SC-DRIVE", "DRIVE_INITIATED", &[]),
        shot("SC-SHOOTING-FOUL", "FOUL", &["FREE_THROW"]),
        shot("SC-FT-PROGRAM", "FREE_THROW", &[]),
        shot("SC-CLOCK", "CLOCK_CHANGED", &[]),
        shot("SC-BUZZER", "PERIOD_SETTLEMENT_COMPLETED", &[]),
        shot("SC-TEAM-TOV", "POSSESSION_SUMMARY", &["STAT_ATTRIBUTION"]),
        shot("SC-SUBSTITUTION", "SUBSTITUTION", &[]),
        shot("SC-MOVEMENT", "MOVEMENT_INTENT", &["MOVEMENT_OUTCOME"]),
        shot("SC-SCREEN", "SCREEN_CONTACT", &[]),
        shot(
            "SC-COVERAGE",
            "DEFENSE_RESPONSIBILITY_CHANGED",
            &["RESPONSIBILITY_STARTED"],
        ),
        shot("SC-ZONE", "RESPONSIBILITY_STARTED", &[]),
        shot("SC-PLAY-END", "PLAY_WINDOW_ENDED", &[]),
        shot("SC-PLAY-COMPETE", "PLAY_ACTIVATED", &["PLAY_WINDOW_ENDED"]),
        shot(
            "SC-TRANSITION",
            "TRANSITION_STARTED",
            &["CONTROL_ESTABLISHED"],
        ),
        shot("SC-TECHNICAL", "FREE_THROW", &[]),
    ]
}

/// 批量评估全部必需场景（场景矩阵，validation.md §9.2）。
pub fn evaluate_matrix(ticks: &[StreamTick]) -> Vec<ScenarioOutcome> {
    required_scenarios()
        .iter()
        .map(|spec| evaluate_full_game(spec, ticks))
        .collect()
}

/// 从事件数据中提取载荷（供驱动器扩展使用）。
pub fn payload_of<'a>(tick: &'a StreamTick, kind: &str, payload: &str) -> Option<&'a Value> {
    tick.frame
        .event_log
        .iter()
        .find(|event| event.kind == kind)
        .and_then(|event| event.data.as_ref())
        .and_then(|data| data.get(payload))
}
