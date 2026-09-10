use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 战术槽位（Slot），包含能力需求与预设空间位置
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TacticalSlot {
    pub id: String,
    pub pos_hint: [f32; 2],
    #[serde(default)]
    pub requirements: HashMap<String, f32>,
}

/// 战术动作声明
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TacticalAction {
    pub action: String,
    pub actor: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub duration_sec: Option<f32>,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub decision_point: bool,
}

/// 战术阵型定义
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TacticalFormation {
    pub slots: Vec<TacticalSlot>,
}

/// 战术触发与克制条件
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TacticalTriggers {
    #[serde(default)]
    pub preferred_score_range: Option<[f32; 2]>,
    #[serde(default)]
    pub pace_multiplier: Option<f32>,
    #[serde(default)]
    pub counter_to_defense: Vec<String>,
}

/// 进攻体系声明式战术档案（tactics.md §2-§2-1 OffensiveSystem）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OffensiveSystem {
    pub id: String,
    pub name: String,
    pub formation: TacticalFormation,
    #[serde(default)]
    pub sequence: Vec<TacticalAction>,
    #[serde(default)]
    pub triggers: TacticalTriggers,
}

/// 防守体系策略参数（tactics.md §2 节）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct OnBallDefenseConfig {
    pub pressure: f32,
    pub contest_height_penalty: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct HelpDefenseConfig {
    pub help_aggressiveness: f32,
    pub rotation_speed: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ScreenDefenseConfig {
    pub strategy: String,
    pub switch_threshold: f32,
    pub mismatch_tolerance: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MatchupRule {
    pub condition: String,
    pub action: String,
}

/// 防守体系声明式档案（tactics.md §2 节 DefensiveSystem）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DefensiveSystem {
    pub id: String,
    pub name: String,
    pub base_scheme: String,
    #[serde(default)]
    pub on_ball: OnBallDefenseConfig,
    #[serde(default)]
    pub help: HelpDefenseConfig,
    #[serde(default)]
    pub screen_defense: ScreenDefenseConfig,
    #[serde(default)]
    pub matchup_rules: Vec<MatchupRule>,
}

impl DefensiveSystem {
    pub fn from_json(json_str: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json_str)
    }
}

/// 特殊情境战术覆盖（tactics.md §2 节 SituationalTactics）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SituationalTactics {
    pub context_name: String,
    pub trigger_condition: String,
    pub pace_override: Option<f32>,
    pub preferred_play_type: Option<String>,
    pub foul_tactic_enabled: bool,
}
impl OffensiveSystem {
    pub fn from_json(json_str: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json_str)
    }

    pub fn high_pick_and_roll() -> Self {
        const JSON: &str = include_str!("../../../data/tactics/high_pick_and_roll.json");
        Self::from_json(JSON).expect("内置 high_pick_and_roll.json 必须合法")
    }

    pub fn five_out_motion() -> Self {
        const JSON: &str = include_str!("../../../data/tactics/five_out_motion.json");
        Self::from_json(JSON).expect("内置 five_out_motion.json 必须合法")
    }
}

/// 兼容老接口的槽位规格（TacticalSlotSpec）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TacticalSlotSpec {
    pub role: String,
    pub name_zh: String,
    pub base_offset_x: f32,
    pub base_offset_y: f32,
    #[serde(default)]
    pub target_lane: u8,
}

/// 兼容老接口的阵型规格（TacticalSetSpec）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TacticalSetSpec {
    pub id: String,
    pub name_zh: String,
    pub spacing_style: String,
    pub slots: Vec<TacticalSlotSpec>,
}

impl TacticalSetSpec {
    pub fn from_json(json_str: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json_str)
    }

    pub fn high_pick_and_roll() -> Self {
        const JSON: &str = include_str!("../../../data/tactics/high_pick_and_roll.json");
        Self::from_json(JSON).expect("内置 high_pick_and_roll.json 必须合法")
    }

    pub fn five_out_motion() -> Self {
        const JSON: &str = include_str!("../../../data/tactics/five_out_motion.json");
        Self::from_json(JSON).expect("内置 five_out_motion.json 必须合法")
    }

    pub fn builtin(id: &str) -> Option<Self> {
        match id {
            "off_horns_pnr" | "high_pick_and_roll" => Some(Self::high_pick_and_roll()),
            "off_motion_spacing" | "five_out_motion" => Some(Self::five_out_motion()),
            _ => None,
        }
    }
}
