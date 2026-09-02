use nba_domain::GameRules;
use nba_engine::{MatchEngine, MatchSetup};
use nba_invariants::{InvariantChecker, ViolationTaxonomy};
use nba_protocol::StreamTick;
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::time::Instant;

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

/// 评判工件落盘（M8：judgments.ndjson + attribution_report.json）。
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
            let _ = writeln!(w, "{}", serde_json::to_string(j).unwrap_or_default());
        }
    }
    fs::write(&report_path, serde_json::to_string_pretty(&report).unwrap_or_default())?;
    Ok(report)
}

fn run_single_simulation(
    seed: u64,
    out_path: &str,
    scope: &str,
    rules: GameRules,
) -> std::io::Result<()> {
    println!("🏀 Initializing NBA-Sim Rust Engine with Rapier2D Physics...");
    println!("   Seed: {}, Output: {}, Scope: {}", seed, out_path, scope);

    let start = Instant::now();
    let setup = MatchSetup::builtin(rules.clone());
    let mut engine = MatchEngine::with_setup(setup, seed);

    let summary = engine.simulate_scope_and_export(scope, out_path)?;
    let elapsed = start.elapsed();

    let total_points = engine.home_score + engine.away_score;
    let completed_poss = engine.completed_possessions;
    let avg_poss_sec = if completed_poss > 0 {
        engine.current_time / completed_poss as f32
    } else {
        0.0
    };

    println!("\n📊 Box Score & Advanced Statistics:");
    println!(
        "   Score: Home {} - {} Away (Total: {})",
        engine.home_score, engine.away_score, total_points
    );
    println!(
        "   Possessions: {} | Avg Duration: {:.2}s",
        completed_poss, avg_poss_sec
    );
    println!(
        "   2PT FG: {}/{} ({:.1}%)",
        summary.box_score.fg2_made,
        summary.box_score.fg2_attempts,
        summary.box_score.fg2_pct() * 100.0
    );
    println!(
        "   3PT FG: {}/{} ({:.1}%)",
        summary.box_score.fg3_made,
        summary.box_score.fg3_attempts,
        summary.box_score.fg3_pct() * 100.0
    );
    println!(
        "   FT:     {}/{} ({:.1}%)",
        summary.box_score.ft_made,
        summary.box_score.ft_attempts,
        summary.box_score.ft_pct() * 100.0
    );
    println!(
        "   Turnovers: {} | Fouls: {}",
        summary.box_score.turnovers, summary.box_score.fouls
    );

    let violation_count = summary.violations.len();
    if violation_count > 0 {
        println!(
            "\n❌ Simulation failed: {} Axiom Violations detected across {} ticks.",
            violation_count, summary.ticks
        );
        println!("   Taxonomy Breakdown: {:?}", summary.taxonomy);
        let violation_file = format!("{}.violations.ndjson", out_path);
        if let Ok(file) = File::create(&violation_file) {
            let mut writer = std::io::BufWriter::new(file);
            use std::io::Write;
            for v in &summary.violations {
                let _ = writeln!(writer, "{}", serde_json::to_string(v).unwrap_or_default());
            }
            println!("   📁 Exported structured violation ledger to: {}", violation_file);
        }
        for v in &summary.violations {
            println!("   {}", v);
        }
        std::process::exit(1);
    } else {
        let tps = (summary.ticks as f64 / elapsed.as_secs_f64().max(0.001)) as u64;
        println!(
            "\n✅ Completed {} ({} ticks) in {:.2}s ({} ticks/sec): 0 Axiom Violations.\n",
            summary.scope_desc, summary.ticks, elapsed.as_secs_f64(), tps
        );
    }

    // M8 评判工件：judgments.ndjson + attribution_report.json 与流同落盘。
    if let Ok(stream) = fs::read_to_string(out_path) {
        let ticks = nba_evaluator::parse_stream(&stream);
        let fixture = nba_evaluator::ReferenceDistributions::for_league(&rules.league.name);
        let judgments = nba_evaluator::evaluate_stream(&ticks, &fixture)
            .into_iter()
            .map(|mut j| {
                j.seed = Some(seed);
                j
            })
            .collect::<Vec<_>>();
        match write_judgment_artifacts(out_path, &judgments, &fixture) {
            Ok(report) => println!(
                "🧾 Realism index: {:.3} ({} judgments, {} defects) → {}.judgments.ndjson",
                report.realism_index, report.total_judgments, report.defect_count, out_path
            ),
            Err(e) => eprintln!("⚠️ judgment artifacts failed: {}", e),
        }
    }

    Ok(())
}

fn run_batch_simulation(
    seeds: &[u64],
    scope: &str,
    rules: GameRules,
    out_path: Option<&str>,
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

        let out_path = format!("/tmp/nba_batch_{}.ndjson", seed);
        let summary = engine.simulate_scope_and_export(scope, &out_path)?;

        let pts = engine.home_score + engine.away_score;
        let poss = engine.completed_possessions;
        let dur = if poss > 0 { engine.current_time / poss as f32 } else { 0.0 };
        let game_violations = summary.violations.len();
        total_violations += game_violations;

        // 单场违规工件：仅在存在违反时落盘（与单场模式同名约定）。
        if game_violations > 0 {
            let violation_file = format!("{}.violations.ndjson", out_path);
            if let Ok(file) = File::create(&violation_file) {
                let mut writer = std::io::BufWriter::new(file);
                use std::io::Write;
                for v in &summary.violations {
                    let _ = writeln!(writer, "{}", serde_json::to_string(v).unwrap_or_default());
                }
            }
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
            let _ = writeln!(writer, "{}", line);
        }

        println!(
            "   [Game {:02}/{:02}] Seed={:<3} Total={:<3} Poss={:<3} Dur={:.2}s 3P={:.1}% Violations={}",
            i + 1,
            seeds.len(),
            seed, pts, poss, dur, summary.box_score.fg3_pct() * 100.0, game_violations
        );

        total_points_list.push(pts as f32);
        avg_dur_list.push(dur);
        fg3_pct_list.push(summary.box_score.fg3_pct() * 100.0);

        // M8 评判：逐场评判并聚合（batch 与 violations 同落盘）。
        if let Ok(stream) = fs::read_to_string(&out_path) {
            let ticks = nba_evaluator::parse_stream(&stream);
            let fixture = nba_evaluator::ReferenceDistributions::for_league(&rules.league.name);
            all_judgments.extend(
                nba_evaluator::evaluate_stream(&ticks, &fixture)
                    .into_iter()
                    .map(|mut j| {
                        j.seed = Some(seed);
                        j
                    }),
            );
        }
    }

    if let Some(writer) = stats_out.as_mut() {
        use std::io::Write;
        writer.flush()?;
    }

    let fixture = nba_evaluator::ReferenceDistributions::for_league(&rules.league.name);
    let report = nba_evaluator::attribution_report(&all_judgments, &fixture.version);
    if !all_judgments.is_empty() {
        let base = out_path.unwrap_or("/tmp/nba_batch_aggregate");
        let judgments_path = format!("{}.judgments.ndjson", base);
        if let Ok(jf) = File::create(&judgments_path) {
            let mut w = std::io::BufWriter::new(jf);
            use std::io::Write;
            for j in &all_judgments {
                let _ = writeln!(w, "{}", serde_json::to_string(j).unwrap_or_default());
            }
            let _ = w.flush();
        }
        let _ = fs::write(
            format!("{}.attribution_report.json", base),
            serde_json::to_string_pretty(&report).unwrap_or_default(),
        );
    }

    total_points_list.sort_by(|a, b| a.partial_cmp(b).unwrap());
    avg_dur_list.sort_by(|a, b| a.partial_cmp(b).unwrap());
    fg3_pct_list.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let median_pts = total_points_list[total_points_list.len() / 2];
    let median_dur = avg_dur_list[avg_dur_list.len() / 2];
    let median_3p = fg3_pct_list[fg3_pct_list.len() / 2];

    println!("\n📈 Batch Summary (N={}):", seeds.len());
    println!("   Median Total Points: {:.1} (Min: {:.1}, Max: {:.1})", median_pts, total_points_list[0], total_points_list[total_points_list.len() - 1]);
    println!("   Median Poss Duration: {:.2}s", median_dur);
    println!("   Median 3P Accuracy: {:.1}%", median_3p);
    println!("   Total Axiom Violations: {}", total_violations);
    println!(
        "   Realism Index: {:.3} ({} judgments, {} defects) — fixture {}",
        report.realism_index, report.total_judgments, report.defect_count, report.fixture_version
    );
    println!("   Elapsed Wall Time: {:.2}s\n", start.elapsed().as_secs_f64());

    if total_violations > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// 离线评判（quality 评判规范）：对既有 ndjson 流产出评判工件。
fn run_evaluate(stream_path: &str) -> std::io::Result<()> {
    let stream = fs::read_to_string(stream_path)?;
    let ticks = nba_evaluator::parse_stream(&stream);
    let fixture = nba_evaluator::ReferenceDistributions::nba_v1();
    let judgments = nba_evaluator::evaluate_stream(&ticks, &fixture);
    let report = write_judgment_artifacts(stream_path, &judgments, &fixture)?;
    println!("🧾 Evaluated {} ticks: {} judgments, {} defects",
        ticks.len(), report.total_judgments, report.defect_count);
    for row in report.defects_by_criterion.iter().take(5) {
        println!("   [{:>3}x] {} ({})", row.count, row.criterion, row.attribution);
    }
    println!("   Realism index: {:.3}", report.realism_index);
    Ok(())
}

/// 转换外部 PBP 数据为 ReferenceDistributions fixture（M8 扩展位）。
fn run_pbp_convert(input_path: &str, out_path: &str, league: &str, version: &str) -> std::io::Result<()> {
    let content = fs::read_to_string(input_path)?;
    let events: Vec<nba_evaluator::PbpEvent> = if content.trim_start().starts_with('[') {
        serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?
    } else {
        content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)))
            .collect::<Result<Vec<_>, _>>()?
    };

    let fixture = nba_evaluator::convert_pbp_events_to_fixture(&events, version, league);
    let serialized = serde_json::to_string_pretty(&fixture)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    fs::write(out_path, serialized)?;
    println!("📊 Converted {} PBP events → {} (version: {}, league: {})", events.len(), out_path, version, league);
    Ok(())
}
/// release 模式下跑固定 tick 数，对照 ≥ 20,000 ticks/s 预算。
fn run_benchmark(ticks: usize) -> std::io::Result<()> {
    const BUDGET_TICKS_PER_SEC: f64 = 20_000.0;
    println!("⏱️  Benchmark: {} ticks, release mode\n", ticks);

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

    println!("   Completed {} ticks in {:.2}s", completed, elapsed);
    println!("   Throughput: {:.0} ticks/sec (budget ≥ {:.0})", tps, BUDGET_TICKS_PER_SEC);
    if tps >= BUDGET_TICKS_PER_SEC {
        println!("   ✅ Within performance budget.");
    } else {
        println!(
            "   ❌ Below performance budget: {:.0}% of target.",
            tps / BUDGET_TICKS_PER_SEC * 100.0
        );
        std::process::exit(1);
    }
    Ok(())
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
    println!("🔍 Starting Offline Event Stream Semantic Audit: {}\n", file_path);
    let start = Instant::now();
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);

    let mut checker = InvariantChecker::new();
    let mut total_ticks = 0;
    let mut all_violations = Vec::new();

    for (line_idx, line_res) in reader.lines().enumerate() {
        let line = line_res?;
        if line.trim().is_empty() {
            continue;
        }
        let tick: StreamTick = serde_json::from_str(&line).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("JSON parse error at line {}: {}", line_idx + 1, e),
            )
        })?;

        let violations = checker.check_tick(&tick);
        all_violations.extend(violations);
        total_ticks += 1;
    }

    let elapsed = start.elapsed();
    let taxonomy = ViolationTaxonomy::from_violations(&all_violations);

    println!("📊 Audit Summary:");
    println!("   Total Ticks Scanned: {}", total_ticks);
    println!("   Total Invariant Violations: {}", all_violations.len());
    println!("   Taxonomy Breakdown: {:?}", taxonomy);
    println!("   Scan Speed: {} ticks/sec\n", (total_ticks as f64 / elapsed.as_secs_f64().max(0.001)) as u64);

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

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 && args[1] == "audit" {
        let path = args.get(2).map(|s| s.as_str()).unwrap_or("output/game.ticks.ndjson");
        return run_audit_stream(path);
    }

    if args.len() > 1 && args[1] == "evaluate" {
        let path = args.get(2).map(|s| s.as_str()).unwrap_or("output/game.ticks.ndjson");
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
        let mut ticks = 20_000usize;
        let mut i = 2;
        while i < args.len() {
            if args[i] == "--ticks" {
                if let Some(v) = args.get(i + 1).and_then(|s| s.parse().ok()) {
                    ticks = v;
                    i += 2;
                    continue;
                }
            }
            i += 1;
        }
        return run_benchmark(ticks);
    }

    let mut rules_path: Option<&str> = None;
    let mut league: Option<String> = None;
    let mut seeds: Option<Vec<u64>> = None;
    let mut out_path: Option<&str> = None;
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
            // 兼容旧形态：--batch N = 种子 1..=N（design.md §2 前置口径）。
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
    let batch_mode =
        seeds.is_some() || positional.first().copied() == Some("batch");
    let positional: Vec<&str> = positional
        .into_iter()
        .filter(|p| *p != "batch")
        .collect();

    if batch_mode {
        let seeds = seeds.unwrap_or_else(|| (1..=10).collect());
        let scope = positional.first().copied().unwrap_or("1q");
        run_batch_simulation(&seeds, scope, rules, out_path)?;
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

        run_single_simulation(seed, out_path, scope, rules)?;
    }

    Ok(())
}
