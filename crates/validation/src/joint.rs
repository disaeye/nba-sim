//! 联合与连续过程检查（validation.md §8 JOINT 层）。
//!
//! JOINT 判据的正式判定依赖参考数据（分箱由构建集固定）与机会提取
//! 器的联合通道。本模块从流事实产出条件样本的观测汇总；参考数据
//! 缺失时全部必需 JOINT 叶项统一 `InsufficientEvidence`（不改名为
//! 诊断）。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use nba_protocol::StreamTick;
use serde_json::Value;

/// 回合观测（用于连续过程与条件分布）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PossessionObservation {
    pub possession_index: u64,
    pub offense_team: String,
    /// 终结类别（引擎分类，待核对）。
    pub terminal: String,
    pub duration_seconds: f64,
    pub passes_count: u32,
    /// 本回合得分。
    pub points: u8,
}

/// JOINT-Q01 的连续回合转移矩阵观测。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TransitionMatrixObservation {
    /// (前回合终结类别, 本回合终结类别) → 计数。
    pub counts: BTreeMap<(String, String), u64>,
    pub lag2_counts: BTreeMap<(String, String, String), u64>,
}

impl TransitionMatrixObservation {
    pub fn observe(ticks: &[StreamTick]) -> Self {
        let mut observations: Vec<PossessionObservation> = Vec::new();
        for tick in ticks {
            for event in &tick.frame.event_log {
                let Some(data) = event.data.as_ref() else {
                    continue;
                };
                let Some(summary) = data.get("PossessionSummary") else {
                    continue;
                };
                let Ok(parsed) = serde_json::from_value::<EngineSummaryView>(summary.clone())
                else {
                    continue;
                };
                observations.push(PossessionObservation {
                    possession_index: parsed.possession_index,
                    offense_team: parsed.offense_team,
                    terminal: classify(&parsed.terminal_event),
                    duration_seconds: f64::from(parsed.duration_seconds),
                    passes_count: parsed.passes_count,
                    points: 0,
                });
            }
        }
        let mut matrix = Self::default();
        for window in observations.windows(2) {
            *matrix
                .counts
                .entry((window[0].terminal.clone(), window[1].terminal.clone()))
                .or_insert(0) += 1;
        }
        for window in observations.windows(3) {
            *matrix
                .lag2_counts
                .entry((
                    window[0].terminal.clone(),
                    window[1].terminal.clone(),
                    window[2].terminal.clone(),
                ))
                .or_insert(0) += 1;
        }
        matrix
    }

    /// 观测到的回合对数量。
    pub fn transition_count(&self) -> u64 {
        self.counts.values().sum()
    }

    /// 正式裁决：参考分箱缺失 → InsufficientEvidence（必需项不改名）。
    pub fn verdict(&self) -> crate::verdict::Verdict {
        if self.transition_count() == 0 {
            return crate::verdict::Verdict::InsufficientEvidence;
        }
        crate::verdict::Verdict::InsufficientEvidence
    }
}

#[derive(Debug, Clone, Deserialize)]
struct EngineSummaryView {
    possession_index: u64,
    offense_team: String,
    terminal_event: String,
    duration_seconds: f32,
    passes_count: u32,
}

fn classify(terminal: &str) -> String {
    if terminal.starts_with("TURNOVER") {
        "turnover".to_string()
    } else {
        terminal.to_ascii_lowercase()
    }
}

/// 转换类型观测（JOINT-C02）：篮板/抢断/松球转换的推进统计。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TransitionConversionObservation {
    pub rebounds: u64,
    pub steals: u64,
    pub loose_balls: u64,
    /// 转换开始后 8 秒内的出手数（第一出手时间的分子）。
    pub early_shots: u64,
}

impl TransitionConversionObservation {
    pub fn observe(ticks: &[StreamTick]) -> Self {
        let mut observation = Self::default();
        let mut transition_until: Option<(u64, u64)> = None;
        for tick in ticks {
            let tick_index = tick.frame.tick_index;
            for event in &tick.frame.event_log {
                match event.kind.as_str() {
                    "TRANSITION_STARTED" => {
                        let origin = event
                            .data
                            .as_ref()
                            .and_then(|data| data.get("TransitionStarted"))
                            .and_then(|payload| payload.get("origin"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        match origin {
                            "Rebound" | "rebound" => observation.rebounds += 1,
                            "Steal" | "steal" => observation.steals += 1,
                            _ => observation.loose_balls += 1,
                        }
                        transition_until = Some((tick_index, tick_index + 200));
                    }
                    "SHOT_RELEASE" => {
                        if let Some((start, deadline)) = transition_until {
                            if tick_index <= deadline {
                                observation.early_shots += 1;
                                transition_until = Some((start, deadline));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        observation
    }
}

/// JOINT 层统一报告：观测产出 + 证据汇总。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct JointReport {
    pub transition_matrix: TransitionMatrixObservation,
    pub conversions: TransitionConversionObservation,
    pub evidence: crate::verdict::EvidenceSummary,
}

impl JointReport {
    pub fn run(ticks: &[StreamTick]) -> Self {
        let mut report = Self {
            transition_matrix: TransitionMatrixObservation::observe(ticks),
            conversions: TransitionConversionObservation::observe(ticks),
            evidence: Default::default(),
        };
        report.evidence.record(report.transition_matrix.verdict());
        // 转换观测的正式判定同样依赖参考条件分布。
        report
            .evidence
            .record(crate::verdict::Verdict::InsufficientEvidence);
        report
    }
}
