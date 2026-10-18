//! 开发调试服务器：内嵌静态页 + 动作链内核的会话/流/验证 HTTP API。
//!
//! 只服务于本地开发调试，不是生产服务。std-only：手写最小 HTTP/1.1 解析，
//! 避免引入 web 框架依赖。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use nba_domain::GameRules;
use nba_sim_core::SessionService;
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

#[allow(clippy::arc_with_non_send_sync)]
fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(31100);
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind debug server port");
    let core: SharedCore = Arc::new(Mutex::new(SessionService::new(42)));
    let validation: SharedValidation = Arc::new(Mutex::new(idle_state()));
    println!("nba-debug-server listening on http://127.0.0.1:{}", port);
    for stream in listener.incoming().flatten() {
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
    // 克隆的 socket 与原 socket 共享接收队列：解析放在缓冲读取器里，
    // 原 socket 留给回复，两处都不会读到半个请求头。
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

fn route(req: &Request, core: &SharedCore, validation: &SharedValidation) -> Response {
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
        ("GET", "/api/simulate") | ("POST", "/api/simulate") => api_simulate(req, validation),
        ("POST", "/api/session/setup") => core_api::api_core_setup(req, core),
        ("GET", "/api/session") | ("GET", "/api/session/state") => core_api::api_core_state(core),
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
        ("POST", "/api/validation/run") => api_validation_run(req, validation),
        ("GET", "/api/validation/status") => api_validation_status(validation),
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

pub(crate) fn bad_request(msg: String) -> Response {
    (
        "400 Bad Request",
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

struct RunRequest {
    seed: u64,
    scope: String,
}

fn parse_run_request(req: &Request) -> Result<RunRequest, String> {
    let mut seed = 42u64;
    let mut scope = "5p".to_string();

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
                _ => {}
            }
        }
    }
    Ok(RunRequest { seed, scope })
}

/// POST/GET /api/simulate：跑一场并返回 NDJSON tick 流，同时缓存这场
/// 比赛给验证入口重放。
fn api_simulate(req: &Request, validation: &SharedValidation) -> Response {
    let run = match parse_run_request(req) {
        Ok(run) => run,
        Err(e) => return bad_request(e),
    };
    cache_game(validation, run.seed, &run.scope);
    core_api::core_simulate_response(run.seed, &run.scope)
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

    fn request(method: &str, path: &str, body: Vec<u8>) -> Request {
        Request {
            method: method.to_string(),
            path: path.to_string(),
            query: String::new(),
            body,
        }
    }

    fn validation_shared() -> SharedValidation {
        Arc::new(Mutex::new(idle_state()))
    }

    #[test]
    fn validation_run_requires_cached_game() {
        let validation = validation_shared();
        let response = api_validation_run(
            &request("POST", "/api/validation/run", Vec::new()),
            &validation,
        );
        assert_eq!(response.0, "400 Bad Request");
    }

    /// `/api/simulate` 只走动作链内核：响应是纯 NDJSON tick 流，
    /// 每一行都带时间与比赛时钟。
    #[test]
    fn simulate_streams_core_ticks() {
        let validation = validation_shared();
        let (status, content_type, body) = api_simulate(
            &Request {
                method: "GET".to_string(),
                path: "/api/simulate".to_string(),
                query: "seed=1&scope=1p".to_string(),
                body: Vec::new(),
            },
            &validation,
        );
        assert_eq!(status, "200 OK");
        assert_eq!(content_type, "application/x-ndjson; charset=utf-8");
        let text = String::from_utf8(body).expect("stream must be UTF-8");
        let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
        assert!(lines.len() > 2, "expected a tick stream");
        for line in lines {
            let tick: Value = serde_json::from_str(line).expect("tick line must be JSON");
            assert!(tick["t"].as_f64().is_some(), "tick line must carry time");
            assert!(tick["tick_index"].as_u64().is_some(), "tick line must carry an index");
        }
        assert_eq!(
            crate::validation_api::cached_seed(&validation),
            Some(1),
            "simulate must cache the game for the validation entry"
        );
    }
}
