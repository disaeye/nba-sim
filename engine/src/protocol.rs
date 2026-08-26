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
    pub stm: f32,
    #[serde(rename = "stmMax")]
    pub stm_max: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderBall {
    pub x: f32,
    pub y: f32,
    pub status: String,
    #[serde(rename = "holderId")]
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
    #[serde(rename = "shotClock")]
    pub shot_clock: f32,
    pub period: u32,
    pub phase: String,
    pub score: RenderScore,
    pub players: Vec<RenderPlayer>,
    pub ball: RenderBall,
    #[serde(rename = "eventType")]
    pub event_type: Option<String>,
    pub callout: Option<String>,
    pub intensity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamTick {
    #[serde(flatten)]
    pub frame: RenderFrame,
    #[serde(rename = "gameClock")]
    pub game_clock: f32,
    #[serde(rename = "keyframeIndex")]
    pub keyframe_index: Option<u64>,
}
