use nba_sim_core::params::workspace_root;
use nba_validation::five_dimension::{run_five_dimension, FiveDimensionReport};

const DIMENSIONS: [(&str, [&str; 4]); 5] = [
    (
        "decision",
        [
            "trace_coverage",
            "mean_candidates",
            "decomposition_finite_share",
            "mean_beta",
        ],
    ),
    (
        "situation",
        [
            "leverage_time_step_gap",
            "leverage_magnitude_monotone_share",
            "win_probability_monotone_share",
            "endgame_three_share",
        ],
    ),
    (
        "tactics",
        [
            "play_activation_share",
            "action_family_entropy_bits",
            "pass_share",
            "tactical_bonus_share",
        ],
    ),
    (
        "motion",
        [
            "speed_cap_compliance",
            "accel_cap_compliance",
            "momentum_cap_compliance",
            "reaction_delay_seconds",
        ],
    ),
    (
        "rules",
        [
            "violations_per_1k_events",
            "finished_game_share",
            "score_reconcile_share",
            "events_per_game",
        ],
    ),
];

fn assert_schema(report: &FiveDimensionReport) {
    assert_eq!(report.dimensions.len(), 5, "维度数不是五个");
    for (name, keys) in DIMENSIONS {
        let dimension = report
            .dimension(name)
            .unwrap_or_else(|| panic!("缺少维度 {name}"));
        let actual: Vec<&str> = dimension
            .metrics
            .iter()
            .map(|metric| metric.key.as_str())
            .collect();
        assert_eq!(actual, keys, "维度 {name} 的字段集发生变化");
        for metric in &dimension.metrics {
            assert!(metric.value.is_finite(), "{name}/{} 不是有限值", metric.key);
            assert!(
                metric.lower <= metric.upper,
                "{name}/{} 的上下界颠倒",
                metric.key
            );
        }
        let recomputed = dimension.metrics.iter().all(|metric| metric.passed);
        assert_eq!(dimension.passed, recomputed, "维度 {name} 的汇总判定与指标不一致");
    }
    assert_eq!(
        report.passed,
        report.dimensions.iter().all(|dimension| dimension.passed),
        "总判定与维度判定不一致"
    );
}

#[test]
fn report_covers_every_dimension_with_stable_fields() {
    let report = run_five_dimension(1, 2).expect("五维报告生成失败");
    assert_schema(&report);
    assert_eq!(report.seeds, vec![1, 2]);
    assert!(
        report.passed,
        "五维报告未通过：{:?}",
        report.failed_metrics()
    );
}

#[test]
fn report_is_deterministic_for_the_same_seeds() {
    let first = run_five_dimension(21, 2).expect("五维报告生成失败");
    let second = run_five_dimension(21, 2).expect("五维报告生成失败");
    let left = serde_json::to_string(&first).expect("序列化失败");
    let right = serde_json::to_string(&second).expect("序列化失败");
    assert_eq!(left, right, "同一批种子的报告不稳定，无法当回归基线");
}

#[test]
fn committed_evidence_matches_the_schema() {
    let path = workspace_root()
        .join("docs")
        .join("dev")
        .join("evidence")
        .join("action_chain_validation")
        .join("five_dimension.json");
    let text = std::fs::read_to_string(&path).expect("证据文件必须存在");
    let report: FiveDimensionReport =
        serde_json::from_str(&text).expect("证据文件必须是合法的五维报告");
    assert_schema(&report);
    assert!(report.passed, "证据文件里的五维报告未通过");
}
