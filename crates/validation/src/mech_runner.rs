//! MECH 实验执行与机制层判定工件（validation.md §7.1 / §10）。
//!
//! 从 `runner.rs` 拆出（D29 行数上限）：执行首批实验规格（`mech.rs`
//! 的冻结-执行-判定链），把 MECH 叶项判定写进 `mechanism_results.json`。
//! 判定口径：
//! - 有实验覆盖的叶项：取其全部实验的单调性判定的合取
//!   （Defect > InsufficientEvidence > Pass）；
//! - 无实验覆盖的叶项：如实 `InsufficientEvidence`
//!   （reason `experiment_spec_missing`）。
//!
//! 执行成本：三项实验 × 每项最多 30 检查点 × 4 组 × 400 tick 的短跑，
//! 分钟级；母局各走一遍（实验间不共享母局走位——各实验谓词与 seed
//! 不同，共享无收益）。

use std::collections::BTreeMap;

use serde::Serialize;

use crate::criteria::{CriterionRegistry, Family};
use crate::mech::{
    initial_experiment_specs, monotonicity_verdict, run_checkpoint_experiment, spec_digest,
    verify_spec_digest, MechExperimentOutcome,
};

/// 机制层结果工件（mechanism_results.json 的载荷）。
#[derive(Debug, Serialize)]
pub struct MechanismResults {
    pub registered_criteria: Vec<String>,
    pub verdicts: Vec<MechCriterionVerdict>,
    pub experiments: Vec<ExperimentRecord>,
    pub note: String,
}

/// 单条 MECH 叶项裁决。
#[derive(Debug, Serialize)]
pub struct MechCriterionVerdict {
    pub criterion_id: String,
    pub verdict: String,
    pub reason_code: String,
}

/// 单个实验的执行记录（规格、摘要、判定、档位命中率）。
#[derive(Debug, Serialize)]
pub struct ExperimentRecord {
    pub experiment_id: String,
    pub criterion_id: String,
    pub spec_digest: u64,
    pub digest_verified: bool,
    pub status: String,
    pub checkpoints_hit: u32,
    pub monotonicity: BTreeMap<String, String>,
    /// 各组命中率（基线/低/中/高；attempt 数为零时为 null）。
    pub group_rates: [Option<f64>; 4],
    /// 各组出手总数（诊断统计功效）。
    pub group_attempts: [u64; 4],
    /// 各组命中总数。
    pub group_makes: [u64; 4],
}

/// 执行首批实验并产出机制层判定。
///
/// 独立可调（`validate --mech` 与全量 validate 共用）；`artifacts_dir`
/// 用于写实验规格摘要文件（冻结纪律的登记侧）。
pub fn mechanism_results_with_experiments(artifacts_dir: &std::path::Path) -> MechanismResults {
    let specs = initial_experiment_specs();
    let mut experiments: Vec<ExperimentRecord> = Vec::new();
    let mut outcomes: Vec<(String, String)> = Vec::new();
    for spec in &specs {
        let digest = spec_digest(spec);
        // 摘要登记写盘（规格冻结的登记侧；执行器与人工审核共用）。
        let digest_path = artifacts_dir.join(format!("{}_spec_digest.txt", spec.experiment_id));
        let recorded = std::fs::read_to_string(&digest_path)
            .ok()
            .and_then(|text| u64::from_str_radix(text.trim().trim_start_matches("0x"), 16).ok());
        let digest_verified = match recorded {
            Some(recorded_digest) => verify_spec_digest(spec, recorded_digest).is_ok(),
            None => true, // 首次执行：写盘登记，视为一致
        };
        if digest_verified {
            let _ = std::fs::write(&digest_path, format!("{digest:#018x}\n"));
            let attribute = spec.attribute.resolve();
            let outcome = run_checkpoint_experiment(spec, attribute);
            let monotonicity =
                monotonicity_verdict(&outcome, spec.expected_direction, 800, spec.mother_seed);
            outcomes.push((spec.criterion_id.clone(), monotonicity
                .get("monotonicity")
                .cloned()
                .unwrap_or_else(|| "InsufficientEvidence".to_string())));
            experiments.push(ExperimentRecord {
                experiment_id: spec.experiment_id.clone(),
                criterion_id: spec.criterion_id.clone(),
                spec_digest: digest,
                digest_verified,
                status: outcome.status.clone(),
                checkpoints_hit: outcome.checkpoints_hit,
                monotonicity,
                group_rates: group_hit_rates(&outcome),
                group_attempts: outcome.total_attempts,
                group_makes: outcome.total_makes,
            });
        } else {
            experiments.push(ExperimentRecord {
                experiment_id: spec.experiment_id.clone(),
                criterion_id: spec.criterion_id.clone(),
                spec_digest: digest,
                digest_verified: false,
                status: "spec_digest_mismatch".to_string(),
                checkpoints_hit: 0,
                monotonicity: BTreeMap::new(),
                group_rates: [None; 4],
                group_attempts: [0; 4],
                group_makes: [0; 4],
            });
        }
    }

    // 叶项级合取：每叶项的全部实验判定合取（Defect > Insufficient > Pass）。
    let registry = CriterionRegistry::full();
    let mut mech_ids: Vec<String> = registry
        .by_family(Family::Mech)
        .into_iter()
        .map(|entry| entry.id.clone())
        .collect();
    mech_ids.sort();
    let mut verdicts: Vec<MechCriterionVerdict> = Vec::new();
    for criterion_id in &mech_ids {
        let related: Vec<&String> = outcomes
            .iter()
            .filter(|(id, _)| id.as_str() == criterion_id.as_str())
            .map(|(_, verdict)| verdict)
            .collect();
        if related.is_empty() {
            verdicts.push(MechCriterionVerdict {
                criterion_id: criterion_id.clone(),
                verdict: "InsufficientEvidence".to_string(),
                reason_code: "experiment_spec_missing".to_string(),
            });
            continue;
        }
        let (verdict, reason) = if related.iter().any(|v| v.as_str() == "Defect") {
            ("Defect", "mechanism_monotonicity_violated")
        } else if related.iter().any(|v| v.as_str() == "InsufficientEvidence") {
            ("InsufficientEvidence", "mechanism_insufficient_trials")
        } else {
            ("Pass", "mechanism_monotonicity_confirmed")
        };
        verdicts.push(MechCriterionVerdict {
            criterion_id: criterion_id.clone(),
            verdict: verdict.to_string(),
            reason_code: reason.to_string(),
        });
    }

    MechanismResults {
        registered_criteria: mech_ids,
        verdicts,
        experiments,
        note: "MECH verdicts from frozen checkpoint experiments (validation.md §7.1); \
               leaves without experiment specs stay InsufficientEvidence"
            .to_string(),
    }
}

/// 各组命中率（基线/低/中/高；无出手观测的组为 None）。
fn group_hit_rates(outcome: &MechExperimentOutcome) -> [Option<f64>; 4] {
    let mut rates = [None; 4];
    for (slot, rate) in rates.iter_mut().enumerate() {
        if outcome.total_attempts[slot] > 0 {
            *rate = Some(
                outcome.total_makes[slot] as f64 / outcome.total_attempts[slot] as f64,
            );
        }
    }
    rates
}
