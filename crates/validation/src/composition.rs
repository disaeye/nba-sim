//! 构成组级批次判定（validation.md §6.3 份额向量整体比较）。
//!
//! 从 `stat_batch` 拆出（D29 行数上限，按职责划分）：份额向量（失误原因
//! 聚合、bonus 首进犯规序、回合开始原因）的合并构成与登记参照比 TV，
//! `compare_total_variation` bootstrap。逐指标判定仍在 `stat_batch.rs`。

use std::collections::BTreeMap;

use crate::artifacts::GameMetrics;
use crate::reference::{RegistrationValue, StatDeltaRegistration};
use serde::Serialize;

/// 单场字段求和（stat_batch 的 `m_count` 同义；拆模块后本地复制避免跨模块 pub）。
fn field_sum(metrics: &[GameMetrics], f: impl Fn(&GameMetrics) -> u64) -> f64 {
    metrics.iter().map(f).sum::<u64>() as f64
}

/// 构成组级判定结果（TV 距离口径）。
#[derive(Debug, Clone, Serialize)]
pub struct BatchGroupVerdict {
    pub criterion_id: String,
    pub group: String,
    /// 模拟批次的合并构成（份额向量，键序同登记）。
    pub simulated_composition: BTreeMap<String, f64>,
    pub reference_composition: BTreeMap<String, f64>,
    /// 观测 TV 距离与 bootstrap 区间。
    pub observed_tv: f64,
    pub interval_lower: f64,
    pub interval_upper: f64,
    pub allowed_tv: f64,
    pub verdict: String,
}
/// 构成组级判定：把模拟批次的合并构成与登记构成比 TV。
///
/// 组：STAT-T01 失误原因聚合五类、JOINT-C05 bonus 首进犯规序六桶、
/// JOINT-Q01 回合开始原因七类。转移矩阵的行级 TV 由矩阵格判定补充，
/// 不在此重复（行级允许带 transition_matrix_row_tv 作用于每行构成）。
pub fn evaluate_composition_groups(
    metrics: &[GameMetrics],
    registration: &StatDeltaRegistration,
    bootstrap_replicates: usize,
    seed: u64,
) -> Vec<BatchGroupVerdict> {
    use crate::stats::{compare_total_variation, BootstrapConfig};

    /// 待判定构成组的中间记录（criterion、组名、模拟构成、参照构成、TV 允许带）。
    type PendingGroup = (String, &'static str, BTreeMap<String, f64>, BTreeMap<String, f64>, f64);
    let mut groups: Vec<PendingGroup> = Vec::new();

    // STAT-T01 失误原因聚合五类。
    let to_total: f64 = metrics
        .iter()
        .map(|m| {
            m.to_reason_pass_lost
                + m.to_reason_ball_lost
                + m.to_reason_violation
                + m.to_reason_offensive_foul
                + m.to_reason_other
        })
        .sum::<u64>() as f64;
    if to_total > 0.0 {
        let sim: BTreeMap<String, f64> = [
            ("agg_pass_lost", field_sum(metrics, |m| m.to_reason_pass_lost)),
            ("agg_ball_lost", field_sum(metrics, |m| m.to_reason_ball_lost)),
            ("agg_violation", field_sum(metrics, |m| m.to_reason_violation)),
            ("agg_offensive_foul", field_sum(metrics, |m| m.to_reason_offensive_foul)),
            ("agg_other", field_sum(metrics, |m| m.to_reason_other)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v / to_total))
        .collect();
        if let Some(entry) = registration
            .deltas
            .iter()
            .find(|e| e.criterion_id == "STAT-T01")
        {
            let reference = aggregate_reference_composition(&entry.reference_value);
            if let Some(allowed) = entry
                .allowed_delta
                .as_ref()
                .and_then(|delta| match delta {
                    RegistrationValue::Map(map) => {
                        map.get("reason_shares_tv").and_then(serde_json::Value::as_f64)
                    }
                    _ => None,
                })
            {
                groups.push((
                    "STAT-T01".to_string(),
                    "turnover_reason_composition",
                    sim,
                    reference,
                    allowed,
                ));
            }
        }
    }

    // JOINT-C05 bonus 首进犯规序（1..=5 与 6p 共六桶）。
    let bonus_total: f64 = metrics
        .iter()
        .map(|m| m.bonus_entry_order_hist.iter().sum::<u64>())
        .sum::<u64>() as f64;
    if bonus_total > 0.0 {
        let mut sim: BTreeMap<String, f64> = BTreeMap::new();
        for slot in 0..5 {
            let count: f64 = metrics
                .iter()
                .map(|m| m.bonus_entry_order_hist[slot])
                .sum::<u64>() as f64;
            sim.insert(format!("foul_{}", slot + 1), count / bonus_total);
        }
        let late: f64 = metrics
            .iter()
            .map(|m| m.bonus_entry_order_hist[5..].iter().sum::<u64>())
            .sum::<u64>() as f64;
        sim.insert("foul_6p".to_string(), late / bonus_total);
        if let Some(entry) = registration
            .deltas
            .iter()
            .find(|e| e.criterion_id == "JOINT-C05")
        {
            let reference = bonus_reference_composition(&entry.reference_value);
            if let Some(allowed) = entry
                .allowed_delta
                .as_ref()
                .and_then(|delta| match delta {
                    RegistrationValue::Map(map) => map
                        .get("bonus_entry_foul_order_shares_tv")
                        .and_then(serde_json::Value::as_f64),
                    _ => None,
                })
            {
                groups.push((
                    "JOINT-C05".to_string(),
                    "bonus_entry_order_composition",
                    sim,
                    reference,
                    allowed,
                ));
            }
        }
    }

    // JOINT-Q01 回合开始原因七类边缘构成。
    let starts_total: f64 = metrics
        .iter()
        .map(|m| {
            m.starts_field_goal + m.starts_free_throw + m.starts_steal
                + m.starts_dead_ball + m.starts_timeout + m.starts_blocked_shot
        })
        .sum::<u64>() as f64;
    if starts_total > 0.0 {
        // starts_other 未单列：七类中 other 为余项。
        let counted: f64 = [
            field_sum(metrics, |m| m.starts_field_goal),
            field_sum(metrics, |m| m.starts_free_throw),
            field_sum(metrics, |m| m.starts_steal),
            field_sum(metrics, |m| m.starts_dead_ball),
            field_sum(metrics, |m| m.starts_timeout),
            field_sum(metrics, |m| m.starts_blocked_shot),
        ]
        .iter()
        .sum();
        let mut sim: BTreeMap<String, f64> = [
            ("field_goal", field_sum(metrics, |m| m.starts_field_goal)),
            ("free_throw", field_sum(metrics, |m| m.starts_free_throw)),
            ("steal", field_sum(metrics, |m| m.starts_steal)),
            ("dead_ball", field_sum(metrics, |m| m.starts_dead_ball)),
            ("timeout", field_sum(metrics, |m| m.starts_timeout)),
            ("blocked_shot", field_sum(metrics, |m| m.starts_blocked_shot)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v / starts_total))
        .collect();
        sim.insert("other".to_string(), (counted - starts_total).abs() / starts_total);
        if let Some(entry) = registration
            .deltas
            .iter()
            .find(|e| e.criterion_id == "JOINT-Q01")
        {
            let reference = start_reason_reference_composition(&entry.reference_value);
            if let Some(allowed) = entry
                .allowed_delta
                .as_ref()
                .and_then(|delta| match delta {
                    RegistrationValue::Map(map) => map
                        .get("start_reason_shares_tv")
                        .and_then(serde_json::Value::as_f64),
                    _ => None,
                })
            {
                groups.push((
                    "JOINT-Q01".to_string(),
                    "start_reason_composition",
                    sim,
                    reference,
                    allowed,
                ));
            }
        }
    }

    let mut out = Vec::new();
    for (criterion_id, group, simulated, reference, allowed) in groups {
        if reference.is_empty() {
            continue;
        }
        let config = BootstrapConfig {
            replicates: bootstrap_replicates as u32,
            family_size: 10,
            alpha: 0.05,
            seed,
        };
        let (interval, verdict) =
            compare_total_variation(&simulated, &reference, allowed, &config);
        let observed = crate::stats::total_variation(
            &simulated.values().copied().collect::<Vec<_>>(),
            &reference.values().copied().collect::<Vec<_>>(),
        );
        out.push(BatchGroupVerdict {
            criterion_id,
            group: group.to_string(),
            simulated_composition: simulated,
            reference_composition: reference,
            observed_tv: observed,
            interval_lower: interval.lower,
            interval_upper: interval.upper,
            allowed_tv: allowed,
            verdict: match verdict {
                crate::stats::EquivalenceVerdict::Pass => "Pass".to_string(),
                crate::stats::EquivalenceVerdict::Defect => "Defect".to_string(),
                crate::stats::EquivalenceVerdict::InsufficientEvidence => {
                    "InsufficientEvidence".to_string()
                }
            },
        });
    }
    out
}

/// STAT-T01 登记参照的聚合构成（14 类 → 五类，口径同模拟侧）。
fn aggregate_reference_composition(value: &Option<RegistrationValue>) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    let Some(RegistrationValue::Map(map)) = value else {
        return out;
    };
    let Some(shares) = map.get("reason_shares").and_then(serde_json::Value::as_object) else {
        return out;
    };
    let get = |name: &str| shares.get(name).and_then(serde_json::Value::as_f64);
    let Some(pass_lost) = get("BadPass").zip(get("OutOfBoundsBadPass")).map(|(a, b)| a + b)
    else {
        return out;
    };
    out.insert("agg_pass_lost".to_string(), pass_lost);
    if let Some(ball_lost) = get("LostBall").zip(get("OutOfBoundsLostBall")).map(|(a, b)| a + b) {
        out.insert("agg_ball_lost".to_string(), ball_lost);
    }
    if let (Some(shot_clock), Some(travel), Some(backcourt), Some(dd), Some(lane), Some(kick), Some(ftv)) = (
        get("ShotClock"), get("Travel"), get("Backcourt"), get("DiscontinueDribble"),
        get("LaneViolation"), get("KickBall"), get("FTViolation"),
    ) {
        out.insert(
            "agg_violation".to_string(),
            shot_clock + travel + backcourt + dd + lane + kick + ftv,
        );
    }
    if let Some(foul) = get("OffensiveFoul") {
        out.insert("agg_offensive_foul".to_string(), foul);
    }
    if let Some(other) = get("Other") {
        out.insert("agg_other".to_string(), other);
    }
    out
}

/// JOINT-C05 登记参照的犯规序构成（1..=5 直取，6+ 为余项）。
fn bonus_reference_composition(value: &Option<RegistrationValue>) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    let Some(RegistrationValue::Map(map)) = value else {
        return out;
    };
    let Some(shares) = map
        .get("bonus_entry_foul_order_shares")
        .and_then(serde_json::Value::as_object)
    else {
        return out;
    };
    let mut acc = 0.0;
    for slot in 1..=5 {
        if let Some(v) = shares.get(&format!("foul_{slot}")).and_then(serde_json::Value::as_f64) {
            out.insert(format!("foul_{slot}"), v);
            acc += v;
        }
    }
    out.insert("foul_6p".to_string(), (1.0 - acc).max(0.0));
    out
}

/// JOINT-Q01 登记参照的开始原因构成。
fn start_reason_reference_composition(value: &Option<RegistrationValue>) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    let Some(RegistrationValue::Map(map)) = value else {
        return out;
    };
    let Some(shares) = map
        .get("start_reason_shares")
        .and_then(serde_json::Value::as_object)
    else {
        return out;
    };
    for (key, value) in shares {
        if let Some(v) = value.as_f64() {
            out.insert(key.clone(), v);
        }
    }
    out
}