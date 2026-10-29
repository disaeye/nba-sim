use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::review::{build_review, write_review};

#[test]
fn review_selects_twenty_possessions_and_covers_the_process_checklist() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(".work")
        .join("review-seed-1");
    let report = build_review(1, &workspace_calibration_dir(), 20).expect("review builds");
    assert_eq!(report.possessions.len(), 20);
    assert!(
        report.all_checks_passed(),
        "checklist failed: {:?}",
        report
            .checklist
            .iter()
            .filter(|check| !check.passed)
            .map(|check| check.item.as_str())
            .collect::<Vec<_>>()
    );
    for label in ["half_court", "transition", "inbound", "free_throw"] {
        assert!(
            report
                .possessions
                .iter()
                .any(|possession| possession.label == label),
            "review has no {label} possession"
        );
    }
    for possession in &report.possessions {
        assert!(!possession.frames.is_empty(), "possession {} has no frames", possession.id);
        for tick in &possession.frames {
            assert!(tick.frame.t_game.is_finite());
            assert_eq!(tick.frame.players.len(), 10);
            assert!(tick.frame.ball.z.is_finite());
        }
    }
    let (json, html) = write_review(&report, &dir).expect("review artifacts write");
    assert!(json.is_file() && html.is_file());
    let html_body = std::fs::read_to_string(&html).expect("html readable");
    assert!(html_body.contains("review"));
}
