//! 检查当前回放已经产生的比赛事实与账本。

use std::sync::{Arc, Mutex};

use nba_sim_core::GameReport;
use nba_validation::core_validation::check_ledger_scoped;
use serde_json::{json, Value};

use crate::{bad_request, Request, Response};

struct CachedGame {
    summary: Value,
    detail: Value,
}

#[derive(Default)]
pub struct ValidationState {
    cached_game: Option<CachedGame>,
    shown: bool,
}

pub type SharedValidation = Arc<Mutex<ValidationState>>;

fn json_response(value: &Value) -> Response {
    ("200 OK", "application/json; charset=utf-8", value.to_string().into_bytes())
}

pub fn api_validation_run(_req: &Request, validation: &SharedValidation) -> Response {
    let mut state = validation.lock().expect("validation state");
    let Some(game) = &state.cached_game else {
        return bad_request("no cached replay evidence (/api/simulate)".to_string());
    };
    let result = json_response(&json!({ "result": game.summary }));
    state.shown = true;
    result
}

pub fn api_validation_status(validation: &SharedValidation) -> Response {
    let state = validation.lock().expect("validation state");
    match state.cached_game.as_ref().filter(|_| state.shown) {
        Some(game) => json_response(&json!({ "status": "done", "result": game.summary })),
        None => json_response(&json!({ "status": "idle" })),
    }
}

pub fn api_validation_report(validation: &SharedValidation) -> Response {
    let state = validation.lock().expect("validation state");
    match state.cached_game.as_ref().filter(|_| state.shown) {
        Some(game) => json_response(&game.detail),
        None => bad_request("no report for the current replay".to_string()),
    }
}

pub fn cache_game(validation: &SharedValidation, scope: &str, report: GameReport, finished: bool) {
    let ledger = check_ledger_scoped(report.seed, &report, finished);
    let ledger_failures = usize::from(!ledger.passed());
    let geometry = report.violations.iter().filter(|item| item.starts_with("geometry:")).count();
    let context = report.violations.iter().filter(|item| item.starts_with("context:")).count();
    let mut failures = report.violations.clone();
    if ledger_failures != 0 {
        failures.push(ledger.detail.clone());
    }
    let mut state = validation.lock().expect("validation state");
    state.shown = false;
    state.cached_game = Some(CachedGame {
        summary: json!({
            "exit_code": u8::from(!failures.is_empty()),
            "geometry_violations": geometry,
            "context_violations": context,
            "ledger_failures": ledger_failures,
            "ticks": report.ticks,
            "events": report.events.len(),
            "open_business_instances": report.open_business_instances,
            "failures": failures,
            "games": 1,
            "scope": scope,
        }),
        detail: json!({ "games": [{
            "seed": report.seed,
            "home_score": report.home_score,
            "away_score": report.away_score,
            "possessions": report.possessions.len(),
            "ticks": report.ticks,
            "events": report.events.len(),
            "open_business_instances": report.open_business_instances,
            "ledger": ledger,
            "violations": report.violations,
        }] }),
    });
}

pub fn idle_state() -> ValidationState {
    ValidationState::default()
}
