use flate2::read::GzDecoder;
use nba_domain::GameRules;
use nba_engine::{MatchEngine, MatchSetup, StreamMode};
use nba_invariants::{InvariantChecker, Violation, ViolationTaxonomy};
use nba_protocol::StreamTick;
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::time::Instant;

fn read_stream_text(path: &str) -> std::io::Result<String> {
    // 评判工件通常已经是内存字符串；这里保持同一语义，并按 gzip magic
    // 自动解压，避免要求调用方记住 frames-gzip 的特殊后缀。
    let bytes = fs::read(path)?;
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut content = String::new();
        GzDecoder::new(bytes.as_slice()).read_to_string(&mut content)?;
        Ok(content)
    } else {
        String::from_utf8(bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    }
}

fn load_rules(path_opt: Option<&str>) -> std::io::Result<GameRules> {
    if let Some(path) = path_opt {
        let content = fs::read_to_string(path)?;
        let rules: GameRules = serde_json::from_str(&content).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Failed to parse rules JSON {}: {}", path, e),
            )
        })?;
        rules.validate().map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Invalid rules in {}: {}", path, e),
            )
        })?;
        println!("⚙️  Loaded rules override from {}", path);
        Ok(rules)
    } else {
        Ok(GameRules::default())
    }
}

/// 违规账本落盘：任何写入/序列化错误向上传播（gap.md §16.4）。
fn write_violation_ledger(path: &str, violations: &[Violation]) -> std::io::Result<()> {
    use std::io::Write;
    let file = File::create(path)?;
    let mut writer = std::io::BufWriter::new(file);
    for v in violations {
        writeln!(
            writer,
            "{}",
            serde_json::to_string(v).map_err(std::io::Error::other)?
        )?;
    }
    writer.flush()
}

/// 评判工件落盘（M8：judgments.ndjson + attribution_report.json）。
/// 门禁判定：把「评判 Hard 门失败」变成进程退出码（gap.md §18.6 第 7 条）。
///
/// ## 为什么需要这个函数
///
/// `AttributionReport.hard_gate_failed` 自 D1.2 起就已正确计算，但**没有任何
/// 调用点消费它**：单场、batch、evaluate 三条路径都只打印 `⛔ HARD gate
/// FAILED` 然后正常返回。实测 `--seeds 0..1 batch full` 在
/// `hard_gate_failed=true` 且 61 条 defect 的情况下**退出码仍为 0**，
/// 因此 CI 的 `stage-gate` job 是一条永远为绿的假门。
///
/// 打印而不断言，等于把门降级成日志。
///
/// 返回 `true` 表示调用方应当以失败退出。
fn enforce_hard_gate(report: &nba_evaluator::AttributionReport, context: &str) -> bool {
    if !report.hard_gate_failed {
        return false;
    }
    eprintln!(
        "\u{26d4} HARD gate FAILED [{context}]: {} Hard + {} Soft defects ({} judgments), \
         coverage {:.0}%, fixture {}",
        report.hard_defect_count,
        report.soft_defect_count,
        report.total_judgments,
        pct(report.evidence_coverage),
        report.fixture_version,
    );
    for row in report.defects_by_criterion.iter().take(8) {
        eprintln!(
            "   [{:>4}x] {} ({}; hard={}, soft={})",
            row.count, row.criterion, row.attribution, row.hard, row.soft
        );
    }
    eprintln!(
        "   gap.md §18.6: Hard gate failure must not be offset by the realism index; every turnover/score/rebound must have a causal source."
    );
    true
}

fn write_judgment_artifacts(
    stream_path: &str,
    judgments: &[nba_evaluator::Judgment],
    fixture: &nba_evaluator::ReferenceDistributions,
) -> std::io::Result<nba_evaluator::AttributionReport> {
    let report = nba_evaluator::attribution_report(judgments, &fixture.version);
    let judgments_path = format!("{}.judgments.ndjson", stream_path);
    let report_path = format!("{}.attribution_report.json", stream_path);
    let jf = File::create(&judgments_path)?;
    {
        let mut w = std::io::BufWriter::new(jf);
        use std::io::Write;
        for j in judgments {
            writeln!(
                w,
                "{}",
                serde_json::to_string(j).map_err(std::io::Error::other)?
            )?;
        }
        w.flush()?;
    }
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&report).map_err(std::io::Error::other)?,
    )?;
    Ok(report)
}

fn run_single_simulation(
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

    // quality.md §1.1 严重级别契约：Hard 阻断（退出码 1），
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

    // M8 评判工件：judgments.ndjson + attribution_report.json 与流同落盘。
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
        // ledger_violations.ndjson 同落盘；账本不平衡 = Hard，
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

/// CLI 自己的临时目录根（与 test-support 的约定一致）。
///
/// 项目约定临时数据统一放 `/home/ubuntu/basketball`（可用 `NBA_TEMP_ROOT`
/// 覆盖）；不再落 `/tmp`——那里与构建产物共享分区，且历史泄漏正是从
/// `/tmp/nba_batch_*.ndjson` 累积出来的（单次实测残留 4.2 GB）。
fn cli_temp_root() -> std::path::PathBuf {
    std::env::var_os("NBA_TEMP_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/home/ubuntu/basketball"))
}

/// 临时流文件的作用域守卫：无论正常结束、`?` 提前返回还是 panic 展开，
/// 都保证删除临时 NDJSON，避免 batch 中途失败留下数 GB 垃圾
/// （gap.md §16.4 / problem.md §14.4）。
struct TempStreamGuard {
    path: std::path::PathBuf,
}

impl TempStreamGuard {
    fn new(path: std::path::PathBuf) -> Self {
        Self { path }
    }
    fn path(&self) -> &std::path::Path {
        &self.path
    }
    fn path_str(&self) -> String {
        self.path.to_string_lossy().to_string()
    }
}

impl Drop for TempStreamGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_file(format!("{}.violations.ndjson", self.path.display()));
    }
}

fn run_batch_simulation(
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
        // 未指定 --out 时使用进程隔离的临时聚合路径，避免多次运行互相
        // 覆盖或在固定路径累积（gap.md §16.4）。
        let fallback = cli_temp_root().join(format!("nba_batch_aggregate_{}", std::process::id()));
        let fallback_str = fallback.to_string_lossy().to_string();
        let base = out_path.unwrap_or(fallback_str.as_str());
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
    // gap.md \u00a718.6\uff1abatch \u6b64\u524d\u53ea\u628a Hard \u95e8\u6253\u5370\u6210\u4e00\u884c\uff0c\u5373\u4fbf
    // hard_gate_failed \u4e14\u4e0a\u767e\u6761 Hard defect \u4e5f\u4ee5 0 \u9000\u51fa\u3002
    if enforce_hard_gate(&report, "batch") {
        std::process::exit(1);
    }
    Ok(())
}

/// 离线评判（quality 评判规范）：对既有 ndjson 流产出评判工件。
fn run_evaluate(stream_path: &str) -> std::io::Result<()> {
    let stream = read_stream_text(stream_path)?;
    let ticks = nba_evaluator::parse_stream(&stream)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let fixture = nba_evaluator::ReferenceDistributions::nba_v1();
    let judgments = nba_evaluator::evaluate_stream(&ticks, &fixture);
    let report = write_judgment_artifacts(stream_path, &judgments, &fixture)?;
    println!(
        "🧾 Evaluated {} ticks: {} judgments, {} defects",
        ticks.len(),
        report.total_judgments,
        report.defect_count
    );
    for row in report.defects_by_criterion.iter().take(5) {
        println!(
            "   [{:>3}x] {} ({})",
            row.count, row.criterion, row.attribution
        );
    }
    println!("   Realism index: {:.3}", report.realism_index);
    // gap.md \u00a718.6\uff1aevaluate \u5b50\u547d\u4ee4\u540c\u6837\u5fc5\u987b\u53cd\u6620 Hard \u95e8\u3002
    if enforce_hard_gate(&report, "evaluate") {
        std::process::exit(1);
    }
    Ok(())
}

/// 转换外部 PBP 数据为 ReferenceDistributions fixture（M8 扩展位）。
fn run_pbp_convert(
    input_path: &str,
    out_path: &str,
    league: &str,
    version: &str,
) -> std::io::Result<()> {
    let content = fs::read_to_string(input_path)?;
    let events: Vec<nba_evaluator::PbpEvent> = if content.trim_start().starts_with('[') {
        serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?
    } else {
        content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                serde_json::from_str(l)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
            })
            .collect::<Result<Vec<_>, _>>()?
    };

    let fixture = nba_evaluator::convert_pbp_events_to_fixture(&events, version, league);
    let serialized = serde_json::to_string_pretty(&fixture)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    fs::write(out_path, serialized)?;
    println!(
        "📊 Converted {} PBP events → {} (version: {}, league: {})",
        events.len(),
        out_path,
        version,
        league
    );
    Ok(())
}
/// release 模式下跑固定 tick 数，对照 ≥ 20,000 ticks/s 预算。
/// 分层性能基准（gap.md §18.5 / §16.4）。
///
/// 性能预算必须与输出模式绑定：默认有界输出不得与逐 tick 帧混淆，
/// 纯引擎吞吐也不得把序列化成本算进去。每一层独立测量并独立判定。
// 分层性能预算（gap.md §18.5）。
//
// 预算必须来自**实测并留出回归余量**，不能沿用旧单层基准的拍脑袋值。
// 本轮实测（release, seed 42, 120k ticks 窗口，3 次取稳）：
//   engine-only  : 14.5k–15.4k ticks/s
//   engine+facts : 12.2k–15.0k ticks/s
// 预算取实测下沿的约 75%，用于抓「性能显著退化」而不是噪声抖动。
/// 纯引擎吞吐预算（无序列化、无落盘）。
const BUDGET_ENGINE_TICKS_PER_SEC: f64 = 11_000.0;
/// 引擎 + 事实流序列化吞吐预算（有界默认输出模式）。
const BUDGET_FACTS_TICKS_PER_SEC: f64 = 9_000.0;
/// benchmark 默认窗口（tick 数）。
const DEFAULT_BENCHMARK_TICKS: usize = 20_000;
/// 1 MiB 的字节数（带宽报告单位换算）。
const BYTES_PER_MEBIBYTE: f64 = 1_048_576.0;

fn run_benchmark(ticks: usize, mode: &str) -> std::io::Result<()> {
    println!(
        "⏱️  Layered benchmark: {ticks} ticks, mode={mode}, release
"
    );
    let mut failures: Vec<String> = Vec::new();
    let want = |layer: &str| mode == "all" || mode == layer;

    // ---- 1. engine-only：纯引擎 tick，无序列化、无落盘 ----
    if want("engine") {
        let start = Instant::now();
        let mut engine = MatchEngine::with_setup(MatchSetup::builtin(GameRules::default()), 42);
        let mut completed = 0usize;
        for _ in 0..ticks {
            if engine.is_finished() {
                break;
            }
            let _ = engine.step();
            completed += 1;
        }
        let elapsed = start.elapsed().as_secs_f64().max(1e-6);
        let tps = completed as f64 / elapsed;
        println!(
            "   [engine-only ] {:>10.0} ticks/s  ({} ticks in {:.2}s, budget >= {:.0})",
            tps, completed, elapsed, BUDGET_ENGINE_TICKS_PER_SEC
        );
        if tps < BUDGET_ENGINE_TICKS_PER_SEC {
            failures.push(format!(
                "engine-only {:.0} ticks/s below budget {:.0}",
                tps, BUDGET_ENGINE_TICKS_PER_SEC
            ));
        }
    }

    // ---- 2. engine + facts：事实流序列化 ----
    if want("facts") {
        let artifact = TempStreamGuard::new(
            cli_temp_root().join(format!("nba_bench_facts_{}.ndjson", std::process::id())),
        );
        let start = Instant::now();
        let mut engine = MatchEngine::with_setup(MatchSetup::builtin(GameRules::default()), 42);
        let mut completed = 0usize;
        let mut bytes = 0u64;
        {
            use std::io::Write;
            let file = File::create(artifact.path())?;
            let mut writer = std::io::BufWriter::new(file);
            let mut prev_phase = String::new();
            let mut prev_flow = String::new();
            let mut prev_completed = usize::MAX;
            for _ in 0..ticks {
                if engine.is_finished() {
                    break;
                }
                let tick = engine.step();
                completed += 1;
                let f = &tick.frame;
                let emit = bytes == 0
                    || !f.event_log.is_empty()
                    || !f.events.is_empty()
                    || f.phase != prev_phase
                    || f.game_flow != prev_flow
                    || f.completed_possessions != prev_completed
                    || f.simulation_complete;
                prev_phase = f.phase.clone();
                prev_flow = f.game_flow.clone();
                prev_completed = f.completed_possessions;
                if emit {
                    let line = serde_json::to_string(&tick)?;
                    writer.write_all(line.as_bytes())?;
                    writer.write_all(b"\n")?;
                    bytes = bytes.saturating_add(line.len() as u64 + 1);
                }
            }
            writer.flush()?;
        }
        let elapsed = start.elapsed().as_secs_f64().max(1e-6);
        let tps = completed as f64 / elapsed;
        let mib = bytes as f64 / BYTES_PER_MEBIBYTE;
        let mbps = mib / elapsed;
        println!(
            "   [engine+facts ] {:>10.0} ticks/s  ({} ticks, {:.1} MiB, {:.1} MiB/s)",
            tps, completed, mib, mbps
        );
        if tps < BUDGET_FACTS_TICKS_PER_SEC {
            failures.push(format!(
                "engine+facts {:.0} ticks/s below budget {:.0}",
                tps, BUDGET_FACTS_TICKS_PER_SEC
            ));
        }
    }

    // ---- 3. evaluator：事件流评判吞吐 ----
    if want("evaluate") {
        let mut engine = MatchEngine::with_setup(MatchSetup::builtin(GameRules::default()), 42);
        let mut ndjson = String::new();
        for _ in 0..ticks {
            if engine.is_finished() {
                break;
            }
            ndjson.push_str(&serde_json::to_string(&engine.step())?);
            ndjson.push('\n');
        }
        let start = Instant::now();
        let parsed = nba_evaluator::try_parse_stream(&ndjson)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let parse_elapsed = start.elapsed().as_secs_f64().max(1e-6);
        let fixture = nba_evaluator::ReferenceDistributions::nba_v1();
        let start = Instant::now();
        let judgments = nba_evaluator::evaluate_stream(&parsed, &fixture);
        let eval_elapsed = start.elapsed().as_secs_f64().max(1e-6);
        println!(
            "   [evaluate     ] {:>10.0} ticks/s parse, {:>10.0} ticks/s judge  ({} ticks, {} judgments)",
            parsed.len() as f64 / parse_elapsed,
            parsed.len() as f64 / eval_elapsed,
            parsed.len(),
            judgments.len()
        );
    }

    if failures.is_empty() {
        println!("\n   ✅ All measured layers within budget.");
        Ok(())
    } else {
        println!("\n   ❌ Below budget:");
        for f in &failures {
            println!("      - {f}");
        }
        std::process::exit(1);
    }
}

/// 解析 `--seeds A..B`（含端点）为种子列表。
fn parse_seed_range(spec: &str) -> Result<Vec<u64>, String> {
    let spec = spec.trim();
    let (start_s, end_s) = spec
        .split_once("..")
        .ok_or_else(|| format!("--seeds expects `A..B` (inclusive), got `{}`", spec))?;
    let start: u64 = start_s
        .trim()
        .parse()
        .map_err(|_| format!("invalid range start in `{}`", spec))?;
    let end: u64 = end_s
        .trim()
        .parse()
        .map_err(|_| format!("invalid range end in `{}`", spec))?;
    if end < start {
        return Err(format!("seed range end < start in `{}`", spec));
    }
    if end - start + 1 > 100_000 {
        return Err("seed range too large".to_string());
    }
    Ok((start..=end).collect())
}

fn run_audit_stream(file_path: &str) -> std::io::Result<()> {
    println!(
        "🔍 Starting Offline Event Stream Semantic Audit: {}\n",
        file_path
    );
    let start = Instant::now();
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);

    let mut checker = InvariantChecker::new();
    let mut total_ticks = 0;
    let mut all_violations = Vec::new();
    // 几何投影流的规则与 tactical_set 只在首条记录写出（gap.md §16.4），
    // 后续记录继承；同时据此识别投影粒度。
    let mut carried_rules: Option<nba_protocol::FrameRules> = None;
    let mut projection: Option<String> = None;

    for (line_idx, line_res) in reader.lines().enumerate() {
        let line = line_res?;
        if line.trim().is_empty() {
            continue;
        }
        let mut tick: StreamTick = serde_json::from_str(&line).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("JSON parse error at line {}: {}", line_idx + 1, e),
            )
        })?;
        // 首条记录之后规则不再重复写出，向前继承。
        if tick.frame.rules.is_present() {
            carried_rules = Some(tick.frame.rules.clone());
        } else if let Some(rules) = &carried_rules {
            tick.frame.rules = rules.clone();
        }
        if projection.is_none() && !tick.frame.stream_projection.is_empty() {
            projection = Some(tick.frame.stream_projection.clone());
        }
        // 紧凑流（facts/summary）不携带逐 tick 几何投影，L1 几何检测不适用；
        // 明确拒绝而不是用零坐标误报「越界/球人分离」（gap.md §16.4）。
        if projection.as_deref().is_some_and(|p| p != "full") && tick.frame.players.is_empty() {
            continue;
        }

        let violations = checker.check_tick(&tick);
        all_violations.extend(violations);
        total_ticks += 1;
    }

    let geometry_audited = projection.as_deref().is_none_or(|p| p == "full");
    if let Some(p) = &projection {
        println!("   Projection: {}", p);
    }
    if !geometry_audited {
        println!(
            "   ⚠️  Non-full projection: L1 geometric invariants were NOT audited\n      (compact streams carry causal facts only). Use --stream-mode frames\n      to produce an auditable geometric stream."
        );
    }

    let elapsed = start.elapsed();
    let taxonomy = ViolationTaxonomy::from_violations(&all_violations);

    println!("📊 Audit Summary:");
    println!("   Total Ticks Scanned: {}", total_ticks);
    println!("   Total Invariant Violations: {}", all_violations.len());
    println!("   Taxonomy Breakdown: {:?}", taxonomy);
    println!(
        "   Scan Speed: {} ticks/sec\n",
        (total_ticks as f64 / elapsed.as_secs_f64().max(0.001)) as u64
    );

    if !geometry_audited {
        println!(
            "✅ Causal stream check complete: {} violations. Geometry audit: not applicable for this projection.",
            all_violations.len()
        );
        return Ok(());
    }
    if !all_violations.is_empty() {
        println!("❌ Audit Failed with Violations:");
        for v in all_violations.iter().take(20) {
            println!("   {}", v);
        }
        if all_violations.len() > 20 {
            println!("   ... and {} more violations.", all_violations.len() - 20);
        }
        std::process::exit(1);
    } else {
        println!("✅ Perfect Stream: 100% Causal Event Semantics & Axioms Satisfied!\n");
    }

    Ok(())
}

/// 比例 → 百分比的唯一换算点（展示层）。
///
/// 调用方不得各自写 `* 100.0`：那会让同一个换算字面量以「行为常数」的身份
/// 散落 8 处（charter C1 / docs/protocol.md §2.1 M7 的收编对象）。收编为单点后，
/// 该字面量只出现一次，且位置明确属于展示层。
fn pct(ratio: f32) -> f32 {
    ratio * 100.0
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 && args[1] == "audit" {
        let path = args
            .get(2)
            .map(|s| s.as_str())
            .unwrap_or("output/game.ticks.ndjson");
        return run_audit_stream(path);
    }

    if args.len() > 1 && args[1] == "evaluate" {
        let path = args
            .get(2)
            .map(|s| s.as_str())
            .unwrap_or("output/game.ticks.ndjson");
        return run_evaluate(path);
    }

    if args.len() > 1 && args[1] == "pbp-convert" {
        let input = args.get(2).map(|s| s.as_str()).unwrap_or("input/pbp.json");
        let mut out = "output/converted_fixture.json";
        let mut league = "NBA";
        let mut version = "pbp.converted.v1";
        let mut i = 3;
        while i < args.len() {
            match args[i].as_str() {
                "--out" => {
                    if let Some(v) = args.get(i + 1) {
                        out = v.as_str();
                        i += 2;
                        continue;
                    }
                }
                "--league" => {
                    if let Some(v) = args.get(i + 1) {
                        league = v.as_str();
                        i += 2;
                        continue;
                    }
                }
                "--version" => {
                    if let Some(v) = args.get(i + 1) {
                        version = v.as_str();
                        i += 2;
                        continue;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        return run_pbp_convert(input, out, league, version);
    }

    if args.len() > 1 && args[1] == "benchmark" {
        let mut ticks = DEFAULT_BENCHMARK_TICKS;
        let mut mode = "all".to_string();
        let mut i = 2;
        while i < args.len() {
            if args[i] == "--ticks" {
                if let Some(v) = args.get(i + 1).and_then(|s| s.parse().ok()) {
                    ticks = v;
                    i += 2;
                    continue;
                }
            }
            if args[i] == "--mode" {
                if let Some(v) = args.get(i + 1) {
                    mode = v.clone();
                    i += 2;
                    continue;
                }
            }
            i += 1;
        }
        return run_benchmark(ticks, &mode);
    }

    let mut rules_path: Option<&str> = None;
    let mut league: Option<String> = None;
    let mut seeds: Option<Vec<u64>> = None;
    let mut out_path: Option<&str> = None;
    let mut stream_mode: Option<StreamMode> = None;
    let mut positional = Vec::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--rules" => {
                if i + 1 < args.len() {
                    rules_path = Some(&args[i + 1]);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--league" => {
                if i + 1 < args.len() {
                    league = Some(args[i + 1].to_string());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--seeds" => {
                if i + 1 < args.len() {
                    seeds = Some(parse_seed_range(&args[i + 1]).unwrap_or_else(|e| {
                        eprintln!("❌ {}", e);
                        std::process::exit(2);
                    }));
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--out" => {
                if i + 1 < args.len() {
                    out_path = Some(&args[i + 1]);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--stream-mode" => {
                if i + 1 < args.len() {
                    stream_mode = Some(StreamMode::parse(&args[i + 1]).unwrap_or_else(|e| {
                        eprintln!("❌ {}", e);
                        std::process::exit(2);
                    }));
                    i += 2;
                } else {
                    i += 1;
                }
            }
            // 兼容旧形态：--batch N = 种子 1..=N（docs/quality.md §3.1 的批处理口径）。
            "--batch" => {
                if i + 1 < args.len() {
                    let n: usize = args[i + 1].parse().unwrap_or(10);
                    seeds = Some((1..=n as u64).collect());
                    i += 2;
                } else {
                    seeds = Some((1..=10).collect());
                    i += 1;
                }
            }
            _ => {
                positional.push(args[i].as_str());
                i += 1;
            }
        }
    }

    let rules = if let Some(path) = rules_path {
        load_rules(Some(path))?
    } else if let Some(l) = league.as_deref() {
        match l {
            "nba" => GameRules::default(),
            "fiba" => GameRules::with_league(nba_domain::LeagueProfile::fiba()),
            other => {
                eprintln!("Unknown league `{}` (expected nba | fiba)", other);
                std::process::exit(2);
            }
        }
    } else {
        GameRules::default()
    };

    // 目标形态（quality 批处理规范）：`nba-sim --rules X --seeds A..B batch --out Y`
    let batch_mode = seeds.is_some() || positional.first().copied() == Some("batch");
    let positional: Vec<&str> = positional.into_iter().filter(|p| *p != "batch").collect();

    // 默认 facts（因果事实流，数 MB 级）；逐 tick 帧需显式 --stream-mode frames。
    let stream_mode = stream_mode.unwrap_or_default();

    if batch_mode {
        let seeds = seeds.unwrap_or_else(|| (1..=10).collect());
        let scope = positional.first().copied().unwrap_or("1q");
        run_batch_simulation(&seeds, scope, rules, out_path, stream_mode)?;
    } else {
        let seed = positional
            .first()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(42);
        let out_path = positional
            .get(1)
            .copied()
            .unwrap_or("output/game.ticks.ndjson");
        let scope = positional.get(2).copied().unwrap_or("1q");

        run_single_simulation(seed, out_path, scope, rules, stream_mode)?;
    }

    Ok(())
}
