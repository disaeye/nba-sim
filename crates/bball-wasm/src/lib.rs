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
        if !dt_ms.is_finite() || dt_ms < 0.0 || dt_ms > f32::MAX as f64 * 1000.0 {
            return Err(JsError::new(
                "tick duration must be finite, non-negative, and fit in f32 milliseconds",
            ));
        }
        self.inner
            .tick(FixedDt((dt_ms / 1000.0) as f32))
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
        if !duration_ms.is_finite() || duration_ms < 0.0 || duration_ms > f32::MAX as f64 * 1000.0 {
            return Err(JsError::new(
                "fast-forward duration must be finite, non-negative, and fit in f32 milliseconds",
            ));
        }
        self.inner
            .fast_forward((duration_ms / 1000.0) as f32)
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
