//! `validate` 子命令：跑比赛、输出验证工件并返回
//! validation.md §10 的退出码（0 通过 / 1 缺陷 / 2 执行错误 / 3 证据不足）。
//!
//! `--games N`（N > 1）时跑 N 场批次（validation.md §6.4 样本计划），
//! 并对登记参照（stat_deltas）执行批次 STAT 等效判定，
//! 输出 `batch_statistical_results.json`。

use std::path::PathBuf;

use nba_validation::runner::{
    run_batch_with_facts, run_validation, BatchRunConfig, ExitCode, ValidationRunConfig,
};

/// 单场入口：`nba-sim validate --seed N --scope S --out DIR`
pub fn run_validate(seed: u64, scope: &str, out_dir: &str) -> std::io::Result<()> {
    let config = ValidationRunConfig {
        seed,
        scope: scope.to_string(),
        output_dir: PathBuf::from(out_dir),
        max_ticks: 200_000,
    };
    let run = match run_validation(&config) {
        Ok(run) => run,
        // 执行错误优先返回 2（validation.md §10）。
        Err(message) => {
            eprintln!("❌ validation execution error: {message}");
            std::process::exit(i32::from(ExitCode::ExecutionError as u8));
        }
    };
    match nba_validation::runner::write_artifacts(&run, &config.output_dir) {
        Ok(paths) => {
            println!("✅ validation artifacts:");
            for path in paths {
                println!("   {}", path.display());
            }
        }
        Err(message) => {
            eprintln!("❌ validation artifact write failed: {message}");
            std::process::exit(i32::from(ExitCode::ExecutionError as u8));
        }
    }
    // 汇总报告。
    println!(
        "验证运行 seed={} scope={}：刻数 {}，事件 {}，场景 {}（pass {} / defect {} / insufficient {}）",
        config.seed,
        config.scope,
        run.trailer.records,
        run.trailer.events,
        run.scenario_outcomes.len(),
        run.scenario_outcomes
            .iter()
            .filter(|outcome| outcome.verdict == nba_validation::Verdict::Pass)
            .count(),
        run.scenario_outcomes
            .iter()
            .filter(|outcome| outcome.verdict == nba_validation::Verdict::Defect)
            .count(),
        run.scenario_outcomes
            .iter()
            .filter(|outcome| outcome.verdict == nba_validation::Verdict::InsufficientEvidence)
            .count(),
    );
    // 缺陷与未判定并存时报告两者且返回 1（validation.md §10）。
    let has_defect = run.exit_code() == ExitCode::Defect;
    let has_insufficient = run.exit_code() == ExitCode::InsufficientEvidence;
    if has_defect {
        eprintln!("⛔ validation defects found");
        std::process::exit(i32::from(ExitCode::Defect as u8));
    }
    if has_insufficient {
        eprintln!("⚠️ required evidence insufficient");
        std::process::exit(i32::from(ExitCode::InsufficientEvidence as u8));
    }
    println!("✅ validation passed");
    Ok(())
}

/// 批次入口：`nba-sim validate --seed N --games M --scope S --out DIR`。
/// 单场判定照常执行（首场种子），随后跑满 M 场做批次 STAT 等效判定。
pub fn run_validate_batch(
    seed: u64,
    games: u32,
    scope: &str,
    out_dir: &str,
) -> std::io::Result<()> {
    if games < 2 {
        return run_validate(seed, scope, out_dir);
    }
    // 首场：单场工件与场景判定照常。
    run_validate(seed, scope, out_dir)?;
    // 批次：validation.md §6.4 固定种子序列（first_seed..first_seed+n）。
    let config = BatchRunConfig {
        first_seed: seed,
        n_games: games,
        scope: scope.to_string(),
        output_dir: PathBuf::from(out_dir),
        max_ticks: 200_000,
    };
    if scope != "full" {
        eprintln!(
            "⚠️ 批次 STAT 判定的参照值来自整场比赛（scope=full）；当前 scope={scope} 的批次统计量与参照口径不一致，结果仅作诊断"
        );
    }
    println!("⏳ 批次运行 {} 场（seed {} 起）…", games, seed);
    let (metrics, sampling_facts) = match run_batch_with_facts(&config) {
        Ok(result) => result,
        Err(message) => {
            eprintln!("❌ validation batch execution error: {message}");
            std::process::exit(i32::from(ExitCode::ExecutionError as u8));
        }
    };
    let registration_path = nba_validation::reference::default_registration_path();
    let (registration, _) =
        match nba_validation::reference::load_stat_registrations(&registration_path) {
            Ok(result) => result,
            Err(message) => {
                eprintln!("❌ reference registration load failed: {message}");
                std::process::exit(i32::from(ExitCode::ExecutionError as u8));
            }
        };
    let report = nba_validation::stat_batch::evaluate_batch(&metrics, &registration, 1000, seed);
    let report_path = PathBuf::from(out_dir).join("batch_statistical_results.json");
    let text = serde_json::to_string_pretty(&report)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    std::fs::write(&report_path, text)?;
    println!("✅ batch statistical results: {}", report_path.display());
    // 分支覆盖工件：采样事实 × 规则域登记对照（三档 + 认证范围限制）。
    let coverage = nba_validation::sampling_coverage::branch_coverage_report(
        games,
        &sampling_facts,
    );
    let coverage_path = PathBuf::from(out_dir).join("branch_coverage.json");
    let coverage_text = serde_json::to_string_pretty(&coverage)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    std::fs::write(&coverage_path, coverage_text)?;
    println!(
        "✅ branch coverage: {} (sampled {} / scenario_only {} / uncovered {})",
        coverage_path.display(),
        coverage.sampled,
        coverage.scenario_only,
        coverage.uncovered
    );
    // 汇总输出（validation.md §6.3 三出口）。
    let passes = report
        .verdicts
        .iter()
        .filter(|verdict| verdict.verdict == "Pass")
        .count();
    let defects = report
        .verdicts
        .iter()
        .filter(|verdict| verdict.verdict == "Defect")
        .count();
    let insufficient = report.verdicts.len() - passes - defects;
    println!(
        "批次 STAT 判定 {} 场：指标 {} 项（pass {} / defect {} / insufficient {}）",
        games,
        report.verdicts.len(),
        passes,
        defects,
        insufficient
    );
    for (criterion, verdict) in &report.criterion_verdicts {
        println!("   {criterion}: {verdict}");
    }
    if defects > 0 {
        eprintln!("⛔ batch statistical defects found");
        std::process::exit(i32::from(ExitCode::Defect as u8));
    }
    println!("✅ batch validation passed");
    Ok(())
}
