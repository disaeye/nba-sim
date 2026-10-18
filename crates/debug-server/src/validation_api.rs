//! 五层验证的页面入口（validation.md §10 工件与裁决输出）。
//!
//! `/api/simulate` 返回给页面的比赛刻序列缓存在这里；`/api/validation/run`
//! 对同一场比赛执行全部层判定（页面看到什么就检什么），`/status` 轮询、
//! `/report` 出逐场景与账本对账详单。页面只渲染结论，不重算判据。

use std::sync::{Arc, Mutex};

use nba_protocol::StreamTick;
use nba_validation::artifacts::{project_instance_store, LedgerComparison};
use nba_validation::reconstruct::{PairingAudit, RebuiltBoxScore, RebuiltClockAccount};
use nba_validation::runner::write_artifacts;
use nba_validation::scenario::ScenarioOutcome;
use nba_validation::verdict::EvidenceSummary;
use serde_json::Value;

/// HTTP 响应三元组（与 main.rs 的 Response 同形，避免循环依赖）。
type Response = (&'static str, &'static str, Vec<u8>);

/// 页面当前正在查看的比赛：种子、范围与全部刻。验证入口对它判定。
pub struct CachedGame {
    pub seed: u64,
    pub scope: String,
    pub ticks: Vec<StreamTick>,
}

/// 一次验证的裁决快照（页面渲染输入，validation.md §10 裁决字段）。
#[derive(Clone)]
pub struct ValidationSnapshot {
    pub exit_code: u8,
    pub evidence: EvidenceSummary,
    pub scenario_results: Vec<ScenarioOutcome>,
    pub ledger: LedgerComparison,
    pub ticks: u64,
    pub events: u64,
    pub open_business_instances: u64,
    pub open_instances_from_store: u64,
    pub insufficient_reasons: Vec<String>,
}

/// 验证任务状态机：Idle（可启动）→ Running（进度）→ Done/Failed。
/// 当前判定是同步一次完成（1q 数十毫秒、full 数秒），Running 保留
/// 为进度报告位；改为分块推进时把阶段写入 `stage`。
#[allow(dead_code)]
pub enum ValidationState {
    Idle {
        cached_game: Option<CachedGame>,
        last_result: Option<ValidationSnapshot>,
    },
    Running {
        cached_game: Option<CachedGame>,
        last_result: Option<ValidationSnapshot>,
        stage: String,
    },
    Done {
        cached_game: Option<CachedGame>,
        result: ValidationSnapshot,
    },
    Failed {
        cached_game: Option<CachedGame>,
        error: String,
    },
}

pub type SharedValidation = Arc<Mutex<ValidationState>>;

/// main.rs 的坏请求响应（保持同一形状，路由层直接透传）。
fn bad_request(message: String) -> Response {
    (
        "400 Bad Request",
        "application/json; charset=utf-8",
        serde_json::json!({ "error": message })
            .to_string()
            .into_bytes(),
    )
}

fn internal_error(message: String) -> Response {
    (
        "500 Internal Server Error",
        "application/json; charset=utf-8",
        serde_json::json!({ "error": message })
            .to_string()
            .into_bytes(),
    )
}

/// POST /api/validation/run：对页面当前查看的比赛执行全部层判定。
///
/// 同步执行（1q 规模数十毫秒级、full 规模秒级；页面轮询 status 取
/// 结果）。artifacts 写入 .work/validation-view/（gitignored 运行目录）。
pub fn api_validation_run(_req: &crate::Request, validation: &SharedValidation) -> Response {
    // 取出缓存比赛并置 Running（阻塞后续重复启动）。
    let (seed, scope, ticks) = {
        let state = validation.lock().expect("validation state");
        let cached = match &*state {
            ValidationState::Idle { cached_game, .. }
            | ValidationState::Running { cached_game, .. }
            | ValidationState::Done { cached_game, .. }
            | ValidationState::Failed { cached_game, .. } => cached_game,
        };
        let Some(CachedGame { seed, scope, ticks }) = cached else {
            return bad_request(
                "no cached game to validate: run a simulation first (/api/simulate)".to_string(),
            );
        };
        (*seed, scope.clone(), ticks.clone())
    };
    let config = nba_validation::runner::ValidationRunConfig {
        seed,
        scope: scope.clone(),
        output_dir: nba_sim_core::params::workspace_validation_dir(),
        max_ticks: nba_validation::runner::DEFAULT_MAX_TICKS,
    };
    // 引擎账本的流尾未完成实例数：刻序列直接推进的会话用实例存储
    // 投影数与流尾部交叉核对（这里缓存刻来自已完成比赛，取投影值）。
    let (store_input, open_instances) = project_instance_store(&ticks);
    let run = match nba_validation::runner::run_validation_on_ticks(
        &config,
        ticks,
        open_instances.len() as u64,
    ) {
        Ok(run) => run,
        Err(error) => {
            *validation.lock().expect("validation state") = ValidationState::Failed {
                cached_game: None,
                error: error.clone(),
            };
            return internal_error(error);
        }
    };
    let artifact_error = write_artifacts(&run, &config.output_dir).err();
    let snapshot = build_snapshot(&run, &store_input, &open_instances);
    *validation.lock().expect("validation state") = ValidationState::Done {
        cached_game: Some(CachedGame {
            seed,
            scope,
            ticks: run.ticks.clone(),
        }),
        result: snapshot.clone(),
    };
    if let Some(error) = artifact_error {
        return internal_error(format!("artifacts write failed: {error}"));
    }
    let body = serde_json::json!({ "status": "done", "result": snapshot_json(&snapshot) });
    (
        "200 OK",
        "application/json; charset=utf-8",
        body.to_string().into_bytes(),
    )
}

/// GET /api/validation/layers：统计层与机制层判定工件（页面第 3/4/5 层详单）。
///
/// 工件由 `write_artifacts` 在 run 后写入 output_dir；本端点纯读取。
pub fn api_validation_layers(_req: &crate::Request) -> Response {
    let dir = nba_sim_core::params::workspace_validation_dir();
    let read = |name: &str| -> Value {
        std::fs::read_to_string(dir.join(name))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null)
    };
    let body = serde_json::json!({
        "statistical": read("statistical_results.json"),
        "mechanism": read("mechanism_results.json"),
    });
    ("200 OK", "application/json; charset=utf-8", body.to_string().into_bytes())
}

/// GET /api/validation/certification：批量认证总览（certification.json +
/// batch_statistical_results.json + branch_coverage.json 的合并视图）。
///
/// 工件由 CLI 批量判定跑（`validate --games N --out .work/validation-view`）
/// 写入 output_dir；本端点纯读取，工件缺失时字段为 null（页面显示
/// 「尚未运行批量判定」）。
pub fn api_validation_certification(_req: &crate::Request) -> Response {
    let dir = nba_sim_core::params::workspace_validation_dir();
    let read = |name: &str| -> Value {
        std::fs::read_to_string(dir.join(name))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null)
    };
    let body = serde_json::json!({
        "certification": read("certification.json"),
        "batch": read("batch_statistical_results.json"),
        "branch_coverage": read("branch_coverage.json"),
    });
    ("200 OK", "application/json; charset=utf-8", body.to_string().into_bytes())
}

/// GET /api/validation/status：轻量轮询（状态 + 顶层证据 + 退出码）。
pub fn api_validation_status(validation: &SharedValidation) -> Response {
    let state = validation.lock().expect("validation state");
    let body = match &*state {
        ValidationState::Idle { last_result, .. } => serde_json::json!({
            "status": "idle",
            "last_result": last_result.as_ref().map(snapshot_json),
        }),
        ValidationState::Running { stage, .. } => serde_json::json!({
            "status": "running",
            "stage": stage,
        }),
        ValidationState::Done { result, .. } => serde_json::json!({
            "status": "done",
            "result": snapshot_json(result),
        }),
        ValidationState::Failed { error, .. } => serde_json::json!({
            "status": "failed",
            "error": error,
        }),
    };
    (
        "200 OK",
        "application/json; charset=utf-8",
        body.to_string().into_bytes(),
    )
}

/// GET /api/validation/report：逐场景裁决与账本对账（页面详单）。
pub fn api_validation_report(validation: &SharedValidation) -> Response {
    let state = validation.lock().expect("validation state");
    let result = match &*state {
        ValidationState::Done { result, .. } => result,
        ValidationState::Idle { last_result, .. }
        | ValidationState::Running { last_result, .. } => match last_result {
            Some(result) => result,
            None => {
                return bad_request("validation has not run yet".to_string());
            }
        },
        ValidationState::Failed { error, .. } => {
            return internal_error(error.clone());
        }
    };
    let scenarios: Vec<Value> = result
        .scenario_results
        .iter()
        .map(|outcome| {
            serde_json::json!({
                "scenario_id": outcome.scenario_id,
                "verdict": outcome.verdict,
                "trigger_occurrences": outcome.trigger_occurrences,
                "missing_required": outcome.missing_required,
                "forbidden_seen": outcome.forbidden_seen,
            })
        })
        .collect();
    let ledger = &result.ledger;
    let body = serde_json::json!({
        "exit_code": result.exit_code,
        "evidence": result.evidence,
        "scenarios": scenarios,
        "ledger": {
            "box_score_rebuilt": ledger.box_score_rebuilt,
            "box_score_declared": ledger.box_score_declared,
            "possession_rebuilt": ledger.possession.rebuilt_count,
            "possession_engine": ledger.possession.engine_count,
            "terminal_mismatches": ledger.possession.terminal_mismatches,
            "pairing": ledger.pairing,
            "clock": ledger.clock,
        },
        "ticks": result.ticks,
        "events": result.events,
        "open_business_instances": result.open_business_instances,
        "open_instances_from_store": result.open_instances_from_store,
        "insufficient_reasons": result.insufficient_reasons,
    });
    (
        "200 OK",
        "application/json; charset=utf-8",
        body.to_string().into_bytes(),
    )
}

fn build_snapshot(
    run: &nba_validation::runner::ValidationRun,
    _store_input: &nba_validation::artifacts::InstanceStoreInput,
    open_from_store: &[String],
) -> ValidationSnapshot {
    let (rebuilt, declared) = RebuiltBoxScore::rebuild(&run.ticks);
    let clock = RebuiltClockAccount::rebuild(&run.ticks);
    let ledger = LedgerComparison {
        box_score_rebuilt: rebuilt,
        box_score_declared: declared,
        possession: run.possessions.clone(),
        clock,
        pairing: PairingAudit {
            shots_without_release: run.pairing.shots_without_release,
            passes_without_release: run.pairing.passes_without_release,
            free_throws_without_award: run.pairing.free_throws_without_award,
        },
    };
    let mut insufficient_reasons = Vec::new();
    for outcome in &run.scenario_outcomes {
        if outcome.verdict == nba_validation::Verdict::InsufficientEvidence {
            insufficient_reasons.push(format!("{}: 未触发必需分支", outcome.scenario_id));
        }
    }
    let fact = &run.fact_report.evidence;
    if fact.insufficient > 0 {
        insufficient_reasons.push(format!(
            "事实层 {}/{} 项证据不足",
            fact.insufficient, fact.opportunities
        ));
    }
    ValidationSnapshot {
        exit_code: run.exit_code() as u8,
        evidence: run.evidence.clone(),
        scenario_results: run.scenario_outcomes.clone(),
        ticks: run.trailer.records,
        events: run.trailer.events,
        open_business_instances: run.trailer.open_business_instances,
        open_instances_from_store: open_from_store.len() as u64,
        insufficient_reasons,
        ledger,
    }
}

fn snapshot_json(snapshot: &ValidationSnapshot) -> Value {
    serde_json::json!({
        "exit_code": snapshot.exit_code,
        "evidence": snapshot.evidence,
        "ticks": snapshot.ticks,
        "events": snapshot.events,
        "open_business_instances": snapshot.open_business_instances,
        "open_instances_from_store": snapshot.open_instances_from_store,
        "insufficient_reasons": snapshot.insufficient_reasons,
    })
}
