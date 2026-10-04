//! 规则策略组：`GameRules` 聚合的各子策略及它们的默认值与校验。
//!
//! 本模块是 `rules` 的叶子层：这些策略结构不依赖 `GameRules`，
//! 方向是单向的 `GameRules` → 各策略。

use glam::Vec2;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenDefenseRules {
    pub drop_depth_ft: f32,
    pub hedge_distance_ft: f32,
    pub switch_trigger_distance_ft: f32,
    pub recover_timeout_seconds: f32,
}

impl Default for ScreenDefenseRules {
    fn default() -> Self {
        Self {
            drop_depth_ft: 0.0,
            hedge_distance_ft: 0.0,
            switch_trigger_distance_ft: 5.0,
            recover_timeout_seconds: 1.2,
        }
    }
}

/// 能力映射层的曲线系数（`capability.rs` 的六个 D27 维度）。
///
/// ## 为何需要这些字段
///
/// 六个 `effective_*` 函数原先的返回值只由 `PlayerAttributes` 与
/// `attribute_response_floor` 决定，**不读任何具体规则系数**，
/// 因此它们是属性的纯函数：改规则参数无法改变行为，
/// 「属性 → 物理量」的换算曲线不可校准。
/// 本结构把六个维度的截距与斜率送入规则通道（charter C1），
/// 使返回值同时受属性与规则控制。
///
/// ## 命名
///
/// 每个维度带 `*_base`（属性为零时的返回值）与 `*_gain`（该属性从 0 增到 1
/// 时的增量）；权重类维度（`boxout` / `post_defense`）额外带
/// `*_secondary_gain`，表示第二属性（`strength` 或 `decision_iq`）的增量。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CapabilityCurveRules {
    /// `effective_risk_tolerance` = base - iq * gain（风险容忍随决策智商下降）。
    pub risk_tolerance_base: f32,
    pub risk_tolerance_gain: f32,
    /// `effective_defensive_boxout_bonus`：主属性 `defensive_rebound` 与次属性 `strength`。
    pub boxout_bonus_base: f32,
    pub boxout_bonus_primary_gain: f32,
    pub boxout_bonus_secondary_gain: f32,
    /// `effective_putback_bias`：属性 `finishing`。
    pub putback_bias_base: f32,
    pub putback_bias_gain: f32,
    /// `effective_help_awareness`：主属性 `defense_interior` 与次属性 `decision_iq`。
    pub help_awareness_primary_gain: f32,
    pub help_awareness_secondary_gain: f32,
    /// `effective_post_defense_physicality`：主属性 `defense_interior` 与次属性 `strength`。
    pub post_defense_primary_gain: f32,
    pub post_defense_secondary_gain: f32,
    /// `effective_transition_leakout_chance`：属性 `speed`。
    pub transition_leakout_base: f32,
    pub transition_leakout_gain: f32,
    /// 快下机会阈值：本队最大 `transition_leakout_chance` 达到此值时才
    /// 在防守篮板后向快下球员出球（而不是交给控卫重新组织）。
    pub transition_leakout_threshold: f32,
    /// 低位背身的防守阻力曲线：`mult = floor + gain × (mid - resistance)`。
    /// `resistance` 是对位防守人的 `effective_post_defense_physicality`
    /// （已夹在 [0,1]），因此中性对位（`mid`）时乘数为 floor。
    pub post_defense_resistance_floor: f32,
    pub post_defense_resistance_gain: f32,
    /// 能力的中性参考值：属性缺失或对位人不可知时的回退值
    /// （属性域的 0.5 中点，也是 `contest` 类度量的中性点）。
    pub neutral_attribute: f32,
    /// `effective_turn_decel_retention`：变向减速后保留的速度变化量。
    ///
    /// `attributes.md` §2.2 声明 `agility` 的消费链是「防守滑步最大速度、
    /// 变向减速代价」——后者就是本值。锚点是 `TacticalRules.turn_decel_retention`，
    /// 本值表示 `agility` 高于或低于中位（0.5）时保留比例的增减。
    pub turn_decel_retention_gain: f32,
    /// 该维度属性的独立下限。
    ///
    /// 不得复用 `attribute_response_floor`（默认 0.5）：那会把 0..0.5 整段
    /// agility 压成同一个值，底部半程成为无效区间——实测 `agility = 0.05` 与
    /// `agility = 0.5` 行为逐位相同，而 `attributes.md` §4 明令禁止
    /// 「内联 floor 把有效区间压缩成无效区间」。
    pub turn_decel_retention_attribute_floor: f32,
}

impl Default for CapabilityCurveRules {
    fn default() -> Self {
        Self {
            risk_tolerance_base: 0.80,
            risk_tolerance_gain: 0.35,
            boxout_bonus_base: 0.10,
            boxout_bonus_primary_gain: 0.15,
            boxout_bonus_secondary_gain: 0.10,
            putback_bias_base: 0.05,
            putback_bias_gain: 0.35,
            help_awareness_primary_gain: 0.60,
            help_awareness_secondary_gain: 0.60,
            post_defense_primary_gain: 0.50,
            post_defense_secondary_gain: 0.50,
            transition_leakout_base: 0.10,
            transition_leakout_gain: 0.40,
            // 阈值高于基线 0.10 + 0.40 × 0.5 = 0.30，因此速度平庸的球队
            // 仍走「交给控卫」的稳定路线，只有快攻型阵容才触发快下。
            transition_leakout_threshold: 0.36,
            post_defense_resistance_floor: 0.8,
            post_defense_resistance_gain: 0.4,
            neutral_attribute: 0.5,
            // 锚点为全局 `turn_decel_retention = 0.4`：`agility = 0.5` 时该系数
            // 对结果零影响，高于/低于中位各 0.5 时保留比例在 0.4 上下浮动 ±0.1。
            turn_decel_retention_gain: 0.2,
            turn_decel_retention_attribute_floor: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModulationRules {
    pub hot_hand_makes: u32,
    pub frustrated_misses: u32,
    pub frustrated_turnovers: u32,
    pub hot_hand_bias: f32,
    pub clutch_bias: f32,
    pub frustrated_bias: f32,
    pub exhausted_bias: f32,
    pub late_game_period: u32,
    pub late_game_seconds: f32,
    pub trailing_score_margin: i32,
    pub leading_score_margin: i32,
    pub trailing_pace_factor: f32,
    pub trailing_three_point_bias: f32,
    pub trailing_defense_aggression: f32,
    pub leading_pace_factor: f32,
    pub leading_three_point_bias: f32,
    pub leading_defense_aggression: f32,
    pub clutch_period: u32,
    pub clutch_time_remaining: f32,
    pub clutch_score_margin: i32,
    /// 士气幅度的**动作族权重**（ModulationRules 通道，charter C1）。
    ///
    /// ## 为何需要权重（round-20 审计）
    ///
    /// 士气修正原先作为**加性常数**加在全部候选的效用上，而采样是
    /// `exp((u - max_u) / temperature)` 的 softmax：同一持球人的全部候选共享
    /// 同一个加性项，比值不变，因此 `hot_hand_bias` / `clutch_bias` /
    /// `frustrated_bias` / `exhausted_bias` 对选择分布**零影响**
    /// （`wiring_proof.rs::rules_wiring_clutch_modulation_changes_simulation`
    /// 实测 4 个 seed 中 0 个行为改变）。
    ///
    /// 修正：把士气标量按动作族加权后再相加，使同一个标量对不同候选
    /// 产生不同修正——加性项因此在 softmax 中不再抵消。
    ///
    /// ## 语义轴
    ///
    /// 标量承载的是**自信与主动性**（热手与关键时段的沉稳为正值，低迷与
    /// 体力耗尽为负值），权重声明该主动性在各动作族上的分配：主动性高则
    /// 终结与突破更多、组织观察更少。这是单一标量能诚实表达的唯一语义轴；
    /// 「热手」与「急躁」是两种心理，不共用此通道。
    pub morale_shoot_affinity: f32,
    pub morale_drive_affinity: f32,
    pub morale_pass_affinity: f32,
    /// 持球观察族的权重（在效用中**减**去，使主动性高时少观察）。
    pub morale_dwell_affinity: f32,
}

impl Default for ModulationRules {
    fn default() -> Self {
        Self {
            hot_hand_makes: 2,
            frustrated_misses: 3,
            frustrated_turnovers: 2,
            hot_hand_bias: 0.10,
            clutch_bias: 0.08,
            frustrated_bias: -0.05,
            exhausted_bias: -0.12,
            late_game_period: 4,
            late_game_seconds: 120.0,
            trailing_score_margin: 6,
            leading_score_margin: 6,
            trailing_pace_factor: 1.35,
            trailing_three_point_bias: 1.6,
            trailing_defense_aggression: 1.4,
            leading_pace_factor: 0.75,
            leading_three_point_bias: 0.8,
            leading_defense_aggression: 1.0,
            clutch_period: 4,
            clutch_time_remaining: 120.0,
            clutch_score_margin: 5,
            morale_shoot_affinity: 0.35,
            morale_drive_affinity: 0.30,
            morale_pass_affinity: 0.12,
            morale_dwell_affinity: 0.25,
        }
    }
}

impl ModulationRules {
    pub fn validate(&self) -> Result<(), String> {
        let values = [
            self.hot_hand_bias,
            self.clutch_bias,
            self.frustrated_bias,
            self.exhausted_bias,
            self.late_game_seconds,
            self.trailing_pace_factor,
            self.trailing_three_point_bias,
            self.trailing_defense_aggression,
            self.leading_pace_factor,
            self.leading_three_point_bias,
            self.leading_defense_aggression,
            self.clutch_time_remaining,
            self.morale_shoot_affinity,
            self.morale_drive_affinity,
            self.morale_pass_affinity,
            self.morale_dwell_affinity,
        ];
        if values.iter().any(|value| !value.is_finite()) {
            return Err("modulation policy contains a non-finite value".to_string());
        }
        if self.hot_hand_makes == 0
            || self.frustrated_misses == 0
            || self.frustrated_turnovers == 0
            || self.late_game_period == 0
            || self.clutch_period == 0
            || self.late_game_seconds < 0.0
            || self.clutch_time_remaining < 0.0
            || self.trailing_score_margin < 0
            || self.leading_score_margin < 0
            || self.clutch_score_margin < 0
            || self.trailing_pace_factor < 0.0
            || self.trailing_three_point_bias < 0.0
            || self.trailing_defense_aggression < 0.0
            || self.leading_pace_factor < 0.0
            || self.leading_three_point_bias < 0.0
            || self.leading_defense_aggression < 0.0
            || self.morale_shoot_affinity < 0.0
            || self.morale_drive_affinity < 0.0
            || self.morale_pass_affinity < 0.0
            || self.morale_dwell_affinity < 0.0
        {
            return Err("modulation policy contains an invalid value".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SemanticRules {
    pub contact_minor_speed_ratio: f32,
    pub contact_positional_speed_ratio: f32,
    pub contact_foul_candidate_speed_ratio: f32,
    pub screen_stationary_speed_ratio: f32,
    pub spacing_corner_weight: f32,
    pub spacing_weak_side_weight: f32,
    pub spacing_lane_weight: f32,
    pub spacing_paint_penalty: f32,
    pub opponent_density_capacity: f32,
    pub pass_base_probability: f32,
    pub pass_openness_weight: f32,
    pub pass_lane_risk_weight: f32,
    pub density_distance_weight: f32,
    pub minimum_entity_distance_ft: f32,
    /// Perimeter windows used to classify the ball side and its opposite side.
    pub perimeter_side_margin_ratio: f32,
    pub perimeter_depth_ratio: f32,
    pub perimeter_density_capacity: f32,
    pub strong_side_ball_weight: f32,
    pub strong_side_defender_weight: f32,
    pub weak_side_defender_weight: f32,
    pub crowded_zone_threshold: f32,
    pub dense_zone_threshold: f32,
    pub crowded_zone_utility_penalty: f32,
    pub crowded_zone_risk: f32,
    pub dense_zone_utility_penalty: f32,
    pub dense_zone_risk: f32,
    pub heavily_contested_threshold: f32,
    pub heavily_contested_utility_penalty: f32,
    pub heavily_contested_risk: f32,
    pub contested_threshold: f32,
    pub contested_utility_penalty: f32,
    pub contested_risk: f32,
    pub open_shot_contest_threshold: f32,
    pub contest_dist_weight: f32,
    pub contest_speed_weight: f32,
    pub contest_speed_factor_cap: f32,
    /// 全场防守压迫密度核的作用半径（ft）。
    ///
    /// 「压迫」与 `contest_intensity` 是两个不同的空间量：后者是**最近一名**
    /// 防守人的距离/朝向/速度合成的单人对位干扰；前者是**多名**防守人的
    /// 覆盖叠加，用于判断某个位置是否处在协防网之内。两者不重复，
    /// 压力核只统计这个半径内的防守人。
    pub pressure_radius_ft: f32,
    /// 压力核的距离衰减尺度（ft）：核形状为 `alignment / ((d/scale)^2 + 1)`，
    /// 数值越大压迫衰减越慢。
    pub pressure_distance_scale_ft: f32,
    /// 防守人背向进攻人时的最小朝向系数（避免背身防守完全不计压力）。
    pub pressure_facing_floor: f32,
}

impl SemanticRules {
    /// 位置 `pos` 处的防守压迫密度：`pressure_radius_ft` 内每名防守人的
    /// 朝向投影与距离衰减之和。零名防守人在范围内时为 0。
    pub fn defensive_pressure(
        &self,
        pos: Vec2,
        defenders: impl IntoIterator<Item = (Vec2, Vec2)>,
    ) -> f32 {
        let mut press = 0.0_f32;
        for (defender_pos, facing) in defenders {
            let delta = pos - defender_pos;
            let distance = delta.length();
            if distance >= self.pressure_radius_ft || distance <= f32::EPSILON {
                continue;
            }
            let to_offense = delta / distance;
            let alignment = facing.dot(to_offense).max(self.pressure_facing_floor);
            let scale = self.pressure_distance_scale_ft.max(f32::EPSILON);
            press += alignment / ((distance / scale).powi(2) + 1.0);
        }
        press
    }
}

impl Default for SemanticRules {
    fn default() -> Self {
        Self {
            contact_minor_speed_ratio: 0.25,
            contact_positional_speed_ratio: 0.50,
            contact_foul_candidate_speed_ratio: 0.58,
            screen_stationary_speed_ratio: 0.20,
            // D3.1 校准（dev 方案 §6.2）：spacing_bonus 三项权重原和为 1.0，
            // 空位时直接叠加近 +1.0 命中率（3P 64% 主因）。下调至和 0.16，
            // 使 spacing 成为小幅调制而非主导项。8 seed full 证据：3P 64→35.4%、
            // 2P 73→63.8%，无交叉准则退化，扰动测试 9/9 绿。
            spacing_corner_weight: 0.05,
            spacing_weak_side_weight: 0.05,
            spacing_lane_weight: 0.06,
            spacing_paint_penalty: 0.12,
            opponent_density_capacity: 3.0,
            pass_base_probability: 0.50,
            pass_openness_weight: 0.40,
            pass_lane_risk_weight: 0.35,
            density_distance_weight: 0.50,
            minimum_entity_distance_ft: 0.001,
            perimeter_side_margin_ratio: 0.32,
            perimeter_depth_ratio: 0.22,
            perimeter_density_capacity: 2.0,
            strong_side_ball_weight: 0.65,
            strong_side_defender_weight: 0.35,
            weak_side_defender_weight: 0.65,
            crowded_zone_threshold: 0.45,
            dense_zone_threshold: 0.25,
            crowded_zone_utility_penalty: 0.18,
            crowded_zone_risk: 0.10,
            dense_zone_utility_penalty: 0.08,
            dense_zone_risk: 0.05,
            heavily_contested_threshold: 0.75,
            heavily_contested_utility_penalty: 0.30,
            heavily_contested_risk: 0.25,
            contested_threshold: 0.50,
            contested_utility_penalty: 0.15,
            contested_risk: 0.12,
            open_shot_contest_threshold: 0.35,
            contest_dist_weight: 0.8,
            contest_speed_weight: 1.0,
            contest_speed_factor_cap: 0.4,
            pressure_radius_ft: 16.0,
            pressure_distance_scale_ft: 4.0,
            pressure_facing_floor: 0.1,
        }
    }
}

/// 防守体系参数（round-6 审计修复）：把「防守方案」从展示字符串变成因果输入。
///
/// 每个防守方案实例化一份，经 `GameRules` 通道进入防守目标点生成。
/// 三个倍率都围绕**已有**的基准量调整（`help_sag_ratio`、`defensive_gap_ft`），
/// 因此默认值 `1.0/1.0/1.0` 完全等价于历史行为——保证修复前的黄金哈希
/// 在不指定防守方案时不受影响。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DefenseRules {
    /// 无球防守人协防深度倍率：>1 收缩（联防/沉退），<1 外扩（紧逼）。
    pub sag_multiplier: f32,
    /// 领防人距对位人的间隔倍率：<1 贴得更近（紧逼），>1 放得更远（沉退）。
    pub on_ball_gap_multiplier: f32,
    /// 弱侧协防优先级：越大越愿意放空外线收缩护框。
    pub help_priority: f32,
    /// 换防激进程度：0 = 不换防，1 = 逢掩护必换（用于后续 switch 执行链）。
    pub switch_aggressiveness: f32,
    /// 弱侧协防方向的人-筐基准权重（`help_priority == 0.5` 时生效）。
    ///
    /// 历史公式为 `to_hoop * 0.7 + to_carrier * 0.3`；把两个权重参数化，
    /// 使 `help_priority` 只需在基准上倾斜，且默认值逐位复原历史行为。
    pub help_hoop_weight_base: f32,
    /// `help_priority` 高于或低于 0.5 时对人-筐权重的倾斜系数。
    pub help_priority_tilt_gain: f32,
    /// 人-筐权重下限（防止协防完全脱离篮筐方向）。
    pub help_hoop_weight_min: f32,
    /// 人-筐权重上限（防止协防退化为纯护框而放弃外线）。
    pub help_hoop_weight_max: f32,
    /// 挡拆/掩护防守行为参数（D17 / schemes.json v2）
    pub screen_defense: ScreenDefenseRules,
    /// 沉退方案下低位人（弱侧离筐最近的防守人）的护筐引力增益。
    /// 真实沉退防守的大个子几乎整回合缩在护框纵深内不跟出去；
    /// 缺省 1.0 = 与盯人无差异。schemes.json 的 `drop_low_man_rim_gain`
    /// 通道（man 1.0 / drop 2.6 / zone 2.2）。
    pub drop_low_man_rim_gain: f32,
    /// 多体势能场求解器的参数（`decision::potential_field`）。
    ///
    /// ## 为何进规则通道（charter C1）
    ///
    /// 势能场是防守跑位的**生成器**：它的系数直接决定每个无球防守人跑去哪里。
    /// 这些量曾以字段默认值与字面量两种形式散在 `potential_field.rs` 里，
    /// 既无法用 `--rules` 覆盖，也无法被常数守卫看到（该文件当时不在预算名单内）。
    pub potential_field: PotentialFieldRules,
}

/// 多体势能场参数（`decision::potential_field`）。
///
/// 势能分量 = 对位牵引（弹簧） + 护筐引力 + 外线真空吸力；系数全部可校准。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PotentialFieldRules {
    /// 篮筐威胁特征半径（ft）：突破深度对全场势能的非线性放大陡峭度。
    pub threat_radius_ft: f32,
    /// 禁区内线局部响应半径（ft）：不同距篮距离的引力衰减。
    pub rim_response_radius_ft: f32,
    /// 对位羁绊基础弹性系数。
    pub k_man_base: f32,
    /// 禁区护筐威胁引力基准系数。
    pub k_threat_base: f32,
    /// 空间覆盖真空吸力系数（X-Out 驱动源）。
    pub k_void_base: f32,
    /// 外线对位的下沉距离（ft，对位人距篮超过 22 ft 时）。
    pub sag_distance_perimeter_ft: f32,
    /// 中距离/内线对位的下沉距离（ft）。
    pub sag_distance_interior_ft: f32,
    /// 下沉锚点方向「朝篮筐」权重的合成配方（`help_blend`，与朝持球人权重互补，两者和应为 1）。
    ///
    /// 实际锚点权重 = `clamp(base + tilt_gain × help_priority, min, max)`，
    /// 在 `solve_equilibrium` 的下沉锚点混合处求解（help_blend 接入，plan_play.md #22）。
    pub help_hoop_weight_base: f32,
    /// `help_priority` 对锚点权重的倾斜系数：每单位协防优先级向篮筐方向的增量。
    pub help_priority_tilt_gain: f32,
    /// 锚点权重的方案无关下限：防止协防完全脱离篮筐方向。
    pub help_hoop_weight_min: f32,
    /// 锚点权重的方案无关上限：防止协防退化为纯护框而放弃外线。
    pub help_hoop_weight_max: f32,
    /// 护筐目标点距篮筐的缓冲带（ft）：威胁中心位于持球人与篮筐连线上此距离处。
    pub rim_buffer_ft: f32,
    /// 弱侧低位人（Low-man）的护筐引力倍率。
    pub low_man_threat_gain: f32,
    /// 弱侧高位人（High-man）的护筐引力倍率（保留在外线，防备三分）。
    pub high_man_threat_gain: f32,
    /// 其余防守人的护筐引力倍率。
    pub default_threat_gain: f32,
    /// 真空吸力倍率。
    pub void_gain: f32,
    /// 涌现为「护筐轮转」的威胁占比阈值。
    pub rim_help_threat_ratio: f32,
    /// 突破激励的护筐引力增益（D26）：持球人突破（Drive 球态）时，弱侧
    /// 防守人的 w_threat 以此增益重新计算，使 threat_ratio 越过
    /// rim_help_threat_ratio 而涌现 ROTATE_RIM_HELP——NBA 语义里突破
    /// 时弱侧收缩是协防铁律，此前只有 low-man 被激励（实测响应率 64-70%）。
    pub drive_help_threat_gain: f32,
    /// 涌现为「X-Out 补位」的真空占比阈值。
    pub x_out_void_ratio: f32,
    /// 判定为「已在护筐位置」的距篮距离（ft）。
    pub rim_help_radius_ft: f32,
    /// 弱侧判定的人力横向差值（ft）：与持球人 y 相差超过此值的进攻人
    /// 归入弱侧轮转区（在持球人居中时补充中轴线的几何判定）。
    pub weak_side_lateral_ft: f32,
    /// 对位人距篮超过此值时按外线处理（贴防阻截出手），否则按内线处理。
    pub perimeter_attribution_ft: f32,
    /// 掩护判定半径（ft）：持球人与掩护人相距小于此值即视为正在发生掩护。
    pub screen_detection_radius_ft: f32,
    /// 换防激进程度的两个档：（高，低）。
    ///
    /// - 大于 `switch_high_threshold` 时逢掩护必换；
    /// - 大于 `switch_low_threshold` 且距掩护小于档案的触发距离时换防。
    pub switch_high_threshold: f32,
    pub switch_low_threshold: f32,
    /// 换防后防守人距被接管者的分离距离（ft）。
    pub switch_anchor_gap_ft: f32,
    /// 换防后对掩护人的分离距离（ft）。
    pub switch_screener_gap_ft: f32,
    /// 领防人间隔中「距篮比例」因子的上限（防止远离篮筐时间隔过大）。
    pub on_ball_gap_hoop_ratio: f32,
    /// 领防人间隔的下限（ft）。
    pub on_ball_gap_min_ft: f32,
    /// 下沉系数 `sag_multiplier` 的可用区间下限。
    pub sag_multiplier_min: f32,
    /// 下沉系数 `sag_multiplier` 的可用区间上限。
    pub sag_multiplier_max: f32,
    /// 势能权重的极小正数下限（避免三分量同时为零时除以零）。
    pub total_weight_floor: f32,
    /// 体能衰减通道（plan_play.md #24）的保底倍率：防守人体能归零时，
    /// 护筐引力与真空吸力这两个主动跑动分量仍保留此比例（0.6 = 保留 60%）。
    pub stamina_floor: f32,
    /// 体能衰减幂指数：衰减系数 = `stamina_floor + (1 - stamina_floor) ×
    /// stamina^stamina_gain`。指数越大，中等体能区间的衰减越平缓、
    /// 低体能区间越陡；满体能（1.0）时系数恒为 1，行为与无衰减一致。
    pub stamina_gain: f32,
    /// 「协防人被拉离走廊」（`help_pulled_off`）进入态的 `threat_ratio` 高阈值：
    /// 当 tick 观测值超过它才开始计进入（tactics.md §2.5 滞回双阈值）。
    pub help_off_enter_threat_ratio: f32,
    /// 「协防人被拉离走廊」退出态的 `threat_ratio` 低阈值：观测值低于它才计退出；
    /// 介于两阈值之间为保持区，维持上一稳定值。
    pub help_off_exit_threat_ratio: f32,
    /// 「弱侧真空」（`weak_side_vacant`）进入态的 `void_ratio` 高阈值。
    pub weak_vacant_enter_void_ratio: f32,
    /// 「弱侧真空」退出态的 `void_ratio` 低阈值。
    pub weak_vacant_exit_void_ratio: f32,
    /// 布尔稳定量翻转后的最小保持时间（tick）：保持计数不足时拒绝再次翻转。
    pub min_hold_ticks: u32,
}

impl Default for DefenseRules {
    /// 中性档案 = `data/defense/schemes.json` 的 `def_man_conservative`
    /// （sag=1.0 / gap=1.0 / help=0.5 / switch=0.0）。
    ///
    /// 从**同一数据源**派生而非在代码里重写四个字面量：既消除重复定义
    /// （单一事实源），也避免默认值随档案调整而静默失配。
    fn default() -> Self {
        Self::for_scheme("def_man_conservative")
            .expect("schemes.json must define the neutral scheme")
    }
}

impl DefenseRules {
    /// 方案无关的 `help_blend` 参数（schemes.json 顶层同名块）。
    ///
    /// 与 `all()` 共用同一数据源，保证「方案档案」与「锚点权重合成配方」
    /// 单一事实源。base 为合成基线，min/max 是方案无关的权重边界。
    fn help_blend() -> (f32, f32, f32, f32) {
        static HELP_BLEND: std::sync::OnceLock<(f32, f32, f32, f32)> = std::sync::OnceLock::new();
        *HELP_BLEND.get_or_init(|| {
            #[derive(serde::Deserialize)]
            struct Blend {
                hoop_weight_base: f32,
                priority_tilt_gain: f32,
                hoop_weight_min: f32,
                hoop_weight_max: f32,
            }
            #[derive(serde::Deserialize)]
            struct File {
                help_blend: Blend,
            }
            const RAW: &str = include_str!("../../../../data/defense/schemes.json");
            let parsed: File = serde_json::from_str(RAW)
                .expect("data/defense/schemes.json must be valid (charter C1 data channel)");
            (
                parsed.help_blend.hoop_weight_base,
                parsed.help_blend.priority_tilt_gain,
                parsed.help_blend.hoop_weight_min,
                parsed.help_blend.hoop_weight_max,
            )
        })
    }

    /// 按防守方案 id 返回参数档案（tactics.md §2.2 防守覆盖模型）。
    ///
    /// ## 数据来源（charter C1）
    ///
    /// 六个方案的参数写在 `data/defense/schemes.json`，经 `include_str!` 编译期内联。
    /// 这样做的理由与 `data/tactics/*.json` 相同：行为参数属数据资产，
    /// **不写在代码里**——写在代码里会以「内联行为常数」的形式绕过规则通道，
    /// 而这正是 charter C1 与 docs/protocol.md §2.1 M7 要消灭的东西。
    ///
    /// 未知 id 返回 `None`，由调用方决定降级或报错——不得静默回退，
    /// 否则又是一个「声明了但无效」的隐形参数。
    pub fn for_scheme(id: &str) -> Option<Self> {
        Self::all()
            .into_iter()
            .find(|(scheme_id, _)| *scheme_id == id)
            .map(|(_, rules)| rules)
    }

    /// 全部方案档案（顺序与 `schemes.json` 一致）。
    pub fn all() -> Vec<(&'static str, Self)> {
        #[derive(serde::Deserialize)]
        struct File {
            help_blend: HelpBlend,
            schemes: Vec<Entry>,
        }
        #[derive(serde::Deserialize)]
        struct HelpBlend {
            hoop_weight_base: f32,
            priority_tilt_gain: f32,
            hoop_weight_min: f32,
            hoop_weight_max: f32,
        }
        #[derive(serde::Deserialize)]
        struct Entry {
            id: String,
            sag_multiplier: f32,
            on_ball_gap_multiplier: f32,
            help_priority: f32,
            switch_aggressiveness: f32,
            #[serde(default)]
            screen_defense: ScreenDefenseRules,
            #[serde(default = "default_drop_low_man_rim_gain")]
            drop_low_man_rim_gain: f32,
        }

        fn default_drop_low_man_rim_gain() -> f32 {
            1.0
        }
        // 本文件比 `rules.rs` 深一层，因此路径多一个 `../`。
        const RAW: &str = include_str!("../../../../data/defense/schemes.json");
        let parsed: File = serde_json::from_str(RAW)
            .expect("data/defense/schemes.json must be valid (charter C1 data channel)");
        parsed
            .schemes
            .into_iter()
            .map(|e| {
                (
                    // 泄漏为 'static：档案在编译期内联，生命周期与程序一致。
                    Box::leak(e.id.into_boxed_str()) as &'static str,
                    Self {
                        sag_multiplier: e.sag_multiplier,
                        on_ball_gap_multiplier: e.on_ball_gap_multiplier,
                        help_priority: e.help_priority,
                        switch_aggressiveness: e.switch_aggressiveness,
                        help_hoop_weight_base: parsed.help_blend.hoop_weight_base,
                        help_priority_tilt_gain: parsed.help_blend.priority_tilt_gain,
                        help_hoop_weight_min: parsed.help_blend.hoop_weight_min,
                        help_hoop_weight_max: parsed.help_blend.hoop_weight_max,
                        screen_defense: e.screen_defense,
                        drop_low_man_rim_gain: e.drop_low_man_rim_gain,
                        potential_field: PotentialFieldRules::default(),
                    },
                )
            })
            .collect()
    }
}

impl Default for PotentialFieldRules {
    fn default() -> Self {
        // help_blend 参数从 schemes.json 数据通道读取（charter C1）：
        // 这些量是 `solve_equilibrium` 下沉锚点权重的唯一来源，必须与
        // 方案档案同源。base/min/max 各有明确角色：base 为合成基线，
        // min/max 是方案无关的权重边界。
        let (
            help_hoop_weight_base,
            help_priority_tilt_gain,
            help_hoop_weight_min,
            help_hoop_weight_max,
        ) = DefenseRules::help_blend();
        Self {
            threat_radius_ft: 15.0,
            rim_response_radius_ft: 18.0,
            k_man_base: 1.0,
            k_threat_base: 1.6,
            k_void_base: 1.2,
            sag_distance_perimeter_ft: 2.0,
            sag_distance_interior_ft: 5.5,
            help_hoop_weight_base,
            help_priority_tilt_gain,
            help_hoop_weight_min,
            help_hoop_weight_max,
            rim_buffer_ft: 3.0,
            low_man_threat_gain: 2.8,
            high_man_threat_gain: 0.15,
            default_threat_gain: 0.4,
            void_gain: 4.0,
            rim_help_threat_ratio: 0.40,
            drive_help_threat_gain: 3.0,
            x_out_void_ratio: 0.32,
            rim_help_radius_ft: 12.0,
            weak_side_lateral_ft: 12.0,
            perimeter_attribution_ft: 22.0,
            screen_detection_radius_ft: 16.0,
            switch_high_threshold: 0.6,
            switch_low_threshold: 0.2,
            switch_anchor_gap_ft: 3.0,
            switch_screener_gap_ft: 2.5,
            on_ball_gap_hoop_ratio: 0.4,
            on_ball_gap_min_ft: 2.5,
            sag_multiplier_min: 0.5,
            sag_multiplier_max: 2.5,
            total_weight_floor: 0.001,
            stamina_floor: 0.6,
            stamina_gain: 1.5,
            help_off_enter_threat_ratio: 0.45,
            help_off_exit_threat_ratio: 0.30,
            weak_vacant_enter_void_ratio: 0.38,
            weak_vacant_exit_void_ratio: 0.25,
            min_hold_ticks: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TacticalRules {
    pub initiation_distance_ratio: f32,
    pub action_duration_seconds: f32,
    pub drive_min_duration_seconds: f32,
    pub drive_distance_ratio: f32,
    pub drive_max_duration_seconds: f32,
    pub drive_speed_ratio: f32,
    pub drive_early_finish_dist_ft: f32,
    pub drive_mid_range_pullup_dist_ft: f32,
    pub drive_kickout_pass_dist_ft: f32,
    pub drive_lane_offset_ft: f32,
    /// 冲框（直接攻击篮筐）相对肘区路线的拥堵折扣（round-18）。
    ///
    /// 走廊选择原本只算拥堵成本：篮筐永远是防守最密处，于是肘区几乎
    /// 总被选中——实测 88% 的突破终点离筐 14-18 ft（>16 ft 停滞门槛），
    /// 篮下出手仅 3%（真实 25-50%）。真实篮球里冲框值得冒险：篮下
    /// ~1.3 PPP vs 中距 ~0.8。折扣按持球人 `finishing` 能力缩放
    /// （终结强的球员更应冲击篮筐），从冲框走廊的拥堵中扣除。
    pub drive_rim_attack_bias: f32,
    /// 突破终结的起跳延伸距离（ft）。
    ///
    /// 真实篮球里突破者的最后一步是起跳：身体越过防守人，出手点在
    /// 停点与篮筐之间。引擎的「停点即出手点」模型把分离投影推开的
    /// 接触约束停点直接当出手位置，导致上篮/抛投出手普遍停在
    /// 4-16 ft（实测 rim(4ft) 出手占比 1.7%，NBA 真实 25-50%）。
    /// 成功终结时出手位置向篮筐延伸此距离（不超过到筐距离）。
    pub drive_finish_extend_ft: f32,
    /// 被过防守人的恢复窗口（秒，round-19）。
    ///
    /// 让位目标若被战术层每 tick 重新指派，防守人会立刻被派回护框位，
    /// 让位形同虚设。窗口内战术层不得重派被过者——他处于「失去身位、
    /// 扑向回追位」的状态（与 GambleInterception 的 failure_recovery
    /// 语义同源）。
    pub drive_beaten_recovery_seconds: f32,
    pub drive_finish_range_ft: f32,
    pub drive_dunk_max_dist_ft: f32,
    pub drive_floater_min_dist_ft: f32,
    pub drive_dunk_min_finishing: f32,
    pub drive_dunk_max_lane_density: f32,
    pub drive_kickout_max_crowding: f32,
    pub drive_kickout_min_defender_dist_ft: f32,
    pub drive_pullup_min_crowding: f32,
    pub drive_decision_check_interval_seconds: f32,
    pub screen_distance_ratio: f32,
    pub defensive_gap_ft: f32,
    pub help_sag_ratio: f32,
    pub transition_speed_ratio: f32,
    pub carrier_speed_ratio: f32,
    pub screener_speed_ratio: f32,
    pub support_speed_ratio: f32,
    pub defender_speed_ratio: f32,
    /// 冲抢篮板与拼抢活球时的冲刺速度占球员速度上限的比例。
    ///
    /// 历史上引擎在战术层内联固定数值，现在收编进规则通道（charter C1），
    /// 可经规则档案覆盖与校准。
    pub rebound_chase_speed_ratio: f32,
    /// 接球人向冻结点收敛的速度上限倍率（round-6 审计修复）。
    ///
    /// ## 为什么需要它
    ///
    /// 此前接球人被硬编码为 20 ft/s 全速冲向冻结点，而**没有任何减速模型**。
    /// 传球飞行时长受 `max_pass_duration_seconds` 限制（默认 1.4s），但长传
    /// （40–60 ft 的跨场 outlet）在 1.4s 内实际只需 0.5–0.7s 即可到达，接球人
    /// 于是在整个（被拉长的）飞行期内持续全速前进。实测：接球人**越过**冻结点
    /// 5–12.5 ft，且越位方向几乎垂直于传球线。
    ///
    /// 后果：`PASS_CORRIDOR_REACHABLE` 在 8 seed 下报 8–17 条 Hard defect，
    /// 而 gap.md §9.5 要求接球人「向 frozen_to_pos 收敛」。
    ///
    /// 修复：按**剩余距离与制动能力**反解接近速度（减速模型），使接球人在
    /// 冻结点附近自然减速，而不是全速冲过头。
    pub receive_approach_speed_ratio: f32,
    /// 接球制动安全裕量（ft）：减速目标点提前于冻结点该距离，
    /// 替球员的身体半径与单 tick 离散误差留余量。
    pub receive_stop_margin_ft: f32,
    /// 接球逼近速度下限倍率：进入安全裕量内仍保留的逼近速度，
    /// 避免因 `v=sqrt(2as)→0` 而停在冻结点之外。
    pub receive_min_approach_speed_ratio: f32,
    /// 领传提前量增益（round-7）：`lead = receiver_velocity × flight × gain`。
    /// 1.0 = 完全按接球人当前速度外推；<1 表示他会在飞行中减速。
    pub pass_lead_gain: f32,
    /// 领传提前量上限（ft）：防止高速接球人被外推到不可达或贴边位置。
    pub pass_lead_max_ft: f32,
    /// APF 动态排斥场有效感应距离（呎）
    pub apf_repulsion_radius_ft: f32,
    /// APF 队友间空间拉开斥力系数（ft/s^2）
    pub apf_teammate_repulsion_accel: f32,
    /// APF 对手障碍斥力系数（ft/s^2）
    pub apf_opponent_repulsion_accel: f32,
    /// 攻防转换退守判定距离与半场长度之比（进攻人未越过此边界前防守全员退回前场阵地）
    pub transition_defense_threshold_ratio: f32,
    /// 转换推进期间前场空间拉开速度系数（全速冲刺拉开）
    pub transition_sprint_ratio: f32,
    /// 转换进攻篮下终结窗口（秒，G6a 链 3）：回合前段防守未落位，
    /// 突破攻框的效用加成只在此窗口内生效，避免把阵地战的攻框比例
    /// 一并抬高（此前校准迭代 4 的教训：强抬攻框砸穿 3P% 带）。
    pub transition_finish_window_seconds: f32,
    /// 转换期篮下终结的效用加成基准。
    pub transition_finish_bonus: f32,
    /// 掩护人身位卡位距离（ft）：持球人与掩护人距离小于此值时判定掩护墙确立
    pub screen_hold_separation_ft: f32,
    /// 掩护人顺下触发的持球人纵向摆脱距离（ft）：持球人越过掩护人此距离后触发顺下
    pub screen_roll_separation_ft: f32,
    /// 弱侧背切的切入深度比例（G6a 链 1）：翼位到篮筐向量按此比例乘以
    /// 进攻进度，决定无球切入的终点深度。
    pub backdoor_cut_depth_ratio: f32,
    /// 下沉禁区的深度比例（G6a 链 4）：底角/翼位沿篮筐方向按此比例
    /// 乘以进攻进度，决定外线人下沉到篮下区域的落位深度。
    pub dip_to_rim_depth_ratio: f32,
    /// 沉退防守中锋纵深距筐距离（ft）
    pub drop_coverage_depth_ft: f32,
    /// 防守方案对比赛的影响系数（round-6 审计修复）。
    ///
    /// ## 为什么需要这一组参数
    ///
    /// `DefensiveTactic` 此前对比赛结果**零影响**：它只被用于生成展示字符串，
    /// 物理层与决策管线从不消费。实测 6 种方案各跑一场全场模拟，逐字节相同
    /// （ticks=84029、score=96081、行为哈希全为 `0x3ba37e7fa9e5ec7d`）。
    ///
    /// 修复方式是让方案通过**规则通道**（而非代码分支，charter C1/C3）
    /// 影响防守人的目标点生成：
    /// - `sag_multiplier`：无球防守人的协防深度倍率（联防收缩、紧逼外扩）；
    /// - `on_ball_gap_multiplier`：领防人距对位人的间隔倍率；
    /// - `help_priority`：弱侧协防权重（越大越倾向于放空外线收缩护框）。
    ///
    /// 三个方案各自实例见 `DefensiveSchemeSpec`。
    pub defense: DefenseRules,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DecisionRules {
    pub shoot_base: f32,
    /// 早出手的机会成本（DecisionRules 通道）。
    ///
    /// 进攻时间充足时出手意味着放弃可能更好的后续机会；此前效用只随
    /// shot clock 递减，没有时间价值项，导致 46% 出手发生在 8 秒内
    /// （真实约 15%），每回合传球仅 1.3 次（真实 ~3.5）。
    pub early_shot_penalty: f32,
    /// 出手效用中空位加成的量级：`open_bonus = contest_free_score × 本值`。
    ///
    /// 真实篮球里「空位」的价值随出手距离的变长而变高，但空间拉扯
    /// （五外站位）会让三分线外常年空位 —— 实测旧值 0.5 时三分线外
    /// 出手占 74%（真实 40%），空位加成压倒了命中率的距离衰减。
    pub shot_openness_weight: f32,
    /// 出手效用中距离衰减的斜率：`distance_factor = 1 − dist / 参考`。
    /// 与参考距离共同决定「篮下 vs 三分」的效用落差，必须能覆盖
    /// 空位加成的量级，否则外线空位永远压倒篮下攻击。
    pub shot_distance_slope: f32,
    /// `Advance`（后场推进）在 8 秒规则下的效用放大倍数。
    ///
    /// 紧迫度 = backcourt_elapsed / backcourt_seconds，效用 =
    /// `dwell_base × (1 + urgency × 该倍数)`，使推进能压过原地动作。
    pub advance_urgency_boost: f32,
    /// `Advance` 目标越过中线的余量比例（× 场地长度）。
    /// 越过少许可避免卡在中线上反复触发后场计时。
    pub advance_overshoot_ratio: f32,
    pub pass_base: f32,
    pub dwell_base: f32,
    /// Strength of stamina's influence on action utility.
    pub stamina_sensitivity: f32,
    /// Softmax temperature; lower values make choices more deterministic.
    pub temperature: f32,
    /// Receiver lead time used when creating pass targets.
    pub pass_lead_time_seconds: f32,
    /// Utility penalty applied to risk reported by constraints.
    pub risk_aversion: f32,
    /// Influence of player tendencies on utility.
    pub tendency_weight: f32,
    /// Influence of team style traits on utility.
    pub team_style_weight: f32,
    /// 三分投篮效用折损系数（反映三分球相对近距离攻框的期望难度）。
    pub three_point_utility_multiplier: f32,
    /// 突破攻框基础效用权重。
    pub drive_base: f32,
    /// 低位背身单打的基础效用。
    ///
    /// ## 为何必须独立于 `drive_base`（D27 实测）
    ///
    /// 原式用 `drive_base × finishing × (1 - dist/15)`。`PostUp` 只在距篮
    /// 18 ft 内生成，而 `Drive` 在全场都生成，两者却共用同一个基础系数：
    /// 在距篮 8 ft 处 Drive 得 ≈0.65（其距离因子按全场宽度归一），
    /// PostUp 只得 ≈0.20，因此 PostUp 进入候选 15 次、被选中 **0** 次
    /// （seed 42、20000 tick 的决策追踪实测）。
    /// 低位背身是独立的动作族，需要自己的量级与错位收益项。
    pub post_up_base: f32,
    /// 低位背身的错位收益权重：以背身者的 `strength` 优势对抗
    /// 对位防守人的 `effective_post_defense_physicality`。
    /// 这是低位背身的战术意义所在（大打小、错位惩罚），
    /// 也是它与 `Drive` 的结构差异：Drive 看的是道路空旷，PostUp 看的是对位强弱。
    pub post_up_mismatch_weight: f32,
    /// 持球姿态（面框/背身）技术选择的评估权重：
    /// 背身亲和度 = strength×w + shooting_near×w + finishing×w
    /// − ball_handling×w − shooting_three×w，再乘接球区域因子。
    pub orientation_strength_weight: f32,
    pub orientation_near_weight: f32,
    pub orientation_finishing_weight: f32,
    pub orientation_handling_penalty: f32,
    pub orientation_three_penalty: f32,
    /// 背身成立的接球区域：距篮不超过 `post_zone_ft` 时区域因子满 1，
    /// 到 `zone_fade_ft` 线性衰减到 0（与 PostUp 候选生成的 18ft 门对齐）。
    pub orientation_post_zone_ft: f32,
    pub orientation_zone_fade_ft: f32,
    /// 背身亲和度超过该阈值时选择背身，否则面框。
    pub orientation_threshold: f32,
    /// 姿态对后续动作效用的耦合：已背身时 PostUp 增益比例；面框下选
    /// PostUp 的转身成本；背身下 Drive 的转身启动成本；背身下外线
    /// 拔起投篮（三分/中距 PullUp）的惩罚。
    pub orientation_post_up_match_bonus: f32,
    pub orientation_face_up_turn_cost: f32,
    pub orientation_back_drive_penalty: f32,
    pub orientation_back_perimeter_penalty: f32,
    /// Play 候选偏好与软抑制的效用缩放系数。
    ///
    /// 偏好加到动作效用上，软抑制从动作效用中扣除；零值关闭两种调整。默认单位倍率
    /// 保留 Play 档案里声明的效用增量。
    pub play_effect_weight: f32,
    /// 最大持球组织衰减比例（随着进攻时间消耗，Dwell 价值衰减的最大幅度）。
    pub dwell_decay_max: f32,
    /// 进攻迫近时受干扰惩罚的保底系数。
    pub contested_patience_floor: f32,
    /// 24 秒倒计时迫近时的投篮效用加成。
    pub urgency_shoot_boost: f32,
    /// 24 秒倒计时迫近时的突破效用加成。
    pub urgency_drive_boost: f32,
    /// 24 秒倒计时迫近时的传球效用惩罚。
    pub urgency_pass_penalty: f32,
    /// 24 秒倒计时迫近时的持球组织惩罚。
    pub urgency_dwell_penalty: f32,
    /// 传球效用距离衰减起点（ft）：低于此距离不施加衰减（round-15 已接线
    /// 到 `pipeline.rs` 的 Pass 效用）。
    pub pass_distance_free_ft: f32,
    /// 传球效用距离衰减参考距离（ft）：超过 free 距离后线性衰减到此处的参考强度。
    pub pass_distance_decay_reference_ft: f32,
    /// 传球衰减最大比例（衰减因子下限 = 1 - 此值）。
    pub pass_distance_max_decay: f32,
    /// 篮下接球攻框加成（G6a 链 1 最后一环）：接球人处于篮下
    /// （`ShotZone::Rim`，距篮小于 `RIM_ZONE_MAX_DIST_FT`）时传球效用按此值乘以接球人 finishing
    /// 技能叠加，使切入者的篮下空位成为优先传球终点。
    pub rim_catch_bonus: f32,
    /// 防守自主体基础效用乘数（由规则层提供基准，严禁决策层硬编码）。
    pub def_steal_gamble_base: f32,
    pub def_steal_risk_penalty: f32,
    pub def_rim_help_base: f32,
    pub def_corner_threat_weight: f32,
    pub def_drop_contain_base: f32,
    pub def_hedge_contain_base: f32,
    pub def_switch_base: f32,
}

impl Default for DecisionRules {
    fn default() -> Self {
        Self {
            shoot_base: 0.60,
            early_shot_penalty: 0.35,
            // 出手效用形状（G6a 三次校准迭代的结论，负结果全记录）：
            // 原值 (0.5, 0.7)；激进 (0.25, 1.2) → 3P% 23.5 跳带；
            // 折中 (0.45, 0.8) → 29.8 仍跳带；微调 (0.5, 0.75) → 25.9 更糟。
            // 命中率对效用形状高度敏感且方向非单调 —— 分布失真的根因是
            // 攻框行为链缺失（见 gap.md G6a），效用参数无法闭合，
            // 故恢复 v66 原值，禁止再盲调这两个系数。
            shot_openness_weight: 0.50,
            shot_distance_slope: 0.70,
            advance_urgency_boost: 3.0,
            advance_overshoot_ratio: 0.075,
            // round-16 调整（A/B 证据 §17.5）：0.82 → 1.15。
            // 0.82 == dwell_base 使传球与原地持球打平，n（传球/回合）
            // 卡在 1.45（真实 3.0）；1.15 使 n=2.46、失败率 15.0%→10.7%。
            // 代价 e 0.180→0.231，需配合拦截率标定（intercept_* 斜率）
            // 联合收敛到 e≈0.145。
            pass_base: 1.15,
            dwell_base: 0.82,
            stamina_sensitivity: 0.5,
            temperature: 0.12,
            pass_lead_time_seconds: 0.65,
            risk_aversion: 0.8,
            tendency_weight: 0.35,
            team_style_weight: 0.25,
            three_point_utility_multiplier: 0.68,
            drive_base: 0.85,
            // 低位背身的量级：与 Drive 同阶，使两者在距篮较近时真正竞争。
            post_up_base: 2.2,
            post_up_mismatch_weight: 0.9,
            // 持球姿态评估权重：内线技术（strength/near/finishing）推背身，
            // 外线技术（handling/three）推面框；区域门与 PostUp 候选一致。
            orientation_strength_weight: 0.35,
            orientation_near_weight: 0.25,
            orientation_finishing_weight: 0.20,
            orientation_handling_penalty: 0.25,
            orientation_three_penalty: 0.20,
            orientation_post_zone_ft: 12.0,
            orientation_zone_fade_ft: 18.0,
            orientation_threshold: 0.45,
            orientation_post_up_match_bonus: 0.35,
            orientation_face_up_turn_cost: 0.15,
            orientation_back_drive_penalty: 0.20,
            orientation_back_perimeter_penalty: 0.45,
            play_effect_weight: f32::from(1u8),
            dwell_decay_max: 0.85,
            contested_patience_floor: 0.25,
            // D3.4 校准：紧逼加成原值（shoot 0.25 / drive 0.10 / dwell -0.20）
            // 量级远小于 pass_base=0.82，无法在倒计时阶段真正压低组织/传球、
            // 抬高出手。实测提升后单场违例从 53 降至 ~70/3 场均值，TO% 由
            // 60% 降至 46%。三个系数仍全部走 GameRules 通道。
            urgency_shoot_boost: 1.20,
            urgency_drive_boost: 0.60,
            urgency_pass_penalty: 0.30,
            urgency_dwell_penalty: 0.80,
            pass_distance_free_ft: 20.0,
            pass_distance_decay_reference_ft: 55.0,
            pass_distance_max_decay: 0.75,
            rim_catch_bonus: 0.8,
            def_steal_gamble_base: 0.70,
            def_steal_risk_penalty: 0.85,
            def_rim_help_base: 0.80,
            def_corner_threat_weight: 0.90,
            def_drop_contain_base: 0.75,
            def_hedge_contain_base: 0.72,
            def_switch_base: 0.65,
        }
    }
}

impl DecisionRules {
    pub fn validate(&self) -> Result<(), String> {
        // 零是非负效用权重的结构性下界。
        let zero = f32::from(0u8);
        let values = [
            self.shoot_base,
            self.early_shot_penalty,
            self.advance_urgency_boost,
            self.advance_overshoot_ratio,
            self.pass_base,
            self.dwell_base,
            self.stamina_sensitivity,
            self.temperature,
            self.pass_lead_time_seconds,
            self.risk_aversion,
            self.tendency_weight,
            self.team_style_weight,
            self.three_point_utility_multiplier,
            self.drive_base,
            self.post_up_base,
            self.post_up_mismatch_weight,
            self.orientation_strength_weight,
            self.orientation_near_weight,
            self.orientation_finishing_weight,
            self.orientation_handling_penalty,
            self.orientation_three_penalty,
            self.orientation_post_zone_ft,
            self.orientation_zone_fade_ft,
            self.orientation_threshold,
            self.orientation_post_up_match_bonus,
            self.orientation_face_up_turn_cost,
            self.orientation_back_drive_penalty,
            self.orientation_back_perimeter_penalty,
            self.play_effect_weight,
            self.dwell_decay_max,
            self.contested_patience_floor,
            self.urgency_shoot_boost,
            self.urgency_drive_boost,
            self.urgency_pass_penalty,
            self.urgency_dwell_penalty,
            self.pass_distance_free_ft,
            self.pass_distance_decay_reference_ft,
            self.pass_distance_max_decay,
            self.def_steal_gamble_base,
            self.def_steal_risk_penalty,
            self.def_rim_help_base,
            self.def_corner_threat_weight,
            self.def_drop_contain_base,
            self.def_hedge_contain_base,
            self.def_switch_base,
        ];
        if values.iter().any(|value| !value.is_finite())
            || self.shoot_base < zero
            || self.pass_base < zero
            || self.dwell_base < zero
            || self.play_effect_weight < zero
            || !(0.0..=1.0).contains(&self.stamina_sensitivity)
            || self.temperature <= 0.0
            || self.pass_lead_time_seconds < 0.0
            || self.risk_aversion < zero
            || self.tendency_weight < zero
            || self.team_style_weight < zero
            || self.post_up_base < zero
            || self.post_up_mismatch_weight < zero
            || self.orientation_strength_weight < zero
            || self.orientation_near_weight < zero
            || self.orientation_finishing_weight < zero
            || self.orientation_handling_penalty < zero
            || self.orientation_three_penalty < zero
            || self.orientation_post_zone_ft <= zero
            || self.orientation_zone_fade_ft <= self.orientation_post_zone_ft
            || !(zero..=f32::from(1u8)).contains(&self.orientation_threshold)
            || self.orientation_post_up_match_bonus < zero
            || self.orientation_face_up_turn_cost < zero
            || self.orientation_back_drive_penalty < zero
            || self.orientation_back_perimeter_penalty < zero
            || self.pass_distance_free_ft < zero
            || self.pass_distance_decay_reference_ft <= self.pass_distance_free_ft
            || !(0.0..=1.0).contains(&self.pass_distance_max_decay)
            || self.def_steal_gamble_base < zero
            || self.def_steal_risk_penalty < zero
            || self.def_rim_help_base < zero
            || self.def_corner_threat_weight < zero
            || self.def_drop_contain_base < zero
            || self.def_hedge_contain_base < zero
            || self.def_switch_base < zero
        {
            return Err("decision policy contains an invalid value".to_string());
        }
        Ok(())
    }
}

/// 轮换与换人调度参数（tactics.md §2.3.3 / gap.md G6）。
///
/// ## 裁决时机
///
/// 换人意图统一在死球窗口评估（发球准备、罚球间隙、节末），活球中
/// 产生的枯竭信号先被登记，进入下一个死球窗口时才被消费——真实篮球
/// 里教练无法在活球中换人。每条规则都有对应的 `SubstitutionReason`。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RotationRules {
    /// 体力枯竭换人阈值（归一化体力，0..1）：低于此值的在场球员进入
    /// 换人候选。`stamina_exhausted_threshold`（0.35）决定士气进入
    /// `Exhausted`，此阈值应高于它——枯竭到影响心态之前就该休息。
    pub fatigue_substitution_threshold: f32,
    /// 垃圾时间换人的分差阈值（分）：分差绝对值超过此值时，双方进入
    /// 垃圾时间轮换（替补优先登场）。
    pub garbage_time_score_margin: i32,
    /// 垃圾时间换人的最少剩余比赛时间（秒）：两者同时满足才触发。
    pub garbage_time_remaining_seconds: f32,
    /// 换下者在再次登场前必须休息的最短时间（秒）：防止抖动换人
    /// （同一对球员来回切换）。
    pub min_rest_seconds: f32,
    /// 同一死球窗口内每队最多换人次数：真实篮球的换人由教练发出，
    /// 一个停表窗口内同时换下全部五人极少见。
    pub max_substitutions_per_window: u32,
    /// 领先方垃圾时间的体力换人阈值放宽量：垃圾时间里领先方更愿意
    /// 让主力休息，枯竭阈值按此值上浮。
    pub garbage_time_fatigue_relief: f32,
}

impl Default for RotationRules {
    fn default() -> Self {
        Self {
            fatigue_substitution_threshold: 0.42,
            garbage_time_score_margin: 20,
            garbage_time_remaining_seconds: 300.0,
            min_rest_seconds: 240.0,
            max_substitutions_per_window: 2,
            garbage_time_fatigue_relief: 0.15,
        }
    }
}

impl RotationRules {
    pub fn validate(&self) -> Result<(), String> {
        let fatigue = self.fatigue_substitution_threshold;
        if !(0.0..=1.0).contains(&fatigue)
            || self.garbage_time_score_margin < 0
            || self.garbage_time_remaining_seconds < 0.0
            || self.min_rest_seconds < 0.0
            || self.max_substitutions_per_window == 0
            || self.garbage_time_fatigue_relief < 0.0
        {
            return Err("rotation policy contains an invalid value".to_string());
        }
        Ok(())
    }
}
