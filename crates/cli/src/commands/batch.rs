//! `batch` 子命令：多种子批量模拟与聚合门禁。
//!
//! 这是 CI 的 `stage-gate` 走的路径，因此它的门禁与单场模式必须同源：
//! 违规数、账本对平、Hard 门三者都在此判定。

use nba_domain::GameRules;
use nba_engine::{MatchEngine, MatchSetup, StreamMode};
use std::fs::{self, File};
use std::time::Instant;

use crate::commands::simulate::TempStreamGuard;
use crate::{
    cli_temp_root, enforce_hard_gate, pct, read_stream_text, write_violation_ledger,
};

pub(crate) fn run_batch_simulation(
    seeds: &[u64],
    scope: &str,
    rules: GameRules,
    out_path: Option<&str>,
    stream_mode: StreamMode,
) -> std::io::Result<()> {
    println!(
        "🚀 Starting Batch Simulation ({} Games, Seeds {}..={}, Scope={})...\n",
        seeds.len(),
        seeds.first().copied().unwrap_or(0),
        seeds.last().copied().unwrap_or(0),
        scope
    );

    let start = Instant::now();
    let mut total_points_list = Vec::with_capacity(seeds.len());
    let mut avg_dur_list = Vec::with_capacity(seeds.len());
    let mut fg3_pct_list = Vec::with_capacity(seeds.len());
    let mut total_violations = 0;
    let mut all_judgments: Vec<nba_evaluator::Judgment> = Vec::new();
    // 账本对平（plan §7.2 出口门「账本和事件工件完整」）。
    //
    // 此前 `check_ledger()` 只在单场模式被调用，批量模式（即 CI 的
    // `stage-gate` job 跑的那条路径）**从未做过账本对平**，batch jsonl
    // 也没有任何 ledger 字段。实测见 evidence/problem.md §30.4。
    let mut total_ledger_violations = 0usize;
    let mut ledger_report: Vec<serde_json::Value> = Vec::new();

    let stats_writer = out_path.map(|path| {
        let file = File::create(path)?;
        Ok::<_, std::io::Error>(std::io::BufWriter::new(file))
    });
    let mut stats_out = match stats_writer {
        Some(writer) => Some(writer?),
        None => None,
    };

    for (i, &seed) in seeds.iter().enumerate() {
        let setup = MatchSetup::builtin(rules.clone());
        let mut engine = MatchEngine::with_setup(setup, seed);

        // batch 为临时单场流：使用调用方指定模式（默认 facts），
        // 由 RAII 守卫保证单场结束（含错误提前返回）即删除，
        // 避免累积占用磁盘（gap.md §16.4）。
        let guard = TempStreamGuard::new(cli_temp_root().join(format!(
            "nba_batch_{}_{}.ndjson",
            std::process::id(),
            seed
        )));
        let out_path = guard.path_str();
        let summary = engine.simulate_scope_and_export_with_mode(scope, &out_path, stream_mode)?;

        let pts = engine.home_score() + engine.away_score();
        let poss = engine.completed_possessions();
        let dur = if poss > 0 {
            engine.current_time() / poss as f32
        } else {
            0.0
        };
        let game_violations = summary.violations.len();
        total_violations += game_violations;

        // 单场违规工件：仅在存在违反时落盘（与单场模式同名约定）。
        if game_violations > 0 {
            let violation_file = format!("{}.violations.ndjson", out_path);
            write_violation_ledger(&violation_file, &summary.violations)?;
        }

        // 每场一行统计 JSON（quality batch --out 工件）。
        if let Some(writer) = stats_out.as_mut() {
            use std::io::Write;
            let line = serde_json::json!({
                "seed": seed,
                "total_points": pts,
                "possessions": poss,
                "avg_possession_secs": dur,
                "fg2_made": summary.box_score.fg2_made,
                "fg2_attempts": summary.box_score.fg2_attempts,
                "fg3_made": summary.box_score.fg3_made,
                "fg3_attempts": summary.box_score.fg3_attempts,
                "fg3_pct": summary.box_score.fg3_pct(),
                "ft_made": summary.box_score.ft_made,
                "ft_attempts": summary.box_score.ft_attempts,
                "turnovers": summary.box_score.turnovers,
                "fouls": summary.box_score.fouls,
                "violations": game_violations,
            });
            writeln!(writer, "{}", line)?;
        }

        println!(
            "   [Game {:02}/{:02}] Seed={:<3} Total={:<3} Poss={:<3} Dur={:.2}s 3P={:.1}% Violations={}",
            i + 1,
            seeds.len(),
            seed, pts, poss, dur, pct(summary.box_score.fg3_pct()), game_violations
        );

        total_points_list.push(pts as f32);
        avg_dur_list.push(dur);
        fg3_pct_list.push(pct(summary.box_score.fg3_pct()));

        // M8 评判：逐场评判并聚合（batch 与 violations 同落盘）。
        if let Ok(stream) = read_stream_text(&out_path) {
            let ticks = match nba_evaluator::parse_stream(&stream) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!(
                        "⚠️ batch stream parse failed (strict): {} — skipping game",
                        e
                    );
                    continue;
                }
            };
            let fixture = nba_evaluator::ReferenceDistributions::for_league(&rules.league.name);
            all_judgments.extend(
                nba_evaluator::evaluate_stream(&ticks, &fixture)
                    .into_iter()
                    .map(|mut j| {
                        j.seed = Some(seed);
                        j
                    }),
            );
            // 账本对平：与单场模式同一函数，避免第二套口径。
            let ledger_violations = nba_evaluator::check_ledger(&ticks);
            total_ledger_violations += ledger_violations.len();
            ledger_report.push(serde_json::json!({
                "seed": seed,
                "ledger_violations": ledger_violations.len(),
                "equations_checked": nba_evaluator::LEDGER_EQUATION_COUNT,
            }));
            if !ledger_violations.is_empty() {
                for v in ledger_violations.iter().take(5) {
                    eprintln!(
                        "   ⚠️ seed {} ledger violation: {} — {}",
                        seed, v.equation, v.detail
                    );
                }
            }
        }
        // 临时流由 `guard` 在作用域结束时删除（RAII，异常安全）。
        drop(guard);
    }

    if let Some(writer) = stats_out.as_mut() {
        use std::io::Write;
        writer.flush()?;
    }

    let fixture = nba_evaluator::ReferenceDistributions::for_league(&rules.league.name);
    let report = nba_evaluator::attribution_report(&all_judgments, &fixture.version);
    if !all_judgments.is_empty() {
        // 未指定 `--out` 时，聚合工件是**中间产物**：它们的消费者是本次进程
        // 的控制台汇总，而不是调用方。这类产物必须随进程退出自动删除，
        // 否则每次不带 `--out` 的 batch 都会在临时目录留下三个文件。
        //
        // 实测（本会话）：`nba_batch_aggregate_*` 累积到 24 个、共 23.8 MiB，
        // `scripts/check_disk_budget.py` 报 FAILED；而该守卫在
        // `run-tests.sh` 里以 `|| true` 调用，因此这些残留不会使测试变红，
        // 只会静默占盘。单场流早已用 `TempStreamGuard` 做了同样的事，
        // 这里补齐同一约定。
        let fallback_guard = out_path.is_none().then(|| {
            TempStreamGuard::new(
                cli_temp_root().join(format!("nba_batch_aggregate_{}", std::process::id())),
            )
        });
        let fallback_str;
        let base = match &fallback_guard {
            Some(guard) => {
                fallback_str = guard.path_str();
                fallback_str.as_str()
            }
            None => out_path.expect("out_path is Some when fallback is None"),
        };
        let judgments_path = format!("{}.judgments.ndjson", base);
        {
            use std::io::Write;
            let jf = File::create(&judgments_path)?;
            let mut w = std::io::BufWriter::new(jf);
            for j in &all_judgments {
                writeln!(
                    w,
                    "{}",
                    serde_json::to_string(j).map_err(std::io::Error::other)?
                )?;
            }
            w.flush()?;
        }
        fs::write(
            format!("{}.attribution_report.json", base),
            serde_json::to_string_pretty(&report).map_err(std::io::Error::other)?,
        )?;
        // 逐场账本结果落盘：与单场模式的 ledger_violations.ndjson 同源同义。
        fs::write(
            format!("{}.ledger_report.json", base),
            serde_json::to_string_pretty(&serde_json::json!({
                "games": ledger_report,
                "total_ledger_violations": total_ledger_violations,
            }))
            .map_err(std::io::Error::other)?,
        )?;
        // 临时聚合工件由 `fallback_guard` 在作用域结束时删除（RAII）。
        drop(fallback_guard);
    }

    total_points_list.sort_by(|a, b| a.partial_cmp(b).unwrap());
    avg_dur_list.sort_by(|a, b| a.partial_cmp(b).unwrap());
    fg3_pct_list.sort_by(|a, b| a.partial_cmp(b).unwrap());

    if total_points_list.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "batch requires at least one seed",
        ));
    }
    let median_pts = total_points_list[total_points_list.len() / 2];
    let median_dur = avg_dur_list[avg_dur_list.len() / 2];
    let median_3p = fg3_pct_list[fg3_pct_list.len() / 2];

    println!("\n📈 Batch Summary (N={}):", seeds.len());
    println!(
        "   Median Total Points: {:.1} (Min: {:.1}, Max: {:.1})",
        median_pts,
        total_points_list[0],
        total_points_list[total_points_list.len() - 1]
    );
    println!("   Median Poss Duration: {:.2}s", median_dur);
    println!("   Median 3P Accuracy: {:.1}%", median_3p);
    println!("   Total Axiom Violations: {}", total_violations);
    // 账本对平结果必须与违规数并列可见：轴门与账本是两类不同的失败，
    // 不能只看其中一个（plan §7.2）。
    println!(
        "   Total Ledger Violations: {} (5 equations per game)",
        total_ledger_violations
    );
    println!(
        "   Realism Index: {:.3} ({} judgments, {} defects) — fixture {}",
        report.realism_index, report.total_judgments, report.defect_count, report.fixture_version
    );
    println!(
        "   Elapsed Wall Time: {:.2}s\n",
        start.elapsed().as_secs_f64()
    );

    if total_violations > 0 {
        std::process::exit(1);
    }
    // 账本不平衡 = Hard（与单场模式同一判据）：金额式不对平意味着
    // 事实流不能独立重建比赛，比任何分布偏差都更严重。
    if total_ledger_violations > 0 {
        eprintln!(
            "⛔ batch LEDGER gate FAILED: {} violation(s) across {} games",
            total_ledger_violations,
            seeds.len()
        );
        std::process::exit(1);
    }
    // gap.md §18.6：batch 此前只把 Hard 门打印成一行，即便
    // hard_gate_failed 且上百条 Hard defect 也以 0 退出。
    if enforce_hard_gate(&report, "batch") {
        std::process::exit(1);
    }
    Ok(())
}
