use crate::court::CourtGeometry;
use serde::{Deserialize, Serialize};

/// Static player capabilities consumed by decision, movement, and officiating systems.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerAttributes {
    pub speed: f32,
    pub acceleration: f32,
    pub agility: f32,
    pub strength: f32,
    pub vertical: f32,
    pub stamina: f32,
    pub ball_handling: f32,
    pub passing: f32,
    pub shooting_close: f32,
    pub shooting_mid: f32,
    pub shooting_three: f32,
    pub free_throw: f32,
    pub finishing: f32,
    pub defense_perimeter: f32,
    pub defense_interior: f32,
    pub steal: f32,
    pub block: f32,
    pub offensive_rebound: f32,
    pub defensive_rebound: f32,
    pub decision_iq: f32,
    pub off_ball_sense: f32,
}

impl Default for PlayerAttributes {
    fn default() -> Self {
        Self {
            speed: 0.5,
            acceleration: 0.5,
            agility: 0.5,
            strength: 0.5,
            vertical: 0.5,
            stamina: 0.5,
            ball_handling: 0.5,
            passing: 0.5,
            shooting_close: 0.5,
            shooting_mid: 0.5,
            shooting_three: 0.5,
            free_throw: 0.5,
            finishing: 0.5,
            defense_perimeter: 0.5,
            defense_interior: 0.5,
            steal: 0.5,
            block: 0.5,
            offensive_rebound: 0.5,
            defensive_rebound: 0.5,
            decision_iq: 0.5,
            off_ball_sense: 0.5,
        }
    }
}

impl PlayerAttributes {
    #[inline]
    pub fn defense_on_ball(&self) -> f32 {
        self.defense_perimeter
    }
    #[inline]
    pub fn defense_help(&self) -> f32 {
        self.defense_interior
    }
    #[inline]
    pub fn rebounding(&self) -> f32 {
        (self.offensive_rebound + self.defensive_rebound) * 0.5
    }
    #[inline]
    pub fn basketball_iq(&self) -> f32 {
        self.decision_iq
    }
    #[inline]
    pub fn positioning(&self) -> f32 {
        self.off_ball_sense
    }
}
impl PlayerAttributes {
    pub fn validate(&self) -> Result<(), String> {
        let values = [
            self.speed,
            self.acceleration,
            self.agility,
            self.strength,
            self.vertical,
            self.stamina,
            self.ball_handling,
            self.passing,
            self.shooting_close,
            self.shooting_mid,
            self.shooting_three,
            self.free_throw,
            self.finishing,
            self.defense_perimeter,
            self.defense_interior,
            self.steal,
            self.block,
            self.offensive_rebound,
            self.defensive_rebound,
            self.decision_iq,
            self.off_ball_sense,
        ];
        if values
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err("player attributes must be finite and within 0..=1".to_string());
        }
        Ok(())
    }
}
impl PlayerTendencies {
    pub fn validate(&self) -> Result<(), String> {
        let values = [
            self.shoot_frequency,
            self.drive_frequency,
            self.pass_frequency,
            self.cut_frequency,
            self.screen_frequency,
            self.offensive_rebound_frequency,
            self.risk_tolerance,
            self.transition_sprint,
        ];
        if values
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err("player tendencies must be finite and within 0..=1".to_string());
        }
        Ok(())
    }
}

impl TeamTraits {
    pub fn validate(&self) -> Result<(), String> {
        let values = [
            self.pace,
            self.three_point_emphasis,
            self.rim_pressure,
            self.defense_aggression,
            self.rebound_emphasis,
        ];
        if values
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err("team traits must be finite and within 0..=1".to_string());
        }
        Ok(())
    }
}

/// Decision preferences are separate from ability so the same skill can produce
/// different styles without branching on a roster-specific player id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerTendencies {
    pub shoot_frequency: f32,
    pub drive_frequency: f32,
    pub pass_frequency: f32,
    pub cut_frequency: f32,
    pub screen_frequency: f32,
    pub offensive_rebound_frequency: f32,
    pub risk_tolerance: f32,
    pub transition_sprint: f32,
}

impl Default for PlayerTendencies {
    fn default() -> Self {
        Self {
            shoot_frequency: 0.5,
            drive_frequency: 0.5,
            pass_frequency: 0.5,
            cut_frequency: 0.5,
            screen_frequency: 0.5,
            offensive_rebound_frequency: 0.5,
            risk_tolerance: 0.5,
            transition_sprint: 0.5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerData {
    pub id: String,
    pub name: String,
    pub jersey: String,
    pub team_id: String,
    pub height_cm: u16,
    pub weight_kg: u16,
    pub age: u8,
    pub attributes: PlayerAttributes,
    pub tendencies: PlayerTendencies,
    /// 是否为首发（round-10 Step4a）。
    ///
    /// ## 为什么首发必须是数据而不是数组位置
    ///
    /// 此前 `default_lineup` 直接取**数组前 5 个**作为首发 —— 于是
    /// 「顺序即身份」：把名册数组轮转一下，首发阵容就变了（并能触发
    /// `starter H_06 starts outside the court geometry` 这类错误）。
    ///
    /// 现在首发由档案显式声明，数组顺序不再携带任何语义。
    #[serde(default)]
    pub starter: bool,
    /// Initial court position in the engine's feet coordinate system.
    pub initial_position_ft: (f32, f32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TeamTraits {
    pub pace: f32,
    pub three_point_emphasis: f32,
    pub rim_pressure: f32,
    pub defense_aggression: f32,
    pub rebound_emphasis: f32,
}

impl Default for TeamTraits {
    fn default() -> Self {
        Self {
            pace: 0.5,
            three_point_emphasis: 0.5,
            rim_pressure: 0.5,
            defense_aggression: 0.5,
            rebound_emphasis: 0.5,
        }
    }
}

/// 教练策略档案（tactics.md §2.4 CoachProfile）。
///
/// 消费点：`MatchEngine` 在每次回合转换边界重建教练策略（比分差、节次、
/// 剩余时间 → `CoachStrategy`），再由决策效用读取 `pace_factor` /
/// `three_point_bias` / `defense_aggression`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoachProfile {
    pub id: String,
    pub name: String,
    pub pace_bias: f32,
    pub timeout_threshold_run: u8,
    pub offensive_system_preference: Vec<String>,
    pub defensive_system_preference: Vec<String>,
    pub substitution_tendency: f32,
    pub garbage_time_margin: u8,
}

impl Default for CoachProfile {
    fn default() -> Self {
        Self {
            id: "coach_default".to_string(),
            name: "Default Coach".to_string(),
            pace_bias: 0.5,
            timeout_threshold_run: 8,
            offensive_system_preference: vec!["off_horns_pnr".to_string()],
            defensive_system_preference: vec!["def_drop_coverage".to_string()],
            substitution_tendency: 0.5,
            garbage_time_margin: 20,
        }
    }
}

/// 轮换表配置项（tactics.md §2.3.3 RotationEntry）。
///
/// 消费点：`MatchEngine` 在死球窗口按 `foul_trouble_threshold` 与
/// `stint_max_sec` 判定是否换人；`target_minutes` 用于节间重置累计出场时间。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RotationEntry {
    pub player_id: String,
    pub target_minutes: u8,
    pub stint_max_sec: u16,
    pub foul_trouble_threshold: u8,
}

/// 换人原因（tactics.md §2.3.3 SubstitutionReason）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubstitutionReason {
    FoulTrouble,
    StaminaExhaustion,
    TacticalAdjustment,
    GarbageTime,
    Injury,
}

impl SubstitutionReason {
    /// 换人播报文案：原因决定措辞，替补登场动作用词统一。
    pub fn callout(&self, out_player: &str, in_player: &str) -> String {
        let why = match self {
            Self::FoulTrouble => "犯满离场",
            Self::StaminaExhaustion => "体力耗尽下场休息",
            Self::TacticalAdjustment => "教练战术调整换下",
            Self::GarbageTime => "垃圾时间轮换休息",
            Self::Injury => "受伤离场",
        };
        format!(
            "球员 {} {}，替补 {} 死球登场入位！",
            out_player, why, in_player
        )
    }
}

/// 换人请求（tactics.md §2.3.3 SubstitutionEvent）。
///
/// 这是**请求**而不是事实：引擎在死球窗口评估它，接受时发出
/// `GameEvent::Substitution`，被拒绝时记入强制项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubstitutionEvent {
    pub team_id: String,
    pub player_out: String,
    pub player_in: String,
    pub reason: SubstitutionReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamData {
    pub id: String,
    pub name: String,
    pub short_name: String,
    pub players: Vec<PlayerData>,
    pub default_offense_tactic: String,
    pub default_defense_tactic: String,
    pub team_traits: TeamTraits,
}

impl TeamData {
    pub fn builtin_home() -> Self {
        Self::builtin_home_with_geometry(CourtGeometry::default())
    }

    pub fn builtin_home_with_geometry(geometry: CourtGeometry) -> Self {
        Self {
            id: "north_city_hawks".to_string(),
            name: "North City Hawks".to_string(),
            short_name: "Hawks".to_string(),
            players: load_roster(HOME_ROSTER_JSON, geometry),
            default_offense_tactic: "off_horns_pnr".to_string(),
            default_defense_tactic: "def_drop_coverage".to_string(),
            team_traits: TeamTraits {
                pace: 0.62,
                three_point_emphasis: 0.58,
                rim_pressure: 0.64,
                defense_aggression: 0.48,
                rebound_emphasis: 0.55,
            },
        }
    }

    pub fn builtin_away() -> Self {
        Self::builtin_away_with_geometry(CourtGeometry::default())
    }

    pub fn builtin_away_with_geometry(geometry: CourtGeometry) -> Self {
        Self {
            id: "south_bay_mariners".to_string(),
            name: "South Bay Mariners".to_string(),
            short_name: "Mariners".to_string(),
            players: load_roster(AWAY_ROSTER_JSON, geometry),
            default_offense_tactic: "off_motion_spacing".to_string(),
            default_defense_tactic: "def_man_conservative".to_string(),
            team_traits: TeamTraits {
                pace: 0.48,
                three_point_emphasis: 0.64,
                rim_pressure: 0.50,
                defense_aggression: 0.56,
                rebound_emphasis: 0.61,
            },
        }
    }

    pub fn builtin_pair() -> [Self; 2] {
        Self::builtin_pair_with_geometry(CourtGeometry::default())
    }

    pub fn builtin_pair_with_geometry(geometry: CourtGeometry) -> [Self; 2] {
        [
            Self::builtin_home_with_geometry(geometry),
            Self::builtin_away_with_geometry(geometry),
        ]
    }
}

/// 名册档案的反序列化外壳（round-10 Step4a）。
#[derive(serde::Deserialize)]
struct RosterFile {
    players: Vec<PlayerData>,
}

/// 从声明式档案载入名册（单一人事实源）。
///
/// ## 为什么改为数据档案（P-2 / charter C1）
///
/// 此前名册由 `builtin_attributes(index)` / `builtin_roles(index)` /
/// `builtin_tendencies(index)` 按**数组下标**分派，且 `id = "{prefix}_{index+1}"`
/// 让 id 本身编码了下标。后果：**顺序即身份**——球员"是什么"取决于他在数组里
/// 排第几，而文档（`attributes.md §2.7/§2.9/T1`、`tactics.md TA3`）要求
/// 「角色是槽位不是身份」，且 `roles` 字段应当移除。
///
/// 现在球员作为 `data/roster/*.json` 数据资产声明：
/// - **数组顺序不携带语义**（放哪都一样）；
/// - **无 `roles` 字段**（文档要求）；
/// - `id` 不再承载身份序号。
///
/// `initial_position_ft` 仍随档案声明（几何相关），因此载入后按当前
/// 场地几何做一次等比缩放，兼容自定义 `CourtGeometry`。
const HOME_ROSTER_JSON: &str = include_str!("../../../data/roster/home.json");
const AWAY_ROSTER_JSON: &str = include_str!("../../../data/roster/away.json");

fn load_roster(json: &str, geometry: CourtGeometry) -> Vec<PlayerData> {
    let mut file: RosterFile =
        serde_json::from_str(json).expect("data/roster/*.json must be valid (charter C1)");
    // 档案以默认几何书写；换几何时把初始站位等比缩放。
    let def = CourtGeometry::default();
    if (def.width_ft - geometry.width_ft).abs() > f32::EPSILON
        || (def.height_ft - geometry.height_ft).abs() > f32::EPSILON
    {
        let sx = geometry.width_ft / def.width_ft;
        let sy = geometry.height_ft / def.height_ft;
        for p in file.players.iter_mut() {
            p.initial_position_ft = (p.initial_position_ft.0 * sx, p.initial_position_ft.1 * sy);
        }
    }
    file.players
}

/// D5.1b：slot fill 的能力画像（tactics.md §3 规定）。
///
/// 承载全部 21 个能力维度，因为档案槽位的 `requirements` 可以声明任意一维
/// （`TacticalSlotSpec::requirements`），只暴露子集会让档案声明一维不在
/// 子集内的能力时静默拿到 0 分。本身不承担任何可变状态。
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerSlotFitness {
    pub player_id: String,
    pub attributes: PlayerAttributes,
}

impl PlayerSlotFitness {
    /// 从球员档案投影（纯函数）。
    pub fn from_player(player: &PlayerData) -> Self {
        Self {
            player_id: player.id.clone(),
            attributes: player.attributes.clone(),
        }
    }

    /// 按维度键取能力值（slot fill 的唯一取值入口）。
    pub fn attribute(&self, key: crate::tactics::PlayerAttributeKey) -> f32 {
        use crate::tactics::PlayerAttributeKey as K;
        let a = &self.attributes;
        match key {
            K::Speed => a.speed,
            K::Acceleration => a.acceleration,
            K::Agility => a.agility,
            K::Strength => a.strength,
            K::Vertical => a.vertical,
            K::Stamina => a.stamina,
            K::BallHandling => a.ball_handling,
            K::Passing => a.passing,
            K::ShootingClose => a.shooting_close,
            K::ShootingMid => a.shooting_mid,
            K::ShootingThree => a.shooting_three,
            K::FreeThrow => a.free_throw,
            K::Finishing => a.finishing,
            K::DefensePerimeter => a.defense_perimeter,
            K::DefenseInterior => a.defense_interior,
            K::Steal => a.steal,
            K::Block => a.block,
            K::OffensiveRebound => a.offensive_rebound,
            K::DefensiveRebound => a.defensive_rebound,
            K::DecisionIq => a.decision_iq,
            K::OffBallSense => a.off_ball_sense,
        }
    }
}

/// 展示用角色的**纯函数投影**（attributes.md §2.7/§2.9）。
///
/// ## 为什么是纯函数而不是字段
///
/// 文档要求 `roles` 从球员档案移除、降级为「上层派生视图」——因为
/// 「角色是槽位不是身份」（`tactics.md TA3`）。原先的
/// `PlayerData.roles: Vec<PlayerRole>` 由 `builtin_roles(index)` 按**数组下标**
/// 分派，使「顺序即身份」：球员"是什么"取决于他在名册里排第几。
///
/// 现在展示标签由 `(attributes, tendencies)` 重算，因此：
/// - 名册数组顺序不携带语义（打乱顺序不改变任何标签）；
/// - 标签不参与任何行为判定（只用于 UI 展示）；
/// - 同一份数据必得同一标签（纯函数，可复现）。
///
/// 判定顺序按"最能区分该球员的维度"降序：先看极端专长，再看通用倾向。
pub fn project_display_role(
    attributes: &PlayerAttributes,
    tendencies: &PlayerTendencies,
) -> String {
    // 阈值来自"显著高于联盟中位"的常识口径；不参与行为，故不进规则通道
    // （仅影响展示字符串，charter C1 的"行为常数"定义不覆盖展示）。
    let a = attributes;
    let t = tendencies;

    // 极端专长优先
    if a.defense_interior >= 0.9 && a.block >= 0.9 {
        return "RimProtector".to_string();
    }
    if a.offensive_rebound >= 0.88 && a.defensive_rebound >= 0.9 {
        return "Rebounder".to_string();
    }
    if a.ball_handling >= 0.85 && a.passing >= 0.8 {
        return "PrimaryCreator".to_string();
    }
    if a.shooting_three >= 0.88 && t.shoot_frequency >= 0.8 {
        return "Shooter".to_string();
    }
    if t.screen_frequency >= 0.8 {
        return "Screener".to_string();
    }
    if t.cut_frequency >= 0.8 {
        return "Cutter".to_string();
    }
    if a.defense_perimeter >= 0.82 {
        return "Defender".to_string();
    }
    if a.passing >= 0.75 {
        return "SecondaryCreator".to_string();
    }
    "Player".to_string()
}
