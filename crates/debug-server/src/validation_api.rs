//! 验证面板的 HTTP 入口。
//!
//! 判定由动作链内核的验证模块产出：单场批次走 `core_validation`，
//! 分层详单读 `validate-core` 写入的工件。本模块不构造引擎，也不保存
//! 第二份比赛状态。

use std::sync::{Arc, Mutex};

use nba_sim_core::params::workspace_validation_dir;
use nba_validation::core_validation::{run_core_validation, CoreValidationReport};
use serde_json::Value;

type Response = (&'static str, &'static str, Vec<u8>);

/// 页面当前查看的比赛：验证入口对同一个种子重放。
pub struct CachedGame {
    pub seed: u64,
    pub scope: String,
}

/// 面板状态：只保留缓存比赛与最近一次结果，判定本身是同步一次完成。
pub enum ValidationState {
    Idle {
        cached_game: Option<CachedGame>,
        last_result: Option<Value>,
    },
}

pub type SharedValidation = Arc<Mutex<ValidationState>>;

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

fn json_response(value: &Value) -> Response {
    (
        "200 OK",
        "application/json; charset=utf-8",
        value.to_string().into_bytes(),
    )
}

/// POST /api/validation/run：对页面正在查看的种子跑一小批判定。
pub fn api_validation_run(_req: &crate::Request, validation: &SharedValidation) -> Response {
    let Some(seed) = cached_seed(validation) else {
        return no_cached_game();
    };
    let report = match run_core_validation(seed, 8) {
        Ok(report) => report,
        Err(error) => return internal_error(error),
    };
    let snapshot = snapshot_json(&report);
    {
        let mut state = validation.lock().expect("validation state");
        if let ValidationState::Idle { last_result, .. } = &mut *state {
            *last_result = Some(snapshot.clone());
        }
    }
    json_response(&serde_json::json!({ "result": snapshot }))
}

/// GET /api/validation/status：最近的判定结果。
pub fn api_validation_status(validation: &SharedValidation) -> Response {
    let state = validation.lock().expect("validation state");
    let body = match &*state {
        ValidationState::Idle { last_result, .. } => match last_result {
            Some(result) => serde_json::json!({
                "status": "done",
                "result": result,
            }),
            None => serde_json::json!({ "status": "idle" }),
        },
    };
    json_response(&body)
}

/// GET /api/validation/report：逐场账本对账详单。
pub fn api_validation_report(validation: &SharedValidation) -> Response {
    let Some(seed) = cached_seed(validation) else {
        return no_cached_game();
    };
    let report = match run_core_validation(seed, 8) {
        Ok(report) => report,
        Err(error) => return internal_error(error),
    };
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
    json_response(&serde_json::json!({
        "seed": seed,
        "games": games,
        "gate": report.gate,
    }))
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
    let opportunities = report.gate.layers.len();
    let passes = report
        .gate
        .layers
        .iter()
        .filter(|layer| layer.passed)
        .count();
    let mut defects = opportunities - passes + report.ledger_failures;
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
            "opportunities": opportunities,
            "defects": defects,
            "insufficient": 0,
        },
        "ticks": report.ticks,
        "events": report.events,
        "open_business_instances": report.open_business_instances,
        "insufficient_reasons": report.failed_criteria(),
        "games": report.games.len(),
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
        last_result: None,
    }
}

pub fn cached_seed(validation: &SharedValidation) -> Option<u64> {
    let state = validation.lock().expect("validation state");
    match &*state {
        ValidationState::Idle { cached_game, .. } => cached_game.as_ref().map(|game| game.seed),
    }
}

pub fn is_configured(validation: &SharedValidation) -> bool {
    cached_seed(validation).is_some()
}

pub fn no_cached_game() -> Response {
    bad_request("no cached game to validate: run a simulation first (/api/simulate)".to_string())
}
