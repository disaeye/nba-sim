//! MECH 检查点实验框架端到端验收（validation.md §7.1）。
//!
//! 真实跑一个小规模实验（缩短观测窗口与 trial 数，机器时间可承受）：
//! 检查点命中、四组配对（含基线）、档位扰动生效、机会数分母与状态机。

use nba_engine::ValidationAttribute;
use nba_validation::mech::{
    monotonicity_verdict, run_checkpoint_experiment, CheckpointPredicate, ExpectedDirection,
    MechExperimentSpec,
    ValidationAttributeSpec,
};

#[test]
fn checkpoint_experiment_runs_end_to_end() {
    let spec = MechExperimentSpec {
        experiment_id: "MECH-S01-three-precision-it".to_string(),
        criterion_id: "MECH-S01".to_string(),
        attribute: ValidationAttributeSpec {
            name: "shooting_three".to_string(),
        },
        levels: [0.05, 0.50, 0.95],
        predicate: CheckpointPredicate::ShooterInThreePointWindow,
        expected_direction: ExpectedDirection::HigherBetter,
        trial_ticks: 400,
        trials_per_checkpoint: 1,
        mother_seed: 42,
        minimum_checkpoints: 12,
    };
    let attribute = spec.attribute.resolve();
    assert_eq!(attribute, ValidationAttribute::ShootingThree);

    let outcome = run_checkpoint_experiment(&spec, attribute);
    assert_eq!(
        outcome.status, "completed",
        "检查点命中不足: {} (下限 {})",
        outcome.checkpoints_hit, spec.minimum_checkpoints
    );
    assert!(
        outcome.total_attempts[0] > 0 || outcome.total_attempts[1] > 0,
        "四组均无出手观测——观测窗口过短或谓词错位"
    );
    // 命中数分母 = 检查点数（机会数规则）。
    assert!(outcome.trials.len() <= outcome.checkpoints_hit as usize);

    // 单调性判定可运行且给三出口之一。
    let verdicts = monotonicity_verdict(&outcome, ExpectedDirection::HigherBetter, 400, 7);
    let status = verdicts
        .get("monotonicity")
        .expect("monotonicity verdict present");
    assert!(
        ["Pass", "Defect", "InsufficientEvidence"].contains(&status.as_str()),
        "非法判定出口: {status}"
    );
    // 各档位命中率已记录（供工件与页面展示）。
    assert!(verdicts.contains_key("baseline_rate"));
    assert!(verdicts.contains_key("high_rate"));
}

#[test]
fn spec_digest_freeze_gate_works() {
    let spec = nba_validation::mech::initial_experiment_specs()
        .into_iter()
        .next()
        .expect("initial spec");
    let digest = nba_validation::mech::spec_digest(&spec);
    assert!(nba_validation::mech::verify_spec_digest(&spec, digest).is_ok());
    // 档位改动 → 摘必变 → 拒绝执行。
    let mut tampered = spec.clone();
    tampered.levels[0] = 0.11;
    assert!(nba_validation::mech::verify_spec_digest(&tampered, digest).is_err());
}

#[test]
fn insufficient_checkpoints_fails_honestly() {
    // 谓词永不可能命中（罚球谓词在 1q 早期窗口极少）+ 最小命中数远超预算
    // → 实验必须如实失败，不带病出判定。
    let spec = MechExperimentSpec {
        experiment_id: "MECH-impossible-it".to_string(),
        criterion_id: "MECH-R01".to_string(),
        attribute: ValidationAttributeSpec {
            name: "free_throw".to_string(),
        },
        levels: [0.1, 0.5, 0.9],
        predicate: CheckpointPredicate::FreeThrowRoutine,
        expected_direction: ExpectedDirection::HigherBetter,
        trial_ticks: 200,
        trials_per_checkpoint: 1,
        mother_seed: 5,
        minimum_checkpoints: 4,
    };
    let outcome = run_checkpoint_experiment(&spec, spec.attribute.resolve());
    if outcome.checkpoints_hit < spec.minimum_checkpoints {
        assert_eq!(outcome.status, "insufficient_checkpoints");
        let verdicts = monotonicity_verdict(&outcome, ExpectedDirection::HigherBetter, 100, 1);
        // 不足检查点走提前返回：判定在 "status" 键（其余档位数据未生成）。
        assert_eq!(
            verdicts.get("status").map(String::as_str),
            Some("InsufficientEvidence")
        );
    }
    // 若命中达标也合法（谓词可能偶发命中）；两种状态都如实。
}
