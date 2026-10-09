//! Runner、工件输出与门评估（validation.md §10）。
//!
//! 编排一次验证运行：跑引擎比赛 → 写验证流（首部/刻/尾部）→ 读回 →
//! 各层判定 → 输出工件。完成实例写 ndjson（运行目录由调用方给定且
//! 必须在项目忽略清单内）；`game_metrics.parquet` 使用成熟 parquet
//! 库写入（成熟格式禁止手写解析/生成）。退出码：0 全部通过、1 有
//! 确定缺陷、2 输入或执行错误、3 必需证据不足。

use std::io::Write as _;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;

use nba_engine::MatchEngine;
use nba_protocol::{
    EvidenceLevel, StreamCapabilities, StreamTick, UnitDeclaration, ValidationStreamHeader,
    ValidationStreamTrailer,
};
use serde::Serialize;

use crate::criteria::{CriterionRegistry, Family};
use crate::joint::JointReport;
use crate::movement_tact::FactAuditReport;
use crate::reconstruct::{
    PairingAudit, PossessionReconciliation, RebuiltBoxScore, RebuiltClockAccount,
};
use crate::scenario::{evaluate_matrix, ScenarioOutcome};
use crate::stream::InstanceIndex;
use crate::verdict::{EvidenceSummary, Verdict};

use crate::artifacts::{GameMetrics, LedgerComparison};

/// 默认 tick 预算（manifest 的资源上限登记值）。
pub const DEFAULT_MAX_TICKS: u64 = 200_000;
/// 默认字节数预算（manifest 的资源上限登记值；单场流实测个位数 MB，
/// 上限取 4 GiB 供批量运行）。
pub const DEFAULT_MAX_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// 验证运行配置。
#[derive(Debug, Clone)]
pub struct ValidationRunConfig {
    pub seed: u64,
    /// 引擎范围（`full` / `1q`）。
    pub scope: String,
    /// 运行输出目录（必须已被 gitignore）。
    pub output_dir: std::path::PathBuf,
    /// 最大 tick 预算。
    pub max_ticks: u64,
}

/// 运行产出（内存中的全部层结果 + 工件路径）。
#[derive(Debug)]
pub struct ValidationRun {
    pub header: ValidationStreamHeader,
    pub trailer: ValidationStreamTrailer,
    pub ticks: Vec<StreamTick>,
    pub scenario_outcomes: Vec<ScenarioOutcome>,
    pub fact_report: FactAuditReport,
    pub pairing: PairingAudit,
    pub possessions: PossessionReconciliation,
    pub joint: JointReport,
    pub instance_index: InstanceIndex,
    pub evidence: EvidenceSummary,
    pub artifact_paths: Vec<std::path::PathBuf>,
}

/// 退出码（validation.md §10）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    /// 所请求认证全部通过。
    AllPass = 0,
    /// 有确定缺陷。
    Defect = 1,
    /// 输入或执行错误。
    ExecutionError = 2,
    /// 必需证据不足。
    InsufficientEvidence = 3,
}

impl ValidationRun {
    /// 聚合退出码：执行错误优先 2；缺陷与未判定并存返回 1；全部通过
    /// 返回 0；仅未判定返回 3（validation.md §10）。
    pub fn exit_code(&self) -> ExitCode {
        if self.trailer.end_reason == "truncated" {
            return ExitCode::ExecutionError;
        }
        let scenario_defects = self
            .scenario_outcomes
            .iter()
            .filter(|outcome| outcome.verdict == Verdict::Defect)
            .count();
        let scenario_insufficient = self
            .scenario_outcomes
            .iter()
            .filter(|outcome| outcome.verdict == Verdict::InsufficientEvidence)
            .count();
        if scenario_defects > 0 || self.pairing.shots_without_release > 0 {
            return ExitCode::Defect;
        }
        if scenario_insufficient > 0 {
            return ExitCode::InsufficientEvidence;
        }
        if self.fact_report.evidence.defects > 0 {
            return ExitCode::Defect;
        }
        if self.fact_report.evidence.insufficient > 0 {
            return ExitCode::InsufficientEvidence;
        }
        ExitCode::AllPass
    }
}

/// 跑一场固定种子的生产比赛并收集验证流刻（含流尾未完成实例真实值）。
pub fn run_engine_game_with_trailer(
    config: &ValidationRunConfig,
) -> Result<(Vec<StreamTick>, u64), String> {
    let mut engine = MatchEngine::new(config.seed);
    engine
        .set_scope(&config.scope)
        .map_err(|error| format!("invalid scope `{}`: {error}", config.scope))?;
    let mut ticks = Vec::new();
    let mut count = 0u64;
    while !engine.is_finished() && count < config.max_ticks {
        ticks.push(engine.step());
        count += 1;
    }
    if !engine.is_finished() {
        return Err(format!(
            "game seed {} did not complete within {} ticks",
            config.seed, config.max_ticks
        ));
    }
    // 引擎在 is_finished 后未清算的业务实例数（validation.md §3.1 流尾）。
    let open = engine.open_business_instances();
    Ok((ticks, open))
}

/// 跑一场固定种子的生产比赛并收集验证流刻。
pub fn run_engine_game(config: &ValidationRunConfig) -> Result<Vec<StreamTick>, String> {
    run_engine_game_with_trailer(config).map(|(ticks, _)| ticks)
}

/// 构建验证流首部与尾部（validation.md §3.1）。
pub fn build_header_and_trailer(
    config: &ValidationRunConfig,
    ticks: &[StreamTick],
    open_business_instances: u64,
) -> (ValidationStreamHeader, ValidationStreamTrailer) {
    let last = ticks.last();
    let header = ValidationStreamHeader {
        protocol_version: "validation-1".to_string(),
        input_digest: format!("engine-seed-{}", config.seed),
        rng_algorithm: "chacha8".to_string(),
        seed: config.seed,
        scope: config.scope.clone(),
        evidence_level: EvidenceLevel::Frames,
        capabilities: StreamCapabilities {
            geometry_frames: true,
            business_identity: true,
            event_relations: true,
            observation_payloads: true,
            stat_attribution: true,
            checkpoints: false,
        },
        expected_games: 1,
        certification_scope: "structural".to_string(),
        league_profile: "nba-v1".to_string(),
        units: UnitDeclaration {
            length: "feet".to_string(),
            time: "seconds".to_string(),
            coordinates: "left_hoop_origin_x_right_y_down".to_string(),
            tick_seconds: 0.04,
        },
    };
    let trailer = ValidationStreamTrailer {
        end_reason: "game_end".to_string(),
        records: ticks.len() as u64,
        events: ticks
            .iter()
            .map(|tick| tick.frame.events.len() as u64)
            .sum(),
        last_tick_index: last.map(|tick| tick.frame.tick_index).unwrap_or(0),
        integrity_digest: format!("{:x}", ticks.len() as u64),
        open_business_instances,
        final_score_home: last.map(|tick| tick.frame.score.home).unwrap_or(0),
        final_score_away: last.map(|tick| tick.frame.score.away).unwrap_or(0),
        periods_played: last.map(|tick| tick.frame.period).unwrap_or(0),
    };
    (header, trailer)
}

/// 写全部工件（validation.md §10 新增输出）。写入运行配置指定的目录。
/// 完整清单：`validation_manifest.json`、`coverage.json`、
/// `instance_judgments.ndjson`、`ledger_comparison.json`、
/// `game_metrics.parquet`、`statistical_results.json`、
/// `mechanism_results.json`、`scenario_results.json`、
/// `validation_report.json`、`failure_segments.json`、
/// `instances.sqlite` 与旧兼容视图（`judgments.ndjson`、
/// `attribution_report.json`，同一裁决生成）。
pub fn write_artifacts(
    run: &ValidationRun,
    output_dir: &std::path::Path,
) -> Result<Vec<std::path::PathBuf>, String> {
    let dir = output_dir;
    std::fs::create_dir_all(dir).map_err(|error| format!("create output dir failed: {error}"))?;
    let mut paths = Vec::new();
    // instance_judgments.ndjson：场景与事实层裁决逐条落盘。
    let judgments_path = dir.join("instance_judgments.ndjson");
    let file = std::fs::File::create(&judgments_path)
        .map_err(|error| format!("create {judgments_path:?} failed: {error}"))?;
    let mut writer = std::io::BufWriter::new(file);
    for outcome in &run.scenario_outcomes {
        serde_json::to_writer(&mut writer, outcome)
            .map_err(|error| format!("serialize scenario outcome failed: {error}"))?;
        writer
            .write_all(b"\n")
            .map_err(|error| format!("write scenario outcome failed: {error}"))?;
    }
    writer
        .flush()
        .map_err(|error| format!("flush judgments failed: {error}"))?;
    paths.push(judgments_path);
    // coverage.json：场景矩阵覆盖。
    let coverage_path = dir.join("coverage.json");
    let coverage = CoverageReport::from_run(run);
    write_json(&coverage, &coverage_path)?;
    paths.push(coverage_path);
    // scenario_results.json：逐场景裁决（instance_judgments 的 JSON 载体）。
    let scenario_path = dir.join("scenario_results.json");
    write_json(&run.scenario_outcomes, &scenario_path)?;
    paths.push(scenario_path);
    // ledger_comparison.json：独立重建 vs 声明（CAUSE-B01/B02/B03）。
    let (rebuilt, declared) = RebuiltBoxScore::rebuild(&run.ticks);
    let clock = RebuiltClockAccount::rebuild(&run.ticks);
    let ledger = LedgerComparison {
        box_score_rebuilt: rebuilt.clone(),
        box_score_declared: declared,
        possession: run.possessions.clone(),
        clock,
        pairing: PairingAudit {
            shots_without_release: run.pairing.shots_without_release,
            passes_without_release: run.pairing.passes_without_release,
            free_throws_without_award: run.pairing.free_throws_without_award,
        },
    };
    let ledger_path = dir.join("ledger_comparison.json");
    write_json(&ledger, &ledger_path)?;
    paths.push(ledger_path);
    // game_metrics.parquet：每场充分统计量（arrow/parquet 库写入）。
    let metrics = GameMetrics::from_stream(
        run.header.seed,
        &run.header.scope,
        &run.ticks,
        &rebuilt,
        &ledger.clock,
        run.possessions.engine_count,
    );
    let parquet_path = dir.join("game_metrics.parquet");
    crate::artifacts::write_game_metrics_parquet(std::slice::from_ref(&metrics), &parquet_path)?;
    paths.push(parquet_path);
    // instances.sqlite：完成实例 + 关系索引（§10 查询存储），open
    // 实例计数与流尾部核对。
    let (store_input, open_instances) = crate::artifacts::project_instance_store(&run.ticks);
    let sqlite_path = dir.join("instances.sqlite");
    crate::artifacts::write_instance_store(&store_input, &sqlite_path)?;
    paths.push(sqlite_path);
    // 统计层结果：逐条按参考登记（data/reference/stat_deltas.v1.json）
    // 给状态与原因码——容差已登记的条目区分「待审核」与「参考字段缺
    // 失」，不统一填同一原因码（validation.md §6.3/§11 数据准备任务）。
    let stat_registrations = match crate::reference::load_stat_registrations(
        &crate::reference::default_registration_path(),
    ) {
        Ok((_, registrations)) => registrations,
        Err(error) => {
            return Err(format!(
                "stat delta registration required for statistical_results.json: {error}"
            ));
        }
    };
    let mut stat_verdicts = Vec::new();
    for criterion_id in registered_ids(&Family::Stat) {
        match stat_registrations.get(&criterion_id) {
            Some(entry) => stat_verdicts.push(CriterionVerdict {
                criterion_id,
                verdict: "InsufficientEvidence".to_string(),
                reason_code: entry.reason_code.clone(),
            }),
            None => stat_verdicts.push(CriterionVerdict {
                criterion_id,
                verdict: "InsufficientEvidence".to_string(),
                reason_code: "reference_data_unregistered".to_string(),
            }),
        }
    }
    // JOINT 正式判定同一登记通道：全部叶项当前 blocked（条件分布的参
    // 考侧需要 PBP 时间线或追踪数据），逐条给缺失字段与获取任务。
    let mut joint_verdicts = Vec::new();
    for criterion_id in registered_ids(&Family::Joint) {
        match stat_registrations.get(&criterion_id) {
            Some(entry) => joint_verdicts.push(CriterionVerdict {
                criterion_id,
                verdict: "InsufficientEvidence".to_string(),
                reason_code: entry.reason_code.clone(),
            }),
            None => joint_verdicts.push(CriterionVerdict {
                criterion_id,
                verdict: "InsufficientEvidence".to_string(),
                reason_code: "reference_data_unregistered".to_string(),
            }),
        }
    }
    let statistical = StatisticalResults {
        registered_criteria: registered_ids(&Family::Stat),
        joint_registered_criteria: registered_ids(&Family::Joint),
        verdicts: stat_verdicts,
        joint_verdicts,
        note: "STAT verdicts follow data/reference/stat_deltas.v1.json; \
               pending-review and missing-field items stay InsufficientEvidence \
               until the reference reviewer approves (validation.md §2.1/§6.3); \
               JOINT items are all blocked on PBP/timeline reference data"
            .to_string(),
    };
    let stat_path = dir.join("statistical_results.json");
    write_json(&statistical, &stat_path)?;
    paths.push(stat_path);
    let mechanism = crate::mech_runner::mechanism_results_with_experiments(dir);
    let mech_path = dir.join("mechanism_results.json");
    write_json(&mechanism, &mech_path)?;
    paths.push(mech_path);
    // 失败片段索引：缺陷裁决按片段定位（固定抽样规则含普通通过样本，
    // §8「防止只观察极端回合」）。
    let failures = FailureSegmentIndex::from_run(run);
    let failures_path = dir.join("failure_segments.json");
    write_json(&failures, &failures_path)?;
    paths.push(failures_path);
    // 旧工件兼容视图：同一裁决生成（§10 禁止第二套判据）。
    let fact = &run.fact_report.evidence;
    let (legacy_rows, legacy_report) = crate::artifacts::legacy_views(
        &run.scenario_outcomes,
        fact.defects,
        fact.insufficient,
        fact.opportunities,
        fact.passes,
    );
    let legacy_judgments_path = dir.join("judgments.ndjson");
    let file = std::fs::File::create(&legacy_judgments_path)
        .map_err(|error| format!("create {legacy_judgments_path:?} failed: {error}"))?;
    let mut writer = std::io::BufWriter::new(file);
    for row in &legacy_rows {
        serde_json::to_writer(&mut writer, row)
            .map_err(|error| format!("serialize legacy judgment failed: {error}"))?;
        writer
            .write_all(b"\n")
            .map_err(|error| format!("write legacy judgment failed: {error}"))?;
    }
    writer
        .flush()
        .map_err(|error| format!("flush legacy judgments failed: {error}"))?;
    paths.push(legacy_judgments_path);
    let legacy_report_path = dir.join("attribution_report.json");
    write_json(&legacy_report, &legacy_report_path)?;
    paths.push(legacy_report_path);
    // validation_manifest.json：本次运行的输入与准则摘要。
    let manifest = crate::manifest::ValidationManifest {
        manifest_version: "validation-manifest-1".to_string(),
        input_digest: format!("engine-seed-{}", run.header.seed),
        code_version: format!("engine-stream-{}", run.trailer.last_tick_index),
        rng_algorithm: run.header.rng_algorithm.clone(),
        seed: run.header.seed,
        scope: run.header.scope.clone(),
        protocol_version: run.header.protocol_version.clone(),
        criteria_digest: registry_digest(),
        reference_digest: String::new(),
        units: crate::manifest::UnitsDeclaration {
            length: run.header.units.length.clone(),
            time: run.header.units.time.clone(),
            coordinates: run.header.units.coordinates.clone(),
            tick_seconds: run.header.units.tick_seconds,
        },
        budget_max_ticks: DEFAULT_MAX_TICKS,
        budget_max_bytes: DEFAULT_MAX_BYTES,
        expected_games: run.header.expected_games,
        certification_scope: run.header.certification_scope.clone(),
    };
    let manifest_path = dir.join("validation_manifest.json");
    write_json(&manifest, &manifest_path)?;
    paths.push(manifest_path);
    // validation_report.json：顶层证据汇总。
    let report_path = dir.join("validation_report.json");
    let report = TopLevelReport {
        evidence: run.evidence.clone(),
        scenario_passes: run
            .scenario_outcomes
            .iter()
            .filter(|outcome| outcome.verdict == Verdict::Pass)
            .count() as u64,
        scenario_defects: run
            .scenario_outcomes
            .iter()
            .filter(|outcome| outcome.verdict == Verdict::Defect)
            .count() as u64,
        scenario_insufficient: run
            .scenario_outcomes
            .iter()
            .filter(|outcome| outcome.verdict == Verdict::InsufficientEvidence)
            .count() as u64,
        pairing_shots_without_release: run.pairing.shots_without_release,
        pairing_passes_without_release: run.pairing.passes_without_release,
        open_business_instances: run.trailer.open_business_instances,
        open_instances_from_store: open_instances.len() as u64,
        exit_code: run.exit_code() as u8,
    };
    write_json(&report, &report_path)?;
    paths.push(report_path);
    Ok(paths)
}

fn write_json<T: serde::Serialize>(value: &T, path: &std::path::Path) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    std::fs::write(path, text).map_err(|error| format!("write {path:?} failed: {error}"))
}

/// 场景覆盖报告（coverage.json）。
#[derive(Debug, Serialize)]
struct CoverageReport {
    total_scenarios: usize,
    triggered: u64,
    untriggered: Vec<String>,
}

impl CoverageReport {
    fn from_run(run: &ValidationRun) -> Self {
        let untriggered = run
            .scenario_outcomes
            .iter()
            .filter(|outcome| outcome.trigger_occurrences == 0)
            .map(|outcome| outcome.scenario_id.clone())
            .collect();
        Self {
            total_scenarios: run.scenario_outcomes.len(),
            triggered: run
                .scenario_outcomes
                .iter()
                .filter(|outcome| outcome.trigger_occurrences > 0)
                .count() as u64,
            untriggered,
        }
    }
}

/// 顶层验证报告（validation_report.json）。
#[derive(Debug, Serialize)]
struct TopLevelReport {
    evidence: EvidenceSummary,
    scenario_passes: u64,
    scenario_defects: u64,
    scenario_insufficient: u64,
    pairing_shots_without_release: u64,
    pairing_passes_without_release: u64,
    open_business_instances: u64,
    /// 实例存储投影发现的 open instance 数（与流尾部计数交叉核对）。
    open_instances_from_store: u64,
    exit_code: u8,
}

/// 统计层结果工件（statistical_results.json）。
#[derive(Debug, Serialize)]
struct StatisticalResults {
    registered_criteria: Vec<String>,
    /// JOINT 叶项（同一登记通道，validation.md §8 条件分布参考）。
    joint_registered_criteria: Vec<String>,
    verdicts: Vec<CriterionVerdict>,
    joint_verdicts: Vec<CriterionVerdict>,
    note: String,
}

/// 单条准则裁决记录（validation.md §10 裁决字段）。
#[derive(Debug, Serialize)]
struct CriterionVerdict {
    criterion_id: String,
    verdict: String,
    reason_code: String,
}

/// 失败片段索引（failure_segments.json）：缺陷与未判定裁决的定位，
/// 固定抽样规则同时保留普通通过样本计数（validation.md §8）。
#[derive(Debug, Serialize)]
struct FailureSegmentIndex {
    failures: Vec<SegmentEntry>,
    /// 按固定规则保留的通过样本数（每 10 个通过实例抽 1 个）。
    retained_pass_samples: u64,
    total_passes: u64,
}

#[derive(Debug, Serialize)]
struct SegmentEntry {
    criterion_id: String,
    verdict: String,
    detail: String,
}

impl FailureSegmentIndex {
    fn from_run(run: &ValidationRun) -> Self {
        let mut failures = Vec::new();
        for outcome in &run.scenario_outcomes {
            if matches!(
                outcome.verdict,
                Verdict::Defect | Verdict::InsufficientEvidence
            ) {
                failures.push(SegmentEntry {
                    criterion_id: outcome.scenario_id.clone(),
                    verdict: match outcome.verdict {
                        Verdict::Defect => "Defect".to_string(),
                        Verdict::InsufficientEvidence => "InsufficientEvidence".to_string(),
                        _ => unreachable!("filtered above"),
                    },
                    detail: format!(
                        "missing_required={:?} forbidden_seen={:?}",
                        outcome.missing_required, outcome.forbidden_seen
                    ),
                });
            }
        }
        let fact = &run.fact_report.evidence;
        if fact.defects > 0 || fact.insufficient > 0 {
            failures.push(SegmentEntry {
                criterion_id: "MOVE_TACT_FACT_LAYER".to_string(),
                verdict: if fact.defects > 0 { "Defect" } else { "InsufficientEvidence" }.to_string(),
                detail: format!(
                    "movement double_closed={} implausible={} responsibility ends_without_start={} play ends_without_activation={}",
                    run.fact_report.movement.double_closed,
                    run.fact_report.movement.implausible_completions,
                    run.fact_report.responsibility.ends_without_start,
                    run.fact_report.play.ends_without_activation,
                ),
            });
        }
        if run.pairing.shots_without_release > 0
            || run.pairing.passes_without_release > 0
            || run.pairing.free_throws_without_award > 0
        {
            failures.push(SegmentEntry {
                criterion_id: "CAUSE-I04".to_string(),
                verdict: "Defect".to_string(),
                detail: format!(
                    "shots_without_release={} passes_without_release={} free_throws_without_award={}",
                    run.pairing.shots_without_release,
                    run.pairing.passes_without_release,
                    run.pairing.free_throws_without_award,
                ),
            });
        }
        let total_passes = run
            .scenario_outcomes
            .iter()
            .filter(|outcome| outcome.verdict == Verdict::Pass)
            .count() as u64;
        Self {
            failures,
            retained_pass_samples: total_passes / 10,
            total_passes,
        }
    }
}

/// 注册表中指定族的全部编号（排序）。
fn registered_ids(family: &Family) -> Vec<String> {
    let registry = CriterionRegistry::full();
    let mut ids: Vec<String> = registry
        .by_family(*family)
        .into_iter()
        .map(|entry| entry.id.clone())
        .collect();
    ids.sort();
    ids
}

/// 对既有刻序列运行验证（网页会话入口）：比赛已在 MatchService 中
/// 推进过，验证器只读刻序列，不再重新跑引擎。流尾未完成实例数由
/// 调用方从引擎账本读取传入。
pub fn run_validation_on_ticks(
    config: &ValidationRunConfig,
    ticks: Vec<StreamTick>,
    open_instances: u64,
) -> Result<ValidationRun, String> {
    if ticks.is_empty() {
        return Err("validation requires at least one tick".to_string());
    }
    let (header, trailer) = build_header_and_trailer(config, &ticks, open_instances);
    let scenario_outcomes = evaluate_matrix(&ticks);
    let fact_report = FactAuditReport::run(&ticks);
    let pairing = PairingAudit::audit(&ticks);
    let possessions = PossessionReconciliation::compare(&ticks);
    let joint = JointReport::run(&ticks);
    let instance_index = InstanceIndex::build(&ticks);
    let mut evidence = EvidenceSummary::default();
    for outcome in &scenario_outcomes {
        evidence.record(outcome.verdict);
    }
    for verdict in [
        fact_report.movement.verdict(),
        fact_report.responsibility.verdict(),
        fact_report.play.verdict(),
    ] {
        evidence.record(verdict);
    }
    let run = ValidationRun {
        header,
        trailer,
        ticks,
        scenario_outcomes,
        fact_report,
        pairing,
        possessions,
        joint,
        instance_index,
        evidence,
        artifact_paths: Vec::new(),
    };
    let _ = &run.header.input_digest;
    Ok(run)
}

/// 一次完整验证运行（比赛 → 流 → 各层判定 → 工件）。
pub fn run_validation(config: &ValidationRunConfig) -> Result<ValidationRun, String> {
    let (ticks, open_instances) = run_engine_game_with_trailer(config)?;
    // 流尾未完成实例数来自引擎身份账本的只读访问器（validation.md §3.1）；
    // 实例存储投影的 open 计数在 write_artifacts 里与它交叉核对。
    let (header, trailer) = build_header_and_trailer(config, &ticks, open_instances);
    let scenario_outcomes = evaluate_matrix(&ticks);
    let fact_report = FactAuditReport::run(&ticks);
    let pairing = PairingAudit::audit(&ticks);
    let possessions = PossessionReconciliation::compare(&ticks);
    let joint = JointReport::run(&ticks);
    let instance_index = InstanceIndex::build(&ticks);
    let mut evidence = EvidenceSummary::default();
    for outcome in &scenario_outcomes {
        evidence.record(outcome.verdict);
    }
    for verdict in [
        fact_report.movement.verdict(),
        fact_report.responsibility.verdict(),
        fact_report.play.verdict(),
    ] {
        evidence.record(verdict);
    }
    let run = ValidationRun {
        header,
        trailer,
        ticks,
        scenario_outcomes,
        fact_report,
        pairing,
        possessions,
        joint,
        instance_index,
        evidence,
        artifact_paths: Vec::new(),
    };
    let _ = &run.header.input_digest;
    Ok(run)
}

/// 准则注册表快照（清单摘要输入）。
pub fn registry_digest() -> String {
    let registry = CriterionRegistry::full();
    format!("criteria-v1-{}", registry.len())
}

/// 批次运行配置（validation.md §6.4 样本计划：固定 n_games 与种子序列）。
#[derive(Debug, Clone)]
pub struct BatchRunConfig {
    /// 首场种子（后续种子顺序递增）。
    pub first_seed: u64,
    /// 比赛场数。
    pub n_games: u32,
    /// 引擎范围（`full` / `1q`）。
    pub scope: String,
    /// 运行输出目录。
    pub output_dir: std::path::PathBuf,
    /// 单场最大 tick 预算。
    pub max_ticks: u64,
}

/// 多场批次运行：跑 n_games 场不同种子的比赛，产出每场充分统计量。
pub fn run_batch(config: &BatchRunConfig) -> Result<Vec<GameMetrics>, String> {
    run_batch_with_facts(config).map(|(metrics, _facts)| metrics)
}

/// 批量运行并聚合采样事实（分支覆盖工件用；D29：与 `run_batch` 共享执行体）。
pub fn run_batch_with_facts(
    config: &BatchRunConfig,
) -> Result<(Vec<GameMetrics>, crate::sampling_coverage::SamplingFacts), String> {
    run_batch_with_facts_parallel(config, default_batch_threads())
}

/// 默认并行度：可用核数与 3 取小（full 场流内存大，防 OOM；4 核机器取 3）。
pub fn default_batch_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().clamp(1, 3))
        .unwrap_or(1)
}

/// 并行批量运行：每场独立 seed 独立引擎实例，线程池逐场消费；
/// 结果按 seed 升序返回（与串行逐位一致——每场计算只依赖自己的
/// seed，聚合顺序不影响数值）。
/// 并行度由调用方控制（内存约束：每场峰值内存 = 引擎 + 全场流 + 重建器）。
pub fn run_batch_with_facts_parallel(
    config: &BatchRunConfig,
    threads: usize,
) -> Result<(Vec<GameMetrics>, crate::sampling_coverage::SamplingFacts), String> {
    let threads = threads.clamp(1, config.n_games.max(1) as usize);
    let work: Vec<(u64, u32)> = (0..config.n_games)
        .map(|index| (config.first_seed + u64::from(index), index))
        .collect();
    let results: Mutex<Vec<(u32, GameMetrics, crate::sampling_coverage::SamplingFacts)>> =
        Mutex::new(Vec::with_capacity(config.n_games as usize));
    let first_error: Mutex<Option<String>> = Mutex::new(None);
    let work_queue = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                if first_error.lock().expect("batch error lock").is_some() {
                    return;
                }
                let index = work_queue.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some(&(seed, ordinal)) = work.get(index) else {
                    return;
                };
                let run_config = ValidationRunConfig {
                    seed,
                    scope: config.scope.clone(),
                    output_dir: config.output_dir.clone(),
                    max_ticks: config.max_ticks,
                };
                let run = match run_validation(&run_config) {
                    Ok(run) => run,
                    Err(error) => {
                        let mut slot = first_error.lock().expect("batch error lock");
                        if slot.is_none() {
                            *slot = Some(format!("seed {seed}: {error}"));
                        }
                        return;
                    }
                };
                let (rebuilt, _declared) = RebuiltBoxScore::rebuild(&run.ticks);
                let clock = RebuiltClockAccount::rebuild(&run.ticks);
                let metrics = GameMetrics::from_stream(
                    seed,
                    &config.scope,
                    &run.ticks,
                    &rebuilt,
                    &clock,
                    run.possessions.engine_count,
                );
                let facts = crate::sampling_coverage::SamplingFacts::from_single_run(&run.ticks);
                results
                    .lock()
                    .expect("batch results lock")
                    .push((ordinal, metrics, facts));
            });
        }
    });
    if let Some(error) = first_error.into_inner().expect("batch error lock") {
        return Err(error);
    }
    let mut results = results.into_inner().expect("batch results lock");
    results.sort_by_key(|(ordinal, _, _)| *ordinal);
    let mut all_metrics = Vec::with_capacity(results.len());
    let mut facts = crate::sampling_coverage::SamplingFacts::default();
    for (_, metrics, run_facts) in results {
        all_metrics.push(metrics);
        facts.merge(run_facts);
    }
    Ok((all_metrics, facts))
}
