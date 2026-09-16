use crate::match_engine::MatchEngine;
use crate::setup::MatchSetup;
use nba_domain::FixedDt;
use nba_protocol::{FrameEvent, StreamTick};
use serde::{Deserialize, Serialize};

const MAX_SERVICE_STEPS_PER_CALL: usize = 1_000_000;

/// Lifecycle owned by the application service rather than by the simulation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    Ready,
    Running,
    Paused,
    Finished,
}

/// Stable metadata returned after a setup has been accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchInfo {
    pub home_team_id: String,
    pub away_team_id: String,
    pub home_team_name: String,
    pub away_team_name: String,
    pub seed: u64,
}

/// Application boundary for UI, replay tools, and headless controllers.
///
/// The service owns lifecycle commands and the fixed-step accumulator. The
/// engine remains the sole owner of authoritative score, possession, player,
/// and ball state; callers only receive snapshots and event projections.
pub struct MatchService {
    engine: Option<MatchEngine>,
    state: SessionState,
    seed: u64,
    accumulator_seconds: f32,
    event_history: Vec<FrameEvent>,
}

impl Default for MatchService {
    fn default() -> Self {
        Self::new()
    }
}

impl MatchService {
    pub fn new() -> Self {
        Self::with_seed(42)
    }

    pub fn with_seed(seed: u64) -> Self {
        Self {
            engine: None,
            state: SessionState::Ready,
            seed,
            accumulator_seconds: 0.0,
            event_history: Vec::new(),
        }
    }

    /// Validates setup before constructing any runtime backend state.
    pub fn setup_match(&mut self, setup: MatchSetup) -> Result<MatchInfo, String> {
        self.setup_match_with_seed(setup, self.seed)
    }

    pub fn setup_match_with_seed(
        &mut self,
        setup: MatchSetup,
        seed: u64,
    ) -> Result<MatchInfo, String> {
        setup.validate()?;
        let info = MatchInfo {
            home_team_id: setup.home_team.id.clone(),
            away_team_id: setup.away_team.id.clone(),
            home_team_name: setup.home_team.name.clone(),
            away_team_name: setup.away_team.name.clone(),
            seed,
        };
        let engine = MatchEngine::with_setup(setup, seed);
        self.engine = Some(engine);
        self.seed = seed;
        self.state = SessionState::Ready;
        self.accumulator_seconds = 0.0;
        self.event_history.clear();
        Ok(info)
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn is_configured(&self) -> bool {
        self.engine.is_some()
    }

    pub fn setup_match_json(&mut self, json: &str) -> Result<String, String> {
        #[derive(Deserialize)]
        struct SetupRequest {
            setup: MatchSetup,
            #[serde(default)]
            seed: Option<u64>,
        }

        let value: serde_json::Value = serde_json::from_str(json)
            .map_err(|error| format!("match setup JSON invalid: {error}"))?;
        let (setup, seed) = if value.get("setup").is_some() {
            let request: SetupRequest = serde_json::from_value(value)
                .map_err(|error| format!("match setup request invalid: {error}"))?;
            (request.setup, request.seed.unwrap_or(self.seed))
        } else {
            let setup: MatchSetup = serde_json::from_value(value)
                .map_err(|error| format!("match setup invalid: {error}"))?;
            (setup, self.seed)
        };
        serde_json::to_string(&self.setup_match_with_seed(setup, seed)?)
            .map_err(|error| format!("match info serialization failed: {error}"))
    }

    pub fn start(&mut self) -> Result<(), String> {
        if self.engine.is_none() {
            return Err("cannot start before match setup".to_string());
        }
        match self.state {
            SessionState::Ready | SessionState::Paused => {
                self.state = SessionState::Running;
                Ok(())
            }
            SessionState::Running => Ok(()),
            SessionState::Finished => Err("cannot start a finished match".to_string()),
        }
    }

    pub fn pause(&mut self) {
        if self.state == SessionState::Running {
            self.state = SessionState::Paused;
        }
    }

    pub fn resume(&mut self) -> Result<(), String> {
        if self.engine.is_none() {
            return Err("cannot resume before match setup".to_string());
        }
        if self.state == SessionState::Paused {
            self.state = SessionState::Running;
        }
        Ok(())
    }

    /// Accumulates presentation time but advances only whole configured logic steps.
    pub fn tick(&mut self, dt: FixedDt) -> Result<StreamTick, String> {
        if !dt.0.is_finite() || dt.0 < 0.0 {
            return Err("tick duration must be finite and non-negative".to_string());
        }
        let Some(engine) = self.engine.as_ref() else {
            return Err("cannot tick before match setup".to_string());
        };
        if self.state != SessionState::Running {
            return self
                .snapshot()
                .ok_or_else(|| "match snapshot unavailable".to_string());
        }

        let step_seconds = engine.rules().tick_seconds;
        if !step_seconds.is_finite() || step_seconds <= 0.0 {
            return Err("configured logic tick must be finite and positive".to_string());
        }
        let accumulated = self.accumulator_seconds + dt.0;
        if !accumulated.is_finite() {
            return Err("tick duration exceeds the service accumulator range".to_string());
        }
        let available_steps = ((accumulated / step_seconds) + f32::EPSILON).floor();
        if available_steps > MAX_SERVICE_STEPS_PER_CALL as f32 {
            return Err("tick duration exceeds the service step safety limit".to_string());
        }
        self.accumulator_seconds = accumulated;

        let mut steps = 0usize;
        while self.accumulator_seconds + f32::EPSILON >= step_seconds
            && steps < MAX_SERVICE_STEPS_PER_CALL
        {
            self.accumulator_seconds = (self.accumulator_seconds - step_seconds).max(0.0);
            let tick = self.step_once();
            steps += 1;
            if tick.frame.simulation_complete || tick.frame.game_flow == "GameEnd" {
                self.state = SessionState::Finished;
                self.accumulator_seconds = 0.0;
                break;
            }
        }
        if steps == MAX_SERVICE_STEPS_PER_CALL
            && self.accumulator_seconds + f32::EPSILON >= step_seconds
        {
            return Err("tick duration exceeds the service step safety limit".to_string());
        }
        self.snapshot()
            .ok_or_else(|| "match snapshot unavailable".to_string())
    }

    /// Advances the authoritative state by whole fixed steps without changing
    /// the configured logic frequency. The caller's lifecycle state is kept.
    pub fn fast_forward(&mut self, seconds: f32) -> Result<StreamTick, String> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err("fast-forward duration must be finite and non-negative".to_string());
        }
        if self.engine.is_none() {
            return Err("cannot fast-forward before match setup".to_string());
        }
        let prior_state = self.state;
        if prior_state == SessionState::Finished {
            return self
                .snapshot()
                .ok_or_else(|| "match snapshot unavailable".to_string());
        }
        self.state = SessionState::Running;
        let result = self.tick(FixedDt(seconds));
        if self.state != SessionState::Finished {
            self.state = prior_state;
        }
        result
    }

    /// Advances until the next possession boundary, useful for replay controls.
    /// The command is legal while paused and preserves that paused state.
    pub fn next_possession(&mut self) -> Result<StreamTick, String> {
        if self.engine.is_none() {
            return Err("cannot advance before match setup".to_string());
        }
        if self.state == SessionState::Finished {
            return self
                .snapshot()
                .ok_or_else(|| "match snapshot unavailable".to_string());
        }
        let prior_state = self.state;
        let initial_possession_id = self
            .engine
            .as_ref()
            .expect("configured engine checked above")
            .possession_id();
        self.state = SessionState::Running;
        let mut steps = 0usize;
        loop {
            let tick = self.step_once();
            steps += 1;
            if tick.frame.simulation_complete || tick.frame.game_flow == "GameEnd" {
                self.state = SessionState::Finished;
                break;
            }
            let changed = self
                .engine
                .as_ref()
                .map(|current| current.possession_id() != initial_possession_id)
                .unwrap_or(true);
            if changed || steps >= MAX_SERVICE_STEPS_PER_CALL {
                break;
            }
        }
        if steps >= MAX_SERVICE_STEPS_PER_CALL {
            self.state = prior_state;
            return Err("next-possession exceeded the service step safety limit".to_string());
        }
        if self.state != SessionState::Finished {
            self.state = prior_state;
        }
        self.snapshot()
            .ok_or_else(|| "match snapshot unavailable".to_string())
    }

    /// 返回底层引擎的轻量只读快照视图（D14）。
    pub fn engine_snapshot(&self) -> Option<crate::snapshot::EngineSnapshot<'_>> {
        self.engine.as_ref().map(MatchEngine::engine_snapshot)
    }

    pub fn snapshot(&self) -> Option<StreamTick> {
        self.engine.as_ref().map(MatchEngine::snapshot)
    }

    pub fn snapshot_json(&self) -> Result<String, String> {
        let tick = self
            .snapshot()
            .ok_or_else(|| "match snapshot unavailable".to_string())?;
        serde_json::to_string(&tick)
            .map_err(|error| format!("snapshot serialization failed: {error}"))
    }

    /// ## C6.1 修复：游标必须是跨 tick 唯一的 `event_id`
    ///
    /// 原实现按 `sequence` 过滤，而 `sequence` 是 tick 内局部序号（每 tick 从
    /// 0 重新计数，见 protocol frame.rs `FrameEvent.sequence` 注释）。两个后果：
    ///
    /// - 调用方以 "sequence > N" 做增量游标时，后续 tick 中 sequence ≤ N 的
    ///   事件全部漏取；
    /// - `step_once` 按 `sequence` 去重时，不同 tick 的同号事件被误判为重复。
    ///
    /// `event_id` 全场单调递增（D4.1），是协议声明的因果链锚点，改用它。
    pub fn events_since(&self, event_id_cursor: u64) -> Vec<FrameEvent> {
        self.event_history
            .iter()
            .filter(|event| event.event_id > event_id_cursor)
            .cloned()
            .collect()
    }

    pub fn events_since_json(&self, event_id_cursor: u64) -> Result<String, String> {
        serde_json::to_string(&self.events_since(event_id_cursor))
            .map_err(|error| format!("event serialization failed: {error}"))
    }

    pub fn next_possession_json(&mut self) -> Result<String, String> {
        serde_json::to_string(&self.next_possession()?)
            .map_err(|error| format!("snapshot serialization failed: {error}"))
    }

    fn step_once(&mut self) -> StreamTick {
        let tick = self
            .engine
            .as_mut()
            .expect("service step requires a configured engine")
            .step();
        for event in &tick.frame.event_log {
            // C6.1：按 event_id 去重（跨 tick 唯一），见 events_since 注释。
            if let Err(position) = self
                .event_history
                .binary_search_by_key(&event.event_id, |known| known.event_id)
            {
                self.event_history.insert(position, event.clone());
            }
        }
        tick
    }
}
