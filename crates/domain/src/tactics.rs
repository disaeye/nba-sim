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

/// 进攻槽位在档案里的**行为标签**。
///
/// 槽位过去的 `is_screener` / `is_corner_spacer` / `is_wing_relocate` 三个
/// 布尔字段是「角色」而非「能力需求」，且引擎按 `role` 字符串分支
/// （`role.contains("playmaker")`），违反 tactics.md TA3（槽位需求用能力
/// 表达、不按身份字符串分支）与 charter C1。现改为：能力需求进
/// `requirements`，槽位允许的行为进本枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum SlotBehaviour {
    /// 持球在手，向篮筐压迫。
    DribbleTop,
    /// 固定在点位上拉开（不切入、不 relocate）。
    SpotUp,
    /// 在弧顶与内线之间做纵向 relocate，制造切入时机。
    PerimeterRelocate,
    /// 设立掩护后向篮筐滚动。
    HighScreenRoll,
    /// 弱侧背切：沿底线方向向篮筐空切（G6a 链 1，无球切入接球攻框的
    /// 前提——人不到篮下，接球后也没有攻框位置）。
    BackdoorCut,
    /// 下沉禁区：外线球员沿边线下沉到篮下区域争抢内线落位
    /// （G6a 链 4，五外站位不再把全部无球人固定在外线）。
    DipToRim,
}

impl SlotBehaviour {
    /// 该行为在球场上对应的动作标签（进入物理层的 `action` 字段）。
    ///
    /// 参数是当前子阶段与进攻进度 `action_t`（0..1）；同一个行为在不同阶段
    /// 给出不同标签，使播放/渲染与评判能分辨初始站位与后续移动。
    pub fn action_label(self, initiating: bool) -> &'static str {
        match self {
            Self::DribbleTop => {
                if initiating {
                    "DRIBBLE_TOP"
                } else {
                    "DRIVE_OFF_SCREEN"
                }
            }
            Self::SpotUp => "SPOT_UP_3PT",
            Self::PerimeterRelocate => "PERIMETER_CUT",
            Self::HighScreenRoll => {
                if initiating {
                    "SET_HIGH_SCREEN"
                } else {
                    "ROLL_TO_RIM"
                }
            }
            Self::BackdoorCut => {
                if initiating {
                    "SPOT_UP_3PT"
                } else {
                    "BACKDOOR_CUT"
                }
            }
            Self::DipToRim => {
                if initiating {
                    "SPOT_UP_3PT"
                } else {
                    "DIP_TO_RIM"
                }
            }
        }
    }
}

/// 档案槽位的能力需求：`(属性, 权重)` 列表。权重只表达「多看重这项能力」，
/// 不预设任何具体球员。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SlotRequirement {
    pub attribute: PlayerAttributeKey,
    pub weight: f32,
}

/// slot fill 可引用的能力维度。用枚举，不直接用字符串：档案里写错维度名会
/// 在反序列化时报错，不致静默变成 0 分（`attributes.md` §4 「维度必须经
/// 映射层」的可核验形式）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayerAttributeKey {
    Speed,
    Acceleration,
    Agility,
    Strength,
    Vertical,
    Stamina,
    BallHandling,
    Passing,
    ShootingClose,
    ShootingNear,
    ShootingMid,
    ShootingThree,
    FreeThrow,
    Finishing,
    DefensePerimeter,
    DefenseInterior,
    Steal,
    Block,
    OffensiveRebound,
    DefensiveRebound,
    DecisionIq,
    OffBallSense,
}

/// 档案槽位规格：一个槽位 = 空间提示 + 能力需求 + 允许行为。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TacticalSlotSpec {
    /// 槽位标识（档案内唯一；不再作为身份来源，只作为槽位名的来源）。
    pub id: String,
    pub name_zh: String,
    /// 距**进攻底线**的距离（ft）。home 攻右篮时 x = width - base_offset_x。
    pub base_offset_x: f32,
    /// 绝对 y（0 = 一侧边线，height = 另一侧）。
    pub base_offset_y: f32,
    /// 填此槽位需要的能力（权重之和不为 0）。
    pub requirements: Vec<SlotRequirement>,
    /// 此槽位允许的行为。
    pub behaviour: SlotBehaviour,
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
    /// 校验档案自洽：槽位标识唯一、能力需求非空且权重为正。
    ///
    /// 档案是行为输入（charter C1），写错的档案必须在构造时被拒绝，
    /// 不致在场上静默退化为 0 分匹配。
    pub fn validate(&self) -> Result<(), String> {
        if self.slots.is_empty() {
            return Err(format!("tactical spec `{}` declares no slots", self.id));
        }
        let mut seen = std::collections::HashSet::new();
        for slot in &self.slots {
            if slot.id.trim().is_empty() {
                return Err(format!("tactical spec `{}` has a slot without id", self.id));
            }
            if !seen.insert(slot.id.as_str()) {
                return Err(format!(
                    "tactical spec `{}` declares duplicate slot id `{}`",
                    self.id, slot.id
                ));
            }
            if slot.requirements.is_empty() {
                return Err(format!(
                    "slot `{}` declares no capability requirement",
                    slot.id
                ));
            }
            if slot.requirements.iter().any(|r| r.weight < 0.0) {
                return Err(format!("slot `{}` has a negative weight", slot.id));
            }
            if slot.requirements.iter().all(|r| r.weight == 0.0) {
                return Err(format!(
                    "slot `{}` has no positive capability weight",
                    slot.id
                ));
            }
        }
        Ok(())
    }

    pub fn from_json(json_str: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json_str)
    }

    pub fn high_pick_and_roll() -> Self {
        const JSON: &str = include_str!("../../../data/tactics/high_pick_and_roll.json");
        let spec = Self::from_json(JSON).expect("内置 high_pick_and_roll.json 必须合法");
        spec.validate()
            .expect("内置 high_pick_and_roll.json 必须自洽");
        spec
    }

    pub fn five_out_motion() -> Self {
        const JSON: &str = include_str!("../../../data/tactics/five_out_motion.json");
        let spec = Self::from_json(JSON).expect("内置 five_out_motion.json 必须合法");
        spec.validate().expect("内置 five_out_motion.json 必须自洽");
        spec
    }

    pub fn spain_pick_and_roll() -> Self {
        const JSON: &str = include_str!("../../../data/tactics/spain_pick_and_roll.json");
        let spec = Self::from_json(JSON).expect("内置 spain_pick_and_roll.json 必须合法");
        spec.validate().expect("内置 spain_pick_and_roll.json 必须自洽");
        spec
    }

    pub fn builtin(id: &str) -> Option<Self> {
        match id {
            "off_horns_pnr" | "high_pick_and_roll" => Some(Self::high_pick_and_roll()),
            "off_spain_pnr" | "spain_pick_and_roll" => Some(Self::spain_pick_and_roll()),
            "off_motion_spacing" | "five_out_motion" => Some(Self::five_out_motion()),
            _ => None,
        }
    }
}
