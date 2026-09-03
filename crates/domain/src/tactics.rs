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
}
