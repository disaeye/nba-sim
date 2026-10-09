//! 严格流读取、证据等级判定与实例/关系索引（validation.md §3.1–§3.2）。
//!
//! 验证层只读。读取以逐行 NDJSON 为单位：首行必须是
//! `ValidationStreamHeader`，尾行必须是 `ValidationStreamTrailer`，中间
//! 每行是 `StreamTick`。缺尾部即截断流，立即报执行错误；坏行立即终止，
//! 禁止把执行错误降级为 `NotApplicable`。

use std::collections::BTreeMap;

use nba_protocol::{
    EventRelationKind, EvidenceLevel, FrameEvent, StreamTick, ValidationStreamHeader,
    ValidationStreamTrailer,
};
use serde_json::Value;

use crate::verdict::Verdict;

/// 流读取错误（validation.md §1.2 执行状态 `ExecutionError`）。
#[derive(Debug)]
pub enum StreamError {
    /// 流在 trailer 之前结束（validation.md §3.1 截断流）。
    Truncated { records: usize },
    /// 首行非 header 记录。
    MissingHeader { first_kind: String },
    /// 某行不是合法记录或 schema 不匹配。
    BadRecord { line: usize, cause: String },
    /// 底层 IO 失败。
    Io(std::io::Error),
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { records } => {
                write!(
                    f,
                    "stream truncated: missing trailer after {records} records"
                )
            }
            Self::MissingHeader { first_kind } => {
                write!(f, "stream missing header: first record was `{first_kind}`")
            }
            Self::BadRecord { line, cause } => {
                write!(f, "bad record at line {line}: {cause}")
            }
            Self::Io(error) => write!(f, "io error: {error}"),
        }
    }
}

impl std::error::Error for StreamError {}

impl From<std::io::Error> for StreamError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// 一次读取得到的完整流包络。
#[derive(Debug)]
pub struct StreamEnvelope {
    pub header: ValidationStreamHeader,
    pub ticks: Vec<StreamTick>,
    pub trailer: Option<ValidationStreamTrailer>,
}

impl StreamEnvelope {
    /// 严格解析：首行 header、尾行 trailer、其余行 StreamTick。
    ///
    /// `read` 返回的每一行都必须是合法 JSON 记录；任何一行失败都立即
    /// 返回 `StreamError::BadRecord`。
    pub fn parse(mut read: impl std::io::Read) -> Result<Self, StreamError> {
        let mut content = String::new();
        read.read_to_string(&mut content)?;
        let mut lines = content.lines().enumerate().peekable();
        let Some((_, first_line)) = lines.next() else {
            return Err(StreamError::Truncated { records: 0 });
        };
        let first: Value =
            serde_json::from_str(first_line).map_err(|error| StreamError::BadRecord {
                line: 1,
                cause: format!("header parse failed: {error}"),
            })?;
        if first.get("record_type").and_then(Value::as_str) != Some("header") {
            return Err(StreamError::MissingHeader {
                first_kind: first
                    .get("record_type")
                    .and_then(Value::as_str)
                    .unwrap_or("<absent>")
                    .to_string(),
            });
        }
        let header: ValidationStreamHeader =
            serde_json::from_value(first).map_err(|error| StreamError::BadRecord {
                line: 1,
                cause: format!("header schema mismatch: {error}"),
            })?;
        let mut ticks = Vec::new();
        let mut trailer = None;
        for (index, line) in lines {
            let line_number = index + 1;
            if line.trim().is_empty() {
                continue;
            }
            let record: Value =
                serde_json::from_str(line).map_err(|error| StreamError::BadRecord {
                    line: line_number,
                    cause: format!("record parse failed: {error}"),
                })?;
            match record.get("record_type").and_then(Value::as_str) {
                Some("header") => {
                    return Err(StreamError::BadRecord {
                        line: line_number,
                        cause: "duplicate header".to_string(),
                    });
                }
                Some("trailer") => {
                    if trailer.is_some() {
                        return Err(StreamError::BadRecord {
                            line: line_number,
                            cause: "duplicate trailer".to_string(),
                        });
                    }
                    trailer = Some(serde_json::from_value(record).map_err(|error| {
                        StreamError::BadRecord {
                            line: line_number,
                            cause: format!("trailer schema mismatch: {error}"),
                        }
                    })?);
                }
                _ => {
                    let tick: StreamTick =
                        serde_json::from_value(record).map_err(|error| StreamError::BadRecord {
                            line: line_number,
                            cause: format!("tick schema mismatch: {error}"),
                        })?;
                    ticks.push(tick);
                }
            }
        }
        Ok(Self {
            header,
            ticks,
            trailer,
        })
    }

    /// 截断检查：缺 trailer 即截断（validation.md §3.1）。
    pub fn require_complete(&self) -> Result<&ValidationStreamTrailer, StreamError> {
        self.trailer.as_ref().ok_or(StreamError::Truncated {
            records: self.ticks.len(),
        })
    }

    /// 流声明的证据等级对应的可用性：`Summary` 只能跑汇总指标，
    /// `Facts` 无法验证连续运动。
    pub fn geometry_verifiable(&self) -> bool {
        matches!(
            self.header.evidence_level,
            EvidenceLevel::Frames | EvidenceLevel::FramesGzip
        )
    }
}

/// 一条事件在索引中的定位：流内序号与 `(tick_index, sequence)` 身份。
#[derive(Debug, Clone, Copy)]
pub struct EventLocation {
    pub tick_index: u64,
    pub sequence: u64,
    pub event_id: u64,
}

/// 业务实例索引：身份 → 全部关联事件（validation.md §3.2）。
///
/// 一个实例从创建到终结身份不变；索引把同一 `shot_id`/`pass_id`/...
/// 的全部事件按发布顺序串起来，供配对完整性与关系基数检查使用。
#[derive(Debug, Default)]
pub struct InstanceIndex {
    by_action: BTreeMap<String, Vec<EventLocation>>,
    by_shot: BTreeMap<String, Vec<EventLocation>>,
    by_pass: BTreeMap<String, Vec<EventLocation>>,
    by_loose_ball: BTreeMap<String, Vec<EventLocation>>,
    by_foul_program: BTreeMap<String, Vec<EventLocation>>,
    by_responsibility: BTreeMap<String, Vec<EventLocation>>,
    by_movement_intent: BTreeMap<String, Vec<EventLocation>>,
    by_play_instance: BTreeMap<String, Vec<EventLocation>>,
}

impl InstanceIndex {
    pub fn build(ticks: &[StreamTick]) -> Self {
        let mut index = Self::default();
        for tick in ticks {
            let tick_index = tick.frame.tick_index;
            for event in &tick.frame.event_log {
                let location = EventLocation {
                    tick_index,
                    sequence: event.sequence,
                    event_id: event.event_id,
                };
                if let Some(id) = &event.action_id {
                    index
                        .by_action
                        .entry(id.clone())
                        .or_default()
                        .push(location);
                }
                if let Some(id) = &event.shot_id {
                    index.by_shot.entry(id.clone()).or_default().push(location);
                }
                if let Some(id) = &event.pass_id {
                    index.by_pass.entry(id.clone()).or_default().push(location);
                }
                if let Some(id) = &event.loose_ball_id {
                    index
                        .by_loose_ball
                        .entry(id.clone())
                        .or_default()
                        .push(location);
                }
                if let Some(id) = &event.foul_program_id {
                    index
                        .by_foul_program
                        .entry(id.clone())
                        .or_default()
                        .push(location);
                }
                if let Some(id) = &event.responsibility_id {
                    index
                        .by_responsibility
                        .entry(id.clone())
                        .or_default()
                        .push(location);
                }
                if let Some(id) = &event.movement_intent_id {
                    index
                        .by_movement_intent
                        .entry(id.clone())
                        .or_default()
                        .push(location);
                }
                if let Some(id) = &event.play_instance_id {
                    index
                        .by_play_instance
                        .entry(id.clone())
                        .or_default()
                        .push(location);
                }
            }
        }
        index
    }

    /// 实例身份只结算一次（validation.md §5.2 CAUSE-I01）：
    /// 找出被多个「终结事件」引用的实例（同一身份在两个终结事件上
    /// 出现即双结算）。终结判定按事件 kind 的已知终态集合。
    pub fn double_closed_instances(&self, ticks: &[StreamTick]) -> Vec<String> {
        let mut closed: BTreeMap<String, u32> = BTreeMap::new();
        for tick in ticks {
            for event in &tick.frame.event_log {
                if !is_terminal_kind(&event.kind) {
                    continue;
                }
                for id in instance_ids(event).into_iter().flatten() {
                    *closed.entry(id.clone()).or_insert(0) += 1;
                }
            }
        }
        closed
            .into_iter()
            .filter(|(_, count)| *count > 1)
            .map(|(id, _)| id)
            .collect()
    }

    pub fn shot_instance_count(&self) -> usize {
        self.by_shot.len()
    }

    pub fn pass_instance_count(&self) -> usize {
        self.by_pass.len()
    }
}

/// 事件 kind 是否为某业务实例的终态（validation.md §5.1 各终态分支）。
///
/// 终态按实例类型区分：loose_ball 只被 `LOOSE_BALL_SECURED`/`REBOUND`
/// 关闭（`PASS_DROPPED`/`PASS_TIPPED`/`SHOT_ATTEMPT_RESOLVED`/
/// `BLOCKED_SHOT`/`BALL_POKED_LOOSE` 是它的创建点）；shot 只被
/// `SCORE`/`SHOT_MISS` 关闭（`SHOT_ATTEMPT_RESOLVED` 是同事实的裁决
/// 记录，不重复计终结）。动作与 Play/责任实例的终态按各自生命周期。
pub fn is_terminal_kind(kind: &str) -> bool {
    matches!(
        kind,
        "SCORE"
            | "SHOT_MISS"
            | "PASS_RECEIVED"
            | "STEAL"
            | "LOOSE_BALL_SECURED"
            | "REBOUND"
            | "ACTION_COMPLETED"
            | "ACTION_CANCELLED"
            | "ACTION_FAILED"
            | "PLAY_WINDOW_ENDED"
            | "MOVEMENT_OUTCOME"
            | "RESPONSIBILITY_ENDED"
    )
}

/// 事件携带的全部实例身份（用于双结算检查）。
fn instance_ids(event: &FrameEvent) -> [Option<&String>; 8] {
    [
        event.action_id.as_ref(),
        event.shot_id.as_ref(),
        event.pass_id.as_ref(),
        event.loose_ball_id.as_ref(),
        event.foul_program_id.as_ref(),
        event.responsibility_id.as_ref(),
        event.movement_intent_id.as_ref(),
        event.play_instance_id.as_ref(),
    ]
}

/// 事件携带的全部实例（类型名，身份），按身份字段顺序。
/// 实例类型名与存储表 `instances.instance_kind` 一致：action/shot/
/// pass/loose_ball/foul_program/responsibility/movement_intent/play。
pub fn instance_kind_of(event: &FrameEvent) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if let Some(id) = &event.action_id {
        out.push(("action", id.clone()));
    }
    if let Some(id) = &event.shot_id {
        out.push(("shot", id.clone()));
    }
    if let Some(id) = &event.pass_id {
        out.push(("pass", id.clone()));
    }
    if let Some(id) = &event.loose_ball_id {
        out.push(("loose_ball", id.clone()));
    }
    if let Some(id) = &event.foul_program_id {
        out.push(("foul_program", id.clone()));
    }
    if let Some(id) = &event.responsibility_id {
        out.push(("responsibility", id.clone()));
    }
    if let Some(id) = &event.movement_intent_id {
        out.push(("movement_intent", id.clone()));
    }
    if let Some(id) = &event.play_instance_id {
        out.push(("play", id.clone()));
    }
    out
}

/// 关系检查结果（validation.md §3.2 关系类型与基数约束）。
#[derive(Debug, Default)]
pub struct RelationAudit {
    /// 指向更晚事件（或自身）的回写关系 —— 禁止。
    pub forward_relations: u64,
    /// 未知事件 ID 的悬空关系。
    pub dangling_relations: u64,
    /// 非法关系种类（serde 已拦截，此处兜底计数）。
    pub unknown_kinds: u64,
}

impl RelationAudit {
    /// 审计全部事件关系：触发原因必须指向更早的事件；全部
    /// `event_id` 必须在流中存在（validation.md §3.2 禁止回写环）。
    pub fn audit(ticks: &[StreamTick]) -> Self {
        let mut known: std::collections::HashSet<u64> = std::collections::HashSet::new();
        for tick in ticks {
            for event in &tick.frame.event_log {
                known.insert(event.event_id);
            }
        }
        let mut audit = Self::default();
        for tick in ticks {
            for event in &tick.frame.event_log {
                for relation in &event.relations {
                    match relation.kind {
                        EventRelationKind::CausedBy
                        | EventRelationKind::CreatedBy
                        | EventRelationKind::InterruptedBy
                        | EventRelationKind::AwardedBy
                        | EventRelationKind::RecoveredFrom
                        | EventRelationKind::Continues
                        | EventRelationKind::InfluencedBy => {}
                    }
                    if relation.event_id >= event.event_id {
                        audit.forward_relations += 1;
                    }
                    if !known.contains(&relation.event_id) {
                        audit.dangling_relations += 1;
                    }
                }
            }
        }
        audit
    }

    pub fn clean(&self) -> bool {
        self.forward_relations == 0 && self.dangling_relations == 0 && self.unknown_kinds == 0
    }
}

/// 事件身份唯一性检查（CAUSE-I01）：全场 `event_id` 单调且唯一。
pub fn event_id_unique_and_monotonic(ticks: &[StreamTick]) -> Result<(), String> {
    let mut previous = 0u64;
    for tick in ticks {
        for event in &tick.frame.event_log {
            if event.event_id == 0 {
                return Err(format!(
                    "tick {} sequence {} has zero event_id",
                    tick.frame.tick_index, event.sequence
                ));
            }
            if event.event_id <= previous {
                return Err(format!(
                    "event_id {} not strictly increasing after {}",
                    event.event_id, previous
                ));
            }
            previous = event.event_id;
        }
    }
    Ok(())
}

/// 能力缺失时对应检查的统一裁决（validation.md §1.2）：旧版本合法流
/// 缺少新能力时给 `InsufficientEvidence`，不能参加要求该能力的认证。
#[allow(dead_code)]
pub fn capability_verdict() -> Verdict {
    Verdict::InsufficientEvidence
}
