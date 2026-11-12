//! `validate-core` 子命令：动作链内核的验证入口。
//!
//! 判定由四层判定（几何、语境、分布）与箱体守恒组成。任何一项失败，
//! 进程以非零退出码结束，避免把失败当成日志。工件目录默认取仓库根下的
//! 运行目录，不从进程当前目录解析。

use std::path::{Path, PathBuf};

use nba_sim_core::params::workspace_validation_dir;
use nba_sim_core::{builtin_manifest, Params};
use nba_validation::core_validation::{
    batch_statistical_artifact, certification_artifact, load_known_insufficient,
    run_core_validation_for_league, CoreValidationReport,
};
use nba_validation::mechanism_core::{run_mechanism_experiments, MechanismConfig, MechanismReport};

pub(crate) fn run_validate_core(
    seed_start: u64,
    games: u64,
    json: bool,
    mechanism: bool,
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
    let mechanism_report = if mechanism {
        let config = MechanismConfig {
            seed_start,
            ..MechanismConfig::default()
        };
        let mechanism_report = run_mechanism_experiments(config)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        print_mechanism(&mechanism_report);
        write_json(&out_dir, "mechanism_core.json", &mechanism_report)?;
        Some(mechanism_report)
    } else {
        None
    };
    let (commit, dirty) = git_state(&out_dir);
    let known = load_known_insufficient(&nba_sim_core::params::workspace_calibration_dir())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let certification = certification_artifact(&report, mechanism_report.as_ref(), &commit, &known);
    write_json(&out_dir, "certification.json", &certification)?;
    let params = Params::load(&nba_sim_core::params::workspace_calibration_dir())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    write_json(
        &out_dir,
        "batch_statistical_results.json",
        &batch_statistical_artifact(&report, &params),
    )?;
    let coverage = certification
        .get("coverage")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    write_json(&out_dir, "branch_coverage.json", &coverage)?;
    // 五维聚合诊断：随本次验证运行生成，同时写入数据目录与证据目录。
    let five = nba_validation::five_dimension::run_five_dimension(
        seed_start,
        games,
        nba_domain::rules::GameRules::for_league(league),
    )
    .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    nba_validation::five_dimension::write_five_dimension(&five, &out_dir)?;
    let root = nba_sim_core::params::workspace_root();
    nba_validation::five_dimension::write_five_dimension(
        &five,
        &root.join("data").join("diagnostics"),
    )?;
    nba_validation::five_dimension::write_five_dimension(
        &five,
        &root
            .join("docs")
            .join("dev")
            .join("evidence")
            .join("action_chain_validation"),
    )?;
    for failure in five.failed_metrics() {
        eprintln!("  五维诊断未通过：{failure}");
    }
    println!(
        "  五维诊断：{}（{} 场，{} 个指标）",
        if five.passed { "通过" } else { "未通过" },
        five.seeds.len(),
        five.dimensions
            .iter()
            .map(|d| d.metrics.len())
            .sum::<usize>()
    );
    write_run_record(
        &out_dir,
        seed_start,
        games,
        mechanism_report.as_ref(),
        &commit,
        dirty,
        league,
    )?;
    println!("  工件目录：{}", out_dir.display());
    let status = certification
        .get("certification_status")
        .and_then(|status| status.as_str())
        .unwrap_or("insufficient_evidence");
    let known = certification
        .get("criteria")
        .and_then(|criteria| criteria.get("known_insufficient"))
        .and_then(|items| items.as_array())
        .map(|items| items.len())
        .unwrap_or(0);
    let new_insufficient = certification
        .get("criteria")
        .and_then(|criteria| criteria.get("new_insufficient"))
        .and_then(|items| items.as_array())
        .map(|items| items.len())
        .unwrap_or(0);
    let not_applicable = certification
        .get("criteria")
        .and_then(|criteria| criteria.get("not_applicable"))
        .and_then(|items| items.as_array())
        .map(|items| items.len())
        .unwrap_or(0);
    let marginal = certification
        .get("criteria")
        .and_then(|criteria| criteria.get("marginal"))
        .and_then(|items| items.as_array())
        .map(|items| items.len())
        .unwrap_or(0);
    println!(
        "  认证：{status}（已知不足 {known} 项，未登记不足 {new_insufficient} 项，当前不适用 {not_applicable} 项，边缘通过 {marginal} 项）"
    );
    Ok(match status {
        "pass" => 0,
        "defects_found" => 1,
        _ => 3,
    })
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
    mechanism: Option<&MechanismReport>,
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
        "mechanism": mechanism.map(|report| serde_json::json!({
            "seeds": report.config.seeds,
            "possessions": report.config.possessions,
            "low_value": report.config.low_value,
            "high_value": report.config.high_value,
            "passed": report.passed,
        })),
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
    for layer in &report.gate.layers {
        println!(
            "  {} {:<28} 观测 {:.3}  基准 {:.3}",
            if layer.passed { "OK " } else { "BAD" },
            layer.criterion,
            layer.observed,
            layer.target
        );
    }
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

fn print_mechanism(report: &MechanismReport) {
    println!();
    println!(
        "机制实验 · 属性扰动响应 · {} 个属性 · 每属性 {} 个种子",
        report.results.len(),
        report.config.seeds
    );
    for kind in ["wiring", "outcome"] {
        let rows: Vec<_> = report
            .results
            .iter()
            .filter(|result| result.evidence_kind == kind)
            .collect();
        if rows.is_empty() {
            continue;
        }
        println!(
            "  ── 证据类别 {}（{}）──",
            kind,
            if kind == "wiring" {
                "度量直接取自消费者公式的输入，检查接线是否接通"
            } else {
                "度量是比赛产物，检查技巧是否改变结果"
            }
        );
        for result in rows {
            let bound = result
                .lower_bound_ratio
                .map(|ratio| format!("{ratio:.2}"))
                .unwrap_or_else(|| "-".to_string());
            let percent = |value: f64| value * 100.0;
            println!(
                "  {:<14} {:<22} {:<24} 基准 {:.4} 扰动 {:.4} 变化 {:+.2}% 区间 [{:+.2}%, {:+.2}%] 下界比 {} 预期 {}{}",
                result.verdict,
                result.attribute,
                result.metric,
                result.baseline,
                result.perturbed,
                percent(result.relative_change),
                percent(result.ci_low),
                percent(result.ci_high),
                bound,
                result.expectation,
                if result.marginal { "（边缘）" } else { "" }
            );
            if !result.reason.is_empty() {
                println!("                 原因：{}", result.reason);
            }
        }
    }
    let defects = report
        .results
        .iter()
        .filter(|result| result.verdict == "defect")
        .count();
    let insufficient = report
        .results
        .iter()
        .filter(|result| result.verdict == "insufficient")
        .count();
    let not_applicable = report
        .results
        .iter()
        .filter(|result| result.verdict == "not_applicable")
        .count();
    println!(
        "  结论：缺陷 {} 项 · 结构上不适用 {} 项 · 效应不足以分辨 {} 项 · 通过 {} 项",
        defects,
        not_applicable,
        insufficient,
        report.results.len() - defects - insufficient - not_applicable
    );
}

const GENERATED_OUTPUT_PREFIXES: &[&str] = &[
    "docs/dev/evidence/action_chain_validation/",
    "docs/dev/evidence/fiba_validation/",
];

const GENERATED_OUTPUT_PATHS: &[&str] = &[
    "docs/dev/evidence/gap_closure_cycle/gate.txt",
    "docs/dev/evidence/gap_closure_cycle/verification.json",
    "docs/dev/evidence/performance.json",
    "data/diagnostics/five_dimension.json",
    "crates/sim/tests/baseline/fingerprint.json",
];

fn is_generated_output(path: &str, out_dir: Option<&str>) -> bool {
    out_dir.is_some_and(|prefix| path.starts_with(prefix))
        || GENERATED_OUTPUT_PREFIXES
            .iter()
            .any(|prefix| path.starts_with(prefix))
        || GENERATED_OUTPUT_PATHS.contains(&path)
}

/// 提交与工作区状态。认证输出不计入 dirty：它们由本次运行写出，
/// 随后统一提交，工作区状态应当只反映代码与输入数据的改动。
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
        assert!(is_generated_output(
            "docs/dev/evidence/action_chain_validation/run.json",
            None
        ));
        assert!(is_generated_output(
            "docs/dev/evidence/fiba_validation/run.json",
            None
        ));
        assert!(is_generated_output(
            "docs/dev/evidence/gap_closure_cycle/gate.txt",
            None
        ));
        assert!(is_generated_output(
            "data/diagnostics/five_dimension.json",
            None
        ));
        assert!(is_generated_output(
            "crates/sim/tests/baseline/fingerprint.json",
            None
        ));
        assert!(!is_generated_output("crates/cli/src/main.rs", None));
    }
}
