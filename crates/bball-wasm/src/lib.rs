use nba_domain::FixedDt;
use nba_engine::{MatchService, SessionState};
use wasm_bindgen::prelude::*;

/// JavaScript-facing facade for the authoritative Rust match service.
///
/// The bridge deliberately exposes JSON strings rather than internal domain
/// structs: JavaScript can observe snapshots and events, but cannot mutate
/// score, possession, player positions, or ball outcomes directly.
#[wasm_bindgen]
pub struct WasmMatchService {
    inner: MatchService,
}

/// 把一个来自 JavaScript 的毫秒时长转换成 `FixedDt`。
///
/// 输入必须有限、非负且换算后能放进 `f32`。未通过就拒绝，而不是截断成
/// 一个看似合法的值——静默截断会把「前端传了 NaN」变成「引擎按 0 推进」。
fn milliseconds_to_fixed_dt(value: f64, label: &str) -> Result<FixedDt, JsError> {
    const MILLIS_PER_SECOND: f64 = 1000.0;
    if !value.is_finite() || value < 0.0 || value > f32::MAX as f64 * MILLIS_PER_SECOND {
        return Err(JsError::new(&format!(
            "{label} must be finite, non-negative, and fit in f32 milliseconds"
        )));
    }
    Ok(FixedDt((value / MILLIS_PER_SECOND) as f32))
}

#[wasm_bindgen]
impl WasmMatchService {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            inner: MatchService::new(),
        }
    }

    #[wasm_bindgen(js_name = "withSeed")]
    pub fn with_seed(seed: u64) -> Self {
        Self {
            inner: MatchService::with_seed(seed),
        }
    }

    pub fn setup_match_json(&mut self, json: &str) -> Result<String, JsError> {
        self.inner
            .setup_match_json(json)
            .map_err(|error| JsError::new(&error))
    }

    pub fn start(&mut self) -> Result<(), JsError> {
        self.inner.start().map_err(|error| JsError::new(&error))
    }

    #[wasm_bindgen(js_name = "tickMs")]
    pub fn tick_ms(&mut self, dt_ms: f64) -> Result<String, JsError> {
        let dt = milliseconds_to_fixed_dt(dt_ms, "tick duration")?;
        self.inner
            .tick(dt)
            .and_then(|tick| {
                serde_json::to_string(&tick)
                    .map_err(|error| format!("tick serialization failed: {error}"))
            })
            .map_err(|error| JsError::new(&error))
    }

    pub fn pause(&mut self) {
        self.inner.pause();
    }

    pub fn resume(&mut self) -> Result<(), JsError> {
        self.inner.resume().map_err(|error| JsError::new(&error))
    }

    #[wasm_bindgen(js_name = "fastForwardMs")]
    pub fn fast_forward_ms(&mut self, duration_ms: f64) -> Result<String, JsError> {
        let duration = milliseconds_to_fixed_dt(duration_ms, "fast-forward duration")?;
        self.inner
            .fast_forward(duration.0)
            .and_then(|tick| {
                serde_json::to_string(&tick)
                    .map_err(|error| format!("snapshot serialization failed: {error}"))
            })
            .map_err(|error| JsError::new(&error))
    }

    pub fn snapshot_json(&self) -> Result<String, JsError> {
        self.inner
            .snapshot_json()
            .map_err(|error| JsError::new(&error))
    }

    pub fn events_since_json(&self, sequence: u64) -> Result<String, JsError> {
        self.inner
            .events_since_json(sequence)
            .map_err(|error| JsError::new(&error))
    }

    pub fn next_possession_json(&mut self) -> Result<String, JsError> {
        self.inner
            .next_possession_json()
            .map_err(|error| JsError::new(&error))
    }

    #[wasm_bindgen(js_name = "sessionState")]
    pub fn session_state(&self) -> String {
        match self.inner.state() {
            SessionState::Ready => "Ready",
            SessionState::Running => "Running",
            SessionState::Paused => "Paused",
            SessionState::Finished => "Finished",
        }
        .to_string()
    }
}

impl Default for WasmMatchService {
    fn default() -> Self {
        Self::new()
    }
}

/// 返回默认 GameRules 的 JSON 字符串（供前端直接初始化规则编辑器）。
#[wasm_bindgen(js_name = "getDefaultRulesJson")]
pub fn get_default_rules_json() -> Result<String, JsError> {
    serde_json::to_string_pretty(&nba_domain::GameRules::default())
        .map_err(|e| JsError::new(&e.to_string()))
}

/// 在浏览器内存中直接运行指定 scope 的模拟，输出 NDJSON 字符串。
/// 完全无需后端服务器参与，0 磁盘消耗。
#[wasm_bindgen(js_name = "simulateToNdjson")]
pub fn simulate_to_ndjson(
    seed: u64,
    scope: &str,
    rules_json: Option<String>,
) -> Result<String, JsError> {
    let rules = if let Some(json_str) = rules_json {
        let trimmed = json_str.trim();
        if trimmed.is_empty() {
            nba_domain::GameRules::default()
        } else {
            serde_json::from_str(trimmed)
                .map_err(|e| JsError::new(&format!("invalid rules json: {e}")))?
        }
    } else {
        nba_domain::GameRules::default()
    };

    let mut engine = nba_engine::MatchEngine::with_rules(seed, rules);
    let trimmed_scope = scope.trim().to_ascii_lowercase();
    let normalized_scope = match trimmed_scope.as_str() {
        "possession" | "1p" => "1p",
        "5p" => "5p",
        "10p" => "10p",
        "quarter" | "1q" => "1q",
        "full" => "full",
        other => other,
    };
    engine
        .set_scope(normalized_scope)
        .map_err(|e| JsError::new(&e))?;

    let mut output = String::new();
    let max_ticks = match normalized_scope {
        "1p" => 2_000,
        "5p" => 10_000,
        "10p" => 20_000,
        "1q" => 30_000,
        _ => 200_000,
    };

    let mut ticks = 0;
    while !engine.is_finished() && ticks < max_ticks {
        let tick = engine.step();
        let line = serde_json::to_string(&tick).map_err(|e| JsError::new(&e.to_string()))?;
        output.push_str(&line);
        output.push('\n');
        ticks += 1;
    }

    let snap = engine.engine_snapshot();
    let summary_obj = serde_json::json!({
        "type": "run_summary",
        "home_score": snap.game.home_score,
        "away_score": snap.game.away_score,
        "period": snap.game.period,
        "current_time": snap.game.current_time,
        "total_ticks": ticks,
        "completed_possessions": engine.completed_possessions(),
    });
    output.push_str(&summary_obj.to_string());
    output.push('\n');

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::WasmMatchService;

    #[test]
    fn bridge_keeps_json_service_boundary() {
        let mut service = WasmMatchService::with_seed(123);
        let setup = serde_json::to_string(&nba_engine::MatchSetup::builtin(
            nba_domain::GameRules::default(),
        ))
        .expect("builtin setup should serialize");
        let info = service
            .setup_match_json(&setup)
            .expect("setup should cross the JSON boundary");
        assert!(serde_json::from_str::<serde_json::Value>(&info).is_ok());
        service.start().expect("configured match should start");
        let snapshot = service.tick_ms(40.0).expect("one logic tick should run");
        let snapshot: serde_json::Value =
            serde_json::from_str(&snapshot).expect("tick should be JSON");
        assert!(snapshot["t"].as_f64().is_some());
        assert_eq!(service.session_state(), "Running");
    }

    #[cfg(target_arch = "wasm32")]
    #[test]
    fn bridge_rejects_invalid_millisecond_duration() {
        let mut service = WasmMatchService::new();
        assert!(service.tick_ms(-1.0).is_err());
        assert!(service.tick_ms(f64::NAN).is_err());
    }
}
