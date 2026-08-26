use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderPlayer {
    pub jersey: String,
    pub team: String,
    pub x: f32,
    pub y: f32,
    pub zone: String,
    pub has_ball: bool,
    pub action: String,
    pub stm: f32,
    pub stm_max: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderBall {
    pub x: f32,
    pub y: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub z: Option<f32>,
    pub status: String,
    #[serde(rename = "holderId", skip_serializing_if = "Option::is_none")]
    pub holder_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderScore {
    pub home: u32,
    pub away: u32,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamTick {
    pub frame: RenderFrame,
    pub game_clock: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyframe_index: Option<u64>,
}
