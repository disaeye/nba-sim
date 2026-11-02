//! 浏览器端适配层：把动作链内核包装成 JSON 边界。
//!
//! JavaScript 只能观察快照与事实，不能改写比分、球权、球员位置或出手结果。
//! 校准表通过编译期内嵌加载，因此 WASM 目标不需要文件系统。

use nba_sim_core::params::RuntimeParams;
use nba_sim_core::SessionService;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct WasmMatchService {
    inner: SessionService,
}

fn setup(service: &mut SessionService, seed: u64) -> Result<String, JsError> {
    let info = service
        .setup(RuntimeParams::embedded(), seed)
        .map_err(|error| JsError::new(&error))?;
    serde_json::to_string(&info).map_err(|error| JsError::new(&error.to_string()))
}

fn milliseconds_to_seconds(value: f64, label: &str) -> Result<f64, JsError> {
    const MILLIS_PER_SECOND: f64 = 1000.0;
    if !value.is_finite() || value < 0.0 || value > f64::MAX / MILLIS_PER_SECOND {
        return Err(JsError::new(&format!(
            "{label} must be finite and non-negative milliseconds"
        )));
    }
    Ok(value / MILLIS_PER_SECOND)
}

#[wasm_bindgen]
impl WasmMatchService {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            inner: SessionService::new(42),
        }
    }

    #[wasm_bindgen(js_name = "withSeed")]
    pub fn with_seed(seed: u64) -> Self {
        Self {
            inner: SessionService::new(seed),
        }
    }

    /// 接受任意 setup JSON，只取其中的 `seed`：动作链内核使用内建名册。
    pub fn setup_match_json(&mut self, json: &str) -> Result<String, JsError> {
        let seed = if json.trim().is_empty() {
            self.inner.seed()
        } else {
            serde_json::from_str::<serde_json::Value>(json)
                .ok()
                .and_then(|value| value.get("seed").and_then(|seed| seed.as_u64()))
                .unwrap_or_else(|| self.inner.seed())
        };
        setup(&mut self.inner, seed)
    }

    pub fn start(&mut self) -> Result<(), JsError> {
        self.inner.start().map_err(|error| JsError::new(&error))
    }

    #[wasm_bindgen(js_name = "tickMs")]
    pub fn tick_ms(&mut self, dt_ms: f64) -> Result<String, JsError> {
        let seconds = milliseconds_to_seconds(dt_ms, "tick duration")?;
        let tick = self
            .inner
            .tick(seconds)
            .map_err(|error| JsError::new(&error))?;
        serde_json::to_string(&tick).map_err(|error| JsError::new(&error.to_string()))
    }

    pub fn pause(&mut self) {
        self.inner.pause();
    }

    pub fn resume(&mut self) -> Result<(), JsError> {
        self.inner.resume().map_err(|error| JsError::new(&error))
    }

    #[wasm_bindgen(js_name = "fastForwardMs")]
    pub fn fast_forward_ms(&mut self, duration_ms: f64) -> Result<String, JsError> {
        let seconds = milliseconds_to_seconds(duration_ms, "fast-forward duration")?;
        let tick = self
            .inner
            .fast_forward(seconds)
            .map_err(|error| JsError::new(&error))?;
        serde_json::to_string(&tick).map_err(|error| JsError::new(&error.to_string()))
    }

    pub fn snapshot_json(&self) -> Result<String, JsError> {
        match self.inner.snapshot() {
            Some(tick) => {
                serde_json::to_string(&tick).map_err(|error| JsError::new(&error.to_string()))
            }
            None => Err(JsError::new("snapshot unavailable before match setup")),
        }
    }

    pub fn events_since_json(&self, sequence: u64) -> Result<String, JsError> {
        let batch = self
            .inner
            .events_since(sequence, 5_000)
            .map_err(|error| JsError::new(&error))?;
        serde_json::to_string(&batch).map_err(|error| JsError::new(&error.to_string()))
    }

    pub fn next_possession_json(&mut self) -> Result<String, JsError> {
        let tick = self
            .inner
            .next_possession()
            .map_err(|error| JsError::new(&error))?;
        serde_json::to_string(&tick).map_err(|error| JsError::new(&error.to_string()))
    }

    #[wasm_bindgen(js_name = "sessionState")]
    pub fn session_state(&self) -> String {
        self.inner.state().as_str().to_string()
    }

    #[wasm_bindgen(js_name = "completionSummaryJson")]
    pub fn completion_summary_json(&self) -> Result<String, JsError> {
        let summary = self
            .inner
            .completion_summary()
            .ok_or_else(|| JsError::new("completion summary unavailable before match setup"))?;
        serde_json::to_string(&summary).map_err(|error| JsError::new(&error.to_string()))
    }

    #[wasm_bindgen(js_name = "fullEventHistoryJson")]
    pub fn full_event_history_json(&self) -> Result<String, JsError> {
        let events = self
            .inner
            .full_event_history()
            .ok_or_else(|| JsError::new("full event history requires a completed match"))?;
        serde_json::to_string(&events).map_err(|error| JsError::new(&error.to_string()))
    }
}

impl Default for WasmMatchService {
    fn default() -> Self {
        Self::new()
    }
}

/// 返回默认 GameRules 的 JSON 字符串（供前端规则编辑器初始化）。
#[wasm_bindgen(js_name = "getDefaultRulesJson")]
pub fn get_default_rules_json() -> Result<String, JsError> {
    serde_json::to_string_pretty(&nba_domain::GameRules::default())
        .map_err(|error| JsError::new(&error.to_string()))
}

/// 在浏览器内存中推演并输出 NDJSON 帧流，末尾附一条 `run_summary`。
#[wasm_bindgen(js_name = "simulateToNdjson")]
pub fn simulate_to_ndjson(
    seed: u64,
    scope: &str,
    _rules_json: Option<String>,
    _setup_json: Option<String>,
) -> Result<String, JsError> {
    let mut service = SessionService::new(seed);
    service
        .setup(RuntimeParams::embedded(), seed)
        .map_err(|error| JsError::new(&error))?;
    service.start().map_err(|error| JsError::new(&error))?;

    let normalized = scope.trim().to_ascii_lowercase();
    let possession_target = match normalized.as_str() {
        "possession" | "1p" => Some(1u32),
        "5p" => Some(5),
        "10p" => Some(10),
        "quarter" | "1q" => None,
        "full" => None,
        other => other.trim_end_matches('p').parse::<u32>().ok(),
    };
    let quarter_target = if matches!(normalized.as_str(), "quarter" | "1q") {
        Some(2u32)
    } else {
        None
    };

    let mut output = String::new();
    let mut ticks = 0u64;
    loop {
        let tick = service
            .tick(nba_sim_core::world::TICK_SECONDS)
            .map_err(|error| JsError::new(&error))?;
        let line =
            serde_json::to_string(&tick).map_err(|error| JsError::new(&error.to_string()))?;
        output.push_str(&line);
        output.push('\n');
        ticks += 1;
        if tick.frame.simulation_complete {
            break;
        }
        if let Some(target) = possession_target {
            if tick.frame.completed_possessions as u32 >= target {
                break;
            }
        }
        if let Some(target) = quarter_target {
            if tick.frame.period >= target {
                break;
            }
        }
        if ticks > 400_000 {
            return Err(JsError::new("simulation exceeded the tick guard"));
        }
    }

    let summary = serde_json::json!({
        "type": "run_summary",
        "total_ticks": ticks,
    });
    output.push_str(&summary.to_string());
    output.push('\n');
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::WasmMatchService;
    use nba_sim_core::params::workspace_calibration_dir;
    use nba_sim_core::{Params, SessionService};

    #[test]
    fn bridge_keeps_json_service_boundary() {
        let mut service = WasmMatchService::with_seed(123);
        let info = service
            .setup_match_json("")
            .expect("setup should cross the JSON boundary");
        assert!(serde_json::from_str::<serde_json::Value>(&info).is_ok());
        service.start().expect("configured match should start");
        let snapshot = service.tick_ms(50.0).expect("one logic tick should run");
        let snapshot: serde_json::Value =
            serde_json::from_str(&snapshot).expect("tick should be JSON");
        assert!(snapshot["t"].as_f64().is_some());
        let events = service.events_since_json(0).expect("initial event query");
        let events: serde_json::Value =
            serde_json::from_str(&events).expect("event batch should be JSON");
        let event_rows = events["events"].as_array().expect("event batch list");
        assert!(!event_rows.is_empty());
        let latest_event_id = event_rows.last().unwrap()["event_id"]
            .as_u64()
            .expect("event id");
        assert_eq!(
            events["next_after_event_id"].as_u64(),
            Some(latest_event_id)
        );
        assert_eq!(service.session_state(), "Running");
    }

    #[test]
    fn wasm_bridge_matches_session_service_frames_and_events() {
        let seed = 211;
        let mut wasm = WasmMatchService::with_seed(seed);
        let mut session = SessionService::new(seed);
        wasm.setup_match_json("").expect("WASM setup");
        session
            .setup(
                Params::load(&workspace_calibration_dir()).expect("calibration tables load"),
                seed,
            )
            .expect("session setup");
        wasm.start().expect("WASM session starts");
        session.start().expect("session starts");

        for _ in 0..40 {
            let wasm_frame = wasm
                .tick_ms(nba_sim_core::world::TICK_SECONDS * 1000.0)
                .expect("WASM tick");
            let session_frame = serde_json::to_string(
                &session
                    .tick(nba_sim_core::world::TICK_SECONDS)
                    .expect("session tick"),
            )
            .expect("session frame serialization");
            assert_eq!(wasm_frame, session_frame);
        }

        let wasm_events = wasm.events_since_json(0).expect("WASM event query");
        let session_events =
            serde_json::to_string(&session.events_since(0, 5_000).expect("session event query"))
                .expect("session event batch JSON");
        assert_eq!(wasm_events, session_events);

        let wasm_frame: serde_json::Value = serde_json::from_str(
            &wasm
                .tick_ms(nba_sim_core::world::TICK_SECONDS * 1000.0)
                .expect("next WASM tick"),
        )
        .expect("next WASM frame JSON");
        let wasm_cursor = wasm_frame["frame"]["event_log"][0]["event_id"]
            .as_u64()
            .unwrap_or(0);
        let wasm_events_after_cursor = wasm
            .events_since_json(wasm_cursor)
            .expect("incremental WASM event query");
        let session_frame: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(
                &session
                    .tick(nba_sim_core::world::TICK_SECONDS)
                    .expect("next session tick"),
            )
            .expect("next session frame serialization"),
        )
        .expect("next session frame JSON");
        assert_eq!(wasm_frame, session_frame);
        let session_events_after_cursor = serde_json::to_string(
            &session
                .events_since(wasm_cursor, 5_000)
                .expect("incremental session event query"),
        )
        .expect("incremental session event batch JSON");
        assert_eq!(wasm_events_after_cursor, session_events_after_cursor);
    }

    #[cfg(target_arch = "wasm32")]
    #[test]
    fn bridge_rejects_invalid_millisecond_duration() {
        let mut service = WasmMatchService::new();
        assert!(service.tick_ms(-1.0).is_err());
        assert!(service.tick_ms(f64::NAN).is_err());
    }
}
