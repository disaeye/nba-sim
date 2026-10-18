//! 动作链内核的 HTTP 接口。
//!
//! 与旧的 `/api/session/*` 并存，供前端逐段切换。会话命令、快照与事件
//! 都来自 [`SessionService`]，服务端不保存第二份比赛状态。

use std::sync::{Arc, Mutex};

use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::SessionService;

use crate::{bad_request, conflict, internal_error, query_u64, Request, Response};

pub(crate) type SharedCore = Arc<Mutex<SessionService>>;

fn lock(core: &SharedCore) -> Result<std::sync::MutexGuard<'_, SessionService>, Response> {
    core.lock()
        .map_err(|_| internal_error("core session lock poisoned".to_string()))
}

fn state_response(service: &SessionService) -> Response {
    let body = serde_json::json!({
        "state": service.state().as_str(),
        "configured": service.is_configured(),
        "seed": service.seed(),
    });
    (
        "200 OK",
        "application/json; charset=utf-8",
        body.to_string().into_bytes(),
    )
}

pub(crate) fn api_core_state(core: &SharedCore) -> Response {
    let service = match lock(core) {
        Ok(service) => service,
        Err(response) => return response,
    };
    state_response(&service)
}

pub(crate) fn api_core_snapshot(core: &SharedCore) -> Response {
    let service = match lock(core) {
        Ok(service) => service,
        Err(response) => return response,
    };
    match service.snapshot() {
        Some(tick) => json_response(&tick),
        None => conflict("cannot read snapshot before match setup".to_string()),
    }
}

pub(crate) fn api_core_events(req: &Request, core: &SharedCore) -> Response {
    let limit = match query_u64(&req.query, "limit") {
        Ok(limit) => {
            if limit == 0 {
                200
            } else {
                limit.min(5_000) as usize
            }
        }
        Err(error) => return bad_request(error),
    };
    let service = match lock(core) {
        Ok(service) => service,
        Err(response) => return response,
    };
    json_response(&service.events(limit))
}

pub(crate) fn api_core_setup(req: &Request, core: &SharedCore) -> Response {
    let seed = if req.body.is_empty() {
        None
    } else {
        let value: serde_json::Value = match serde_json::from_slice(&req.body) {
            Ok(value) => value,
            Err(error) => return bad_request(format!("setup body must be JSON: {error}")),
        };
        value.get("seed").and_then(|seed| seed.as_u64())
    };
    let mut service = match lock(core) {
        Ok(service) => service,
        Err(response) => return response,
    };
    let seed = seed.unwrap_or_else(|| service.seed());
    match service.setup_from_calibration(&workspace_calibration_dir(), seed) {
        Ok(info) => json_response(&info),
        Err(error) => internal_error(error),
    }
}

pub(crate) fn api_core_command(core: &SharedCore, command: &str, req: &Request) -> Response {
    let seconds = if req.body.is_empty() {
        0.0
    } else {
        let value: serde_json::Value = match serde_json::from_slice(&req.body) {
            Ok(value) => value,
            Err(error) => return bad_request(format!("command body must be JSON: {error}")),
        };
        let raw = ["seconds", "dt", "duration"]
            .iter()
            .find_map(|key| value.get(*key))
            .and_then(|seconds| seconds.as_f64())
            .unwrap_or(0.0);
        if !raw.is_finite() || raw < 0.0 {
            return bad_request("duration must be finite and non-negative".to_string());
        }
        raw
    };
    let mut service = match lock(core) {
        Ok(service) => service,
        Err(response) => return response,
    };
    if !service.is_configured() {
        let seed = service.seed();
        if let Err(error) =
            service.setup_from_calibration(&workspace_calibration_dir(), seed)
        {
            return internal_error(error);
        }
    }
    let result = match command {
        "start" => service.start(),
        "pause" => {
            service.pause();
            Ok(())
        }
        "resume" => service.resume(),
        "tick" => service.tick(if seconds > 0.0 { seconds } else { 0.05 }).map(|_| ()),
        "fast-forward" => service.fast_forward(seconds).map(|_| ()),
        "next-possession" => service.next_possession().map(|_| ()),
        other => Err(format!("unknown core session command `{other}`")),
    };
    if let Err(error) = result {
        return bad_request(error);
    }
    match command {
        "start" | "pause" | "resume" => state_response(&service),
        _ => match service.snapshot() {
            Some(tick) => json_response(&tick),
            None => conflict("cannot read snapshot before match setup".to_string()),
        },
    }
}

/// 一次性推演：按种子与回合数跑到边界，返回末帧与比赛报告摘要。
pub(crate) fn api_core_simulate(req: &Request) -> Response {
    let seed = match query_u64(&req.query, "seed") {
        Ok(seed) if seed > 0 => seed,
        Ok(_) => 42,
        Err(error) => return bad_request(error),
    };
    let possessions = match query_u64(&req.query, "possessions") {
        Ok(possessions) if possessions > 0 => possessions.clamp(1, 400) as u32,
        Ok(_) => 20,
        Err(error) => return bad_request(error),
    };
    let mut service = SessionService::new(seed);
    if let Err(error) = service.setup_from_calibration(&workspace_calibration_dir(), seed) {
        return internal_error(error);
    }
    let tick = match service.run_to_possessions(possessions) {
        Ok(tick) => tick,
        Err(error) => return internal_error(error),
    };
    let report = service.report();
    let summary = report.as_ref().map(|report| {
        let home = report.box_score.team_totals(nba_sim_core::TeamSide::Home);
        let away = report.box_score.team_totals(nba_sim_core::TeamSide::Away);
        serde_json::json!({
            "home_score": report.home_score,
            "away_score": report.away_score,
            "possessions": report.possessions.len(),
            "home": home,
            "away": away,
            "violations": report.violations,
        })
    });
    let body = serde_json::json!({
        "tick": tick,
        "report": summary,
    });
    (
        "200 OK",
        "application/json; charset=utf-8",
        body.to_string().into_bytes(),
    )
}

/// 按范围跑一次新内核并返回 NDJSON 帧流。
///
/// 范围口径与旧入口一致：`<n>p` 表示回合数，`1q` 表示一节，`full` 表示整场。
pub(crate) fn run_core_simulation(seed: u64, scope: &str) -> Result<(Vec<u8>, usize), String> {
    use nba_sim_core::{MatchSetup, Params, World};

    let params = Params::load(&workspace_calibration_dir())?;
    let setup = MatchSetup::builtin(nba_domain::GameRules::default());
    let mut world = World::new(setup, params, seed);
    let possession_target = match scope {
        "full" => None,
        "1q" | "2q" | "3q" | "4q" => None,
        other => Some(
            other
                .trim_end_matches('p')
                .parse::<u32>()
                .map_err(|_| format!("unknown scope `{other}`"))?,
        ),
    };
    let quarter_target = match scope {
        "1q" => Some(2),
        "2q" => Some(3),
        "3q" => Some(4),
        "4q" => None,
        _ => None,
    };

    let mut body = Vec::new();
    let mut ticks = 0usize;
    while !world.finished {
        world.step(nba_sim_core::world::TICK_SECONDS);
        let tick = world.stream_tick();
        let line = serde_json::to_string(&tick).map_err(|error| error.to_string())?;
        body.extend_from_slice(line.as_bytes());
        body.push(b'\n');
        ticks += 1;
        let reached_possessions = possession_target
            .map(|target| world.possession_log.len() as u32 >= target)
            .unwrap_or(false);
        let reached_quarter = quarter_target
            .map(|target| world.clock.period >= target)
            .unwrap_or(false);
        if reached_possessions || reached_quarter {
            break;
        }
    }
    Ok((body, ticks))
}

pub(crate) fn core_simulate_response(seed: u64, scope: &str) -> Response {
    match run_core_simulation(seed, scope) {
        Ok((body, _ticks)) => ("200 OK", "application/x-ndjson; charset=utf-8", body),
        Err(error) => internal_error(error),
    }
}

pub(crate) fn route_core(req: &Request, core: &SharedCore) -> Option<Response> {
    let response = match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/api/core/simulate") | ("POST", "/api/core/simulate") => {
            api_core_simulate(req)
        }
        ("GET", "/api/core/state") => api_core_state(core),
        ("GET", "/api/core/snapshot") => api_core_snapshot(core),
        ("GET", "/api/core/events") => api_core_events(req, core),
        ("POST", "/api/core/setup") => api_core_setup(req, core),
        ("POST", "/api/core/start") => api_core_command(core, "start", req),
        ("POST", "/api/core/pause") => api_core_command(core, "pause", req),
        ("POST", "/api/core/resume") => api_core_command(core, "resume", req),
        ("POST", "/api/core/tick") => api_core_command(core, "tick", req),
        ("POST", "/api/core/fast-forward") | ("POST", "/api/core/fast_forward") => {
            api_core_command(core, "fast-forward", req)
        }
        ("POST", "/api/core/next-possession") | ("POST", "/api/core/next_possession") => {
            api_core_command(core, "next-possession", req)
        }
        _ => return None,
    };
    Some(response)
}

fn json_response<T: serde::Serialize>(value: &T) -> Response {
    match serde_json::to_vec(value) {
        Ok(body) => ("200 OK", "application/json; charset=utf-8", body),
        Err(error) => internal_error(format!("serialization failed: {error}")),
    }
}
