//! 比赛内核时钟：整数微秒状态、类型化指令与唯一写入入口。

use nba_domain::rules::{micros_to_seconds, seconds_to_micros, MICROS_PER_SECOND, TICK_MICROS};
use nba_domain::{CheckpointId, EventId, ProgramId};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use crate::state::{ClockAdvance, Phase};

pub mod directives;
pub use directives::{
    ClockCheckpoint, ClockCheckpointError, ClockCommitLog, ClockDirective, ClockDirectiveApplied,
    ClockDirectiveContext, ClockSegmentApplied, TimerId, TimerRunning,
};

/// 全部时钟的整数微秒状态。
///
/// 只有 `ClockController` 能改写计时器值。其余模块读取秒数投影，或者读取
/// 微秒值生成事实；直接字段写入由私有字段与模块边界拒绝。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClockState {
    values: BTreeMap<TimerId, u64>,
    running: TimerRunning,
    checkpoints: BTreeMap<String, ClockCheckpoint>,
    player_count: usize,
    period: u32,
    tick: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClockStateError {
    MissingTimer(TimerId),
    UnexpectedTimer(TimerId),
    InvalidCheckpoint(String, ClockCheckpointError),
    InvalidAdvance(String),
}

impl fmt::Display for ClockStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTimer(timer) => write!(f, "时钟状态缺少 {timer}"),
            Self::UnexpectedTimer(timer) => write!(f, "时钟状态包含未知 {timer}"),
            Self::InvalidCheckpoint(id, source) => {
                write!(f, "计时器检查点 {id} 无效: {source}")
            }
            Self::InvalidAdvance(reason) => write!(f, "时钟推进记录无效: {reason}"),
        }
    }
}

impl Error for ClockStateError {}

/// 时钟提交记录：指令、连续推进段与检查点。
///
/// 定义在 `directives` 模块，与指令、检查点写在一起；此处重导出。
pub use directives::ClockCommit;
impl ClockState {
    pub(crate) fn new(
        player_count: usize,
        game_clock_micros: u64,
        shot_clock_micros: u64,
        period: u32,
    ) -> Self {
        assert!(period > 0, "period number must be positive");
        let mut values = BTreeMap::new();
        for timer in TimerId::all(player_count) {
            values.insert(timer, 0);
        }
        values.insert(TimerId::GameClock, game_clock_micros);
        values.insert(TimerId::ShotClock, shot_clock_micros);
        values.insert(TimerId::PeriodDuration, game_clock_micros);
        values.insert(TimerId::Period, u64::from(period));
        Self {
            values,
            running: TimerRunning::all(),
            checkpoints: BTreeMap::new(),
            player_count,
            period,
            tick: 0,
        }
    }

    /// 所有计时器的运行标志都关闭：时钟从一个停表状态开始。
    ///
    /// 推进会写入正确的运行标志，因此初始标志只影响「第一个推进之前」
    /// 的读数。赛前、节间与暂停期间时钟都是停的。
    pub fn all_stopped(&mut self) {
        self.running = TimerRunning::empty();
    }

    /// 独立验证器的重放起点：所有计时器为零，由首批提交记录写明初值。
    pub fn for_replay(player_count: usize) -> Self {
        let mut state = Self::new(player_count, 0, 0, 1);
        state.running = TimerRunning::all();
        state
    }

    pub fn player_count(&self) -> usize {
        self.player_count
    }

    pub fn timer_micros(&self, timer: TimerId) -> u64 {
        *self
            .values
            .get(&timer)
            .unwrap_or_else(|| panic!("计时器 {timer} 没有初始化"))
    }

    pub fn timer_seconds(&self, timer: TimerId) -> f64 {
        micros_to_seconds(self.timer_micros(timer))
    }

    pub fn running(&self, timer: TimerId) -> bool {
        self.running.get(timer)
    }

    pub fn running_flags(&self) -> TimerRunning {
        self.running
    }

    pub fn period(&self) -> u32 {
        self.period
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn simulation_micros(&self) -> u64 {
        self.timer_micros(TimerId::Simulation)
    }

    pub fn game_clock_micros(&self) -> u64 {
        self.timer_micros(TimerId::GameClock)
    }

    pub fn shot_clock_micros(&self) -> u64 {
        self.timer_micros(TimerId::ShotClock)
    }

    pub fn elapsed_micros(&self) -> u64 {
        self.timer_micros(TimerId::Elapsed)
    }

    pub fn checkpoint(&self, checkpoint_id: CheckpointId) -> Option<&ClockCheckpoint> {
        self.checkpoints.get(&checkpoint_id.to_string())
    }

    pub fn validate(&self) -> Result<(), ClockStateError> {
        assert!(self.period > 0, "period number must be positive");
        let expected: BTreeSet<TimerId> = TimerId::all(self.player_count).into_iter().collect();
        let actual: BTreeSet<TimerId> = self.values.keys().copied().collect();
        if let Some(timer) = expected.difference(&actual).next() {
            return Err(ClockStateError::MissingTimer(*timer));
        }
        if let Some(timer) = actual.difference(&expected).next() {
            return Err(ClockStateError::UnexpectedTimer(*timer));
        }
        for (id, checkpoint) in &self.checkpoints {
            if id.parse::<CheckpointId>().is_err() {
                return Err(ClockStateError::InvalidCheckpoint(
                    id.clone(),
                    ClockCheckpointError::InvalidId(id.clone()),
                ));
            }
            if let Err(source) = checkpoint.validate_for(self.player_count) {
                return Err(ClockStateError::InvalidCheckpoint(id.clone(), source));
            }
        }
        Ok(())
    }

    /// 按公开提交记录重放时钟状态。验证器只读取帧事件里的提交记录。
    pub fn replay(&mut self, commits: &[ClockCommit]) -> Result<(), ClockStateError> {
        for commit in commits {
            match commit {
                ClockCommit::Directive(applied) => {
                    if let ClockDirective::Restore { checkpoint_id, .. } = applied.directive {
                        if self.checkpoint(checkpoint_id).is_none() {
                            return Err(ClockStateError::InvalidCheckpoint(
                                checkpoint_id.to_string(),
                                ClockCheckpointError::InvalidId(checkpoint_id.to_string()),
                            ));
                        }
                    }
                    let context = ClockDirectiveContext {
                        cause_event_id: applied.cause_event_id,
                        program_id: applied.program_id,
                    };
                    let mut sink = ClockCommitLog::new();
                    ClockController::new(self, &mut sink).apply(applied.directive, context);
                }
                ClockCommit::Segment(applied) => self.replay_segment(applied)?,
                ClockCommit::CheckpointSaved(id, checkpoint) => {
                    if id.parse::<CheckpointId>().is_err() {
                        return Err(ClockStateError::InvalidCheckpoint(
                            id.clone(),
                            ClockCheckpointError::InvalidId(id.clone()),
                        ));
                    }
                    if let Err(source) = checkpoint.validate_for(self.player_count) {
                        return Err(ClockStateError::InvalidCheckpoint(id.clone(), source));
                    }
                    self.checkpoints.insert(id.clone(), checkpoint.clone());
                }
            }
        }
        self.validate()
    }

    fn replay_segment(
        &mut self,
        applied: &ClockSegmentApplied,
    ) -> Result<(), ClockStateError> {
        let delta = applied.micros;
        if applied.simulation_before_micros != self.simulation_micros() {
            return Err(ClockStateError::InvalidAdvance(format!(
                "模拟时间起点 {} 与状态 {} 不一致",
                applied.simulation_before_micros,
                self.simulation_micros()
            )));
        }
        let expected_game_clock = if applied.game_clock_running {
            applied.game_clock_before_micros.saturating_sub(delta)
        } else {
            applied.game_clock_before_micros
        };
        let expected_shot_clock = if applied.shot_clock_running {
            applied.shot_clock_before_micros.saturating_sub(delta)
        } else {
            applied.shot_clock_before_micros
        };
        let expected_elapsed = if applied.elapsed_running {
            applied.elapsed_before_micros.checked_add(delta).ok_or_else(|| {
                ClockStateError::InvalidAdvance("累计时间溢出".to_string())
            })?
        } else {
            applied.elapsed_before_micros
        };
        let expected_player_time = if applied.player_time_running {
            applied.player_time_before_micros.checked_add(delta).ok_or_else(|| {
                ClockStateError::InvalidAdvance("球员累计时间溢出".to_string())
            })?
        } else {
            applied.player_time_before_micros
        };
        let values = [
            ("比赛钟", applied.game_clock_after_micros, expected_game_clock),
            ("进攻钟", applied.shot_clock_after_micros, expected_shot_clock),
            ("累计时间", applied.elapsed_after_micros, expected_elapsed),
            ("球员累计时间", applied.player_time_after_micros, expected_player_time),
        ];
        for (label, observed, expected) in values {
            if observed != expected {
                return Err(ClockStateError::InvalidAdvance(format!(
                    "{label} 终点 {observed} 与记录 {expected} 不一致"
                )));
            }
        }
        self.set_tick(applied.tick_end);
        self.set_timer_micros(TimerId::Tick, applied.tick_end);
        self.set_timer_micros(TimerId::Simulation, applied.simulation_after_micros);
        self.set_timer_micros(TimerId::GameClock, applied.game_clock_after_micros);
        self.set_timer_micros(TimerId::ShotClock, applied.shot_clock_after_micros);
        self.set_timer_micros(TimerId::Elapsed, applied.elapsed_after_micros);
        self.set_timer_micros(TimerId::PlayerTimeTotal, applied.player_time_after_micros);
        self.set_running(TimerId::GameClock, applied.game_clock_running);
        self.set_running(TimerId::ShotClock, applied.shot_clock_running);
        self.set_running(TimerId::Elapsed, applied.elapsed_running);
        self.set_running(TimerId::PlayerTimeTotal, applied.player_time_running);
        if applied.player_time_running {
            for handle in &applied.player_handles {
                if *handle >= self.player_count {
                    return Err(ClockStateError::InvalidAdvance(format!(
                        "球员句柄 {handle} 超出时钟状态范围"
                    )));
                }
                let timer = TimerId::PlayerTime(*handle);
                let before = self.timer_micros(timer);
                let after = before.checked_add(delta).ok_or_else(|| {
                    ClockStateError::InvalidAdvance(format!("球员 {handle} 时间溢出"))
                })?;
                self.set_timer_micros(timer, after);
            }
        }
        Ok(())
    }

    pub(crate) fn set_timer_micros(&mut self, timer: TimerId, micros: u64) {
        let value = self
            .values
            .get_mut(&timer)
            .unwrap_or_else(|| panic!("计时器 {timer} 没有初始化"));
        *value = micros;
    }

    pub(crate) fn set_running(&mut self, timer: TimerId, running: bool) {
        self.running.set(timer, running);
    }

    pub(crate) fn set_period(&mut self, period: u32) {
        assert!(period > 0, "period number must be positive");
        self.period = period;
    }

    pub(crate) fn set_tick(&mut self, tick: u64) {
        self.tick = tick;
    }

    pub(crate) fn set_tick_timer(&mut self, tick: u64) {
        self.set_timer_micros(TimerId::Tick, tick);
    }

    pub(crate) fn save_checkpoint(
        &mut self,
        checkpoint_id: CheckpointId,
    ) -> ClockCheckpoint {
        let key = checkpoint_id.to_string();
        let checkpoint = ClockCheckpoint::from_state(self);
        checkpoint
            .validate_for(self.player_count)
            .unwrap_or_else(|error| panic!("计时器检查点 {checkpoint_id} 写入失败: {error}"));
        self.checkpoints.insert(key, checkpoint.clone());
        checkpoint
    }
}

impl<'de> Deserialize<'de> for ClockState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            values: BTreeMap<TimerId, u64>,
            running: TimerRunning,
            checkpoints: BTreeMap<String, ClockCheckpoint>,
            player_count: usize,
            period: u32,
            tick: u64,
        }
        let raw = Raw::deserialize(deserializer)?;
        let state = Self {
            values: raw.values,
            running: raw.running,
            checkpoints: raw.checkpoints,
            player_count: raw.player_count,
            period: raw.period,
            tick: raw.tick,
        };
        state.validate().map_err(D::Error::custom)?;
        Ok(state)
    }
}

/// 全部时钟写入的唯一入口。
pub struct ClockController<'a> {
    state: &'a mut ClockState,
    commits: &'a mut ClockCommitLog,
}

fn directives_from_commits(commits: &[ClockCommit]) -> Vec<ClockDirectiveApplied> {
    commits
        .iter()
        .filter_map(|commit| match commit {
            ClockCommit::Directive(applied) => Some(*applied),
            _ => None,
        })
        .collect()
}

impl<'a> ClockController<'a> {
    pub fn new(state: &'a mut ClockState, commits: &'a mut ClockCommitLog) -> Self {
        Self { state, commits }
    }

    pub fn state(&self) -> ClockState {
        self.state.clone()
    }

    /// 按时钟策略写入运行标志。
    ///
    /// 运行标志是权威状态：只有 `Start` 与 `Stop` 指令能改写它。策略与
    /// 当前标志相同时不发指令，因此提交记录里留下的是「比赛钟在这一时刻
    /// 停表」这样的真实事件，而不是每个 tick 重复一遍同样的状态。
    pub fn apply_policy(
        &mut self,
        policy: ClockAdvance,
        context: ClockDirectiveContext,
    ) -> Vec<ClockDirectiveApplied> {
        policy.validate();
        let wanted = [
            (TimerId::GameClock, policy.game_clock),
            (TimerId::ShotClock, policy.shot_clock),
            (TimerId::Elapsed, policy.elapsed),
            (TimerId::PlayerTimeTotal, policy.player_time),
        ];
        let mut applied = Vec::new();
        for (timer, running) in wanted {
            if self.state.running(timer) == running {
                continue;
            }
            let directive = if running {
                ClockDirective::Start { timer }
            } else {
                ClockDirective::Stop { timer }
            };
            applied.push(self.apply(directive, context));
        }
        applied
    }

    pub fn apply(
        &mut self,
        directive: ClockDirective,
        context: ClockDirectiveContext,
    ) -> ClockDirectiveApplied {
        let timer = directive.timer();
        let before_micros = self.state.timer_micros(timer);
        let after_micros = match directive {
            ClockDirective::Start { .. } => {
                self.state.set_running(timer, true);
                before_micros
            }
            ClockDirective::Stop { .. } => {
                self.state.set_running(timer, false);
                before_micros
            }
            ClockDirective::Set { micros, .. } => {
                if timer == TimerId::Tick {
                    self.state.set_tick(micros);
                }
                if timer == TimerId::Period {
                    let period = u32::try_from(micros)
                        .unwrap_or_else(|_| panic!("节次计时器超出 u32 范围: {micros}"));
                    self.state.set_period(period);
                }
                self.state.set_timer_micros(timer, micros);
                micros
            }
            ClockDirective::Restore { checkpoint_id, .. } => {
                let micros = self
                    .state
                    .checkpoint(checkpoint_id)
                    .unwrap_or_else(|| panic!("计时器检查点 {checkpoint_id} 不存在"))
                    .get(timer)
                    .unwrap_or_else(|error| panic!("{error}"));
                self.state.set_timer_micros(timer, micros);
                micros
            }
        };
        let applied = ClockDirectiveApplied {
            directive,
            timer,
            before_micros,
            after_micros,
            cause_event_id: context.cause_event_id,
            program_id: context.program_id,
        };
        // 指令会打断连续推进：先结算正在积累的段，再写指令。
        self.commits.flush_segment();
        self.commits.push(ClockCommit::Directive(applied));
        applied
    }

    pub fn save_checkpoint(&mut self, checkpoint_id: CheckpointId) {
        let key = checkpoint_id.to_string();
        let checkpoint = self.state.save_checkpoint(checkpoint_id);
        self.commits.flush_segment();
        self.commits
            .push(ClockCommit::CheckpointSaved(key, checkpoint));
    }

    pub fn initialize(
        &mut self,
        context: ClockDirectiveContext,
    ) -> Vec<ClockDirectiveApplied> {
        let mut applied = Vec::new();
        let timers = TimerId::all(self.state.player_count());
        for timer in timers {
            let micros = self.state.timer_micros(timer);
            applied.push(self.apply(
                ClockDirective::Set { timer, micros },
                context,
            ));
        }
        applied
    }

    pub fn set_program_period(
        &mut self,
        period: u32,
        period_micros: u64,
        game_clock_micros: u64,
        shot_clock_micros: u64,
        context: ClockDirectiveContext,
    ) -> Vec<ClockDirectiveApplied> {
        self.state.set_period(period);
        let mut applied = Vec::new();
        for (timer, micros) in [
            (TimerId::Period, u64::from(period)),
            (TimerId::PeriodDuration, period_micros),
            (TimerId::GameClock, game_clock_micros),
            (TimerId::ShotClock, shot_clock_micros),
        ] {
            applied.push(self.apply(
                ClockDirective::Set { timer, micros },
                context,
            ));
        }
        applied
    }

    /// 推进 tick 计数。
    ///
    /// tick 计数不是指令：它是模拟推进的序号，由连续推进段记录，因此
    /// 这里不发提交记录。
    pub fn advance_tick(&mut self) {
        let tick = self.state.tick().checked_add(1).expect("tick counter overflowed");
        self.state.set_tick(tick);
        self.state.set_tick_timer(tick);
    }

    pub fn advance(
        &mut self,
        micros: u64,
        clocks: ClockAdvance,
        player_handles: &[usize],
        context: ClockDirectiveContext,
    ) -> ClockSegmentApplied {
        clocks.validate();
        // 运行标志是权威状态：推进只读取它，不改写它。开始与停止由
        // `Start` 与 `Stop` 指令写入，因此「哪些钟在走」永远有一条
        // 可重建的指令依据。
        let game_clock_running = self.state.running(TimerId::GameClock);
        let shot_clock_running = self.state.running(TimerId::ShotClock);
        let elapsed_running = self.state.running(TimerId::Elapsed);
        let player_time_running = self.state.running(TimerId::PlayerTimeTotal);
        let simulation_before_micros = self.state.simulation_micros();
        let simulation_after_micros = simulation_before_micros
            .checked_add(micros)
            .expect("simulation clock overflowed");
        let game_clock_before_micros = self.state.game_clock_micros();
        let shot_clock_before_micros = self.state.shot_clock_micros();
        let elapsed_before_micros = self.state.elapsed_micros();
        let player_time_before_micros = self.state.timer_micros(TimerId::PlayerTimeTotal);
        let game_clock_after_micros = if game_clock_running {
            game_clock_before_micros.saturating_sub(micros)
        } else {
            game_clock_before_micros
        };
        let shot_clock_after_micros = if shot_clock_running {
            shot_clock_before_micros.saturating_sub(micros)
        } else {
            shot_clock_before_micros
        };
        let elapsed_after_micros = if elapsed_running {
            elapsed_before_micros
                .checked_add(micros)
                .expect("elapsed clock overflowed")
        } else {
            elapsed_before_micros
        };
        let player_time_after_micros = if player_time_running {
            player_time_before_micros
                .checked_add(micros)
                .expect("player time total overflowed")
        } else {
            player_time_before_micros
        };
        self.state
            .set_timer_micros(TimerId::Simulation, simulation_after_micros);
        self.state
            .set_timer_micros(TimerId::GameClock, game_clock_after_micros);
        self.state
            .set_timer_micros(TimerId::ShotClock, shot_clock_after_micros);
        self.state
            .set_timer_micros(TimerId::Elapsed, elapsed_after_micros);
        self.state
            .set_timer_micros(TimerId::PlayerTimeTotal, player_time_after_micros);
        if player_time_running {
            for handle in player_handles {
                let timer = TimerId::PlayerTime(*handle);
                let before = self.state.timer_micros(timer);
                let after = before
                    .checked_add(micros)
                    .expect("player time clock overflowed");
                self.state.set_timer_micros(timer, after);
            }
        }
        let applied = ClockSegmentApplied {
            tick_begin: self.state.tick().saturating_sub(1),
            tick_end: self.state.tick(),
            micros,
            simulation_before_micros,
            simulation_after_micros,
            game_clock_before_micros,
            game_clock_after_micros,
            shot_clock_before_micros,
            shot_clock_after_micros,
            elapsed_before_micros,
            elapsed_after_micros,
            player_time_before_micros,
            player_time_after_micros,
            player_handles: Vec::new(),
            game_clock_running,
            shot_clock_running,
            elapsed_running,
            player_time_running,
            cause_event_id: context.cause_event_id,
            program_id: context.program_id,
        };
        self.commits.record_segment(applied.clone(), player_handles);
        applied
    }
}

pub fn clocks_for(phase: Phase, flight: Option<crate::state::BallFlight>) -> ClockAdvance {
    match flight {
        Some(flight) => flight.kind.clocks(),
        None => phase.clocks(),
    }
}

/// 根据当前比赛阶段推进对应时钟。
pub fn advance(
    phase: Phase,
    flight: Option<crate::state::BallFlight>,
    clock: &mut crate::state::Clock,
    seconds: f64,
) -> ClockAdvance {
    let clocks = clocks_for(phase, flight);
    clock.advance_seconds(seconds, clocks, &[]);
    clocks
}

/// 按阶段推进时钟，并把本 tick 在场球员时间写入对应计时器。
pub fn advance_with_context(
    phase: Phase,
    flight: Option<crate::state::BallFlight>,
    clock: &mut crate::state::Clock,
    seconds: f64,
    player_handles: &[usize],
    context: ClockDirectiveContext,
) -> (ClockAdvance, ClockSegmentApplied) {
    let clocks = clocks_for(phase, flight);
    let applied = clock.advance_seconds_with_context(seconds, clocks, player_handles, context);
    (clocks, applied)
}

pub fn root_context() -> ClockDirectiveContext {
    ClockDirectiveContext::root(ProgramId::from_raw(1))
}

pub fn micros_per_tick() -> u64 {
    TICK_MICROS
}

pub fn micros_from_seconds(seconds: f64) -> u64 {
    seconds_to_micros(seconds)
}

pub fn seconds_from_micros(micros: u64) -> f64 {
    micros_to_seconds(micros)
}

pub fn micros_per_second() -> u64 {
    MICROS_PER_SECOND
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Clock, TeamSide};

    fn context() -> ClockDirectiveContext {
        ClockDirectiveContext {
            cause_event_id: EventId::from_raw(7),
            program_id: ProgramId::from_raw(2),
        }
    }

    #[test]
    fn clock_families_follow_phase_policy() {
        let phases = [
            (Phase::Pregame, false, false, false),
            (
                Phase::DeadBall {
                    team: TeamSide::Home,
                    ready_at: 0.0,
                },
                false,
                false,
                false,
            ),
            (Phase::Live, true, true, true),
            (Phase::Finished, false, false, false),
        ];
        for (phase, game_clock_runs, shot_clock_runs, elapsed_runs) in phases {
            let mut clock = Clock::new(720.0, 24.0);
            let policy = clocks_for(phase, None);
            clock.apply_policy(policy, context());
            let advanced = advance(phase, None, &mut clock, 0.25);
            assert_eq!(advanced.game_clock, game_clock_runs, "{phase:?}");
            assert_eq!(advanced.shot_clock, shot_clock_runs, "{phase:?}");
            assert_eq!(advanced.elapsed, elapsed_runs, "{phase:?}");
            assert_eq!(
                clock.game_clock(),
                if game_clock_runs { 719.75 } else { 720.0 }
            );
            assert_eq!(
                clock.shot_clock(),
                if shot_clock_runs { 23.75 } else { 24.0 }
            );
            assert_eq!(clock.elapsed(), if elapsed_runs { 0.25 } else { 0.0 });
            assert_eq!(clock.simulation_time(), 0.25);
        }
        for (kind, game_clock_runs, shot_clock_runs) in [
            (crate::state::FlightKind::Pass, true, true),
            (crate::state::FlightKind::Shot, true, false),
            (crate::state::FlightKind::FreeThrow, false, false),
        ] {
            let mut clock = Clock::new(720.0, 24.0);
            let flight = crate::state::BallFlight {
                kind,
                resolve_at: 1.0,
            };
            let policy = clocks_for(Phase::Live, Some(flight));
            clock.apply_policy(policy, context());
            let advanced = advance(Phase::Live, Some(flight), &mut clock, 0.25);
            assert_eq!(advanced.game_clock, game_clock_runs, "{kind:?}");
            assert_eq!(advanced.shot_clock, shot_clock_runs, "{kind:?}");
        }
    }

    #[test]
    fn all_four_directives_write_through_the_controller() {
        let mut clock = Clock::new(720.0, 24.0);
        assert_eq!(clock.take_commits().len(), TimerId::all(0).len());
        let checkpoint_id = CheckpointId::from_raw(5);
        {
            let mut controller = clock.controller();
            controller.save_checkpoint(checkpoint_id);
            let stopped = controller.apply(
                ClockDirective::Stop {
                    timer: TimerId::ShotClock,
                },
                context(),
            );
            assert_eq!(stopped.before_micros, 24_000_000);
            assert!(!controller.state().running(TimerId::ShotClock));
            let set = controller.apply(
                ClockDirective::Set {
                    timer: TimerId::ShotClock,
                    micros: 14_000_000,
                },
                context(),
            );
            assert_eq!(set.after_micros, 14_000_000);
            let started = controller.apply(
                ClockDirective::Start {
                    timer: TimerId::ShotClock,
                },
                context(),
            );
            assert!(controller.state().running(TimerId::ShotClock));
            assert_eq!(started.after_micros, 14_000_000);
            let restored = controller.apply(
                ClockDirective::Restore {
                    timer: TimerId::ShotClock,
                    checkpoint_id,
                },
                context(),
            );
            assert_eq!(restored.after_micros, 24_000_000);
        }
        let last = clock.take_commits();
        assert_eq!(last.len(), 5);
        assert!(matches!(last[0], ClockCommit::CheckpointSaved(_, _)));
        assert!(matches!(last[1], ClockCommit::Directive(_)));
        assert_eq!(clock.shot_clock(), 24.0);
    }

    #[test]
    fn tick_and_phase_advances_replay_from_commits() {
        let mut clock = Clock::with_player_count(2, 720.0, 24.0);
        clock.take_commits();
        clock.advance_tick();
        clock.apply_policy(clocks_for(Phase::Live, None), context());
        let (_, applied) =
            advance_with_context(Phase::Live, None, &mut clock, 0.05, &[0, 1], context());
        assert_eq!(clock.simulation_time(), 0.05);
        assert_eq!(clock.tick(), 1);
        assert_eq!(applied.simulation_after_micros, 50_000);
        assert!(applied.game_clock_running && applied.shot_clock_running);
        assert_eq!(clock.state.timer_micros(TimerId::PlayerTime(1)), 50_000);
        assert_eq!(clock.state.timer_micros(TimerId::PlayerTimeTotal), 50_000);
        let commits = clock.drain_commits();
        let mut replay = Clock::with_player_count(2, 720.0, 24.0);
        replay.drain_commits();
        replay.state.replay(&commits).expect("clock commits replay");
        assert_eq!(replay.state, clock.state);
    }

    #[test]
    fn match_clock_and_shot_clock_can_be_set_independently() {
        let mut clock = Clock::new(720.0, 24.0);
        clock.set_shot_clock_at_least(14.0);
        assert_eq!(clock.game_clock(), 720.0);
        assert_eq!(clock.shot_clock(), 24.0);
        clock.set_shot_clock(14.0);
        assert_eq!(clock.shot_clock(), 14.0);
        clock.start_next_period(300.0, 24.0);
        assert_eq!(clock.period(), 2);
        assert_eq!(clock.game_clock(), 300.0);
        assert_eq!(clock.shot_clock(), 24.0);
    }

    #[test]
    fn clock_state_serialization_rejects_missing_timers() {
        let clock = Clock::with_player_count(2, 720.0, 24.0);
        let text = serde_json::to_string(&clock.state).expect("clock state encodes");
        let restored: ClockState = serde_json::from_str(&text).expect("clock state decodes");
        assert_eq!(restored, clock.state);
        let mut value: serde_json::Value = serde_json::from_str(&text).expect("value decodes");
        value["values"]
            .as_object_mut()
            .expect("values object")
            .remove("player_time_1");
        let error = serde_json::from_value::<ClockState>(value)
            .expect_err("missing timer must be rejected");
        assert!(error.to_string().contains("缺少"));
    }
}
