use nba_validation::g4::{g4_report, write_g4};

#[test]
fn g4a_and_g4b_reports_meet_their_bounds() {
    // 两场完整比赛的决策记录已过万，足以稳定均值与最大遗憾统计。
    let report = g4_report(2).expect("g4 report");
    assert!(report.g4a.passed, "{:?}", report.g4a);
    assert!(report.g4b.passed, "{:?}", report.g4b);
    assert!(report.g4a.decisions > 100);
    assert!(report.g4a.mean_regret <= report.g4a.mean_limit);
    assert!(report.g4a.max_regret <= report.g4a.max_limit);
    assert_eq!(report.g4b.gaps.len(), 0);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(".work")
        .join("g4");
    write_g4(&report, &dir).expect("g4 reports write");
    assert!(dir.join("g4a.json").is_file());
    assert!(dir.join("g4b.json").is_file());
}
