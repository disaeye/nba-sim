//! `run` 子命令的单场模拟与临时流文件管理。
//!
//! `TempStreamGuard` 与 `cli_temp_root` 放在此处而非 `main.rs`：
//! 只有模拟路径需要它们（batch 与 single 都写临时流）。
//! 守卫保证无论正常结束、`?` 提前返回还是 panic 展开都删除临时 NDJSON，
//! 避免 batch 中途失败留下数 GB 垃圾（gap.md §16.4 / problem.md §14.4）。

use nba_domain::GameRules;
use nba_engine::{MatchEngine, MatchSetup, StreamMode};
use std::fs::{self, File};
use std::time::Instant;

use crate::{enforce_hard_gate, pct, read_stream_text, write_judgment_artifacts,
    write_violation_ledger};

pub(crate) fn run_single_simulation(
    seed: u64,
    out_path: &str,
    scope: &str,
    rules: GameRules,
    stream_mode: StreamMode,
) -> std::io::Result<()> {
    println!("🏀 Initializing NBA-Sim Rust Engine with Rapier2D Physics...");
    println!(
        "   Seed: {}, Output: {}, Scope: {}, StreamMode: {:?}",
        seed, out_path, scope, stream_mode
    );

    let start = Instant::now();
    let setup = MatchSetup::builtin(rules.clone());
    let mut engine = MatchEngine::with_setup(setup, seed);

    let summary = engine.simulate_scope_and_export_with_mode(scope, out_path, stream_mode)?;
    let elapsed = start.elapsed();

    let total_points = engine.home_score() + engine.away_score();
    let completed_poss = engine.completed_possessions();
    let avg_poss_sec = if completed_poss > 0 {
        engine.current_time() / completed_poss as f32
    } else {
        0.0
    };

    println!("\n📊 Box Score & Advanced Statistics:");
    println!(
        "   Score: Home {} - {} Away (Total: {})",
        engine.home_score(),
        engine.away_score(),
        total_points
    );
    println!(
        "   Possessions: {} | Avg Duration: {:.2}s",
        completed_poss, avg_poss_sec
    );
    println!(
        "   2PT FG: {}/{} ({:.1}%)",
        summary.box_score.fg2_made,
        summary.box_score.fg2_attempts,
        pct(summary.box_score.fg2_pct())
    );
    println!(
        "   3PT FG: {}/{} ({:.1}%)",
        summary.box_score.fg3_made,
        summary.box_score.fg3_attempts,
        pct(summary.box_score.fg3_pct())
    );
    println!(
        "   FT:     {}/{} ({:.1}%)",
        summary.box_score.ft_made,
        summary.box_score.ft_attempts,
        pct(summary.box_score.ft_pct())
    );
    println!(
        "   Turnovers: {} | Fouls: {}",
        summary.box_score.turnovers, summary.box_score.fouls
    );

    // quality.md §1.1 严重级别规定：Hard 阻断（退出码 1），
    // Soft 计数上报但不阻断（退出码 0）。此前把两者一并当作失败，
    // 使合法的几何安全缓冲被当成硬错误。
    let hard_count = summary.taxonomy.hard_count;
    let soft_count = summary.taxonomy.soft_count;
    let violation_count = summary.violations.len();
    if violation_count > 0 {
        let verdict = if hard_count > 0 {
            format!("❌ Simulation FAILED: {} Hard Axiom Violations", hard_count)
        } else {
            format!("⚠️  {} Soft Axiom Warnings (non-blocking)", soft_count)
        };
        println!(
            "\n{} detected across {} ticks (hard={}, soft={}).",
            verdict, summary.ticks, hard_count, soft_count
        );
        println!("   Taxonomy Breakdown: {:?}", summary.taxonomy);
        // gap.md §16.4：写入错误必须向上传播，不得 `let _ = writeln!()`。
        let violation_file = format!("{}.violations.ndjson", out_path);
        write_violation_ledger(&violation_file, &summary.violations)?;
        println!(
            "   📁 Exported structured violation ledger to: {}",
            violation_file
        );
        for v in &summary.violations {
            println!("   {}", v);
        }
        if hard_count > 0 {
            std::process::exit(1);
        }
    }
    if violation_count == 0 {
        let tps = (summary.ticks as f64 / elapsed.as_secs_f64().max(0.001)) as u64;
        println!(
            "\n✅ Completed {} ({} ticks) in {:.2}s ({} ticks/sec): 0 Axiom Violations.\n",
            summary.scope_desc,
            summary.ticks,
            elapsed.as_secs_f64(),
            tps
        );
    }

    // M8 评判工件：judgments.ndjson + attribution_report.json 与流同批写入。
    if let Ok(stream) = read_stream_text(out_path) {
        // D1.3 严格解析：坏行 = 流不可信，跳过评判并告警（不产出假工件）。
        let ticks = match nba_evaluator::parse_stream(&stream) {
            Ok(t) => t,
            Err(e) => {
                eprintln!(
                    "⚠️ stream parse failed (strict): {} — skipping judgment artifacts",
                    e
                );
                return Ok(());
            }
        };
        let fixture = nba_evaluator::ReferenceDistributions::for_league(&rules.league.name);
        let judgments = nba_evaluator::evaluate_stream(&ticks, &fixture)
            .into_iter()
            .map(|mut j| {
                j.seed = Some(seed);
                j
            })
            .collect::<Vec<_>>();
        let mut judgments_report: Option<nba_evaluator::AttributionReport> = None;
        match write_judgment_artifacts(out_path, &judgments, &fixture) {
            Ok(report) => {
                // D1.2：Hard 门失败时指数无效，先报门再报指数。
                if report.hard_gate_failed {
                    println!(
                        "⛔ HARD gate FAILED ({} Hard + {} Soft defects, coverage {:.0}%) → {}.judgments.ndjson",
                        report.hard_defect_count,
                        report.soft_defect_count,
                        pct(report.evidence_coverage),
                        out_path
                    );
                } else {
                    println!(
                        "🧾 Realism index: {:.3} ({} judgments, {} defects, coverage {:.0}%) → {}.judgments.ndjson",
                        report.realism_index,
                        report.total_judgments,
                        report.defect_count,
                        pct(report.evidence_coverage),
                        out_path
                    );
                }
                judgments_report = Some(report);
            }
            Err(e) => eprintln!("⚠️ judgment artifacts failed: {}", e),
        }
        // gap.md §18.6：Hard 门失败必须以失败退出，不得只打印。
        if let Some(report) = judgments_report.as_ref() {
            if enforce_hard_gate(report, "single") {
                std::process::exit(1);
            }
        }

        // D0.2 账本平衡检查（含 §23.10 新增的失误归因式）：
        // ledger_violations.ndjson 同批写入；账本不平衡 = Hard，
        // 违反条数计入输出供批处理门禁消费。
        let ledger_violations = nba_evaluator::check_ledger(&ticks);
        let ledger_path = format!("{}.ledger_violations.ndjson", out_path);
        match File::create(&ledger_path) {
            Ok(f) => {
                use std::io::Write;
                let mut w = std::io::BufWriter::new(f);
                let mut write_err: Option<std::io::Error> = None;
                for v in &ledger_violations {
                    if let Err(e) = writeln!(
                        w,
                        "{}",
                        serde_json::to_string(v)
                            .map_err(std::io::Error::other)
                            .unwrap_or_default()
                    ) {
                        write_err = Some(e);
                        break;
                    }
                }
                if let Some(e) = write_err.or_else(|| w.flush().err()) {
                    eprintln!("⚠️ ledger violations artifact write failed: {}", e);
                } else if ledger_violations.is_empty() {
                    println!(
                        "📒 Ledger: 5 equations balanced (score/possession/time/foul/turnover)."
                    );
                } else {
                    println!(
                        "📒 Ledger: {} HARD violations → {}",
                        ledger_violations.len(),
                        ledger_path
                    );
                }
            }
            Err(e) => eprintln!("⚠️ ledger violations artifact failed: {}", e),
        }
    }

    Ok(())
}

/// 临时流文件的作用域守卫：无论正常结束、`?` 提前返回还是 panic 展开，
/// 都保证删除临时 NDJSON，避免 batch 中途失败留下数 GB 垃圾
/// （gap.md §16.4 / problem.md §14.4）。
pub(crate) struct TempStreamGuard {
    path: std::path::PathBuf,
}

impl TempStreamGuard {
    pub(crate) fn new(path: std::path::PathBuf) -> Self {
        Self { path }
    }
    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }
    pub(crate) fn path_str(&self) -> String {
        self.path.to_string_lossy().to_string()
    }
}

impl Drop for TempStreamGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        // 同名前缀的全部工件：单场流只产生 `.violations.ndjson`，
        // 而 batch 的聚合工件产生 `.judgments.ndjson` /
        // `.attribution_report.json` / `.ledger_report.json`。
        // 守卫按前缀清扫，新增工件类型不需再改这里。
        let base = self.path.to_string_lossy().to_string();
        for suffix in [
            ".violations.ndjson",
            ".judgments.ndjson",
            ".attribution_report.json",
            ".ledger_report.json",
            ".ledger_violations.ndjson",
        ] {
            let _ = fs::remove_file(format!("{}{}", base, suffix));
        }
    }
}
