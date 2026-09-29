use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderPlayer {
    /// Stable roster identity; jersey numbers are presentation data only.
    pub id: String,
    pub jersey: String,
    pub team: String,
    /// 六类位置（attributes.md §2.7a）：Point/Combo/Wing/Forward/Big/Center。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub position: String,
    /// 赛前固定的进攻角色（attributes.md §2.7b），整场不变。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub offensive_role: String,
    /// 赛前固定的防守角色（attributes.md §2.7b），整场不变。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub defensive_role: String,
    pub x: f32,
    pub y: f32,
    pub zone: String,
    #[serde(rename = "hasBall")]
    pub has_ball: bool,
    /// Selected roster members on the bench remain observable but do not enter physics.
    #[serde(rename = "onCourt")]
    pub on_court: bool,
    pub action: String,
    /// 当前动作窗口阶段；没有活动窗口时省略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_phase: Option<String>,
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
    /// Continuous potential-field target produced by the defensive solver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub potential_target_x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub potential_target_y: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub potential_action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub potential_threat_ratio: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub potential_void_ratio: Option<f32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PotentialFieldSample {
    pub x: f32,
    pub y: f32,
    pub target_x: f32,
    pub target_y: f32,
    pub pressure: f32,
    pub drive_x: f32,
    pub drive_y: f32,
    pub action: String,
    pub team: String,
    pub player_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RenderBall {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub status: String,
    #[serde(rename = "holderId")]
    pub holder_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    /// 本决策同 tick 激活的 Play 档案标识（选板语义：每队至多一个激活）。
    /// 无激活 Play 时省略，保持既有流的向后兼容。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_play_id: Option<String>,
    /// 激活 Play 对各候选动作施加的偏好/抑制效果（含效用重算值）。
    /// 无 Play 调整时省略，保持既有流的向后兼容。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub play_adjustments: Vec<DebugPlayAdjustment>,
}

/// 单个候选动作经 Play 调整后的协议投影。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DebugPlayAdjustment {
    pub label: String,
    pub action_family: String,
    pub constraint_feasible: bool,
    pub bonus: f32,
    pub soft_penalty: f32,
    pub hard_inhibited: bool,
    /// 应用 Play 调整后的效用；硬抑制或硬约束剔除的候选无效用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjusted_utility: Option<f32>,
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
    #[serde(rename = "shotClock", default)]
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
    /// Per-tick player projections. Omitted in bounded output mode: the engine
    /// self-audits L1 every tick, so the stream only needs to carry causal and
    /// presentation data (gap.md §16.4 resource governance).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub players: Vec<RenderPlayer>,
    #[serde(default)]
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
    /// 紧凑流只在首条记录携带该字段；后续记录由消费者向前继承
    /// （gap.md §16.4）。`default` 使缺失时可解析，值由继承补齐。
    #[serde(default)]
    pub rules: FrameRules,
    /// 流投影粒度（gap.md §16.4）：
    /// - `full`：携带逐 tick 球员/球几何投影，可做 L1 几何不变量审计；
    /// - `facts` / `summary`：只携带因果事实，几何不变量不适用。
    ///
    /// 审计器据此区分「几何未采集」与「几何缺失/损坏」，
    /// 避免把有界流误判成 L1 违规。
    #[serde(default = "default_stream_projection")]
    pub stream_projection: String,
    /// Sparse samples of the authoritative defensive potential field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub potential_field: Vec<PotentialFieldSample>,
}

fn default_stream_projection() -> String {
    "full".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameEvent {
    pub sequence: u64,
    pub time: f32,
    pub kind: String,
    /// Domain event payload retained for replay, analytics, and debugging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    /// D4.1 稳定事件 ID（dev 方案 §7.1 / gap.md §7.1）：全场单调递增，
    /// 与 `sequence`（逐 tick 内的本地序号）不同——`event_id` 跨 tick 唯一，
    /// 供评判器/账本按 ID 重建因果链。`default` 保证旧流可解析。
    #[serde(default)]
    pub event_id: u64,
    /// D4.1 因果父事件 ID：本事件由哪个事件直接导致（如
    /// `SHOT_RELEASE → SCORE`、`FOUL → FREE_THROW`）。无父事件时为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_event_id: Option<u64>,
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
    /// 个人犯满离场阈值（犯规账本 FOUL_CONSERVATION 的唯一限值来源）。
    #[serde(default = "default_max_personal_fouls")]
    pub max_personal_fouls: u8,
    /// 单节球队犯规进入 bonus 的阈值（犯规账本 FOUL_CONSERVATION 的唯一限值来源）。
    #[serde(default = "default_bonus_fouls_per_period")]
    pub bonus_fouls_per_period: u32,
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
fn default_max_personal_fouls() -> u8 {
    6
}
fn default_bonus_fouls_per_period() -> u32 {
    5
}

impl FrameRules {
    /// 该记录是否显式携带规则（紧凑流只在首条记录写出）。
    ///
    /// 用 `tick_seconds` 作为存在性判据：它必须为正才有物理意义，
    /// 因此零值只能表示「字段缺失、需向前继承」。
    pub fn is_present(&self) -> bool {
        self.tick_seconds > f32::MIN_POSITIVE
    }
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
            // C6.4：取 GameRules::default().player_radius_ft (1.8) 同源值。
            // 原值 1.0 是手抄漂移；全字段一致性测试在 engine/tests 守卫。
            player_radius_ft: 1.8,
            min_player_separation_ft: 3.6,
            // C6.4：取 GameRules::default() (0.05) 同源值——第二个手抄漂移点，
            // 由 engine/tests 的全字段一致性测试抓出。
            separation_safety_margin_ft: 0.05,
            max_player_speed_ftps: 22.0,
            max_player_accel_ftps2: 35.0,
            ball_max_speed_ftps: 85.0,
            three_point_distance_ft: 23.75,
            shot_clock_seconds: default_shot_clock(),
            holder_leash_ft: default_holder_leash(),
            speed_tolerance_ftps: default_speed_tolerance(),
            ball_z_max_ft: default_ball_z_max(),
            max_personal_fouls: default_max_personal_fouls(),
            bonus_fouls_per_period: default_bonus_fouls_per_period(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamTick {
    #[serde(flatten)]
    pub frame: RenderFrame,
    #[serde(rename = "tactical_set", default)]
    pub tactical_set: String,
    #[serde(rename = "gameClock", default)]
    pub game_clock: f32,
    #[serde(rename = "keyframeIndex")]
    pub keyframe_index: Option<u64>,
}
