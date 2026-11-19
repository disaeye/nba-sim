use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{MatchSetup, RuntimeParams};
use nba_validation::paired::{paired_screen_experiment, Arm};

/// 成对实验：同一快照下只改一项登记的初始条件，中间事实随之改变。
#[test]
fn geometry_and_perception_changes_produce_different_middle_facts() {
    let params = RuntimeParams::load(&workspace_calibration_dir()).expect("runtime tables");
    let mut checked = 0;
    let mut comparable = 0;
    let mut findings = Vec::new();
    for seed in 1..=8 {
        let setup = MatchSetup::builtin(nba_domain::GameRules::default());
        let report = match paired_screen_experiment(setup, &params, seed) {
            Ok(report) => report,
            Err(error) => {
                findings.push(format!("seed {seed}: {error}"));
                continue;
            }
        };
        println!(
            "seed {seed}: baseline {:?} farther {:?} hidden {:?}",
            report.arm(Arm::Baseline).map(|arm| (
                arm.defender_observed,
                arm.defender_read_arrival_seconds,
                arm.defender_actual_arrival_seconds,
                arm.defender_read_error_ft,
            )),
            report.arm(Arm::DefenderFarther).map(|arm| (
                arm.defender_observed,
                arm.defender_read_arrival_seconds,
                arm.defender_actual_arrival_seconds,
                arm.defender_read_error_ft,
            )),
            report.arm(Arm::DefenderHidden).map(|arm| (
                arm.defender_observed,
                arm.defender_read_arrival_seconds,
                arm.defender_actual_arrival_seconds,
                arm.defender_read_error_ft,
            )),
        );
        findings.extend(report.findings.iter().map(|finding| format!("seed {seed}: {finding}")));
        comparable += usize::from(report.comparable);
        checked += 1;
    }
    assert!(checked > 0, "没有任何种子产生可比较的成对实验：{findings:?}");
    assert!(comparable > 0, "没有任何种子形成成立的对照：{findings:?}");
    assert!(findings.is_empty(), "{findings:?}");
}
