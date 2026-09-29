//! 防守体系策略组：防守方案倍率、掩护防守与多体势能场参数。
//!
//! 从 `policies.rs` 按职责划分（D29）：这三个结构同属「防守跑位生成器」
//! 的参数族，与进攻/裁决策略分开维护。`policies.rs` 重新导出本模块全部
//! 名称，`rules.rs` 的聚合路径保持不变。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnBallScreenDefenseStrategy {
    #[default]
    StandardContest,
    FightThrough,
    GoUnder,
    Switch,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenDefenseRules {
    pub drop_depth_ft: f32,
    pub hedge_distance_ft: f32,
    pub switch_trigger_distance_ft: f32,
    pub recover_timeout_seconds: f32,
    pub strategy: OnBallScreenDefenseStrategy,
}

impl Default for ScreenDefenseRules {
    fn default() -> Self {
        Self {
            drop_depth_ft: 0.0,
            hedge_distance_ft: 0.0,
            switch_trigger_distance_ft: 5.0,
            recover_timeout_seconds: 1.2,
            strategy: OnBallScreenDefenseStrategy::StandardContest,
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
    /// 协防倾向对护筐引力 `w_threat` 的调制地板：
    /// `w_threat × (help_tendency_floor + help_aggressiveness × 本值)`。
    ///
    /// 协防倾向是风格不是能力（attributes.md §2.6 项 9）：它决定防守人
    /// 多早离开自己对位去护筐，感知威胁的能力仍由 `decision_iq` 与
    /// `defense_interior` 决定。地板值保证最保守的防守人仍会协防。
    pub help_tendency_floor: f32,
    /// 协防倾向的调制跨度（见 `help_tendency_floor`）。
    pub help_tendency_span: f32,
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
            const RAW: &str = include_str!("../../../../../data/defense/schemes.json");
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
        }
        // 本文件比 `rules.rs` 深两层，因此路径多两个 `../`。
        const RAW: &str = include_str!("../../../../../data/defense/schemes.json");
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
            help_tendency_floor: 0.6,
            help_tendency_span: 0.8,
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
