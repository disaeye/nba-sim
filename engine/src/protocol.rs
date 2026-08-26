use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionTrace {
    pub actor_jersey: String,
    pub action: String,
    pub reason: String,
    pub target_jersey: Option<String>,
    pub shot_openness: Option<f32>,
    pub pass_openness: Option<f32>,
    pub drive_lane_space: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageMeta {
    pub id: usize,
    pub stage_type: String,
    pub title: String,
    pub description: String,
    pub start_tick: usize,
    pub end_tick: usize,
    pub duration_s: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderScore {
    pub home: u32,
    pub away: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderPlayer {
    pub id: String,
    pub jersey: String,
    pub team: String,
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub action: String,
    pub zone: Option<String>,
    pub task: Option<String>,
    pub stm: Option<f32>,
    pub stm_max: Option<f32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderBall {
    pub x: f32,
    pub y: f32,
    pub z: Option<f32>,
    pub status: String,
    #[serde(rename = "holderId", skip_serializing_if = "Option::is_none")]
    pub holder_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderFrame {
    pub t: f32,
    pub t_game: f32,
    pub shot_clock: f32,
    pub period: u32,
    pub phase: String,
    pub score: RenderScore,
    pub players: Vec<RenderPlayer>,
    pub ball: RenderBall,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intensity: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_trace: Option<DecisionTrace>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamTick {
    pub frame: RenderFrame,
    pub game_clock: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyframe_index: Option<u64>,
}
