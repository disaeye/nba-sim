//! 独立重建：只读公开事件，重建时钟与控制序列。
//!
//! 验证器不读取引擎内部累计值。它读事件流里的时钟提交记录与控制状态
//! 提交记录，按提交顺序重放，得到比赛钟、进攻钟与控制序列，并与引擎
//! 声明的终局状态核对。

use nba_sim_core::action::Fact;
use nba_sim_core::clock::{ClockCommit, ClockState, TimerId};
use nba_sim_core::GameReport;

/// 独立重建出的时钟终局状态。
#[derive(Debug, Clone, PartialEq)]
pub struct ReconstructedClock {
    /// 比赛钟终局读数（秒）。
    pub game_clock_seconds: f64,
    /// 进攻钟终局读数（秒）。
    pub shot_clock_seconds: f64,
    /// 累计时间（秒）。
    pub elapsed_seconds: f64,
    /// 节次。
    pub period: u32,
    /// tick 计数。
    pub tick: u64,
    /// 时钟提交记录条数。
    pub commits: usize,
}

/// 独立重建出的控制序列。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconstructedControl {
    /// 控制序列的个数。
    pub sequences: usize,
    /// 当前仍然有效的控制序列身份。
    pub active_sequence: Option<u64>,
    /// 控制建立次数。
    pub established: usize,
    /// 控制结束次数。
    pub ended: usize,
}

/// 时钟重建的失败原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClockReconstructionError {
    /// 事件流里没有时钟提交记录。
    NoClockCommits,
    /// 时钟提交记录无法重放。
    ReplayFailed(String),
    /// 重建出的终局状态与引擎声明不一致。
    Mismatch(String),
}

impl std::fmt::Display for ClockReconstructionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoClockCommits => write!(f, "事件流里没有时钟提交记录"),
            Self::ReplayFailed(reason) => write!(f, "时钟提交记录无法重放: {reason}"),
            Self::Mismatch(reason) => write!(f, "重建的时钟与引擎声明不一致: {reason}"),
        }
    }
}

/// 从事件流重建时钟终局状态。
///
/// 重建只读提交记录：验证器按提交顺序重放时钟状态，得到比赛钟与进攻钟
/// 的终局读数。
pub fn reconstruct_clock(report: &GameReport) -> Result<ReconstructedClock, ClockReconstructionError> {
    let mut commits: Vec<ClockCommit> = Vec::new();
    for event in &report.events {
        if let Fact::ClockDirectivesCommitted { commits: batch } = &event.fact {
            commits.extend(batch.iter().cloned());
        }
    }
    if commits.is_empty() {
        return Err(ClockReconstructionError::NoClockCommits);
    }
    // 球员计时器数量由全部公开提交记录给出：推进段带本段上场球员，
    // 指令可以单独写某个球员计时器，检查点自带人数。只从推进段推断
    // 会在「某个球员只在指令或检查点里出现」的比赛里少建计时器。
    let mut player_count = 0usize;
    for commit in &commits {
        match commit {
            ClockCommit::Segment(applied) => {
                for handle in &applied.player_handles {
                    player_count = player_count.max(*handle + 1);
                }
            }
            ClockCommit::Directive(applied) => {
                if let TimerId::PlayerTime(handle) = applied.directive.timer() {
                    player_count = player_count.max(handle + 1);
                }
                if let TimerId::PlayerTime(handle) = applied.timer {
                    player_count = player_count.max(handle + 1);
                }
            }
            ClockCommit::CheckpointSaved(_, checkpoint) => {
                player_count = player_count.max(checkpoint.player_count());
            }
        }
    }
    let mut state = ClockState::for_replay(player_count);
    state
        .replay(&commits)
        .map_err(|error| ClockReconstructionError::ReplayFailed(error.to_string()))?;
    Ok(ReconstructedClock {
        game_clock_seconds: state.game_clock_micros() as f64 / 1_000_000.0,
        shot_clock_seconds: state.shot_clock_micros() as f64 / 1_000_000.0,
        elapsed_seconds: state.elapsed_micros() as f64 / 1_000_000.0,
        period: state.period(),
        tick: state.tick(),
        commits: commits.len(),
    })
}

/// 从事件流重建控制序列。
///
/// 控制序列由控制权交接事实给出：每次交接同时结束上一段、开始新一段。
/// 控制结束事实只在「控制彻底失去」时出现（死球、球权授予），它结束的是
/// 当前仍然有效的那一段。
pub fn reconstruct_control(report: &GameReport) -> ReconstructedControl {
    let mut sequences = std::collections::BTreeSet::new();
    let mut active_sequence: Option<u64> = None;
    let mut established = 0usize;
    let mut ended = 0usize;
    for event in &report.events {
        match &event.fact {
            Fact::ControlHandoverCommitted { handover, .. } => {
                sequences.insert(handover.sequence_id);
                if handover.previous_sequence_id.is_some() {
                    ended += 1;
                }
                established += 1;
                active_sequence = Some(handover.sequence_id);
            }
            Fact::ControlStateCommitted { transition, .. } => {
                sequences.insert(transition.sequence_id);
                match transition.boundary {
                    nba_sim_core::officiating::ControlBoundary::Established => {
                        established += 1;
                        active_sequence = Some(transition.sequence_id);
                    }
                    nba_sim_core::officiating::ControlBoundary::Ended => {
                        ended += 1;
                        if active_sequence == Some(transition.sequence_id) {
                            active_sequence = None;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    ReconstructedControl {
        sequences: sequences.len(),
        active_sequence,
        established,
        ended,
    }
}

/// 控制序列重建的失败原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlReconstructionError {
    /// 事件流里没有控制状态提交记录。
    NoControlCommits,
    /// 建立与结束的次数不一致。
    Unbalanced { established: usize, ended: usize },
    /// 控制序列身份重复建立。
    DuplicateSequence(u64),
}

impl std::fmt::Display for ControlReconstructionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoControlCommits => write!(f, "事件流里没有控制状态提交记录"),
            Self::Unbalanced { established, ended } => {
                write!(f, "控制建立 {established} 次，结束 {ended} 次")
            }
            Self::DuplicateSequence(id) => write!(f, "控制序列 {id} 被重复建立"),
        }
    }
}

/// 从事件流重建控制序列，并检查建立与结束成对。
///
/// 控制权交接是原子的：每次交接同时结束上一段、开始新一段。比赛结束
/// 时最后一段序列被明确关闭，因此建立数与结束数必须完全相等。
pub fn check_control_sequences(
    report: &GameReport,
) -> Result<ReconstructedControl, ControlReconstructionError> {
    check_control_sequences_scoped(report, true)
}

pub(crate) fn check_control_sequences_scoped(
    report: &GameReport,
    require_closed: bool,
) -> Result<ReconstructedControl, ControlReconstructionError> {
    let control = reconstruct_control(report);
    if control.established == 0 && control.ended == 0 {
        return Err(ControlReconstructionError::NoControlCommits);
    }
    let open = usize::from(control.active_sequence.is_some());
    if (require_closed && open != 0) || control.established != control.ended + open {
        return Err(ControlReconstructionError::Unbalanced {
            established: control.established,
            ended: control.ended,
        });
    }
    Ok(control)
}
