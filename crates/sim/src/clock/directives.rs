//! 时钟身份、类型化指令与检查点。

use nba_domain::{CheckpointId, EventId, ProgramId};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::str::FromStr;

/// 计时器身份。`PlayerTime` 携带全场球员句柄。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TimerId {
    Simulation,
    GameClock,
    ShotClock,
    Elapsed,
    PlayerTimeTotal,
    PlayerTime(usize),
    Backcourt,
    Lane,
    Inbound,
    BackToBasket,
    FreeThrow,
    Review,
    Timeout,
    ProgramPrep,
    Period,
    PeriodDuration,
    Tick,
}

impl TimerId {
    pub fn all(player_count: usize) -> Vec<Self> {
        let mut timers = vec![
            Self::Simulation,
            Self::GameClock,
            Self::ShotClock,
            Self::Elapsed,
            Self::PlayerTimeTotal,
            Self::Backcourt,
            Self::Lane,
            Self::Inbound,
            Self::BackToBasket,
            Self::FreeThrow,
            Self::Review,
            Self::Timeout,
            Self::ProgramPrep,
            Self::Period,
            Self::PeriodDuration,
            Self::Tick,
        ];
        timers.extend((0..player_count).map(Self::PlayerTime));
        timers
    }

    pub fn as_str(self) -> String {
        match self {
            Self::Simulation => "simulation".to_string(),
            Self::GameClock => "game_clock".to_string(),
            Self::ShotClock => "shot_clock".to_string(),
            Self::Elapsed => "elapsed".to_string(),
            Self::PlayerTimeTotal => "player_time_total".to_string(),
            Self::PlayerTime(handle) => format!("player_time_{handle}"),
            Self::Backcourt => "backcourt".to_string(),
            Self::Lane => "lane".to_string(),
            Self::Inbound => "inbound".to_string(),
            Self::BackToBasket => "back_to_basket".to_string(),
            Self::FreeThrow => "free_throw".to_string(),
            Self::Review => "review".to_string(),
            Self::Timeout => "timeout".to_string(),
            Self::ProgramPrep => "program_prep".to_string(),
            Self::Period => "period".to_string(),
            Self::PeriodDuration => "period_duration".to_string(),
            Self::Tick => "tick".to_string(),
        }
    }

    const fn bit(self) -> u16 {
        match self {
            Self::Simulation => 1 << 0,
            Self::GameClock => 1 << 1,
            Self::ShotClock => 1 << 2,
            Self::Elapsed => 1 << 3,
            Self::PlayerTimeTotal | Self::PlayerTime(_) => 1 << 4,
            Self::Backcourt => 1 << 5,
            Self::Lane => 1 << 6,
            Self::Inbound => 1 << 7,
            Self::BackToBasket => 1 << 8,
            Self::FreeThrow => 1 << 9,
            Self::Review => 1 << 10,
            Self::Timeout => 1 << 11,
            Self::ProgramPrep => 1 << 12,
            Self::Period => 1 << 13,
            Self::PeriodDuration => 1 << 14,
            Self::Tick => 1 << 15,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimerIdParseError(String);

impl fmt::Display for TimerIdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "未知计时器身份 {}", self.0)
    }
}

impl Error for TimerIdParseError {}

impl FromStr for TimerId {
    type Err = TimerIdParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let timer = match input {
            "simulation" => Self::Simulation,
            "game_clock" => Self::GameClock,
            "shot_clock" => Self::ShotClock,
            "elapsed" => Self::Elapsed,
            "player_time_total" => Self::PlayerTimeTotal,
            "backcourt" => Self::Backcourt,
            "lane" => Self::Lane,
            "inbound" => Self::Inbound,
            "back_to_basket" => Self::BackToBasket,
            "free_throw" => Self::FreeThrow,
            "review" => Self::Review,
            "timeout" => Self::Timeout,
            "program_prep" => Self::ProgramPrep,
            "period" => Self::Period,
            "period_duration" => Self::PeriodDuration,
            "tick" => Self::Tick,
            other => other
                .strip_prefix("player_time_")
                .and_then(|raw| raw.parse::<usize>().ok())
                .map(Self::PlayerTime)
                .ok_or_else(|| TimerIdParseError(other.to_string()))?,
        };
        Ok(timer)
    }
}

impl fmt::Display for TimerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_str())
    }
}

impl Serialize for TimerId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.as_str())
    }
}

impl<'de> Deserialize<'de> for TimerId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(D::Error::custom)
    }
}

/// 每个计时器的运行标志。位集合随 `ClockState` 一起保存。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TimerRunning(u16);

impl TimerRunning {
    pub const fn all() -> Self {
        Self(u16::MAX)
    }

    /// 所有计时器都停表。
    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn get(self, timer: TimerId) -> bool {
        self.0 & timer.bit() != 0
    }

    pub fn set(&mut self, timer: TimerId, running: bool) {
        let bit = timer.bit();
        if running {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }
}

impl Default for TimerRunning {
    fn default() -> Self {
        Self::all()
    }
}

/// 一次时钟写入要求。只有四种形态，未知形态由类型系统拒绝。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockDirective {
    Start { timer: TimerId },
    Stop { timer: TimerId },
    Set { timer: TimerId, micros: u64 },
    Restore { timer: TimerId, checkpoint_id: CheckpointId },
}

impl ClockDirective {
    pub fn timer(self) -> TimerId {
        match self {
            Self::Start { timer }
            | Self::Stop { timer }
            | Self::Set { timer, .. }
            | Self::Restore { timer, .. } => timer,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start { .. } => "start",
            Self::Stop { .. } => "stop",
            Self::Set { .. } => "set",
            Self::Restore { .. } => "restore",
        }
    }

    pub fn value_micros(self) -> Option<u64> {
        match self {
            Self::Set { micros, .. } => Some(micros),
            _ => None,
        }
    }

    pub fn checkpoint_id(self) -> Option<CheckpointId> {
        match self {
            Self::Restore { checkpoint_id, .. } => Some(checkpoint_id),
            _ => None,
        }
    }
}

/// 指令来源。写入事实保留来源事件与程序身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockDirectiveContext {
    pub cause_event_id: EventId,
    pub program_id: ProgramId,
}

impl ClockDirectiveContext {
    pub fn root(program_id: ProgramId) -> Self {
        Self {
            cause_event_id: EventId::from_raw(0),
            program_id,
        }
    }
}

/// 一次指令执行后的前后值。投影层据此发布 `ClockDirectivesCommitted`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockDirectiveApplied {
    pub directive: ClockDirective,
    pub timer: TimerId,
    pub before_micros: u64,
    pub after_micros: u64,
    pub cause_event_id: EventId,
    pub program_id: ProgramId,
}

/// 连续推进的一段时钟。
///
/// 推进期间没有指令写入，运行标志不变，因此整段可以用「起点读数 + 推进
/// 时长」表达。事件流按段发布时钟提交记录，验证器据此重建时钟；逐 tick
/// 发布一条记录会让事件数膨胀几十倍，段记录给出同样的重建能力。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockSegmentApplied {
    /// 段起点的 tick。
    pub tick_begin: u64,
    /// 段终点的 tick。
    pub tick_end: u64,
    /// 本段的推进时长（微秒）。
    pub micros: u64,
    pub simulation_before_micros: u64,
    pub simulation_after_micros: u64,
    pub game_clock_before_micros: u64,
    pub game_clock_after_micros: u64,
    pub shot_clock_before_micros: u64,
    pub shot_clock_after_micros: u64,
    pub elapsed_before_micros: u64,
    pub elapsed_after_micros: u64,
    pub player_time_before_micros: u64,
    pub player_time_after_micros: u64,
    /// 本段计入球员累计时间的人员。
    ///
    /// 队员集合在一段内不变，因此整段共用一份。段的边界由换人决定。
    pub player_handles: Vec<usize>,
    pub game_clock_running: bool,
    pub shot_clock_running: bool,
    pub elapsed_running: bool,
    pub player_time_running: bool,
    pub cause_event_id: EventId,
    pub program_id: ProgramId,
}

/// 时钟提交记录：指令、连续推进段与检查点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClockCommit {
    Directive(ClockDirectiveApplied),
    Segment(ClockSegmentApplied),
    CheckpointSaved(String, ClockCheckpoint),
}

/// 时钟提交记录缓冲。
///
/// 连续推进先存在这里，只有在写入指令之前或者收尾时才变成一条提交记录。
#[derive(Debug, Clone, Default)]
pub struct ClockCommitLog {
    commits: Vec<ClockCommit>,
    pending: Option<ClockSegmentApplied>,
    /// 正在积累的段所使用的队员集合：队员不变时复用同一份缓冲，
    /// 稳态 tick 因此不发生堆分配。
    pending_handles: Vec<usize>,
}

impl ClockCommitLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.commits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commits.is_empty()
    }

    pub fn push(&mut self, commit: ClockCommit) {
        self.commits.push(commit);
    }

    /// 当前正在积累的连续推进段。
    pub fn pending_segment(&self) -> Option<&ClockSegmentApplied> {
        self.pending.as_ref()
    }

    /// 把正在积累的推进段变成一条提交记录。
    pub fn flush_segment(&mut self) {
        if let Some(mut segment) = self.pending.take() {
            // 队员集合随段记录离开内核，因此这里给它一份自己的副本。
            // 日志里的复用缓冲留在原处：下一段推进继续用它，不再分配。
            segment.player_handles = self.pending_handles.clone();
            self.commits.push(ClockCommit::Segment(segment));
        }
    }

    /// 记录一次推进。
    ///
    /// 运行标志与队员集合相同的连续推进合并成一段：段内没有任何指令写入，
    /// 因此整段可以用「起点读数 + 推进时长」表达。运行标志变化的时刻
    /// 另起一段。
    ///
    /// 队员集合用复用缓冲保存：稳态 tick 的队员集合不变，因此不需要
    /// 每 tick 分配一份新的向量。
    pub fn record_segment(&mut self, mut segment: ClockSegmentApplied, player_handles: &[usize]) {
        let mergeable = self.pending.as_ref().is_some_and(|pending| {
            pending.game_clock_running == segment.game_clock_running
                && pending.shot_clock_running == segment.shot_clock_running
                && pending.elapsed_running == segment.elapsed_running
                && pending.player_time_running == segment.player_time_running
                && pending.simulation_after_micros == segment.simulation_before_micros
                && pending.tick_end == segment.tick_begin
        }) && self.pending_handles == player_handles;
        if mergeable {
            let pending = self.pending.as_mut().expect("pending segment exists");
            pending.micros = pending.micros.saturating_add(segment.micros);
            pending.tick_end = segment.tick_end;
            pending.simulation_after_micros = segment.simulation_after_micros;
            pending.game_clock_after_micros = segment.game_clock_after_micros;
            pending.shot_clock_after_micros = segment.shot_clock_after_micros;
            pending.elapsed_after_micros = segment.elapsed_after_micros;
            pending.player_time_after_micros = segment.player_time_after_micros;
            return;
        }
        self.flush_segment();
        // 队员集合存进复用缓冲：整段共用这一份，切段时整体移交。
        self.pending_handles.clear();
        self.pending_handles.extend_from_slice(player_handles);
        segment.player_handles = Vec::new();
        self.pending = Some(segment);
    }

    /// 取出全部已经完成的提交记录。
    ///
    /// 正在积累的推进段不在其中：它要等下一次指令打断它时才算完成。
    /// 这样连续推进只产生一条记录，而不是每个 tick 一条。
    pub fn take_ready(&mut self) -> Vec<ClockCommit> {
        std::mem::take(&mut self.commits)
    }

    /// 结算并取出全部提交记录，包括正在积累的推进段。
    ///
    /// 比赛结束时用它收尾：最后一段没有后续指令来打断它。
    pub fn drain(&mut self) -> Vec<ClockCommit> {
        self.flush_segment();
        std::mem::take(&mut self.commits)
    }
}

/// 计时器检查点。`Restore` 从它读取指定计时器的整数微秒值。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockCheckpoint {
    values: BTreeMap<String, u64>,
    player_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClockCheckpointError {
    MissingTimer(TimerId),
    UnexpectedTimer(String),
    InvalidId(String),
    PlayerCountMismatch { expected: usize, actual: usize },
}

impl fmt::Display for ClockCheckpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTimer(timer) => write!(f, "计时器检查点缺少 {timer}"),
            Self::UnexpectedTimer(timer) => write!(f, "计时器检查点包含未知 {timer}"),
            Self::InvalidId(id) => write!(f, "计时器检查点身份 {id} 无效"),
            Self::PlayerCountMismatch { expected, actual } => write!(
                f,
                "计时器检查点人数 {actual} 与时钟状态人数 {expected} 不一致"
            ),
        }
    }
}

impl Error for ClockCheckpointError {}

impl ClockCheckpoint {
    pub fn new(player_count: usize) -> Self {
        Self {
            values: BTreeMap::new(),
            player_count,
        }
    }

    pub fn from_state(state: &super::ClockState) -> Self {
        let mut checkpoint = Self::new(state.player_count());
        for timer in TimerId::all(state.player_count()) {
            checkpoint.insert(timer, state.timer_micros(timer));
        }
        checkpoint
    }

    pub fn get(&self, timer: TimerId) -> Result<u64, ClockCheckpointError> {
        self.values
            .get(&timer.as_str())
            .copied()
            .ok_or(ClockCheckpointError::MissingTimer(timer))
    }

    pub fn insert(&mut self, timer: TimerId, micros: u64) {
        self.values.insert(timer.as_str(), micros);
    }

    pub fn values(&self) -> &BTreeMap<String, u64> {
        &self.values
    }

    pub fn player_count(&self) -> usize {
        self.player_count
    }

    pub fn validate_for(&self, player_count: usize) -> Result<(), ClockCheckpointError> {
        if self.player_count != player_count {
            return Err(ClockCheckpointError::PlayerCountMismatch {
                expected: player_count,
                actual: self.player_count,
            });
        }
        let expected: BTreeMap<String, TimerId> = TimerId::all(player_count)
            .into_iter()
            .map(|timer| (timer.as_str(), timer))
            .collect();
        let actual: BTreeSet<String> = self.values.keys().cloned().collect();
        for (key, timer) in &expected {
            if !actual.contains(key) {
                return Err(ClockCheckpointError::MissingTimer(*timer));
            }
        }
        let expected_keys: BTreeSet<String> = expected.keys().cloned().collect();
        if let Some(key) = actual.difference(&expected_keys).next() {
            return Err(ClockCheckpointError::UnexpectedTimer(key.clone()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_directive_shape_keeps_its_timer() {
        let checkpoint_id = CheckpointId::from_raw(3);
        let directives = [
            ClockDirective::Start {
                timer: TimerId::GameClock,
            },
            ClockDirective::Stop {
                timer: TimerId::ShotClock,
            },
            ClockDirective::Set {
                timer: TimerId::ShotClock,
                micros: 14_000_000,
            },
            ClockDirective::Restore {
                timer: TimerId::GameClock,
                checkpoint_id,
            },
        ];
        for directive in directives {
            assert!(!directive.timer().as_str().is_empty());
        }
        assert_eq!(directives[0].as_str(), "start");
        assert_eq!(directives[1].as_str(), "stop");
        assert_eq!(directives[2].value_micros(), Some(14_000_000));
        assert_eq!(directives[3].checkpoint_id(), Some(checkpoint_id));
    }

    #[test]
    fn timer_identity_round_trips_as_a_string_value() {
        for timer in TimerId::all(3) {
            let text = serde_json::to_string(&timer).expect("timer encodes");
            let restored: TimerId = serde_json::from_str(&text).expect("timer decodes");
            assert_eq!(restored, timer);
        }
        assert!("unknown_timer".parse::<TimerId>().is_err());
    }

    #[test]
    fn running_flags_reject_stopped_shot_clock() {
        let mut running = TimerRunning::all();
        assert!(running.get(TimerId::Simulation));
        running.set(TimerId::ShotClock, false);
        assert!(!running.get(TimerId::ShotClock));
        assert!(running.get(TimerId::GameClock));
    }

    #[test]
    fn checkpoint_demands_exact_timer_keys() {
        let mut checkpoint = ClockCheckpoint::new(2);
        checkpoint.insert(TimerId::GameClock, 720_000_000);
        assert_eq!(checkpoint.get(TimerId::GameClock), Ok(720_000_000));
        assert_eq!(
            checkpoint.get(TimerId::ShotClock),
            Err(ClockCheckpointError::MissingTimer(TimerId::ShotClock))
        );
        for timer in TimerId::all(2) {
            checkpoint.insert(timer, 0);
        }
        assert_eq!(checkpoint.validate_for(2), Ok(()));
        checkpoint.insert(TimerId::PlayerTime(99), 0);
        assert_eq!(
            checkpoint.validate_for(2),
            Err(ClockCheckpointError::UnexpectedTimer(
                "player_time_99".to_string()
            ))
        );
        assert!(matches!(
            checkpoint.validate_for(3),
            Err(ClockCheckpointError::PlayerCountMismatch { .. })
        ));
    }
}
