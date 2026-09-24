//! 连续受限势能场动力学与空间涌现系统 (Continuous Potential Field Dynamics)
//!
//! 防守战术跑位与轮转由全场球员与空间拓扑构成的多体势能场求导，求解局部
//! 能量极小值平衡点（Equilibrium Point）并由主导场力自然涌现出：
//!
//! 1. 弱侧 Low-man 威胁引力主导的下沉护筐 (ROTATE_RIM_HELP)
//! 2. 弱侧 High-man 空间真空主导的轮转补位 (X_OUT_CLOSEOUT)
//! 3. 稳态球-人-筐三角协防 (HELP_SIDE_SHELL)
//!
//! ## 系数的唯一来源（charter C1）
//!
//! 本模块**不保存任何行为系数**：五个势能分量系数、两个下沉距离、锚点权重、
//! 缓冲带与三个引力倍率、两个涌现阈值全部从 `PotentialFieldRules` 读取。
//! 这些量过去以字段默认值与函数内字面量两种形式散在这里，既无法用 `--rules`
//! 覆盖，也逃过常数守卫（该文件当时不在预算名单内）。
//!
//! ## 场输出的滞回稳定通道（tactics.md §2.5，plan_play.md #21）
//!
//! `threat_ratio` / `void_ratio` 逐 tick 抖动，直接作开关型判定会引发谓词与
//! 选板震荡。`DefenseHysteresisState` 维护逐防守人的双阈值滞回 + 最小保持时间，
//! 输出「上一稳定态」；阈值参数同样全部从 `PotentialFieldRules` 读取。
//! 未经滞回的场量不得进入任何开关型判定（ADR-019 裁定第 4 条）。

use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_domain::PotentialFieldRules;

/// 势能场平衡点求解结果与涌现动作
#[derive(Debug, Clone)]
pub struct PotentialFieldVector {
    pub position: Vec2,
    pub drive: Vec2,
    pub pressure: f32,
}

/// 主动跑动场分量（护筐引力、真空吸力）的体能衰减系数。
///
/// ## 裁定：`w_man` 不衰减的理由
///
/// `w_man` 是对位牵引弹簧：目标点由对位人当前位置派生，防守人跟随的是
/// 对位人的移动，不是自发跑动；而护筐引力与真空吸力要求防守人主动离位
/// 冲向威胁/真空区域，跑动距离才受体能约束。因此衰减只作用于后两者。
/// 系数恒在 `[stamina_floor, 1]` 内：stamina 越低越小，stamina = 1 时为 1
/// （行为与无衰减逐位一致），stamina = 0 时为 `stamina_floor`。
///
/// 输入 `stamina` 为归一化体能（0..1，1 = 满体能），越界值就地 panic
/// （fast-fail：体能口径错误必须在源头暴露，不允许静默钳制）。
pub fn stamina_multiplier(stamina: f32, config: &PotentialFieldRules) -> f32 {
    assert!(
        (0.0..=1.0).contains(&stamina),
        "stamina must be normalized in [0, 1], got {stamina}"
    );
    let floor = config.stamina_floor;
    let gain = config.stamina_gain;
    floor + (1.0 - floor) * stamina.powf(gain)
}

/// 下沉锚点方向「朝篮筐」的权重合成（help_blend，plan_play.md #22）。
///
/// `hoop_weight = clamp(base + tilt_gain × help_priority, min, max)`：
/// base/min/max 来自方案无关的合成配方（`PotentialFieldRules`，数据源
/// schemes.json 顶层 help_blend 块），`help_priority` 是该防守人所在
/// 防守方案的同名字段。协防优先级高的方案锚点重心向篮筐偏移，低的
/// 向外线持球人偏移；clamp 保证协防不完全脱离篮筐方向，也不退化为
/// 纯护框。`carrier_weight = 1 - hoop_weight`，权重和恒为 1。
///
/// 输入 `help_priority` 必须在归一化区间 [0, 1] 内，越界就地 panic
/// （fast-fail：方案档案口径错误必须在源头暴露，不允许静默钳制）。
pub fn help_anchor_hoop_weight(help_priority: f32, config: &PotentialFieldRules) -> f32 {
    assert!(
        (0.0..=1.0).contains(&help_priority),
        "help_priority must be normalized in [0, 1], got {help_priority}"
    );
    (config.help_hoop_weight_base + config.help_priority_tilt_gain * help_priority)
        .clamp(config.help_hoop_weight_min, config.help_hoop_weight_max)
}

#[derive(Debug, Clone)]
pub struct EmergentDefenseTarget {
    /// 势能极小值平衡位置
    pub target_pos: Vec2,
    /// 涌现出的宏观战术动作标签
    pub action: &'static str,
    /// 战术槽位名称
    pub slot: &'static str,
    /// 护筐威胁势能占比 (0.0 ~ 1.0)
    pub threat_ratio: f32,
    /// 空间真空势能占比 (0.0 ~ 1.0)
    pub void_ratio: f32,
    /// 平衡点上的合力方向，用于解释防守人正在被哪一个势能源牵引。
    pub drive: Vec2,
}

/// 连续势能场求解器。
///
/// 构造时只持有系数；求解时不读任何模块级常量。
pub struct DefensePotentialFieldSolver {
    pub config: PotentialFieldRules,
}

impl DefensePotentialFieldSolver {
    pub fn new(config: PotentialFieldRules) -> Self {
        Self { config }
    }

    /// 在指定位置求出连续势能场的合力方向与归一化压力。
    ///
    /// 这个采样接口复用与 `solve_equilibrium` 相同的规则参数和几何输入，
    /// 前端显示的箭头因此来自引擎的实际势能模型。
    /// 求解场上指定防守人在连续势能场中的平衡位置与涌现动作
    ///
    /// # 参数
    /// - `carrier_pos`: 持球人位置
    /// - `hoop_pos`: 进攻篮筐位置
    /// - `off_positions`: 场上 5 名进攻球员当前位置
    /// - `assigned_off_idx`: 该防守人对位的进攻球员索引
    /// - `carrier_idx`: 持球人索引
    /// - `rules`: 比赛规则配置（提供本模块系数的唯一来源）
    /// - `stamina`: 该防守人的归一化体能（0..1，1 = 满体能）；
    ///   仅衰减护筐引力与真空吸力两个主动跑动分量（对位牵引不衰，
    ///   见 `stamina_multiplier` 的裁定）。满体能时衰减系数恒为 1，
    ///   输出与无衰减通道逐位一致。
    ///
    /// 几何 + 规则 + 体能的物理求解参数列表（`&self` 之外 7 个），
    /// 各参数彼此独立，无需引入参数结构体。
    #[allow(clippy::too_many_arguments)]
    pub fn solve_equilibrium(
        &self,
        carrier_pos: Vec2,
        hoop_pos: Vec2,
        off_positions: &[Vec2],
        assigned_off_idx: usize,
        carrier_idx: usize,
        rules: &GameRules,
        stamina: f32,
        drive_active: bool,
    ) -> EmergentDefenseTarget {
        // 系数从规则档案读取（`DefenseRules` 经防守方案实例化，`potential_field`
        // 随方案一起进入规则通道）。
        let config = &rules.tactics.defense.potential_field;
        let assigned_pos = off_positions
            .get(assigned_off_idx)
            .copied()
            .unwrap_or(carrier_pos);

        // 1. 持球人到篮筐的欧几里得距离与全场连续突破威胁度 (0.0 ~ 1.0)
        let carrier_dist_to_hoop = (carrier_pos - hoop_pos).length();
        let r_threat = config.threat_radius_ft;
        // Cauchy-Lorentz 形式连续非线性势能核：深入内线时平滑激增，外线游弋时平滑衰减
        let global_threat = 1.0 / (1.0 + (carrier_dist_to_hoop / r_threat).powi(2));

        // 2. 该防守人所看管对位人距离篮筐的拓扑距离，决定其局域护筐响应权重
        let assigned_dist_to_hoop = (assigned_pos - hoop_pos).length();
        let r_rim = config.rim_response_radius_ft;
        let local_rim_proximity = 1.0 / (1.0 + (assigned_dist_to_hoop / r_rim).powi(2));

        // 3. 寻找弱侧进攻球员：
        // 弱侧定义：远离持球人一侧（跨越球场中轴线 mid_y，或横向差值超过阈值）
        let mid_y = hoop_pos.y;
        let ref_carrier_y = if (carrier_pos.y - mid_y).abs() < 1.0 {
            // 居中突破时，将 y 坐标较低（底角射手一侧）视为主防弱侧轮转区
            mid_y + 1.0
        } else {
            carrier_pos.y
        };

        let mut weak_side_teammates = Vec::new();
        for (i, p) in off_positions.iter().enumerate() {
            if i != carrier_idx {
                let is_opposite = (ref_carrier_y - mid_y) * (p.y - mid_y) < 0.0
                    || (carrier_pos.y - p.y).abs() >= config.weak_side_lateral_ft;
                if is_opposite {
                    weak_side_teammates.push((i, *p, (*p - hoop_pos).length()));
                }
            }
        }
        // 按距篮筐距离升序排序：首位为低位 Low-man，次位为高位 High-man
        weak_side_teammates
            .sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

        let is_low_man = weak_side_teammates.first().map(|t| t.0) == Some(assigned_off_idx);
        let is_high_man = weak_side_teammates.get(1).map(|t| t.0) == Some(assigned_off_idx);

        // 4. 势能分量一：对位牵引势能 (Assignment Spring)
        // 外线射手贴防阻截出手，内线球员深度向篮筐收缩护筐；受方案下沉系数 sag_multiplier 调控
        let sag_mult = rules
            .tactics
            .defense
            .sag_multiplier
            .clamp(config.sag_multiplier_min, config.sag_multiplier_max);
        let dist_assigned_to_hoop = (assigned_pos - hoop_pos).length();
        let base_sag = if dist_assigned_to_hoop > config.perimeter_attribution_ft {
            config.sag_distance_perimeter_ft
        } else {
            config.sag_distance_interior_ft
        };
        let sag_distance = base_sag * sag_mult;
        let to_hoop = (hoop_pos - assigned_pos).normalize_or_zero();
        let to_carrier = (carrier_pos - assigned_pos).normalize_or_zero();
        // 下沉锚点方向经防守方案的协防权重混合（help_blend）调制，
        // 合成配方与 clamp 语义见 `help_anchor_hoop_weight`。
        let hoop_weight = help_anchor_hoop_weight(rules.tactics.defense.help_priority, config);
        let carrier_weight = 1.0 - hoop_weight;
        let shell_anchor = assigned_pos
            + (to_hoop * hoop_weight + to_carrier * carrier_weight).normalize_or_zero()
                * sag_distance;
        let w_man = config.k_man_base;

        // 体能衰减（#24）：护筐引力与真空吸力要求防守人主动跑动，随体能衰减；
        // 对位牵引是被动跟随，不衰。衰减系数在满体能时恒为 1（行为中性）。
        let stamina_mult = stamina_multiplier(stamina, config);

        // 5. 势能分量二：禁区威胁引力势能 (Rim Protection Attraction)
        // 威胁中心点位于持球人与篮筐之间的禁区缓冲带；受 sag_multiplier 协同下沉
        let rim_target =
            hoop_pos + (carrier_pos - hoop_pos).normalize_or_zero() * config.rim_buffer_ft;
        // 只有弱侧低位人拥有高耦合的护筐引力；高位人需保留在外线，防备三分。
        // D26：突破发生时（Drive 球态），弱侧防守人（非领防）获得突破激励
        // 的护筐引力增益（drive_help_threat_gain）——NBA 协防铁律「突破必
        // 收缩」。增益取「原角色增益」与「突破增益」的较大者，使原本被
        // 压低的 default/high 弱侧人的 threat_ratio 能越过 rim_help_threat_ratio。
        let role_gain = if is_low_man {
            config.low_man_threat_gain
        } else if is_high_man {
            config.high_man_threat_gain
        } else {
            config.default_threat_gain
        };
        let effective_gain = if drive_active {
            role_gain.max(config.drive_help_threat_gain)
        } else {
            role_gain
        };
        let sag_factor = if is_high_man { 1.0 } else { sag_mult };
        let w_threat = stamina_mult
            * config.k_threat_base
            * global_threat
            * local_rim_proximity
            * effective_gain
            * sag_factor;

        // 6. 势能分量三：弱侧外线空间真空吸力 (Voronoi Space Deficit Pull)
        // 物理因果律：只有当持球人突破深入且弱侧低位人 (Low-man) 产生显著下沉护筐时，
        // 底角射手才会真正暴露为空位真空，High-man 才会感知到空间覆盖赤字。
        let low_man_pos = weak_side_teammates
            .first()
            .map(|t| t.1)
            .unwrap_or(assigned_pos);
        let low_man_dist_to_hoop = (low_man_pos - hoop_pos).length();
        let low_man_sink_proximity = 1.0 / (1.0 + (low_man_dist_to_hoop / r_rim).powi(2));
        // Low-man 真实的下沉激发度：由突破深入威胁与低位几何接近度非线性自发涌现
        let low_man_excitation = (global_threat * low_man_sink_proximity).powi(2);

        let mut w_void = 0.0_f32;
        let mut void_target = assigned_pos;
        if is_high_man && weak_side_teammates.len() >= 2 {
            let corner_pos = weak_side_teammates[0].1;
            let wing_pos = weak_side_teammates[1].1;
            // 空间真空目标点为弱侧两名射手连线的几何重心
            void_target = (corner_pos + wing_pos) * 0.5;
            // 真空吸力强度严格由 Low-man 的下沉激发度调制，外线无威胁时真空吸力自然归零
            w_void = stamina_mult * config.k_void_base * low_man_excitation * config.void_gain;
        }

        // 7. 多体势能场平衡点闭式求解 (Analytical Force Equilibrium)
        // \sum F = w_man * (shell_anchor - x) + w_threat * (rim_target - x) + w_void * (void_target - x) = 0
        let total_weight = (w_man + w_threat + w_void).max(config.total_weight_floor);
        let equilibrium_pos =
            (shell_anchor * w_man + rim_target * w_threat + void_target * w_void) / total_weight;

        // 8. 场力主导性分析与宏观战术动作涌现
        let threat_ratio = w_threat / total_weight;
        let void_ratio = w_void / total_weight;

        let (action, slot) = if threat_ratio > config.rim_help_threat_ratio
            && (equilibrium_pos - hoop_pos).length() < config.rim_help_radius_ft
        {
            ("ROTATE_RIM_HELP", "LowManRimHelp")
        } else if void_ratio > config.x_out_void_ratio {
            ("X_OUT_CLOSEOUT", "HighManXOut")
        } else {
            ("HELP_SIDE_SHELL", "HelpAnchor")
        };

        EmergentDefenseTarget {
            target_pos: equilibrium_pos,
            action,
            slot,
            threat_ratio,
            void_ratio,
            drive: equilibrium_pos - assigned_pos,
        }
    }
}

/// 单 tick 的场量原始观测（未经滞回）。
///
/// 全部字段从 [`DefensePotentialFieldSolver::solve_equilibrium`] 的既有输出派生，
/// 不重算几何。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldObservation {
    /// 护筐威胁势能占比（对应 `EmergentDefenseTarget::threat_ratio`）。
    pub threat_ratio: f32,
    /// 空间真空势能占比（对应 `EmergentDefenseTarget::void_ratio`）。
    pub void_ratio: f32,
}

/// 单个布尔稳定量的滞回状态机（双阈值 + 最小保持时间）。
///
/// - `value`：上一稳定态，下游读到的永远是它，从不当 tick 原始值；
/// - `pending_ticks`：连续满足翻转条件的 tick 计数；
/// - `hold_ticks`：距离上次翻转已过的 tick 数（保持期内拒绝反翻）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BooleanHysteresis {
    value: bool,
    pending_ticks: u32,
    hold_ticks: u32,
}

impl BooleanHysteresis {
    fn new(value: bool) -> Self {
        Self {
            value,
            pending_ticks: 0,
            hold_ticks: 0,
        }
    }

    /// 推进一帧：`enter` = 观测值越过进入阈值，`exit` = 观测值低于退出阈值，
    /// 其间为保持区（维持上一稳定值）。
    ///
    /// 翻转同时要求「连续越过阈值满 `min_hold_ticks`」与「自上次翻转起
    /// 已保持满 `min_hold_ticks`」：前者防止在阈值附近的高频震荡，后者
    /// 保证翻转后的状态至少存续 `min_hold_ticks` 帧。保持区内连续观测
    /// 计数清零；任一条件不满足都不改变 `value`。
    fn update(&mut self, enter: bool, exit: bool, min_hold_ticks: u32) {
        self.hold_ticks = self.hold_ticks.saturating_add(1);
        let target = if enter {
            true
        } else if exit {
            false
        } else {
            // 保持区：稳定值维持，连续观测计数清零。
            self.pending_ticks = 0;
            return;
        };
        if target == self.value {
            self.pending_ticks = 0;
            return;
        }
        self.pending_ticks = self.pending_ticks.saturating_add(1);
        if self.pending_ticks >= min_hold_ticks && self.hold_ticks >= min_hold_ticks {
            self.value = target;
            self.pending_ticks = 0;
            self.hold_ticks = 0;
        }
    }
}

/// 逐防守人维护的场输出稳定态（solve_equilibrium 的伴生状态）。
///
/// 由调用方持有（战术规划层），不是世界对象（ADR-015 边界）：
/// 每个防守人一份，按索引存入调用方的 `HashMap` 或向量。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefenseHysteresisState {
    help_pulled_off: BooleanHysteresis,
    weak_side_vacant: BooleanHysteresis,
}

impl Default for DefenseHysteresisState {
    /// 初始稳定态为「未拉离、未真空」：比赛开局无突破威胁，与
    /// `solve_equilibrium` 在无威胁时的输出一致。
    fn default() -> Self {
        Self {
            help_pulled_off: BooleanHysteresis::new(false),
            weak_side_vacant: BooleanHysteresis::new(false),
        }
    }
}

/// 滞回后的稳定态快照，供场输出谓词与选板触发消费（#22/#24 及后续）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StableFieldOutput {
    /// 协防人被拉离持球人走廊（由 `threat_ratio` 双阈值稳定）。
    pub help_pulled_off: bool,
    /// 弱侧出现真空（由 `void_ratio` 双阈值稳定）。
    pub weak_side_vacant: bool,
}

impl DefenseHysteresisState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 输入当 tick 的原始观测量，输出稳定态快照。
    ///
    /// 「拉离走廊」由 `threat_ratio` 进入/退出阈值判定，「弱侧真空」由
    /// `void_ratio` 进入/退出阈值判定；保持时间用 tick 计数，阈值与保持
    /// 参数全部从 `PotentialFieldRules` 读取（charter C1）。
    pub fn update(
        &mut self,
        observation: FieldObservation,
        config: &PotentialFieldRules,
    ) -> StableFieldOutput {
        self.help_pulled_off.update(
            observation.threat_ratio > config.help_off_enter_threat_ratio,
            observation.threat_ratio < config.help_off_exit_threat_ratio,
            config.min_hold_ticks,
        );
        self.weak_side_vacant.update(
            observation.void_ratio > config.weak_vacant_enter_void_ratio,
            observation.void_ratio < config.weak_vacant_exit_void_ratio,
            config.min_hold_ticks,
        );
        StableFieldOutput {
            help_pulled_off: self.help_pulled_off.value,
            weak_side_vacant: self.weak_side_vacant.value,
        }
    }

    /// 当前稳定态快照（不推进状态机）。
    pub fn snapshot(&self) -> StableFieldOutput {
        StableFieldOutput {
            help_pulled_off: self.help_pulled_off.value,
            weak_side_vacant: self.weak_side_vacant.value,
        }
    }
}

impl DefensePotentialFieldSolver {
    /// 对当 tick 的势能场求解输出做滞回观测，返回该防守人的稳定态快照。
    ///
    /// 观测量直接取 `emergent.threat_ratio` / `emergent.void_ratio`，
    /// 不重算几何；调用方为每个防守人持有一份 `DefenseHysteresisState`
    /// 并逐 tick 调用本方法。
    pub fn observe_field(
        &self,
        emergent: &EmergentDefenseTarget,
        state: &mut DefenseHysteresisState,
    ) -> StableFieldOutput {
        let config = &self.config;
        state.update(
            FieldObservation {
                threat_ratio: emergent.threat_ratio,
                void_ratio: emergent.void_ratio,
            },
            config,
        )
    }
}
