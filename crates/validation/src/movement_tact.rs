//! MOVE 与 TACT 叶项判定（validation.md §5.3/§5.4）。
//!
//! `/fact` 变体做完整性核对（Integrity）：运动意图与实际轨迹一致、
//! 到位声明满足几何、责任生命周期配对。`/profile` 与 `/response`
//! 变体的统计判定依赖参考数据与机制实验，本模块输出观测汇总，
//! 裁决由统计层给出。

use serde::{Deserialize, Serialize};

use nba_protocol::StreamTick;
use serde_json::Value;

use crate::verdict::{EvidenceSummary, Verdict};

/// 运动意图的观测记录（从 MOVEMENT_INTENT 载荷解析）。
#[derive(Debug, Clone, Deserialize)]
pub struct IntentObservation {
    pub player_id: String,
    pub task: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub target_pos: Option<(f32, f32)>,
    #[serde(default)]
    pub deadline_ticks: u32,
    #[serde(default)]
    pub revision: u32,
}

/// 运动结果的观测记录。
#[derive(Debug, Clone, Deserialize)]
pub struct OutcomeObservation {
    pub player_id: String,
    pub task: String,
    pub outcome: String,
    #[serde(default)]
    pub path_length_ft: Option<f32>,
}

/// MOVE /fact 层的核对结果（MOVE-H02/O01/O03/F03 的完整性部分）。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct MovementFactAudit {
    pub intents: u64,
    pub outcomes: u64,
    /// 声明 completed 但轨迹长度超过意图几何可达上限的记录数
    /// （路径远大于直线距离与速度上限乘积时可疑）。
    pub implausible_completions: u64,
    /// 同一意图身份被两次终态关闭的记录数。
    pub double_closed: u64,
}

impl MovementFactAudit {
    pub fn audit(ticks: &[StreamTick]) -> Self {
        let mut audit = Self::default();
        let mut open: std::collections::BTreeMap<String, u64> = Default::default();
        for tick in ticks {
            for event in &tick.frame.event_log {
                let Some(data) = event.data.as_ref() else {
                    continue;
                };
                match event.kind.as_str() {
                    "MOVEMENT_INTENT" => {
                        audit.intents += 1;
                        if let Some(intent) = data.get("MovementIntent") {
                            if let Ok(parsed) =
                                serde_json::from_value::<IntentObservation>(intent.clone())
                            {
                                open.insert(parsed.player_id, event.event_id);
                            }
                        }
                    }
                    "MOVEMENT_OUTCOME" => {
                        audit.outcomes += 1;
                        if let Ok(parsed) = serde_json::from_value::<OutcomeObservation>(
                            data.get("MovementOutcome").cloned().unwrap_or(Value::Null),
                        ) {
                            if open.remove(&parsed.player_id).is_none() {
                                audit.double_closed += 1;
                            }
                            if parsed.outcome == "completed" {
                                if let Some(length) = parsed.path_length_ft {
                                    // 到位判定半径由规则给出（默认 2 ft）；
                                    // completed 的轨迹长度异常巨大说明
                                    // 声明与几何脱节（粗查阈值 1000 ft）。
                                    if !length.is_finite() || length > 1000.0 {
                                        audit.implausible_completions += 1;
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        audit
    }

    /// `/fact` 裁决（Integrity）。
    pub fn verdict(&self) -> Verdict {
        if self.double_closed > 0 {
            return Verdict::Defect;
        }
        if self.intents == 0 {
            // 无意图流：检查机会缺失，属证据不足而非缺陷。
            return Verdict::InsufficientEvidence;
        }
        if self.implausible_completions > 0 {
            return Verdict::Defect;
        }
        Verdict::Pass
    }
}

/// 防守责任生命周期观测。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ResponsibilityAudit {
    pub started: u64,
    pub ended: u64,
    /// 有结束无开始（配对失败）的记录数。
    pub ends_without_start: u64,
}

impl ResponsibilityAudit {
    pub fn audit(ticks: &[StreamTick]) -> Self {
        let mut audit = Self::default();
        let mut open: std::collections::BTreeMap<String, ()> = Default::default();
        for tick in ticks {
            for event in &tick.frame.event_log {
                let Some(data) = event.data.as_ref() else {
                    continue;
                };
                match event.kind.as_str() {
                    "RESPONSIBILITY_STARTED" => {
                        audit.started += 1;
                        if let Some(defender) = data
                            .get("ResponsibilityStarted")
                            .and_then(|payload| payload.get("defender_id"))
                            .and_then(Value::as_str)
                        {
                            open.insert(defender.to_string(), ());
                        }
                    }
                    "RESPONSIBILITY_ENDED" => {
                        audit.ended += 1;
                        if let Some(defender) = data
                            .get("ResponsibilityEnded")
                            .and_then(|payload| payload.get("defender_id"))
                            .and_then(Value::as_str)
                        {
                            if open.remove(defender).is_none() {
                                audit.ends_without_start += 1;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        audit
    }

    pub fn verdict(&self) -> Verdict {
        if self.ends_without_start > 0 {
            return Verdict::Defect;
        }
        if self.started == 0 {
            return Verdict::InsufficientEvidence;
        }
        Verdict::Pass
    }
}

/// Play 生命周期核对（TACT-P01/P02 的完整性部分）。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PlayLifecycleAudit {
    pub activations: u64,
    pub window_ends: u64,
    /// 窗口结束引用的 play 与任何激活都不匹配的记录数。
    pub ends_without_activation: u64,
    /// 同一激活身份被两次窗口结束关闭的记录数。
    pub double_ended: u64,
}

impl PlayLifecycleAudit {
    pub fn audit(ticks: &[StreamTick]) -> Self {
        let mut audit = Self::default();
        let mut open: std::collections::BTreeMap<String, ()> = Default::default();
        for tick in ticks {
            for event in &tick.frame.event_log {
                let Some(data) = event.data.as_ref() else {
                    continue;
                };
                match event.kind.as_str() {
                    "PLAY_ACTIVATED" => {
                        audit.activations += 1;
                        if let Some(play_id) = data
                            .get("PlayActivated")
                            .and_then(|payload| payload.get("play_id"))
                            .and_then(Value::as_str)
                        {
                            open.insert(play_id.to_string(), ());
                        }
                    }
                    "PLAY_WINDOW_ENDED" => {
                        audit.window_ends += 1;
                        if let Some(play_id) = data
                            .get("PlayWindowEnded")
                            .and_then(|payload| payload.get("play_id"))
                            .and_then(Value::as_str)
                        {
                            if open.remove(play_id).is_none() {
                                audit.ends_without_activation += 1;
                            } else {
                                // 单次激活只允许一次窗口结束。
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        audit
    }

    pub fn verdict(&self) -> Verdict {
        if self.ends_without_activation > 0 {
            return Verdict::Defect;
        }
        if self.activations == 0 {
            return Verdict::NotApplicable;
        }
        Verdict::Pass
    }
}

/// MOVE/TACT 事实层的统一裁决汇总。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct FactAuditReport {
    pub movement: MovementFactAudit,
    pub responsibility: ResponsibilityAudit,
    pub play: PlayLifecycleAudit,
    pub evidence: EvidenceSummary,
}

impl FactAuditReport {
    pub fn run(ticks: &[StreamTick]) -> Self {
        let mut report = Self {
            movement: MovementFactAudit::audit(ticks),
            responsibility: ResponsibilityAudit::audit(ticks),
            play: PlayLifecycleAudit::audit(ticks),
            evidence: EvidenceSummary::default(),
        };
        report.evidence.record(report.movement.verdict());
        report.evidence.record(report.responsibility.verdict());
        report.evidence.record(report.play.verdict());
        report
    }
}
