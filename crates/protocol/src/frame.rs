use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderPlayer {
    /// Stable roster identity; jersey numbers are presentation data only.
    pub id: String,
    pub jersey: String,
    pub team: String,
    pub x: f32,
    pub y: f32,
    pub zone: String,
    #[serde(rename = "hasBall")]
    pub has_ball: bool,
    /// Selected roster members on the bench remain observable but do not enter physics.
    #[serde(rename = "onCourt")]
    pub on_court: bool,
    pub action: String,
    pub slot: String,
    pub morale: String,
    pub stm: f32,

    #[serde(rename = "stmMax")]
    pub stm_max: f32,
    /// Personal fouls are part of the authoritative player projection.
    #[serde(default)]
    pub foul_count: u8,
    /// Tactical target position for spatial play routing (2K-style play-art visualization).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_y: Option<f32>,
    /// Physical facing direction vector for 2K-style player posture and stance rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facing_x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facing_y: Option<f32>,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RenderTeam {
    pub id: String,
    pub name: String,
    pub short_name: String,
}

/// 决策调试层（架构文档 Phase 8：每个决策输出效用分解）。
/// Option 字段跳过序列化，正常观看流不受影响。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DecisionDebug {
    pub player: String,
    pub chosen: String,
    pub utilities: Vec<DebugUtility>,
    pub probabilities: Vec<DebugProb>,
    pub flags: Vec<DebugFlag>,
    pub blocked: Vec<String>,
    /// 本决策时刻实际激活的约束，便于解释阶段覆盖和规则缺失。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_constraints: Vec<String>,
    /// 硬约束/阶段约束产生的执行意图。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enforcement: Vec<String>,
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
    #[serde(default)]
    pub possession_team: String,
    #[serde(default)]
    pub home_team: RenderTeam,
    #[serde(default)]
    pub away_team: RenderTeam,
    pub score: RenderScore,
    pub players: Vec<RenderPlayer>,
    pub ball: RenderBall,
    #[serde(rename = "eventType")]
    pub event_type: Option<String>,
    /// All domain facts emitted during this tick, retained alongside the legacy primary event.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
    /// Macro lifecycle state is separate from the possession sub-phase.
    #[serde(default)]
    pub game_flow: String,
    #[serde(default)]
    pub team_fouls_home: u32,
    #[serde(default)]
    pub team_fouls_away: u32,
    #[serde(default)]
    pub free_throws_remaining: u8,
    /// Active defensive scheme for the team currently defending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defensive_tactic: Option<String>,
    /// Monotonic event sequence for replay consumers.
    #[serde(default)]
    pub event_sequence: u64,
    /// Event log entries emitted on this tick with their simulation timestamps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_log: Vec<FrameEvent>,
    /// True when a requested possession scope has reached its terminal boundary.
    #[serde(default)]
    pub simulation_complete: bool,
    #[serde(default)]
    pub completed_possessions: usize,
    #[serde(default)]
    pub target_possessions: usize,
    pub callout: Option<String>,
    pub intensity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug: Option<Box<DecisionDebug>>,
    /// Geometry and fixed-step policy used to produce this frame.
    /// 必填字段（quality.md §1.1 单一事实源）：不变量限值一律取自帧内
    /// FrameRules，禁止消费方自带限值副本或兜底常数。
    pub rules: FrameRules,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameEvent {
    pub sequence: u64,
    pub time: f32,
    pub kind: String,
    /// Domain event payload retained for replay, analytics, and debugging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameRules {
    pub tick_seconds: f32,
    pub court_width_ft: f32,
    pub court_height_ft: f32,
    pub hoop_left_x_ft: f32,
    pub hoop_right_x_ft: f32,
    pub hoop_y_ft: f32,
    pub player_radius_ft: f32,
    pub min_player_separation_ft: f32,
    #[serde(default)]
    pub separation_safety_margin_ft: f32,
    pub max_player_speed_ftps: f32,
    pub max_player_accel_ftps2: f32,
    pub ball_max_speed_ftps: f32,
    pub three_point_distance_ft: f32,
    /// 进攻时钟上限（不变量 SHOT_CLOCK_BOUNDS 的唯一限值来源）。
    #[serde(default = "default_shot_clock")]
    pub shot_clock_seconds: f32,
    /// 持球人 leash：球与持球者允许的最大距离（BALL_WITH_HOLDER 限值）。
    #[serde(default = "default_holder_leash")]
    pub holder_leash_ft: f32,
    /// 速度不变量的数值容差（PLAYER_SPEED / BALL_SPEED）。
    #[serde(default = "default_speed_tolerance")]
    pub speed_tolerance_ftps: f32,
    /// 球高度上限（BALL_HEIGHT_BOUNDS）。
    #[serde(default = "default_ball_z_max")]
    pub ball_z_max_ft: f32,
}

fn default_shot_clock() -> f32 {
    24.0
}
fn default_holder_leash() -> f32 {
    3.0
}
fn default_speed_tolerance() -> f32 {
    1.5
}
fn default_ball_z_max() -> f32 {
    35.0
}

impl Default for FrameRules {
    fn default() -> Self {
        Self {
            tick_seconds: 0.04,
            court_width_ft: 94.0,
            court_height_ft: 50.0,
            hoop_left_x_ft: 5.25,
            hoop_right_x_ft: 88.75,
            hoop_y_ft: 25.0,
            player_radius_ft: 1.0,
            min_player_separation_ft: 3.6,
            separation_safety_margin_ft: 0.0,
            max_player_speed_ftps: 22.0,
            max_player_accel_ftps2: 35.0,
            ball_max_speed_ftps: 85.0,
            three_point_distance_ft: 23.75,
            shot_clock_seconds: default_shot_clock(),
            holder_leash_ft: default_holder_leash(),
            speed_tolerance_ftps: default_speed_tolerance(),
            ball_z_max_ft: default_ball_z_max(),
        }
    }
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
