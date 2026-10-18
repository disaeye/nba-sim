//! 验证面板的 HTTP 入口。
//!
//! 判定由动作链内核的验证模块产出：面板对缓存种子跑一场判定，分层详单与
//! 认证总览读 `validate-core` 写入的工件。本模块不构造引擎，也不保存第二份
//! 比赛状态。

use std::sync::{Arc, Mutex};

use nba_sim_core::params::workspace_validation_dir;
use nba_validation::core_validation::{
    run_core_validation_scoped, CoreValidationReport, ValidationScope, REGISTERED_GATE_CRITERIA,
};
use serde_json::Value;

type Response = (&'static str, &'static str, Vec<u8>);

/// 页面当前查看的比赛：验证入口对同一个种子重放。
pub struct CachedGame {
    pub seed: u64,
    pub scope: String,
}

/// 面板状态：缓存比赛与最近一次判定（快照供状态轮询，详单供账本对账）。
pub enum ValidationState {
    Idle {
        cached_game: Option<CachedGame>,
        last_snapshot: Option<Value>,
        last_report: Option<Value>,
    },
}

pub type SharedValidation = Arc<Mutex<ValidationState>>;

fn json_response(value: &Value) -> Response {
    (
        "200 OK",
        "application/json; charset=utf-8",
        value.to_string().into_bytes(),
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

fn bad_request(message: String) -> Response {
    (
        "400 Bad Request",
        "application/json; charset=utf-8",
        serde_json::json!({ "error": message })
            .to_string()
            .into_bytes(),
    )
}

/// POST /api/validation/run：对页面正在查看的种子跑一场判定，并缓存结果。
pub fn api_validation_run(_req: &crate::Request, validation: &SharedValidation) -> Response {
    let Some(seed) = cached_seed(validation) else {
        return no_cached_game();
    };
    let scope = cached_scope(validation).unwrap_or_else(|| "full".to_string());
    let report = match run_core_validation_scoped(seed, 1, ValidationScope::parse(&scope)) {
        Ok(report) => report,
        Err(error) => return internal_error(error),
    };
    let snapshot = snapshot_json(&report);
    let detail = report_json(&report);
    {
        let mut state = validation.lock().expect("validation state");
        if let ValidationState::Idle {
            last_snapshot,
            last_report,
            ..
        } = &mut *state
        {
            *last_snapshot = Some(snapshot.clone());
            *last_report = Some(detail);
        }
    }
    json_response(&serde_json::json!({ "result": snapshot }))
}

/// GET /api/validation/status：最近一次判定的摘要。
pub fn api_validation_status(validation: &SharedValidation) -> Response {
    let state = validation.lock().expect("validation state");
    let body = match &*state {
        ValidationState::Idle { last_snapshot, .. } => match last_snapshot {
            Some(result) => serde_json::json!({ "status": "done", "result": result }),
            None => serde_json::json!({ "status": "idle" }),
        },
    };
    json_response(&body)
}

/// GET /api/validation/report：上一次判定的逐场账本详单。
pub fn api_validation_report(validation: &SharedValidation) -> Response {
    let state = validation.lock().expect("validation state");
    match &*state {
        ValidationState::Idle { last_report, .. } => match last_report {
            Some(report) => json_response(report),
            None => {
                bad_request("no validation result: run a simulation and validate first".to_string())
            }
        },
    }
}

/// GET /api/validation/layers：批次统计与机制实验工件。
pub fn api_validation_layers(_req: &crate::Request) -> Response {
    let dir = workspace_validation_dir();
    let read = |name: &str| -> Value {
        std::fs::read_to_string(dir.join(name))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null)
    };
    json_response(&serde_json::json!({
        "statistical": read("batch_statistical_results.json"),
        "mechanism": read("mechanism_core.json"),
    }))
}

/// GET /api/validation/certification：批次判定总览。
pub fn api_validation_certification(_req: &crate::Request) -> Response {
    let dir = workspace_validation_dir();
    let read = |name: &str| -> Value {
        std::fs::read_to_string(dir.join(name))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null)
    };
    json_response(&serde_json::json!({
        "certification": read("certification.json"),
        "batch": read("batch_statistical_results.json"),
        "branch_coverage": read("branch_coverage.json"),
    }))
}

fn snapshot_json(report: &CoreValidationReport) -> Value {
    let evaluated = report.gate.layers.len();
    let passes = report
        .gate
        .layers
        .iter()
        .filter(|layer| layer.passed)
        .count();
    let mut defects = evaluated - passes + report.ledger_failures;
    if report.geometry_violations > 0 {
        defects += 1;
    }
    if report.context_violations > 0 {
        defects += 1;
    }
    let exit_code = if defects > 0 { 1 } else { 0 };
    serde_json::json!({
        "exit_code": exit_code,
        "evidence": {
            "passes": passes,
            "opportunities": evaluated,
            "defects": defects,
        },
        "ticks": report.ticks,
        "events": report.events,
        "open_business_instances": report.open_business_instances,
        "failures": report.failed_criteria(),
        "games": report.games.len(),
        "scope": report.scope,
        "distribution_applicable": report.distribution_applicable,
        "registered_gate_criteria": REGISTERED_GATE_CRITERIA.len(),
    })
}

fn report_json(report: &CoreValidationReport) -> Value {
    let games: Vec<Value> = report
        .games
        .iter()
        .map(|game| {
            serde_json::json!({
                "seed": game.seed,
                "home_score": game.home_score,
                "away_score": game.away_score,
                "possessions": game.possessions,
                "ticks": game.ticks,
                "events": game.events,
                "open_business_instances": game.open_business_instances,
                "ledger": game.ledger,
                "violations": game.violations,
            })
        })
        .collect();
    serde_json::json!({
        "games": games,
        "gate": report.gate,
    })
}

/// 缓存页面正在查看的比赛种子，供 `/api/validation/run` 重放。
pub fn cache_game(validation: &SharedValidation, seed: u64, scope: &str) {
    let mut state = validation.lock().expect("validation state");
    if let ValidationState::Idle { cached_game, .. } = &mut *state {
        *cached_game = Some(CachedGame {
            seed,
            scope: scope.to_string(),
        });
    }
}

pub fn idle_state() -> ValidationState {
    ValidationState::Idle {
        cached_game: None,
        last_snapshot: None,
        last_report: None,
    }
}

pub fn cached_scope(validation: &SharedValidation) -> Option<String> {
    let state = validation.lock().expect("validation state");
    match &*state {
        ValidationState::Idle { cached_game, .. } => {
            cached_game.as_ref().map(|game| game.scope.clone())
        }
    }
}

pub fn cached_seed(validation: &SharedValidation) -> Option<u64> {
    let state = validation.lock().expect("validation state");
    match &*state {
        ValidationState::Idle { cached_game, .. } => cached_game.as_ref().map(|game| game.seed),
    }
}

pub fn no_cached_game() -> Response {
    bad_request("no cached game to validate: run a simulation first (/api/simulate)".to_string())
}
