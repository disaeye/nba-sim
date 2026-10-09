//! 验证流的完整性与能力声明（validation.md §3.1）。
//!
//! 每次可验收运行在事实流的首条记录前写 `ValidationStreamHeader`，
//! 在尾记录后写 `ValidationStreamTrailer`。头部声明必需能力与运行身份；
//! 尾部提供终局原因、记录数、事件数、最后 tick、完整性摘要与未完成
//! 业务实例数量。缺少尾部属于截断流，验证层立即判执行错误。

use serde::{Deserialize, Serialize};

/// 流投影的证据等级（validation.md §3.1）。
///
/// `Frames` 与 `FramesGzip` 保留同等证据；`Facts` 保留事件与必要的局部
/// 几何快照，无法验证连续运动的项目显式未判定；`Summary` 仅可运行
/// 已登记的汇总指标。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceLevel {
    Frames,
    FramesGzip,
    Facts,
    Summary,
}

impl EvidenceLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Frames => "frames",
            Self::FramesGzip => "frames_gzip",
            Self::Facts => "facts",
            Self::Summary => "summary",
        }
    }
}

/// 运行中流所声明的必需能力（validation.md §3.1）。
///
/// 旧版本合法流缺少新能力时，验证层对相应能力项给
/// `InsufficientEvidence`，不能参加要求该能力的完整认证。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamCapabilities {
    /// 帧携带逐 tick 玩家与球的几何投影（连续运动可验证）。
    pub geometry_frames: bool,
    /// 事件携带业务实例身份（action_id/shot_id/pass_id/...）。
    pub business_identity: bool,
    /// 事件携带结构化 relations（CausedBy 等）。
    pub event_relations: bool,
    /// 流发布观测载荷（MovementIntent/Responsibility/PlayWindow/
    /// ControlEstablished/ClockChanged 等）。
    pub observation_payloads: bool,
    /// 流携带 `ShotAttemptResolved` 与 `StatAttribution`。
    pub stat_attribution: bool,
    /// 流携带检查点摘要（机制实验可恢复）。
    pub checkpoints: bool,
}

/// 验证流首部（validation.md §3.1 运行完整性）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationStreamHeader {
    /// 协议版本；不兼容时验证层显式拒绝。
    pub protocol_version: String,
    /// 输入包摘要（setup/规则 JSON 的 SHA-256 十六进制）。
    pub input_digest: String,
    /// 随机算法版本（如 `chacha8`）。
    pub rng_algorithm: String,
    /// 随机种子。
    pub seed: u64,
    /// 运行范围（`1q`/`full`/`Np`）。
    pub scope: String,
    /// 流投影与证据等级。
    pub evidence_level: EvidenceLevel,
    /// 本流声明的必需能力。
    pub capabilities: StreamCapabilities,
    /// 预期比赛数（批量运行中本文件包含的比赛数；单场为 1）。
    pub expected_games: u32,
    /// 认证范围标签（如 `structural` / `full_authenticity_nba_2023`）。
    pub certification_scope: String,
    /// 联赛与规则档案标识。
    pub league_profile: String,
    /// 数值单位与坐标方向声明。
    pub units: UnitDeclaration,
}

/// 数值单位与坐标方向（validation.md §3.1 manifest 要求的字段落进流首部）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitDeclaration {
    /// 长度单位（`feet`）。
    pub length: String,
    /// 时间单位（`seconds`）。
    pub time: String,
    /// 坐标原点与轴方向（`left_hoop_origin_x_right_y_down`）。
    pub coordinates: String,
    /// 采样间隔（秒；固定步长）。
    pub tick_seconds: f32,
}

/// 验证流尾部（validation.md §3.1）。缺少尾部属于截断流。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationStreamTrailer {
    /// 终局原因（`period_end`/`game_end`/`scope_complete` 等）。
    pub end_reason: String,
    /// 已写出的记录（tick 记录）数。
    pub records: u64,
    /// 已发出的事件数。
    pub events: u64,
    /// 最后一个 tick 的 `tick_index`。
    pub last_tick_index: u64,
    /// 完整性摘要：全部记录内容与首部身份的哈希（FNV-1a 十六进制），
    /// 复现运行时逐位核对。
    pub integrity_digest: String,
    /// 未完成业务实例数量：已创建身份但未到达终态的实例。
    pub open_business_instances: u64,
    /// 比赛级最终比分（主/客）。
    pub final_score_home: u32,
    pub final_score_away: u32,
    /// 周期数（含加时）。
    pub periods_played: u32,
}
