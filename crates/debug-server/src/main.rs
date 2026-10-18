//! 开发调试服务器：内嵌静态页 + 模拟流/规则编辑/五层验证 HTTP API。
//!
//! 只服务于本地开发调试（P0–P2 调试工作台），不是生产服务。
//! std-only：手写最小 HTTP/1.1 解析，避免引入 web 框架依赖。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use nba_domain::GameRules;
use nba_engine::MatchEngine;
use nba_sim_core::SessionService;
use nba_invariants::Violation;
use nba_protocol::StreamTick;
use serde_json::Value;

mod core_api;
mod static_page;
mod validation_api;

use core_api::SharedCore;

use validation_api::{
    api_validation_certification, api_validation_layers, api_validation_report,
    api_validation_run, api_validation_status, cache_game, idle_state, SharedValidation,
};

type Response = (&'static str, &'static str, Vec<u8>);

const MAX_REQUEST_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_SIMULATION_TICKS: usize = 500_000;
#[allow(clippy::arc_with_non_send_sync)]
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--write-studio-asset") {
        let body = studio_catalog().unwrap_or_else(|error| panic!("studio catalog: {error}"));
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("static/studio.json");
        std::fs::write(path, body).unwrap_or_else(|error| panic!("studio asset: {error}"));
        return;
    }
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(31100);
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind debug server port");
    let core: SharedCore = Arc::new(Mutex::new(SessionService::new(42)));
    let validation: SharedValidation = Arc::new(Mutex::new(idle_state()));
    println!("nba-debug-server listening on http://127.0.0.1:{}", port);
    for stream in listener.incoming().flatten() {
        // MatchService currently contains a backend trait object without a Send
        // bound. Keep ownership on this accept thread until the engine boundary
        // opts into cross-thread execution; each request still locks the session
        // so route code cannot alias mutable simulation state.
        if let Err(e) = handle(stream, &core, &validation) {
            eprintln!("request error: {}", e);
        }
    }
}

fn handle(
    mut stream: TcpStream,
    core: &SharedCore,
    validation: &SharedValidation,
) -> std::io::Result<()> {
    let req = match read_request(&mut stream) {
        Ok(Some(r)) => r,
        Ok(None) => return Ok(()),
        Err(e) => {
            write_response(
                &mut stream,
                "400 Bad Request",
                "application/json; charset=utf-8",
                format!(r#"{{"error":{}}}"#, serde_json::json!(e.to_string())).as_bytes(),
            )?;
            return Ok(());
        }
    };
    let (status, content_type, body) = route(&req, core, validation);
    write_response(&mut stream, status, content_type, &body)
}

pub(crate) struct Request {
    method: String,
    path: String,
    query: String,
    body: Vec<u8>,
}

fn write_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\n\r\n",
        status,
        content_type,
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<Request>> {
    // A cloned socket shares the receive queue with the original socket. Keeping
    // parsing in a buffered reader prevents partial header reads without adding
    // a third-party HTTP dependency; the original socket remains for the reply.
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_ascii_uppercase();
    let target = parts.next().unwrap_or("/").to_string();
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target, String::new()),
    };

    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        let trimmed = header.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse::<usize>().map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid content-length")
                })?;
            }
        }
    }
    if content_length > MAX_REQUEST_BODY_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "request body too large",
        ));
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }
    Ok(Some(Request {
        method,
        path,
        query,
        body,
    }))
}

fn route(
    req: &Request,
    core: &SharedCore,
    validation: &SharedValidation,
) -> Response {
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => (
            "200 OK",
            "text/html; charset=utf-8",
            static_page::HTML.as_bytes().to_vec(),
        ),
        ("GET", "/style.css") => (
            "200 OK",
            "text/css; charset=utf-8",
            static_page::CSS.as_bytes().to_vec(),
        ),
        ("GET", "/app.js") => (
            "200 OK",
            "application/javascript; charset=utf-8",
            static_page::JS.as_bytes().to_vec(),
        ),
        ("GET", "/wasm/nba_wasm.js") => (
            "200 OK",
            "application/javascript; charset=utf-8",
            static_page::WASM_JS.as_bytes().to_vec(),
        ),
        ("GET", "/wasm/nba_wasm_bg.wasm") => (
            "200 OK",
            "application/wasm",
            static_page::WASM_BIN.to_vec(),
        ),
        ("GET", "/api/rules") => match serde_json::to_vec_pretty(&GameRules::default()) {
            Ok(json) => ("200 OK", "application/json; charset=utf-8", json),
            Err(e) => internal_error(e.to_string()),
        },
        ("GET", "/api/studio") => api_studio(),
        ("GET", "/api/simulate") | ("POST", "/api/simulate") => api_simulate(req, validation),
        ("POST", "/api/session/setup") => core_api::api_core_setup(req, core),
        ("GET", "/api/session") | ("GET", "/api/session/state") => {
            core_api::api_core_state(core)
        }
        ("GET", "/api/session/snapshot") => core_api::api_core_snapshot(core),
        ("GET", "/api/session/events") => core_api::api_core_events(req, core),
        ("POST", "/api/session/start") => core_api::api_core_command(core, "start", req),
        ("POST", "/api/session/pause") => core_api::api_core_command(core, "pause", req),
        ("POST", "/api/session/resume") => core_api::api_core_command(core, "resume", req),
        ("POST", "/api/session/tick") => core_api::api_core_command(core, "tick", req),
        ("POST", "/api/session/fast-forward") | ("POST", "/api/session/fast_forward") => {
            core_api::api_core_command(core, "fast-forward", req)
        }
        ("POST", "/api/session/next-possession") | ("POST", "/api/session/next_possession") => {
            core_api::api_core_command(core, "next-possession", req)
        }
        ("POST", "/api/validation/run") => api_validation_run(req, validation),        ("GET", "/api/validation/status") => api_validation_status(validation),
        ("GET", "/api/validation/report") => api_validation_report(validation),
        ("GET", "/api/validation/layers") => api_validation_layers(req),
        ("GET", "/api/validation/certification") => api_validation_certification(req),
        ("OPTIONS", _) => ("204 No Content", "text/plain; charset=utf-8", Vec::new()),
        _ => {
            if req.path.starts_with("/api/core/") {
                if let Some(response) = core_api::route_core(req, core) {
                    return response;
                }
            }
            (
                "404 Not Found",
                "text/plain; charset=utf-8",
                b"not found".to_vec(),
            )
        }
    }
}

pub(crate) fn internal_error(msg: String) -> Response {
    (
        "500 Internal Server Error",
        "application/json; charset=utf-8",
        serde_json::json!({ "error": msg }).to_string().into_bytes(),
    )
}

pub(crate) fn conflict(msg: String) -> Response {
    (
        "409 Conflict",
        "application/json; charset=utf-8",
        serde_json::json!({ "error": msg }).to_string().into_bytes(),
    )
}

pub(crate) fn query_u64(query: &str, expected_key: &str) -> Result<u64, String> {
    let mut result = 0;
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = match pair.split_once('=') {
            Some(pair) => pair,
            None => continue,
        };
        if key == expected_key {
            let value = url_decode(value)?;
            result = value
                .parse::<u64>()
                .map_err(|_| format!("{expected_key} must be an unsigned integer"))?;
        }
    }
    Ok(result)
}

pub(crate) fn bad_request(msg: String) -> (&'static str, &'static str, Vec<u8>) {
    (
        "400 Bad Request",
        "application/json; charset=utf-8",
        serde_json::json!({ "error": msg }).to_string().into_bytes(),
    )
}

fn api_simulate(
    req: &Request,
    validation: &SharedValidation,
) -> (&'static str, &'static str, Vec<u8>) {
    let run = match parse_run_request(req) {
        Ok(run) => run,
        Err(e) => return bad_request(e),
    };
    if run.engine == "core" {
        cache_game(validation, run.seed, &run.scope);
        return core_api::core_simulate_response(run.seed, &run.scope);
    }
    match run_simulation(run.seed, &run.scope, run.rules, run.setup) {
        Ok((body, ticks)) => {
            // 缓存页面正在查看的这场比赛：验证入口对同一种子重放。
            cache_game(validation, run.seed, &run.scope);
            let _ = ticks;
            ("200 OK", "application/x-ndjson; charset=utf-8", body)
        }
        Err(e) => bad_request(e),
    }
}

struct RunRequest {
    seed: u64,
    scope: String,
    engine: String,
    rules: GameRules,
    setup: Option<nba_engine::MatchSetup>,
}

fn parse_run_request(req: &Request) -> Result<RunRequest, String> {
    let defaults = GameRules::default();
    let mut seed = 42u64;
    let mut scope = "5p".to_string();
    let mut engine = "core".to_string();
    let mut rules_value: Option<Value> = None;
    let mut setup_value: Option<Value> = None;

    if req.method == "POST" {
        let value: Value = serde_json::from_slice(&req.body)
            .map_err(|e| format!("request JSON invalid: {}", e))?;
        if let Some(v) = value.get("seed") {
            seed = v
                .as_u64()
                .ok_or_else(|| "seed must be an unsigned integer".to_string())?;
        }
        if let Some(v) = value.get("scope") {
            scope = v
                .as_str()
                .ok_or_else(|| "scope must be a string".to_string())?
                .to_string();
        }
        rules_value = value.get("rules").cloned();
        setup_value = value.get("setup").cloned();
        if let Some(v) = value.get("engine") {
            engine = v
                .as_str()
                .ok_or_else(|| "engine must be a string".to_string())?
                .to_string();
        }
    } else {
        for pair in req.query.split('&').filter(|s| !s.is_empty()) {
            let (key, value) = match pair.split_once('=') {
                Some(pair) => pair,
                None => continue,
            };
            let value = url_decode(value)?;
            match key {
                "seed" => {
                    seed = value
                        .parse()
                        .map_err(|_| "seed must be an unsigned integer".to_string())?
                }
                "scope" => scope = value,
                "engine" => engine = value,
                "rules" => {
                    rules_value = Some(
                        serde_json::from_str(&value)
                            .map_err(|e| format!("rules JSON invalid: {}", e))?,
                    )
                }
                _ => {}
            }
        }
    }

    let rules = match rules_value {
        None | Some(Value::Null) => defaults,
        Some(Value::String(json)) => serde_json::from_str::<GameRules>(&json)
            .map_err(|e| format!("rules JSON invalid: {}", e))?,
        Some(value) => serde_json::from_value::<GameRules>(value)
            .map_err(|e| format!("rules object invalid: {}", e))?,
    };
    validate_rules(&rules)?;
    let setup = match setup_value {
        None | Some(Value::Null) => None,
        Some(value) => {
            let mut setup = serde_json::from_value::<nba_engine::MatchSetup>(value)
                .map_err(|error| format!("match setup invalid: {error}"))?;
            setup.rules = rules.clone();
            let all_plays = play_catalog();
            if setup.home_playbook.is_empty() {
                setup.home_playbook =
                    plays_for_tactic(&setup.home_lineup.offense_tactic, &all_plays);
            } else {
                retain_compatible_plays(
                    &mut setup.home_playbook,
                    &setup.home_lineup.offense_tactic,
                );
            }
            if setup.away_playbook.is_empty() {
                setup.away_playbook =
                    plays_for_tactic(&setup.away_lineup.offense_tactic, &all_plays);
            } else {
                retain_compatible_plays(
                    &mut setup.away_playbook,
                    &setup.away_lineup.offense_tactic,
                );
            }
            setup
                .validate()
                .map_err(|error| format!("match setup invalid: {error}"))?;
            Some(setup)
        }
    };
    Ok(RunRequest {
        seed,
        scope,
        engine,
        rules,
        setup,
    })
}

fn api_studio() -> Response {
    match studio_catalog() {
        Ok(body) => ("200 OK", "application/json; charset=utf-8", body),
        Err(error) => internal_error(error),
    }
}

fn studio_catalog() -> Result<Vec<u8>, String> {
    let setup = nba_engine::MatchSetup::builtin(GameRules::default());
    let payload = serde_json::json!({
        "default_setup": setup,
        "offense": offense_catalog(),
        "defense": defense_catalog(),
        "plays": play_catalog(),
        "labels": studio_labels(),
    });
    serde_json::to_vec(&payload).map_err(|error| error.to_string())
}

fn offense_catalog() -> Vec<serde_json::Value> {
    [
        ("off_horns_pnr", "牛角高位挡拆体系"),
        ("off_spain_pnr", "西班牙双掩护体系"),
        ("off_motion_spacing", "五外动态进攻体系"),
        ("off_transition_push", "快攻闪击转换体系"),
        ("off_delay_attack", "高位单打体系"),
        ("off_post_split", "低位背身策应体系"),
        ("off_drag_screen", "突分投射体系"),
    ]
    .into_iter()
    .map(|(id, fallback)| {
        let spec = nba_domain::TacticalSetSpec::builtin(id);
        serde_json::json!({
            "id": id,
            "name_zh": spec.as_ref().map(|item| item.name_zh.clone()).unwrap_or_else(|| fallback.to_string()),
            "available": spec.is_some(),
            "spec": spec,
        })
    })
    .collect()
}

fn defense_catalog() -> Vec<serde_json::Value> {
    [
        ("def_man_conservative", "保守人盯人"),
        ("def_man_pressure", "压迫人盯人"),
        ("def_switch_heavy", "大量换防"),
        ("def_drop_coverage", "沉退防守"),
        ("def_hedge_recover", "延误回位"),
        ("def_zone_23", "2-3 联防"),
    ]
    .into_iter()
    .map(|(id, name_zh)| serde_json::json!({ "id": id, "name_zh": name_zh }))
    .collect()
}

fn play_catalog() -> Vec<nba_domain::PlaySpec> {
    const PLAYS: [&str; 9] = [
        include_str!("../../../data/tactics/plays/high_pnr_roll_v1.json"),
        include_str!("../../../data/tactics/plays/spain_pnr_stack_v1.json"),
        include_str!("../../../data/tactics/plays/horns_flare_pop_v1.json"),
        include_str!("../../../data/tactics/plays/delay_dho_handoff_v1.json"),
        include_str!("../../../data/tactics/plays/post_split_cut_v1.json"),
        include_str!("../../../data/tactics/plays/drag_screen_drive_kick_v1.json"),
        include_str!("../../../data/tactics/plays/transition_rim_runner_v1.json"),
        include_str!("../../../data/tactics/plays/corner_backdoor_v1.json"),
        include_str!("../../../data/tactics/plays/weak_side_lift_v1.json"),
    ];
    PLAYS
        .into_iter()
        .map(|json| {
            nba_domain::PlaySpec::from_json(json)
                .unwrap_or_else(|error| panic!("内建 PlaySpec 必须合法: {error}"))
        })
        .collect()
}

fn retain_compatible_plays(plays: &mut Vec<nba_domain::PlaySpec>, tactic_id: &str) {
    let allowed: std::collections::HashSet<&str> = match tactic_id {
        "off_horns_pnr" => ["high_pnr_roll_v1", "horns_flare_pop_v1"]
            .into_iter()
            .collect(),
        "off_spain_pnr" => ["spain_pnr_stack_v1"].into_iter().collect(),
        "off_motion_spacing" => ["weak_side_lift_v1", "corner_backdoor_v1"]
            .into_iter()
            .collect(),
        "off_transition_push" => ["transition_rim_runner_v1"].into_iter().collect(),
        "off_delay_attack" => ["delay_dho_handoff_v1"].into_iter().collect(),
        "off_post_split" => ["post_split_cut_v1"].into_iter().collect(),
        "off_drag_screen" => ["drag_screen_drive_kick_v1"].into_iter().collect(),
        _ => panic!("战术体系必须存在: {tactic_id}"),
    };
    let spec = nba_domain::TacticalSetSpec::builtin(tactic_id)
        .unwrap_or_else(|| panic!("战术体系必须存在: {tactic_id}"));
    let slots: std::collections::HashSet<&str> =
        spec.slots.iter().map(|slot| slot.id.as_str()).collect();
    plays.retain(|play| {
        allowed.contains(play.id.as_str())
            && play
                .rules
                .iter()
                .all(|rule| slots.contains(rule.then.slot.as_str()))
    });
    if plays.is_empty() {
        *plays = plays_for_tactic(tactic_id, &play_catalog());
    }
}

fn plays_for_tactic(
    tactic_id: &str,
    catalog: &[nba_domain::PlaySpec],
) -> Vec<nba_domain::PlaySpec> {
    let tactic_play_ids: &[&str] = match tactic_id {
        "off_horns_pnr" => &["high_pnr_roll_v1", "horns_flare_pop_v1"],
        "off_spain_pnr" => &["spain_pnr_stack_v1"],
        "off_motion_spacing" => &["weak_side_lift_v1", "corner_backdoor_v1"],
        "off_transition_push" => &["transition_rim_runner_v1"],
        "off_delay_attack" => &["delay_dho_handoff_v1"],
        "off_post_split" => &["post_split_cut_v1"],
        "off_drag_screen" => &["drag_screen_drive_kick_v1"],
        _ => &[],
    };
    let spec = nba_domain::TacticalSetSpec::builtin(tactic_id)
        .unwrap_or_else(|| panic!("战术体系必须存在: {tactic_id}"));
    let slots: std::collections::HashSet<&str> =
        spec.slots.iter().map(|slot| slot.id.as_str()).collect();
    catalog
        .iter()
        .filter(|play| {
            tactic_play_ids.contains(&play.id.as_str())
                && play
                    .rules
                    .iter()
                    .all(|rule| slots.contains(rule.then.slot.as_str()))
        })
        .cloned()
        .collect()
}

fn studio_labels() -> serde_json::Value {
    serde_json::json!({
        "attributes": {
            "speed": "速度",
            "acceleration": "加速",
            "agility": "敏捷",
            "strength": "力量",
            "vertical": "弹跳",
            "stamina": "耐力",
            "ball_handling": "控球",
            "passing": "传球",
            "shooting_close": "篮下",
            "shooting_near": "近筐",
            "shooting_mid": "中投",
            "shooting_three": "三分",
            "free_throw": "罚球",
            "finishing": "对抗终结",
            "defense_perimeter": "外线防守",
            "defense_interior": "内线防守",
            "steal": "抢断",
            "block": "封盖",
            "offensive_rebound": "前场篮板",
            "defensive_rebound": "后场篮板",
            "decision_iq": "持球决策",
            "off_ball_sense": "无球感觉"
        },
        "tendencies": {
            "shoot_frequency": "出手倾向",
            "drive_frequency": "突破倾向",
            "pass_frequency": "传球倾向",
            "cut_frequency": "切入倾向",
            "screen_frequency": "掩护倾向",
            "offensive_rebound_frequency": "冲板倾向",
            "risk_tolerance": "风险容忍",
            "transition_sprint": "转换冲刺"
        },
        "traits": {
            "pace": "节奏",
            "three_point_emphasis": "三分倾向",
            "rim_pressure": "攻框倾向",
            "defense_aggression": "防守强度",
            "rebound_emphasis": "篮板投入"
        }
    })
}

fn validate_rules(rules: &GameRules) -> Result<(), String> {
    rules.validate()
}

fn run_simulation(
    seed: u64,
    scope: &str,
    rules: GameRules,
    setup: Option<nba_engine::MatchSetup>,
) -> Result<(Vec<u8>, Vec<StreamTick>), String> {
    let mut engine = match setup {
        Some(mut setup) => {
            setup.rules = rules;
            setup.validate()?;
            MatchEngine::with_setup(setup, seed)
        }
        None => MatchEngine::with_rules(seed, rules),
    };
    engine.set_scope(scope)?;
    let mut body = Vec::with_capacity(1024 * 1024);
    let mut ticks = Vec::new();
    let mut ticks_count = 0usize;
    // C6.6：官方违规随响应下发（gap.md §16.3 调试视图是投影）。
    // 此前引擎不变量判定只存在于进程内，前端只能用 detectAnomalies
    // 自行重算（第二套口径，容差与引擎不一致）。现在违规作为流末的
    // run_summary 记录下发，前端退役本地检查、只做渲染。
    let mut violations: Vec<Violation> = Vec::new();
    while !engine.is_finished() {
        let tick = engine.step();
        violations.extend(engine.last_tick_violations().iter().cloned());
        serde_json::to_writer(&mut body, &tick).map_err(|e| e.to_string())?;
        body.push(b'\n');
        ticks.push(tick);
        ticks_count += 1;
        if ticks_count > MAX_SIMULATION_TICKS {
            return Err(format!(
                "simulation exceeded the {} tick safety limit",
                MAX_SIMULATION_TICKS
            ));
        }
    }
    let summary = serde_json::json!({
        "run_summary": 1,
        "violation_count": violations.len(),
        "violations": violations,
    });
    serde_json::to_writer(&mut body, &summary).map_err(|e| e.to_string())?;
    body.push(b'\n');
    Ok((body, ticks))
}

fn url_decode(input: &str) -> Result<String, String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' => {
                if i + 2 >= bytes.len() {
                    return Err("invalid percent escape in query".to_string());
                }
                let high = hex_value(bytes[i + 1])
                    .ok_or_else(|| "invalid percent escape in query".to_string())?;
                let low = hex_value(bytes[i + 2])
                    .ok_or_else(|| "invalid percent escape in query".to_string())?;
                out.push((high << 4) | low);
                i += 3;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| "query is not valid UTF-8".to_string())
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nba_engine::MatchSetup;

    fn request(method: &str, path: &str, body: Vec<u8>) -> Request {
        Request {
            method: method.to_string(),
            path: path.to_string(),
            query: String::new(),
            body,
        }
    }

    fn json_body(value: Value) -> Vec<u8> {
        serde_json::to_vec(&value).expect("test JSON should serialize")
    }
    #[test]
    fn studio_catalog_round_trips_into_simulation() {
        let validation: SharedValidation = Arc::new(Mutex::new(idle_state()));
        let (status, _, body) = route(
            &request("GET", "/api/studio", Vec::new()),
            &session_unused(),
            &validation,
        );
        assert_eq!(status, "200 OK");
        let catalog: Value = serde_json::from_slice(&body).expect("studio catalog should be JSON");
        let setup = catalog["default_setup"].clone();
        assert!(setup["home_team"]["players"].as_array().unwrap().len() >= 5);
        assert!(catalog["offense"].as_array().unwrap().len() >= 2);
        assert!(catalog["defense"].as_array().unwrap().len() >= 2);
        let response = api_simulate(
            &request(
                "POST",
                "/api/simulate",
                json_body(serde_json::json!({
                    "seed": 7,
                    "scope": "1p",
                    "setup": setup,
                })),
            ),
            &validation,
        );
        assert_eq!(response.0, "200 OK");
        assert!(!response.2.is_empty());
        // 模拟完成后缓存比赛可用，验证入口能对同一刻序列给出结论。
        let run_response = api_validation_run(
            &request("POST", "/api/validation/run", Vec::new()),
            &validation,
        );
        assert_eq!(run_response.0, "200 OK", "validation run after simulate");
        let run_json: Value = serde_json::from_slice(&run_response.2).expect("run JSON");
        assert!(
            run_json["result"]["exit_code"].is_number(),
            "validation run must report an exit code"
        );
        let report_response = api_validation_report(&validation);
        assert_eq!(report_response.0, "200 OK", "report after run");
        let report: Value = serde_json::from_slice(&report_response.2).expect("report JSON");
        assert_eq!(report["games"].as_array().map(Vec::len), Some(8));
        assert!(
            report["games"][0]["ledger"]["points_reconcile"]
                .as_bool()
                .is_some(),
            "each game must carry a ledger reconciliation"
        );
    }

    #[test]
    fn validation_run_requires_cached_game() {
        let validation: SharedValidation = Arc::new(Mutex::new(idle_state()));
        let response = api_validation_run(
            &request("POST", "/api/validation/run", Vec::new()),
            &validation,
        );
        assert_eq!(response.0, "400 Bad Request");
    }

    #[test]
    fn simulate_request_replaces_stale_slot_compatible_playbook() {
        let mut setup = MatchSetup::builtin(GameRules::default());
        setup.home_lineup.offense_tactic = "off_spain_pnr".to_string();
        assert_eq!(setup.home_playbook[0].id, "high_pnr_roll_v1");
        let response = parse_run_request(&request(
            "POST",
            "/api/simulate",
            json_body(serde_json::json!({
                "seed": 42,
                "scope": "1p",
                "setup": setup,
            })),
        ))
        .expect("Spain PnR request should parse");
        let setup = response.setup.expect("request setup should be retained");
        assert_eq!(setup.home_lineup.offense_tactic, "off_spain_pnr");
        assert_eq!(
            setup
                .home_playbook
                .iter()
                .map(|play| play.id.as_str())
                .collect::<Vec<_>>(),
            ["spain_pnr_stack_v1"]
        );
        let (stream, _) =
            run_simulation(response.seed, &response.scope, response.rules, Some(setup))
                .expect("Spain PnR simulation should complete");
        let records: Vec<Value> = std::str::from_utf8(&stream)
            .expect("simulation stream should be UTF-8")
            .lines()
            .map(|line| serde_json::from_str(line).expect("simulation line should be JSON"))
            .collect();
        assert!(records.iter().any(|record| {
            record["tactical_set"] == "西班牙双掩护体系 (Spain Pick-and-Roll)"
        }));
        assert!(records
            .iter()
            .any(|record| { record["debug"]["active_play_id"] == "spain_pnr_stack_v1" }));
        assert!(records.iter().any(|record| {
            record["players"].as_array().is_some_and(|players| {
                players.iter().any(|player| {
                    player["action"] == "BACK_SCREEN" || player["action"] == "SCREEN_POP"
                })
            })
        }));
    }

    #[test]
    fn offense_tactics_load_only_their_assigned_playbooks() {
        let catalog = play_catalog();
        let cases = [
            (
                "off_horns_pnr",
                vec!["high_pnr_roll_v1", "horns_flare_pop_v1"],
            ),
            ("off_spain_pnr", vec!["spain_pnr_stack_v1"]),
            (
                "off_motion_spacing",
                vec!["corner_backdoor_v1", "weak_side_lift_v1"],
            ),
            ("off_transition_push", vec!["transition_rim_runner_v1"]),
            ("off_delay_attack", vec!["delay_dho_handoff_v1"]),
            ("off_post_split", vec!["post_split_cut_v1"]),
            ("off_drag_screen", vec!["drag_screen_drive_kick_v1"]),
        ];
        for (tactic_id, expected_ids) in cases {
            let selected = plays_for_tactic(tactic_id, &catalog);
            let actual_ids: Vec<&str> = selected.iter().map(|play| play.id.as_str()).collect();
            assert_eq!(actual_ids, expected_ids, "playbook for {tactic_id}");

            let mut incompatible = catalog.clone();
            retain_compatible_plays(&mut incompatible, tactic_id);
            let retained_ids: Vec<&str> =
                incompatible.iter().map(|play| play.id.as_str()).collect();
            assert_eq!(
                retained_ids, expected_ids,
                "retained playbook for {tactic_id}"
            );
        }
    }

    #[test]
    fn studio_catalog_matches_static_page_asset() {
        let generated = studio_catalog().expect("studio catalog should serialize");
        let asset = include_bytes!("../static/studio.json");
        assert_eq!(generated, asset);
    }

    #[allow(clippy::arc_with_non_send_sync)]
    fn session_unused() -> SharedCore {
        Arc::new(Mutex::new(SessionService::new(42)))
    }

    fn validation_unused() -> SharedValidation {
        Arc::new(Mutex::new(idle_state()))
    }

    #[test]
    #[allow(clippy::arc_with_non_send_sync)]
    fn session_routes_preserve_setup_and_lifecycle() {
        let session = session_unused();
        let setup = MatchSetup::builtin(GameRules::default());
        let setup_body = serde_json::to_vec(&setup).expect("builtin setup should serialize");
        let (status, content_type, body) = route(
            &request("POST", "/api/session/setup", setup_body),
            &session,
            &validation_unused(),
        );
        assert_eq!(status, "200 OK");
        assert_eq!(content_type, "application/json; charset=utf-8");
        let info: Value = serde_json::from_slice(&body).expect("setup response should be JSON");
        assert_eq!(info["seed"], 42);
        assert!(info["home_team_id"].as_str().is_some());

        let (status, _, body) = route(
            &request("GET", "/api/session/state", Vec::new()),
            &session,
            &validation_unused(),
        );
        assert_eq!(status, "200 OK");
        let state: Value = serde_json::from_slice(&body).expect("state response should be JSON");
        assert_eq!(state["state"], "Ready");
        assert_eq!(state["configured"], true);

        let (status, _, body) = route(
            &request("GET", "/api/session/snapshot", Vec::new()),
            &session,
            &validation_unused(),
        );
        assert_eq!(status, "200 OK");
        let snapshot: Value = serde_json::from_slice(&body).expect("snapshot should be JSON");
        assert!(snapshot["t"].as_f64().is_some());

        let (status, _, body) = route(
            &request("POST", "/api/session/start", Vec::new()),
            &session,
            &validation_unused(),
        );
        assert_eq!(status, "200 OK");
        let state: Value = serde_json::from_slice(&body).expect("start response should be JSON");
        assert_eq!(state["state"], "Running");

        let (status, _, body) = route(
            &request(
                "POST",
                "/api/session/tick",
                json_body(serde_json::json!({ "dt": 0.04 })),
            ),
            &session,
            &validation_unused(),
        );
        assert_eq!(status, "200 OK");
        let tick: Value = serde_json::from_slice(&body).expect("tick response should be JSON");
        assert!(tick["t"].as_f64().is_some());

        let (status, _, body) = route(
            &request("GET", "/api/session/events", Vec::new()),
            &session,
            &validation_unused(),
        );
        assert_eq!(status, "200 OK");
        assert!(serde_json::from_slice::<Value>(&body)
            .expect("events response should be JSON")
            .is_array());
    }
    #[test]
    #[allow(clippy::arc_with_non_send_sync)]
    fn session_routes_reject_invalid_duration_before_engine_call() {
        let session = session_unused();
        let invalid_request = request(
            "POST",
            "/api/session/tick",
            json_body(serde_json::json!({ "dt": -1.0 })),
        );
        let (status, content_type, body) = route(&invalid_request, &session, &validation_unused());
        assert_eq!(status, "400 Bad Request");
        assert_eq!(content_type, "application/json; charset=utf-8");
        let error: Value = serde_json::from_slice(&body).expect("error should be JSON");
        assert!(error["error"]
            .as_str()
            .unwrap_or_default()
            .contains("non-negative"));

        let (status, _, _) = route(
            &request("GET", "/api/session/snapshot", Vec::new()),
            &session,
            &validation_unused(),
        );
        assert_eq!(status, "409 Conflict");
    }
}

#[cfg(test)]
mod c6_6_tests {
    use super::*;

    fn validation_shared() -> SharedValidation {
        Arc::new(Mutex::new(idle_state()))
    }

    /// C6.6 规定：`/api/simulate` 的响应是「tick 帧 + 流末 run_summary 记录」，
    /// run_summary 必须携带引擎官方 violations 数组（gap.md §16.3：调试视图
    /// 是事件与快照的投影，前端不得自行重算不变量）。
    ///
    /// 此前引擎不变量判定只存在于进程内，前端只能以第二套口径
    /// （`detectAnomalies`）重算，两侧容差已经分叉。
    #[test]
    fn simulate_response_ends_with_run_summary_carrying_engine_violations() {
        let (status, content_type, body) = api_simulate(
            &Request {
                method: "GET".to_string(),
                path: "/api/simulate".to_string(),
                query: "seed=1&scope=2p&engine=legacy".to_string(),
                body: Vec::new(),
            },
            &validation_shared(),
        );
        assert_eq!(status, "200 OK", "simulate must succeed");
        assert_eq!(content_type, "application/x-ndjson; charset=utf-8");

        let text = String::from_utf8(body).expect("stream must be UTF-8");
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        assert!(lines.len() > 2, "expected ticks plus a summary record");

        let summary: Value =
            serde_json::from_str(lines[lines.len() - 1]).expect("last line must be JSON");
        assert_eq!(
            summary["run_summary"], 1,
            "last record must be the run summary"
        );
        assert!(
            summary["violations"].is_array(),
            "run summary must carry the engine violations array"
        );
        assert_eq!(
            summary["violation_count"].as_u64().unwrap_or(u64::MAX),
            summary["violations"]
                .as_array()
                .map(|v| v.len() as u64)
                .unwrap_or(u64::MAX),
            "violation_count must match the array length"
        );

        // 其余每一行都必须是可解析的 tick 帧（不得混入其他记录类型）。
        for line in &lines[..lines.len() - 1] {
            let tick: Value = serde_json::from_str(line).expect("tick line must be JSON");
            assert!(
                tick["run_summary"].is_null(),
                "ticks must not be summary records"
            );
            assert!(tick["t"].as_f64().is_some(), "tick line must carry time");
        }
    }
}
