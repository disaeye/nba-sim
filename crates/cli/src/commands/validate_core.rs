//! 验证比赛账本、几何与动作语境。

use std::path::{Path, PathBuf};

use nba_sim_core::params::workspace_validation_dir;
use nba_sim_core::{builtin_manifest, Params};
use nba_validation::core_validation::{run_core_validation_for_league, CoreValidationReport};
use nba_validation::mechanism_core::{run_mechanism_experiment, MechanismAttribute, MechanismConfig};

pub(crate) fn run_validate_core(
    seed_start: u64,
    games: u64,
    json: bool,
    out: Option<PathBuf>,
    league: nba_domain::league::LeagueId,
) -> std::io::Result<u8> {
    let report = run_core_validation_for_league(seed_start, games, league)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    if json {
        let body = serde_json::to_string_pretty(&report)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        println!("{body}");
    } else {
        print_report(&report);
    }
    let out_dir = out.unwrap_or_else(workspace_validation_dir);
    write_json(&out_dir, "core_validation.json", &report)?;
    let (commit, dirty) = git_state(&out_dir);
    write_run_record(
        &out_dir,
        seed_start,
        games,
        &commit,
        dirty,
        league,
    )?;
    println!("  工件目录：{}", out_dir.display());
    Ok(u8::from(!report.passed))
}

fn write_json<T: serde::Serialize>(dir: &Path, name: &str, value: &T) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let body = serde_json::to_string_pretty(value)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    std::fs::write(dir.join(name), body)
}

/// 产出记录：把这一批工件对应的提交、种子与校准表来源写在一起，
/// 使验收记录可以独立复核。
fn write_run_record(
    dir: &Path,
    seed_start: u64,
    games: u64,
    commit: &str,
    dirty: bool,
    league: nba_domain::league::LeagueId,
) -> std::io::Result<()> {
    let params = Params::load(&nba_sim_core::params::workspace_calibration_dir())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let manifest = builtin_manifest(league, seed_start, &params.calibration_version);
    let record = serde_json::json!({
        "commit": commit,
        "working_tree_dirty": dirty,
        "seed_start": seed_start,
        "games": games,
        "ruleset_version": manifest.ruleset_version,
        "calibration_version": manifest.calibration_version,
        "engine_model_version": manifest.engine_model_version,
        "calibration": {
            "shot_make": params.shot_make.provenance,
            "shot_profile": params.shot_profile.provenance,
            "possession": params.possession.provenance,
            "rebound": params.rebound.provenance,
            "freethrow": params.freethrow.provenance,
            "formations": params.formations.provenance,
            "policy": params.policy.provenance,
        },
    });
    write_json(dir, "run.json", &record)?;
    let manifest_path = dir.join("match_manifest.json");
    std::fs::write(
        manifest_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest)
                .map_err(|error| std::io::Error::other(error.to_string()))?
        ),
    )
}

fn print_report(report: &CoreValidationReport) {
    let games = report.games.len().max(1) as f64;
    let points: f64 = report
        .games
        .iter()
        .map(|game| (game.home_score + game.away_score) as f64)
        .sum();
    let possessions: f64 = report
        .games
        .iter()
        .map(|game| game.possessions as f64)
        .sum();
    println!("动作链内核验证 · {} 场", report.games.len());
    println!(
        "  得分 {:.1}  回合 {:.1}  几何违规 {}  语境违规 {}  账本失败 {}",
        points / games,
        possessions / games,
        report.geometry_violations,
        report.context_violations,
        report.ledger_failures
    );
    for failure in report.failed_criteria() {
        println!("  失败：{failure}");
    }
    println!(
        "  结论：{}",
        if report.passed {
            "全部判定通过"
        } else {
            "存在未通过项"
        }
    );
}

pub(crate) fn run_mechanism(
    attribute: MechanismAttribute,
    seed: u64,
    out: Option<PathBuf>,
    json: bool,
) -> std::io::Result<u8> {
    let config = MechanismConfig { seed_start: seed, ..MechanismConfig::default() };
    let result = run_mechanism_experiment(attribute, config).map_err(std::io::Error::other)?;
    let directory = out.unwrap_or_else(workspace_validation_dir);
    let filename = format!("mechanism-{}.json", attribute.key());
    write_json(&directory, &filename, &result)?;
    if json {
        serde_json::to_writer_pretty(std::io::stdout().lock(), &result)?;
        println!();
    } else {
        println!("{}: {}（{} 个种子；证据：{}）", result.attribute, result.verdict, result.seeds, directory.join(filename).display());
    }
    Ok(0)
}

fn is_generated_output(path: &str, out_dir: Option<&str>) -> bool {
    out_dir.is_some_and(|prefix| path.starts_with(prefix))
}

/// 获取提交和工作区状态，忽略这次运行的输出目录。
fn git_state(out_dir: &Path) -> (String, bool) {
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let root = nba_sim_core::params::workspace_root();
    let absolute = if out_dir.is_absolute() {
        out_dir.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| root.clone())
            .join(out_dir)
    };
    let out_dir_prefix = absolute.strip_prefix(&root).ok().map(|path| {
        format!(
            "{}/",
            path.to_string_lossy()
                .replace('\\', "/")
                .trim_end_matches('/')
        )
    });
    let dirty = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout).lines().any(|line| {
                let path = line.get(3..).unwrap_or("").trim();
                !is_generated_output(path, out_dir_prefix.as_deref())
            })
        })
        .unwrap_or(true);
    (commit, dirty)
}

#[cfg(test)]
mod tests {
    use super::is_generated_output;

    #[test]
    fn generated_artifacts_do_not_mark_workspace_dirty() {
        let output = Some(".work/validation/");
        assert!(is_generated_output(".work/validation/run.json", output));
        assert!(!is_generated_output(".work/validation-other/run.json", output));
        assert!(!is_generated_output("crates/cli/src/main.rs", output));
    }
}
