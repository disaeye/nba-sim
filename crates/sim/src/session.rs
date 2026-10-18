//! 会话服务：把动作链世界包装成带生命周期命令的应用边界。
//!
//! 世界拥有比赛真相；服务只拥有生命周期、固定步累加器与事件历史。
//! 调用方得到快照与事实投影，不能改写比赛状态。

use std::path::Path;

use nba_protocol::{FrameEvent, StreamTick};
use serde::{Deserialize, Serialize};

use crate::params::Params;
use crate::world::{GameReport, MatchSetup, World, TICK_SECONDS};

const MAX_STEPS_PER_CALL: usize = 200_000;
const EVENT_HISTORY_LIMIT: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    Ready,
    Running,
    Paused,
    Finished,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Running => "Running",
            Self::Paused => "Paused",
            Self::Finished => "Finished",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchInfo {
    pub home_team_id: String,
    pub away_team_id: String,
    pub home_team_name: String,
    pub away_team_name: String,
    pub seed: u64,
    pub tick_seconds: f64,
}

#[derive(Default)]
pub struct SessionService {
    world: Option<World>,
    seed: u64,
    state: Option<SessionState>,
    accumulator: f64,
    events: Vec<FrameEvent>,
}

impl SessionService {
    pub fn new(seed: u64) -> Self {
        Self {
            world: None,
            seed,
            state: Some(SessionState::Ready),
            accumulator: 0.0,
            events: Vec::new(),
        }
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn state(&self) -> SessionState {
        self.state.unwrap_or(SessionState::Ready)
    }

    pub fn is_configured(&self) -> bool {
        self.world.is_some()
    }

    pub fn setup(&mut self, params: Params, seed: u64) -> Result<MatchInfo, String> {
        let rules = nba_domain::GameRules::default();
        let setup = MatchSetup::builtin(rules);
        let info = MatchInfo {
            home_team_id: setup.home.id.clone(),
            away_team_id: setup.away.id.clone(),
            home_team_name: setup.home.name.clone(),
            away_team_name: setup.away.name.clone(),
            seed,
            tick_seconds: TICK_SECONDS,
        };
        self.world = Some(World::new(setup, params, seed));
        self.seed = seed;
        self.state = Some(SessionState::Ready);
        self.accumulator = 0.0;
        self.events.clear();
        Ok(info)
    }

    pub fn setup_from_calibration(&mut self, calibration: &Path, seed: u64) -> Result<MatchInfo, String> {
        let params = Params::load(calibration)?;
        self.setup(params, seed)
    }

    pub fn start(&mut self) -> Result<(), String> {
        match self.state() {
            SessionState::Ready | SessionState::Paused => {
                if self.world.is_none() {
                    return Err("cannot start before match setup".to_string());
                }
                self.state = Some(SessionState::Running);
                Ok(())
            }
            SessionState::Running => Ok(()),
            SessionState::Finished => Err("cannot start a finished match".to_string()),
        }
    }

    pub fn pause(&mut self) {
        if self.state() == SessionState::Running {
            self.state = Some(SessionState::Paused);
        }
    }

    pub fn resume(&mut self) -> Result<(), String> {
        if self.world.is_none() {
            return Err("cannot resume before match setup".to_string());
        }
        if self.state() == SessionState::Paused {
            self.state = Some(SessionState::Running);
        }
        Ok(())
    }

    pub fn snapshot(&self) -> Option<StreamTick> {
        self.world.as_ref().map(|world| world.stream_tick())
    }

    pub fn events(&self, limit: usize) -> Vec<FrameEvent> {
        let start = self.events.len().saturating_sub(limit);
        self.events[start..].to_vec()
    }

    pub fn tick(&mut self, dt: f64) -> Result<StreamTick, String> {
        if !dt.is_finite() || dt < 0.0 {
            return Err("tick duration must be finite and non-negative".to_string());
        }
        if self.world.is_none() {
            return Err("cannot tick before match setup".to_string());
        }
        if self.state() != SessionState::Running {
            return self
                .snapshot()
                .ok_or_else(|| "match snapshot unavailable".to_string());
        }
        self.accumulator += dt;
        if !self.accumulator.is_finite() {
            return Err("tick duration exceeds the accumulator range".to_string());
        }
        let mut steps = 0usize;
        while self.accumulator + f64::EPSILON >= TICK_SECONDS
            && steps < MAX_STEPS_PER_CALL
            && self.state() == SessionState::Running
        {
            self.accumulator = (self.accumulator - TICK_SECONDS).max(0.0);
            self.step_once();
            steps += 1;
        }
        if steps == MAX_STEPS_PER_CALL {
            return Err("tick duration exceeds the service step limit".to_string());
        }
        self.snapshot()
            .ok_or_else(|| "match snapshot unavailable".to_string())
    }

    pub fn fast_forward(&mut self, seconds: f64) -> Result<StreamTick, String> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err("fast-forward duration must be finite and non-negative".to_string());
        }
        if self.world.is_none() {
            return Err("cannot fast-forward before match setup".to_string());
        }
        if self.state() == SessionState::Finished {
            return self
                .snapshot()
                .ok_or_else(|| "match snapshot unavailable".to_string());
        }
        let prior = self.state();
        self.state = Some(SessionState::Running);
        let result = self.tick(seconds);
        if self.state() != SessionState::Finished {
            self.state = Some(prior);
        }
        result
    }

    pub fn next_possession(&mut self) -> Result<StreamTick, String> {
        let Some(world) = self.world.as_ref() else {
            return Err("cannot advance before match setup".to_string());
        };
        if self.state() == SessionState::Finished {
            return self
                .snapshot()
                .ok_or_else(|| "match snapshot unavailable".to_string());
        }
        let prior = self.state();
        let initial = world.possession.id;
        self.state = Some(SessionState::Running);
        let mut steps = 0usize;
        while steps < MAX_STEPS_PER_CALL && self.state() == SessionState::Running {
            self.step_once();
            steps += 1;
            let Some(world) = self.world.as_ref() else {
                break;
            };
            if world.possession.id != initial || world_finished(world) {
                break;
            }
        }
        if self.state() != SessionState::Finished {
            self.state = Some(prior);
        }
        self.snapshot()
            .ok_or_else(|| "match snapshot unavailable".to_string())
    }

    pub fn run_to_possessions(&mut self, target: u32) -> Result<StreamTick, String> {
        if self.world.is_none() {
            return Err("cannot run before match setup".to_string());
        }
        let prior = self.state();
        self.state = Some(SessionState::Running);
        let mut steps = 0usize;
        while steps < MAX_STEPS_PER_CALL && self.state() == SessionState::Running {
            self.step_once();
            steps += 1;
            let Some(world) = self.world.as_ref() else {
                break;
            };
            if world.possession_log.len() as u32 >= target || world_finished(world) {
                break;
            }
        }
        self.state = Some(prior);
        self.snapshot()
            .ok_or_else(|| "match snapshot unavailable".to_string())
    }

    pub fn report(&mut self) -> Option<GameReport> {
        self.world.as_mut().map(|world| {
            let mut report = world.report();
            report.shot_clock_at_release = world.shot_clock_at_release.clone();
            report.contest_samples = world.contest_samples.clone();
            report.shot_zone_counts = world.shot_zone_counts;
            report.shot_zone_makes = world.shot_zone_makes;
            report.creation_counts = world.creation_counts;
            report.violations = world.violations.clone();
            report
        })
    }

    fn step_once(&mut self) {
        let Some(world) = self.world.as_mut() else {
            return;
        };
        world.step(TICK_SECONDS);
        let tick = world.stream_tick();
        self.events.extend(tick.frame.event_log.iter().cloned());
        if self.events.len() > EVENT_HISTORY_LIMIT {
            let excess = self.events.len() - EVENT_HISTORY_LIMIT;
            self.events.drain(0..excess);
        }
        if world.finished {
            self.state = Some(SessionState::Finished);
            self.accumulator = 0.0;
        }
    }
}

fn world_finished(world: &World) -> bool {
    world.finished
}
