use std::path::PathBuf;

use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{build_review, write_review};

pub fn run_review(seed: u64, possessions: usize, out: Option<PathBuf>) -> std::io::Result<()> {
    let dir = out.unwrap_or_else(|| {
        PathBuf::from("output")
            .join("review")
            .join(format!("seed-{seed}"))
    });
    let report = build_review(seed, &workspace_calibration_dir(), possessions)
        .map_err(std::io::Error::other)?;
    let (json, html) = write_review(&report, &dir)?;
    println!(
        "review seed {seed}: {} 回合，{} / {} 清单项通过",
        report.possessions.len(),
        report.checklist.iter().filter(|check| check.passed).count(),
        report.checklist.len()
    );
    println!("  {}", json.display());
    println!("  {}", html.display());
    for check in &report.checklist {
        println!(
            "  [{}] {} ({})",
            if check.passed { "通过" } else { "未出现" },
            check.item,
            check.evidence
        );
    }
    if !report.all_checks_passed() {
        return Err(std::io::Error::other(
            "review checklist has items not observed in the selected possessions",
        ));
    }
    Ok(())
}
