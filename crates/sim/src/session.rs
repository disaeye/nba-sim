//! 会话服务：把动作链世界包装成带生命周期命令的应用边界。
//!
//! 世界拥有比赛真相；服务只拥有生命周期、固定步累加器与事件历史。
//! 调用方得到快照与事实投影，不能改写比赛状态。

use std::path::Path;

use nba_protocol::{FrameEvent, StreamTick};
use serde::{Deserialize, Serialize};

use crate::params::RuntimeParams;
use crate::world::{GameReport, MatchSetup, World, TICK_SECONDS};

const MAX_STEPS_PER_CALL: usize = 200_000;
#[cfg(not(test))]
const EVENT_HISTORY_LIMIT: usize = 20_000;
#[cfg(test)]
const EVENT_HISTORY_LIMIT: usize = 256;

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionSummary {
    pub seed: u64,
    pub home_team: String,
    pub away_team: String,
    pub home_score: u32,
    pub away_score: u32,
    pub periods: u32,
    pub ticks: u64,
    pub possessions: usize,
    pub event_count: u64,
    pub box_score: crate::boxscore::BoxScore,
    pub finished: bool,
}

impl CompletionSummary {
    pub fn from_report(report: &GameReport) -> Self {
        Self {
            seed: report.seed,
            home_team: report.home_team.clone(),
            away_team: report.away_team.clone(),
            home_score: report.home_score,
            away_score: report.away_score,
            periods: report.periods,
            ticks: report.ticks,
            possessions: report.possessions.len(),
            event_count: report.events.len() as u64,
            box_score: report.box_score.clone(),
            finished: report
                .events
                .iter()
                .any(|event| matches!(event.fact, crate::action::Fact::GameEnd)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventBatch {
    pub oldest_available_event_id: Option<u64>,
    pub latest_event_id: u64,
    pub next_after_event_id: u64,
    pub truncated: bool,
    pub events: Vec<FrameEvent>,
}

/// 磁盘检查点描述符。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDescriptor {
    pub version: u32,
    pub seed: u64,
    pub tick: u64,
    pub latest_event_id: u64,
    pub state: SessionState,
    pub home_score: u32,
    pub away_score: u32,
    pub possessions: u64,
}

#[derive(Clone)]
pub struct SessionCheckpoint {
    version: u32,
    world: World,
    seed: u64,
    state: SessionState,
    accumulator: f64,
    events: Vec<FrameEvent>,
    latest_event_id: u64,
}

#[derive(Default)]
pub struct SessionService {
    world: Option<World>,
    seed: u64,
    state: Option<SessionState>,
    accumulator: f64,
    events: Vec<FrameEvent>,
    latest_event_id: u64,
}

impl SessionService {
    pub fn new(seed: u64) -> Self {
        Self {
            world: None,
            seed,
            state: Some(SessionState::Ready),
            accumulator: 0.0,
            events: Vec::new(),
            latest_event_id: 0,
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

    pub fn setup<P: Into<RuntimeParams>>(
        &mut self,
        params: P,
        seed: u64,
    ) -> Result<MatchInfo, String> {
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
        self.latest_event_id = 0;
        self.collect_events();
        Ok(info)
    }

    pub fn setup_from_calibration(
        &mut self,
        calibration: &Path,
        seed: u64,
    ) -> Result<MatchInfo, String> {
        let params = RuntimeParams::load(calibration)?;
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

    pub fn event_history_capacity(&self) -> usize {
        EVENT_HISTORY_LIMIT
    }

    pub fn full_event_history(&self) -> Option<Vec<FrameEvent>> {
        let world = self.world.as_ref()?;
        if !world.finished || self.latest_event_id != world.next_event_id {
            return None;
        }
        Some(
            world
                .events
                .iter()
                .map(|event| world.project_event(event))
                .collect(),
        )
    }

    pub fn event_history_truncated(&self) -> bool {
        self.events.first().is_some_and(|event| event.event_id > 1)
    }

    pub fn events_since(&self, event_id: u64, limit: usize) -> Result<EventBatch, String> {
        if event_id > self.latest_event_id {
            return Err(format!(
                "event cursor {event_id} is ahead of latest event {}",
                self.latest_event_id
            ));
        }
        let first_available = self.events.first().map(|event| event.event_id);
        if let Some(first) = first_available {
            if event_id < first - 1 {
                return Err(format!(
                    "event history gap: requested after {event_id}, oldest available event is {first}"
                ));
            }
        } else if event_id < self.latest_event_id {
            return Err(format!(
                "event history gap: requested after {event_id}, latest event is {} and history is empty",
                self.latest_event_id
            ));
        }
        let events: Vec<FrameEvent> = self
            .events
            .iter()
            .filter(|event| event.event_id > event_id)
            .take(limit)
            .cloned()
            .collect();
        let next_after_event_id = events.last().map_or(event_id, |event| event.event_id);
        Ok(EventBatch {
            oldest_available_event_id: first_available,
            latest_event_id: self.latest_event_id,
            next_after_event_id,
            truncated: next_after_event_id < self.latest_event_id,
            events,
        })
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
        if self.state() != SessionState::Finished {
            self.state = Some(prior);
        }
        self.snapshot()
            .ok_or_else(|| "match snapshot unavailable".to_string())
    }

    pub fn report(&mut self) -> Option<GameReport> {
        self.collect_events();
        self.world.as_mut().map(|world| world.report())
    }

    pub fn completion_summary(&self) -> Option<CompletionSummary> {
        let world = self.world.as_ref()?;
        Some(CompletionSummary {
            seed: world.seed,
            home_team: world.home_team_name.clone(),
            away_team: world.away_team_name.clone(),
            home_score: world.score[0],
            away_score: world.score[1],
            periods: world.clock.period(),
            ticks: world.clock.tick(),
            possessions: world.possession_log.len(),
            event_count: world.next_event_id,
            box_score: world.box_score.clone(),
            finished: world.finished,
        })
    }

    /// 磁盘检查点：记录重建一次会话所需的全部信息。
    ///
    /// 世界本身不序列化（它持有随机流与逐 tick 缓存）；检查点用
    /// 「种子 + tick + 计数指纹」描述状态，恢复时按同一条路径重放，
    /// 指纹对不上就就地失败，而不是继续跑一场已经分叉的比赛。
    pub fn descriptor(&self) -> Result<SessionDescriptor, String> {
        let world = self
            .world
            .as_ref()
            .ok_or_else(|| "cannot checkpoint before match setup".to_string())?;
        Ok(SessionDescriptor {
            version: 1,
            seed: self.seed,
            tick: world.clock.tick(),
            latest_event_id: world.next_event_id,
            state: self.state(),
            home_score: world.score[0],
            away_score: world.score[1],
            possessions: world.possession_count,
        })
    }

    /// 把检查点写到磁盘；写入成功返回实际写下的描述符。
    pub fn save_descriptor(&self, path: &Path) -> Result<SessionDescriptor, String> {
        let descriptor = self.descriptor()?;
        let text = serde_json::to_string_pretty(&descriptor)
            .map_err(|error| format!("serialize checkpoint: {error}"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("create checkpoint dir {}: {error}", parent.display()))?;
        }
        std::fs::write(path, format!("{text}\n"))
            .map_err(|error| format!("write checkpoint {}: {error}", path.display()))?;
        Ok(descriptor)
    }

    /// 从磁盘检查点恢复：按同一份设置重放到检查点所在的 tick，再校验指纹。
    pub fn restore_from_descriptor(
        &mut self,
        path: &Path,
        _setup: MatchSetup,
        params: RuntimeParams,
    ) -> Result<SessionDescriptor, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("read checkpoint {}: {error}", path.display()))?;
        let descriptor: SessionDescriptor = serde_json::from_str(&text)
            .map_err(|error| format!("parse checkpoint {}: {error}", path.display()))?;
        if descriptor.version != 1 {
            return Err(format!(
                "unsupported checkpoint version {}",
                descriptor.version
            ));
        }
        self.setup(params, descriptor.seed)?;
        self.start()?;
        for _ in 0..descriptor.tick {
            if self.state() != SessionState::Running {
                break;
            }
            self.step_once();
        }
        let world = self
            .world
            .as_ref()
            .ok_or_else(|| "restore replay produced no world".to_string())?;
        if world.next_event_id != descriptor.latest_event_id
            || world.score[0] != descriptor.home_score
            || world.score[1] != descriptor.away_score
            || world.possession_count != descriptor.possessions
        {
            return Err(format!(
                "checkpoint replay diverged at tick {}: events {} vs {}, score {}:{} vs {}:{}, possessions {} vs {}",
                descriptor.tick,
                world.next_event_id,
                descriptor.latest_event_id,
                world.score[0],
                world.score[1],
                descriptor.home_score,
                descriptor.away_score,
                world.possession_count,
                descriptor.possessions
            ));
        }
        self.state = Some(descriptor.state);
        Ok(descriptor)
    }

    pub fn checkpoint(&self) -> Result<SessionCheckpoint, String> {
        if !self.accumulator.is_finite() || !(0.0..TICK_SECONDS).contains(&self.accumulator) {
            return Err("session accumulator is outside the fixed-step interval".to_string());
        }
        let world = self
            .world
            .as_ref()
            .ok_or_else(|| "cannot checkpoint before match setup".to_string())?;
        let mut events = self.events.clone();
        if world.next_event_id > self.latest_event_id {
            let first_uncollected = world
                .events
                .partition_point(|event| event.event_id <= self.latest_event_id);
            events.extend(
                world.events[first_uncollected..]
                    .iter()
                    .map(|event| world.frame_event(event)),
            );
        }
        if events.len() > EVENT_HISTORY_LIMIT {
            let excess = events.len() - EVENT_HISTORY_LIMIT;
            events.drain(0..excess);
        }
        let latest_event_id = world.next_event_id;
        Ok(SessionCheckpoint {
            version: 1,
            world: world.clone(),
            seed: self.seed,
            state: self.state(),
            accumulator: self.accumulator,
            events,
            latest_event_id,
        })
    }

    pub fn restore(&mut self, checkpoint: SessionCheckpoint) -> Result<(), String> {
        if checkpoint.version != 1 {
            return Err(format!(
                "unsupported checkpoint version {}",
                checkpoint.version
            ));
        }
        if !checkpoint.accumulator.is_finite()
            || !(0.0..TICK_SECONDS).contains(&checkpoint.accumulator)
        {
            return Err("checkpoint accumulator is outside the fixed-step interval".to_string());
        }
        if checkpoint.events.len() > EVENT_HISTORY_LIMIT {
            return Err("checkpoint event history exceeds its retention limit".to_string());
        }
        if checkpoint
            .events
            .windows(2)
            .any(|pair| pair[0].event_id + 1 != pair[1].event_id)
        {
            return Err("checkpoint event history is not contiguous".to_string());
        }
        if checkpoint.latest_event_id != checkpoint.world.next_event_id {
            return Err(
                "checkpoint event cursor does not match the world event ledger".to_string(),
            );
        }
        let retained_latest_event_id = checkpoint.events.last().map_or(0, |event| event.event_id);
        if retained_latest_event_id != checkpoint.latest_event_id {
            return Err("checkpoint event history does not end at its latest event".to_string());
        }
        if let Some(first) = checkpoint.events.first() {
            let expected_oldest = checkpoint
                .latest_event_id
                .saturating_sub(checkpoint.events.len() as u64)
                + 1;
            if first.event_id != expected_oldest {
                return Err("checkpoint event history exceeds its retention window".to_string());
            }
        }
        if (checkpoint.state == SessionState::Finished) != checkpoint.world.finished {
            return Err("checkpoint session state disagrees with the world lifecycle".to_string());
        }
        self.world = Some(checkpoint.world);
        self.seed = checkpoint.seed;
        self.state = Some(checkpoint.state);
        self.accumulator = checkpoint.accumulator;
        self.events = checkpoint.events;
        self.latest_event_id = checkpoint.latest_event_id;
        Ok(())
    }

    fn collect_events(&mut self) {
        let Some(world) = self.world.as_ref() else {
            return;
        };
        let previous_latest_event_id = self.latest_event_id;
        self.latest_event_id = world.next_event_id;
        let first_uncollected = world
            .events
            .partition_point(|event| event.event_id <= previous_latest_event_id);
        let collected: Vec<FrameEvent> = world.events[first_uncollected..]
            .iter()
            .map(|event| world.frame_event(event))
            .collect();
        self.events.extend(collected);
        if self.events.len() > EVENT_HISTORY_LIMIT {
            let excess = self.events.len() - EVENT_HISTORY_LIMIT;
            self.events.drain(0..excess);
        }
    }

    fn step_once(&mut self) {
        let Some(world) = self.world.as_mut() else {
            return;
        };
        world.step(TICK_SECONDS);
        let finished = world.finished;
        self.collect_events();
        if finished {
            self.state = Some(SessionState::Finished);
            self.accumulator = 0.0;
        }
    }
}

fn world_finished(world: &World) -> bool {
    world.finished
}

#[cfg(test)]
mod tests {
    use super::{SessionService, SessionState, EVENT_HISTORY_LIMIT};
    use crate::params::workspace_calibration_dir;
    use crate::params::RuntimeParams;

    #[test]
    fn real_game_event_history_stays_bounded_and_reports_evicted_cursors() {
        let mut session = SessionService::new(43);
        session
            .setup(
                RuntimeParams::load(&workspace_calibration_dir()).expect("calibration loads"),
                43,
            )
            .expect("session setup");
        session.start().expect("session start");

        for _ in 0..30 {
            session.fast_forward(120.0).expect("real match advances");
            if session.event_history_truncated() || session.state() == SessionState::Finished {
                break;
            }
        }

        assert!(session.event_history_truncated());
        let events = session.events(usize::MAX);
        assert_eq!(events.len(), EVENT_HISTORY_LIMIT);
        assert!(session.events_since(0, 1).is_err());
        let first_id = events[0].event_id;
        let boundary = session
            .events_since(first_id - 1, 1)
            .expect("the cursor immediately before the oldest event remains valid");
        assert_eq!(boundary.events[0].event_id, first_id);
    }
}
