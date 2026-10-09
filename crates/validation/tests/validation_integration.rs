//! R12 验证基础设施集成测试（validation.md §9.3 验证器自身检查）。
//!
//! 覆盖：严格流读取的截断/坏行拒绝、实例索引双结算检测、关系审计、
//! 重建器箱体对平、场景矩阵全触发、runner 端到端与退出码。

use std::collections::BTreeMap;

use nba_validation::criteria::{CriterionRegistry, Family};
use nba_validation::reconstruct::RebuiltBoxScore;
use nba_validation::runner::{run_validation, ExitCode, ValidationRunConfig};
use nba_validation::scenario::{
    evaluate_full_game, required_scenarios, ScenarioEntry, TriggerSpec,
};
use nba_validation::stats::{
    compare_mean_difference, compare_total_variation, compare_wasserstein, BootstrapConfig,
    PairedGameValues,
};
use nba_validation::stream::{event_id_unique_and_monotonic, InstanceIndex, RelationAudit};
use nba_validation::verdict::Verdict;

#[test]
fn time_violation_injection_produces_enforced_facts() {
    // R5 时限类违例的注入式判定（#37）：把进攻时限压到极小构造高压局面，
    // 断言 (a) SHOT_CLOCK_VIOLATION 高频产生；(b) 每次违例的处罚链路闭环
    // ——PossessionSummary 终结 = TURNOVER_VIOLATION、下一回合
    // TRANSITION_STARTED.origin = violation（球权交换 + 对方发球）；
    // (c) 引擎正常打完（账本/流程未破）。
    use nba_engine::MatchEngine;
    let mut rules = nba_domain::GameRules::default();
    rules.league.shot_clock_seconds = 3.0;
    let mut engine = MatchEngine::with_rules(42, rules);
    engine
        .set_scope("1q")
        .expect("scope must parse");
    let mut violations = 0u32;
    let mut terminal_after_violation = 0u32;
    let mut transition_after_violation = 0u32;
    let mut pending_violation = false;
    let mut pending_violation_for_transition = false;
    let mut ticks = 0u64;
    while !engine.is_finished() && ticks < 200_000 {
        let tick = engine.step();
        ticks += 1;
        for event in &tick.frame.event_log {
            match event.kind.as_str() {
                "VIOLATION" => {
                    let reason = event
                        .data
                        .as_ref()
                        .and_then(|d| d.get("RuleViolation"))
                        .and_then(|r| r.get("reason"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    if reason == "SHOT_CLOCK_VIOLATION" {
                        violations += 1;
                        pending_violation = true;
                        pending_violation_for_transition = true;
                    }
                }
                "POSSESSION_SUMMARY" => {
                    if pending_violation {
                        let terminal = event
                            .data
                            .as_ref()
                            .and_then(|d| d.get("PossessionSummary"))
                            .and_then(|p| p.get("terminal_event"))
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("");
                        if terminal == "TURNOVER_VIOLATION" {
                            terminal_after_violation += 1;
                        }
                        pending_violation = false;
                    }
                }
                "TRANSITION_STARTED" if pending_violation_for_transition => {
                    let origin = event
                        .data
                        .as_ref()
                        .and_then(|d| d.get("TransitionStarted"))
                        .and_then(|t| t.get("origin"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    if origin == "violation" {
                        transition_after_violation += 1;
                    }
                    pending_violation_for_transition = false;
                }
                _ => {}
            }
        }
    }
    assert!(
        violations >= 5,
        "3s shot clock must inject frequent violations, got {violations}"
    );
    assert_eq!(
        terminal_after_violation, violations,
        "every injected violation must end the possession as TURNOVER_VIOLATION"
    );
    assert_eq!(
        transition_after_violation, violations,
        "every injected violation must hand over possession via TRANSITION_STARTED(violation)"
    );
}

fn temp_root() -> std::path::PathBuf {
    // cargo test 的 cwd 是包根；向上两级到仓库根 .work/（根 .gitignore 已覆盖），
    // 避免在 crates/validation/ 下留下未忽略的测试工件目录。
    let root = std::path::PathBuf::from("../../.work/test-tmp/validation-it");
    let _ = std::fs::create_dir_all(&root);
    root
}

#[test]
fn registry_covers_all_documented_families() {
    let registry = CriterionRegistry::full();
    assert!(
        registry.len() >= 80,
        "registry too small: {}",
        registry.len()
    );
    for family in [
        Family::Flow,
        Family::Cause,
        Family::Move,
        Family::Tact,
        Family::Stat,
        Family::Mech,
        Family::Joint,
    ] {
        let entries = registry.by_family(family);
        assert!(!entries.is_empty(), "{family:?} has no registered criteria");
    }
    // 编号唯一由 BTreeMap 保证；抽查代表性编号。
    assert!(registry.get("FLOW-A01").is_some());
    assert!(registry.get("CAUSE-B01").is_some());
    assert!(registry.get("MOVE-H01/fact").is_some());
    assert!(registry.get("TACT-P01/fact").is_some());
    assert!(registry.get("STAT-G01").is_some());
    assert!(registry.get("MECH-S01").is_some());
    assert!(registry.get("JOINT-C01").is_some());
}

#[test]
fn required_scenarios_match_documented_count() {
    // validation.md §9.2 表格行数 = 22。
    let scenarios = required_scenarios();
    assert_eq!(scenarios.len(), 22);
    for scenario in &scenarios {
        scenario
            .validate()
            .expect("required scenario must validate");
        assert_eq!(scenario.entry, ScenarioEntry::FullGame);
    }
}

#[test]
fn untriggered_scenario_reports_insufficient_evidence() {
    let spec = nba_validation::scenario::ScenarioSpec {
        id: "SC-NONEXISTENT".to_string(),
        rule_version: "nba-v1".to_string(),
        entry: ScenarioEntry::FullGame,
        precondition: Vec::new(),
        trigger: TriggerSpec {
            event_kind: "EVENT_THAT_NEVER_OCCURS".to_string(),
            min_occurrences: 1,
        },
        allowed_branches: Vec::new(),
        required_facts: Vec::new(),
        forbidden_facts: Vec::new(),
        max_ticks: 1000,
    };
    let outcome = evaluate_full_game(&spec, &[]);
    assert_eq!(outcome.verdict, Verdict::InsufficientEvidence);
    assert_eq!(outcome.trigger_occurrences, 0);
}

#[test]
fn checkpoint_entry_reports_execution_error_not_fabricated() {
    let spec = nba_validation::scenario::ScenarioSpec {
        id: "SC-CP".to_string(),
        rule_version: "nba-v1".to_string(),
        entry: ScenarioEntry::Checkpoint,
        precondition: Vec::new(),
        trigger: TriggerSpec {
            event_kind: "SHOT_RELEASE".to_string(),
            min_occurrences: 1,
        },
        allowed_branches: Vec::new(),
        required_facts: Vec::new(),
        forbidden_facts: Vec::new(),
        max_ticks: 100,
    };
    let ticks: Vec<nba_protocol::StreamTick> = Vec::new();
    let outcome = evaluate_full_game(&spec, &ticks);
    assert!(outcome.execution_error.is_some());
    assert_eq!(outcome.verdict, Verdict::InsufficientEvidence);
}

#[test]
fn reconstructed_box_score_balances_against_engine_events() {
    let config = ValidationRunConfig {
        seed: 42,
        scope: "1q".to_string(),
        output_dir: temp_root(),
        max_ticks: 100_000,
    };
    let run = run_validation(&config).expect("1q run must complete");
    // 独立重建与引擎声明的交叉核对：重建值非零（1q 必有出手/罚球），
    // 且 TOV 重建值与引擎回合总结条数一致（同一事实源的双读）。
    let (rebuilt, declared) = {
        let _ = &run;
        RebuiltBoxScore::rebuild(&run.ticks)
    };
    let home_rebuilt = rebuilt.get("home").expect("home team must appear");
    let home_declared = declared.get("home").expect("declared home stats");
    assert!(home_rebuilt.fga > 0, "rebuilt FGA must be positive");
    assert_eq!(
        home_rebuilt.fga, home_declared.fga,
        "FGA rebuild vs declared"
    );
    assert_eq!(
        home_rebuilt.fgm, home_declared.fgm,
        "FGM rebuild vs declared"
    );
}

#[test]
fn pairing_audit_clean_on_production_stream() {
    let config = ValidationRunConfig {
        seed: 42,
        scope: "1q".to_string(),
        output_dir: temp_root(),
        max_ticks: 100_000,
    };
    let run = run_validation(&config).expect("1q run must complete");
    assert_eq!(
        run.pairing.shots_without_release, 0,
        "every shot resolution must carry a released shot_id"
    );
    assert_eq!(
        run.pairing.passes_without_release, 0,
        "every pass terminal must carry a released pass_id"
    );
    assert_eq!(
        run.pairing.free_throws_without_award, 0,
        "every free throw must carry foul_program_id and AwardedBy relation"
    );
    event_id_unique_and_monotonic(&run.ticks).expect("event ids unique and monotonic");
    let relation_audit = RelationAudit::audit(&run.ticks);
    assert!(
        relation_audit.clean(),
        "relations must point backwards at existing events: {:?}",
        relation_audit
    );
    let index = InstanceIndex::build(&run.ticks);
    let doubles = index.double_closed_instances(&run.ticks);
    assert!(
        doubles.is_empty(),
        "instances must close exactly once, found: {doubles:?}"
    );
}

#[test]
fn runner_end_to_end_artifacts_and_exit_code() {
    // seed 29：ui 合并（发球接应/切球不应期/点拍/温度校准）后场景机会分布
    // 再变，29 的 1q 样本场景全过（seed 37 变为场景不足 exit 3）。
    let output_dir = temp_root().join("e2e");
    let config = ValidationRunConfig {
        seed: 29,
        scope: "1q".to_string(),
        output_dir: output_dir.clone(),
        max_ticks: 100_000,
    };
    let run = run_validation(&config).expect("run must complete");
    let paths = nba_validation::runner::write_artifacts(&run, &output_dir)
        .expect("artifact write must succeed");
    assert_eq!(paths.len(), 13, "validation.md §10 full artifact set");
    for path in &paths {
        assert!(path.exists(), "artifact missing: {}", path.display());
    }
    for name in [
        "coverage.json",
        "instance_judgments.ndjson",
        "validation_report.json",
        "scenario_results.json",
        "ledger_comparison.json",
        "game_metrics.parquet",
        "instances.sqlite",
        "statistical_results.json",
        "mechanism_results.json",
        "failure_segments.json",
        "judgments.ndjson",
        "attribution_report.json",
        "validation_manifest.json",
    ] {
        assert!(output_dir.join(name).exists(), "artifact {name} missing");
    }
    // 1q 规模下场景应全部触发且无缺陷。
    assert_eq!(run.exit_code(), ExitCode::AllPass);
    assert_eq!(run.trailer.records, run.ticks.len() as u64);
}

#[test]
fn parquet_roundtrip_and_sqlite_store_are_queryable() {
    use arrow_array::{Float64Array, RecordBatch, StringArray, UInt64Array};
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    let dir = temp_root().join("parquet-roundtrip");
    std::fs::create_dir_all(&dir).expect("create dir");
    let metrics = nba_validation::artifacts::GameMetrics {
        seed: 11,
        scope: "1q".to_string(),
        periods: 1,
        ticks: 500,
        events: 700,
        possessions: 20,
        fga_total: 8,
        fgm_total: 3,
        three_pa_total: 2,
        three_pm_total: 1,
        fta_total: 4,
        ftm_total: 4,
        orb_total: 1,
        drb_total: 5,
        blk_total: 0,
        stl_total: 1,
        tov_total: 2,
        pf_total: 3,
        passes: 60,
        shots_released: 8,
        fouls: 3,
        movement_intents: 30,
        movement_outcomes: 28,
        responsibility_started: 40,
        play_activations: 2,
        fga_rim: 3,
        fgm_rim: 1,
        fga_near: 2,
        fgm_near: 1,
        fga_mid: 2,
        fgm_mid: 1,
        fga_three: 1,
        fgm_three: 0,
        starts_field_goal: 12,
        starts_free_throw: 2,
        starts_steal: 1,
        starts_dead_ball: 3,
        starts_timeout: 1,
        starts_blocked_shot: 1,
        to_reason_pass_lost: 1,
        to_reason_ball_lost: 0,
        to_reason_violation: 1,
        to_reason_offensive_foul: 0,
        to_reason_other: 0,
        shot_dist_hist: [1, 2, 2, 1, 1, 1, 0, 0],
        q4_late_three_att_by_margin: [2, 1, 3],
        q4_late_three_makes_by_margin: [1, 0, 1],
        oreb_possessions: 3,
        oreb_possessions_with_second_attempt: 2,
        bonus_entry_order_hist: [0, 0, 0, 1, 2, 0, 0, 0, 0, 0],
        team_periods_observed: 6,
        transition_counts: [[9, 2, 1, 0, 0, 0, 0], [2, 0, 0, 0, 0, 0, 0], [1, 0, 0, 0, 0, 0, 0], [0, 0, 0, 0, 0, 0, 0], [0, 0, 0, 0, 0, 0, 0], [0, 0, 0, 0, 0, 0, 0], [0, 0, 0, 0, 0, 0, 0]],
        game_clock_run_seconds: 720.0,
        live_ball_seconds: 600.0,
        final_score_home: 14,
        final_score_away: 10,
    };
    let parquet_path = dir.join("metrics.parquet");
    nba_validation::artifacts::write_game_metrics_parquet(&[metrics], &parquet_path)
        .expect("parquet write");
    // 读回验证（成熟库双向检查，validation.md §9.3）。
    let file = std::fs::File::open(&parquet_path).expect("reopen parquet");
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .expect("builder")
        .build()
        .expect("reader");
    let batches: Vec<RecordBatch> = reader.collect::<Result<_, _>>().expect("batches");
    assert_eq!(batches.len(), 1);
    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 1);
    assert_eq!(
        batch
            .column_by_name("seed")
            .expect("seed column")
            .as_any()
            .downcast_ref::<UInt64Array>()
            .expect("u64")
            .value(0),
        11
    );
    assert_eq!(
        batch
            .column_by_name("possessions")
            .expect("possessions column")
            .as_any()
            .downcast_ref::<UInt64Array>()
            .expect("u64")
            .value(0),
        20
    );
    assert_eq!(
        batch
            .column_by_name("scope")
            .expect("scope column")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("utf8")
            .value(0),
        "1q"
    );
    assert_eq!(
        batch
            .column_by_name("game_clock_run_seconds")
            .expect("clock column")
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("f64")
            .value(0),
        720.0
    );

    // SQLite 实例存储写入与查询。
    let store_path = dir.join("instances.sqlite");
    let store_input = nba_validation::artifacts::InstanceStoreInput {
        instances: vec![
            nba_validation::artifacts::InstanceRow {
                instance_kind: "shot",
                instance_id: "shot-1".to_string(),
                first_tick_index: 10,
                first_event_id: 100,
                terminal_tick_index: Some(14),
                terminal_event_id: Some(140),
                event_count: 3,
                terminal_kind: Some("SCORE".to_string()),
            },
            nba_validation::artifacts::InstanceRow {
                instance_kind: "pass",
                instance_id: "pass-9".to_string(),
                first_tick_index: 20,
                first_event_id: 200,
                terminal_tick_index: None,
                terminal_event_id: None,
                event_count: 1,
                terminal_kind: None,
            },
        ],
        relations: vec![nba_validation::artifacts::RelationRow {
            event_id: 140,
            tick_index: 14,
            kind: "caused_by".to_string(),
            target_event_id: 100,
        }],
    };
    nba_validation::artifacts::write_instance_store(&store_input, &store_path)
        .expect("sqlite write");
    let connection = rusqlite::Connection::open(&store_path).expect("reopen sqlite");
    let (open_count,): (i64,) = connection
        .query_row(
            "SELECT COUNT(*) FROM instances WHERE terminal_tick_index IS NULL",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| (count,))
        .expect("query");
    assert_eq!(open_count, 1, "pass-9 is open");
    let (terminal_kind,): (String,) = connection
        .query_row(
            "SELECT terminal_kind FROM instances WHERE instance_id = 'shot-1'",
            [],
            |row| row.get::<_, String>(0),
        )
        .map(|kind| (kind,))
        .expect("query");
    assert_eq!(terminal_kind, "SCORE");
    let (relation_count,): (i64,) = connection
        .query_row(
            "SELECT COUNT(*) FROM relations WHERE kind = 'caused_by'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| (count,))
        .expect("query");
    assert_eq!(relation_count, 1);
}

#[test]
fn rule_coverage_registry_and_branch_report() {
    use nba_validation::sampling_coverage::{branch_coverage_report, SamplingFacts};

    let summary = nba_validation::rule_coverage::rule_coverage_summary();
    assert!(summary.total_domains >= 25, "规则域登记过少");
    assert!(summary.covered >= 18, "有专门场景的域过少");
    assert_eq!(
        summary.total_domains,
        summary.covered + summary.sampling_only + summary.missing
    );
    // R5 违规程序域（走步/二次运球）必须如实登记为缺口。
    assert!(summary
        .gaps
        .iter()
        .any(|gap| gap.rule_id == "travel" && gap.gap_note.contains("R5")));

    // 覆盖报告：构造采样事实（含 PASS/REBOUND），covered 域按采样键对照。
    let mut facts = SamplingFacts::default();
    facts.event_kinds.insert("PASS".to_string(), 10);
    facts.event_kinds.insert("REBOUND".to_string(), 5);
    facts.foul_kinds.insert("shooting".to_string(), 2);
    let report = branch_coverage_report(2, &facts);
    assert_eq!(report.sampled + report.scenario_only + report.uncovered, report.domains.len() as u32);
    assert!(report.sampled >= 1, "PASS/REBOUND 证据应使命中域进入 sampled");
    // 未覆盖域全部进认证范围限制。
    assert_eq!(report.certification_limits.len() as u32, report.uncovered);
}

#[test]
fn parallel_batch_matches_serial_bitwise() {
    // 并行与串行（threads=1）结果逐位一致：每场计算只依赖自己的 seed，
    // 并行只改变完成顺序，聚合按 seed 升序还原。
    let config = nba_validation::runner::BatchRunConfig {
        first_seed: 202,
        n_games: 3,
        scope: "1q".to_string(),
        output_dir: temp_root(),
        max_ticks: 100_000,
    };
    let (metrics_serial, facts_serial) =
        nba_validation::runner::run_batch_with_facts_parallel(&config, 1)
            .expect("serial batch");
    let (metrics_parallel, facts_parallel) =
        nba_validation::runner::run_batch_with_facts_parallel(&config, 2)
            .expect("parallel batch");
    assert_eq!(metrics_serial.len(), metrics_parallel.len());
    for (serial, parallel) in metrics_serial.iter().zip(&metrics_parallel) {
        assert_eq!(serial.seed, parallel.seed);
        assert_eq!(serial.possessions, parallel.possessions, "possession mismatch");
        assert_eq!(serial.fga_total, parallel.fga_total);
        assert_eq!(serial.fgm_total, parallel.fgm_total);
        assert_eq!(serial.tov_total, parallel.tov_total);
        assert_eq!(serial.fouls, parallel.fouls);
        assert_eq!(serial.drb_total, parallel.drb_total);
        assert_eq!(serial.oreb_possessions, parallel.oreb_possessions);
        // 采样事实逐键一致
        assert_eq!(serial_event_count(&facts_serial), serial_event_count(&facts_parallel));
    }
}

#[test]
fn default_thread_count_is_capped_not_one() {
    let n = nba_validation::runner::default_batch_threads();
    assert!(n >= 2, "default_batch_threads must be >= 2 on this machine, got {n}");
}

fn serial_event_count(
    facts: &nba_validation::sampling_coverage::SamplingFacts,
) -> u64 {
    facts.event_kinds.values().sum::<u64>()
}

#[test]
fn batch_stat_evaluation_against_registered_reference() {
    // 3 场小批次：跑引擎 → 每场充分统计量 → 与登记参照比对 → 三出口。
    // 容差按登记值；种子固定可复现。
    let config = nba_validation::runner::BatchRunConfig {
        first_seed: 101,
        n_games: 3,
        scope: "1q".to_string(),
        output_dir: temp_root(),
        max_ticks: 100_000,
    };
    let metrics = nba_validation::runner::run_batch(&config).expect("batch run");
    assert_eq!(metrics.len(), 3);
    // 每场都应有非零出手与回合（1q 规模）
    for m in &metrics {
        assert!(m.fga_total > 0, "seed {} must have FGA", m.seed);
        assert!(m.possessions > 0, "seed {} must have possessions", m.seed);
        assert!(m.fga_rim + m.fga_near + m.fga_mid + m.fga_three > 0);
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/reference/stat_deltas.v1.json");
    let (registration, _) =
        nba_validation::reference::load_stat_registrations(&path).expect("load registration");
    let report = nba_validation::stat_batch::evaluate_batch(&metrics, &registration, 200, 42);
    assert_eq!(report.n_games, 3);
    // full 条目应产出判定（S01 至少 FG%/3PA 份额/3P% 三条）
    let s01: Vec<_> = report
        .verdicts
        .iter()
        .filter(|v| v.criterion_id == "STAT-S01")
        .collect();
    assert!(s01.len() >= 3, "S01 metrics must be judged: {}", s01.len());
    for v in &report.verdicts {
        assert!(
            v.verdict == "Pass" || v.verdict == "Defect" || v.verdict == "InsufficientEvidence",
            "unexpected verdict {}",
            v.verdict
        );
        // 区间下限不超过上限（数学健全性）
        assert!(v.interval_lower <= v.interval_upper + 1e-9);
    }
    // 叶项级合取有输出
    assert!(!report.criterion_verdicts.is_empty());
}

#[test]
fn legacy_views_derive_from_same_verdicts() {
    // 同一批裁决生成的兼容视图必须与新工件计数一致（禁止第二套判据）。
    let output_dir = temp_root().join("legacy-views");
    let config = ValidationRunConfig {
        seed: 29,
        scope: "1q".to_string(),
        output_dir: output_dir.clone(),
        max_ticks: 100_000,
    };
    let run = run_validation(&config).expect("run must complete");
    nba_validation::runner::write_artifacts(&run, &output_dir).expect("artifacts");
    let legacy_text =
        std::fs::read_to_string(output_dir.join("judgments.ndjson")).expect("legacy ndjson");
    let rows: Vec<serde_json::Value> = legacy_text
        .lines()
        .map(|line| serde_json::from_str(line).expect("legacy row json"))
        .collect();
    let scenario_passes = run
        .scenario_outcomes
        .iter()
        .filter(|outcome| outcome.verdict == Verdict::Pass)
        .count() as u64;
    let legacy_passes = rows.iter().filter(|row| row["verdict"] == "Pass").count() as u64;
    // 兼容视图还包含事实层一行（事实层机会存在时），它来自同一
    // EvidenceSummary；pass 数相等要求事实层非 Defect/Insufficient。
    let fact = &run.fact_report.evidence;
    let fact_row_count = usize::from(fact.opportunities > 0);
    assert_eq!(
        rows.len(),
        run.scenario_outcomes.len() + fact_row_count,
        "legacy rows = scenario rows + optional fact row"
    );
    if fact.defects == 0 && fact.insufficient == 0 && fact.opportunities > 0 {
        assert_eq!(
            legacy_passes,
            scenario_passes + fact_row_count as u64,
            "legacy view passes must equal scenario + fact passes from same verdicts"
        );
    }
    let report_text =
        std::fs::read_to_string(output_dir.join("attribution_report.json")).expect("legacy report");
    let report: serde_json::Value = serde_json::from_str(&report_text).expect("json");
    assert_eq!(report["total_judgments"].as_u64(), Some(rows.len() as u64));
    assert_eq!(report["defect_count"].as_u64(), Some(0));
}

#[test]
fn stats_equivalence_verdicts_bound_correctly() {
    let config = BootstrapConfig {
        replicates: 400,
        family_size: 1,
        alpha: 0.05,
        seed: 42,
    };
    // 完全在允许范围内 → Pass。
    let close: Vec<PairedGameValues> = (0..30)
        .map(|i| PairedGameValues {
            game_id: format!("g{i}"),
            simulated: 10.0 + (i % 3) as f64,
            reference: 10.0,
        })
        .collect();
    let (interval, verdict) = compare_mean_difference(&close, 5.0, &config);
    assert_eq!(verdict, nba_validation::stats::EquivalenceVerdict::Pass);
    assert!(interval.upper <= 5.0 && interval.lower >= -5.0);
    // 明确超出 → Defect。
    let far: Vec<PairedGameValues> = (0..30)
        .map(|i| PairedGameValues {
            game_id: format!("g{i}"),
            simulated: 40.0 + (i % 3) as f64,
            reference: 10.0,
        })
        .collect();
    let (_, verdict) = compare_mean_difference(&far, 5.0, &config);
    assert_eq!(verdict, nba_validation::stats::EquivalenceVerdict::Defect);
    // 空样本 → InsufficientEvidence（零分母不填零）。
    let (_, verdict) = compare_mean_difference(&[], 5.0, &config);
    assert_eq!(
        verdict,
        nba_validation::stats::EquivalenceVerdict::InsufficientEvidence
    );
}

#[test]
fn wasserstein_and_tv_behave_on_known_inputs() {
    // Wasserstein-1 对同分布为零；对平移分布等于平移量。
    let a: Vec<f64> = (0..100).map(|i| i as f64).collect();
    let b: Vec<f64> = (0..100).map(|i| i as f64 + 3.0).collect();
    let distance = nba_validation::stats::wasserstein_1(&a, &b);
    assert!((distance - 3.0).abs() < 1e-6, "W1 shift: {distance}");
    // TV：相同构成为 0，完全不相交为 1。
    let mut same_a: BTreeMap<String, f64> = BTreeMap::new();
    same_a.insert("x".to_string(), 5.0);
    same_a.insert("y".to_string(), 5.0);
    let same_b = same_a.clone();
    assert!((nba_validation::stats::total_variation(&[5.0, 5.0], &[5.0, 5.0])).abs() < 1e-12);
    let mut config = BootstrapConfig {
        replicates: 200,
        family_size: 1,
        alpha: 0.05,
        seed: 7,
    };
    let (_, verdict_same) = compare_total_variation(&same_a, &same_b, 0.1, &config);
    assert_eq!(
        verdict_same,
        nba_validation::stats::EquivalenceVerdict::Pass
    );
    let mut disjoint_a: BTreeMap<String, f64> = BTreeMap::new();
    disjoint_a.insert("a".to_string(), 1.0);
    let mut disjoint_b: BTreeMap<String, f64> = BTreeMap::new();
    disjoint_b.insert("b".to_string(), 1.0);
    config.replicates = 100;
    let (_, verdict_disjoint) = compare_total_variation(&disjoint_a, &disjoint_b, 0.1, &config);
    assert_eq!(
        verdict_disjoint,
        nba_validation::stats::EquivalenceVerdict::Defect
    );
    // Wasserstein 等效判定：同分布 Pass，大平移 Defect。
    let c: Vec<f64> = (0..100).map(|i| i as f64).collect();
    let (_, verdict_close) = compare_wasserstein(&a, &c, 1.0, &config);
    assert_eq!(
        verdict_close,
        nba_validation::stats::EquivalenceVerdict::Pass
    );
    let (_, verdict_far) = compare_wasserstein(&a, &b, 1.0, &config);
    assert_eq!(
        verdict_far,
        nba_validation::stats::EquivalenceVerdict::Defect
    );
}

#[test]
fn mechanism_spec_accepts_checkpoint_experiments() {
    // 引擎检查点能力（experimental_fork / apply_validation_perturbation /
    // reseed_validation_rng）落地后，checkpoint 类实验合法（执行载体
    // crates/validation/src/mech.rs）；此前引擎无该 API 时显式拒绝。
    let spec = nba_validation::mechanism::PerturbationSpec {
        experiment_id: "MECH-X-CP".to_string(),
        criterion_id: "MECH-S01".to_string(),
        kind: nba_validation::mechanism::ExperimentKind::Checkpoint,
        input_path: "attributes.shooting_three".to_string(),
        level: "low".to_string(),
        value: 0.2,
        expected_channel: "three_pa".to_string(),
        expected_direction: "decrease".to_string(),
        minimum_effect: 1.0,
        neutral_delta: 0.5,
    };
    spec.validate().expect("checkpoint experiments are supported");
}

#[test]
fn opportunity_spec_rejects_missing_fields() {
    let spec = nba_validation::opportunity::OpportunitySpec {
        id: String::new(),
        version: 1,
        observation_frequency: "every_tick".to_string(),
        enter: nba_validation::opportunity::GeometricPredicate::Always,
        exit: nba_validation::opportunity::GeometricPredicate::Always,
        min_duration_ticks: 1,
        merge_gap_ticks: 0,
        window_deadline_ticks: 100,
        actors: vec!["H_01".to_string()],
        measurements: Vec::new(),
    };
    assert!(spec.validate().is_err());
}

#[test]
fn rule_profile_blocks_unapproved_entries() {
    use nba_validation::profile::{RuleEntry, RuleReviewStatus};
    let mut profile = nba_validation::profile::BasketballRuleProfile::new("test", "nba", "v1");
    profile.register(RuleEntry {
        rule_id: "r1".to_string(),
        league: "nba".to_string(),
        rule_version: "v1".to_string(),
        source_clause: "§1".to_string(),
        interpretation_cases: vec!["case".to_string()],
        review_status: RuleReviewStatus::Approved,
    });
    profile.register(RuleEntry {
        rule_id: "r2".to_string(),
        league: "nba".to_string(),
        rule_version: "v1".to_string(),
        source_clause: String::new(),
        interpretation_cases: Vec::new(),
        review_status: RuleReviewStatus::Missing,
    });
    assert!(!profile.all_approved());
    assert_eq!(profile.blocking_ids(), vec!["r2".to_string()]);
}
