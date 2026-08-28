use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderPlayer {
    pub jersey: String,
    pub team: String,
    pub x: f32,
    pub y: f32,
    pub zone: String,
    #[serde(rename = "hasBall")]
    pub has_ball: bool,
    pub action: String,
    pub slot: String,
    pub morale: String,
    pub stm: f32,
    #[serde(rename = "stmMax")]
    pub stm_max: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderBall {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub status: String,
    #[serde(rename = "holderId")]
    pub holder_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderScore {
    pub home: u32,
    pub away: u32,
}

/// 决策调试层（架构文档 Phase 8：每个决策输出效用分解）。
/// Option 字段跳过序列化，正常观看流不受影响。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DecisionDebug {
    /// 决策球员
    pub player: String,
    /// 选中动作类型
    pub chosen: String,
    /// 各候选效用
    pub utilities: Vec<DebugUtility>,
    /// 采样概率
    pub probabilities: Vec<DebugProb>,
    /// 触发的软约束/偏好
    pub flags: Vec<DebugFlag>,
    /// 被硬约束剔除的候选
    pub blocked: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugUtility {
    pub kind: String,
    pub utility: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugProb {
    pub kind: String,
    pub prob: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugFlag {
    pub constraint: String,
    pub reason: String,
    pub penalty: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderFrame {
    pub t: f32,
    pub t_game: f32,
    #[serde(rename = "shotClock")]
    pub shot_clock: f32,
    pub period: u32,
    pub phase: String,
    pub possession_id: u32,
    pub score: RenderScore,
    pub players: Vec<RenderPlayer>,
    pub ball: RenderBall,
    #[serde(rename = "eventType")]
    pub event_type: Option<String>,
    pub callout: Option<String>,
    pub intensity: Option<String>,
    /// 决策调试层（可选；调试模式才填充）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug: Option<Box<DecisionDebug>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamTick {
    #[serde(flatten)]
    pub frame: RenderFrame,
    #[serde(rename = "tactical_set")]
    pub tactical_set: String,
    #[serde(rename = "gameClock")]
    pub game_clock: f32,
    #[serde(rename = "keyframeIndex")]
    pub keyframe_index: Option<u64>,
}
