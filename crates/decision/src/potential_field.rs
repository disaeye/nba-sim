//! 连续受限势能场动力学与空间涌现系统 (Continuous Potential Field Dynamics)
//!
//! 依据 ADR-012 与 docs/architecture.md：
//! 防守战术跑位与轮转不再由离散 if-else 规则指定硬编码目标，
//! 而是由全场球员与空间拓扑构成的多体势能场 (Potential Field) 求导，
//! 求解局部能量极小值平衡点（Equilibrium Point）并由主导场力自然涌现出：
//! 1. 弱侧 Low-man 威胁引力主导的下沉护筐 (ROTATE_RIM_HELP)
//! 2. 弱侧 High-man 空间真空主导的轮转补位 (X_OUT_CLOSEOUT)
//! 3. 稳态球-人-筐三角协防 (HELP_SIDE_SHELL)

use glam::Vec2;
use nba_domain::rules::GameRules;

/// 防守势能场配置参数
#[derive(Debug, Clone)]
pub struct PotentialFieldConfig {
    /// 篮筐威胁特征半径 (英尺)，决定突破深度对全场势能的非线性放大陡峭度
    pub threat_radius_ft: f32,
    /// 禁区内线局部响应半径 (英尺)，决定距篮筐不同距离防守人的引力响应衰减
    pub rim_response_radius_ft: f32,
    /// 对位羁绊基础弹性系数
    pub k_man_base: f32,
    /// 禁区护筐威胁引力系数
    pub k_threat_base: f32,
    /// 空间覆盖真空吸力系数 (X-Out 驱动源)
    pub k_void_base: f32,
}

impl Default for PotentialFieldConfig {
    fn default() -> Self {
        Self {
            threat_radius_ft: 15.0,
            rim_response_radius_ft: 18.0,
            k_man_base: 1.0,
            k_threat_base: 1.6,
            k_void_base: 1.2,
        }
    }
}

/// 势能场平衡点求解结果与涌现动作
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
}

/// 连续势能场求解器
pub struct DefensePotentialFieldSolver {
    pub config: PotentialFieldConfig,
}

impl DefensePotentialFieldSolver {
    pub fn new(config: PotentialFieldConfig) -> Self {
        Self { config }
    }

    /// 求解场上指定防守人在连续势能场中的平衡位置与涌现动作
    ///
    /// # 参数
    /// - `carrier_pos`: 持球人位置
    /// - `hoop_pos`: 进攻篮筐位置
    /// - `off_positions`: 场上 5 名进攻球员当前位置
    /// - `assigned_off_idx`: 该防守人对位的进攻球员索引
    /// - `carrier_idx`: 持球人索引
    /// - `screener_idx`: 掩护人索引
    /// - `is_screening`: 是否发生掩护事件
    /// - `rules`: 比赛规则配置
    pub fn solve_equilibrium(
        &self,
        carrier_pos: Vec2,
        hoop_pos: Vec2,
        off_positions: &[Vec2],
        assigned_off_idx: usize,
        carrier_idx: usize,
        rules: &GameRules,
    ) -> EmergentDefenseTarget {
        let assigned_pos = off_positions
            .get(assigned_off_idx)
            .copied()
            .unwrap_or(carrier_pos);

        // 1. 持球人到篮筐的欧几里得距离与全场连续突破威胁度 (0.0 ~ 1.0)
        let carrier_dist_to_hoop = (carrier_pos - hoop_pos).length();
        let r_threat = self.config.threat_radius_ft;
        // Cauchy-Lorentz 形式连续非线性势能核：深入内线时平滑激增，外线游弋时平滑衰减
        let global_threat = 1.0 / (1.0 + (carrier_dist_to_hoop / r_threat).powi(2));

        // 2. 该防守人所看管对位人距离篮筐的拓扑距离，决定其局域护筐响应权重
        let assigned_dist_to_hoop = (assigned_pos - hoop_pos).length();
        let r_rim = self.config.rim_response_radius_ft;
        let local_rim_proximity = 1.0 / (1.0 + (assigned_dist_to_hoop / r_rim).powi(2));

        // 3. 寻找弱侧进攻球员：
        // 弱侧定义：远离持球人一侧（跨越球场中轴线 mid_y，或横向差值 >= 10.0 ft）
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
                    || (carrier_pos.y - p.y).abs() >= 12.0;
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
        let sag_mult = rules.tactics.defense.sag_multiplier.clamp(0.5, 2.5);
        let dist_assigned_to_hoop = (assigned_pos - hoop_pos).length();
        let base_sag = if dist_assigned_to_hoop > 22.0 {
            2.0_f32 // 外线三分射手
        } else {
            5.5_f32 // 中距离/内线：深度下沉收缩
        };
        let sag_distance = base_sag * sag_mult;
        let to_hoop = (hoop_pos - assigned_pos).normalize_or_zero();
        let to_carrier = (carrier_pos - assigned_pos).normalize_or_zero();
        let shell_anchor =
            assigned_pos + (to_hoop * 0.75 + to_carrier * 0.25).normalize_or_zero() * sag_distance;
        let w_man = self.config.k_man_base;

        // 5. 势能分量二：禁区威胁引力势能 (Rim Protection Attraction)
        // 威胁中心点位于持球人与篮筐之间的禁区缓冲带 (距篮筐 3.0 尺)；受 sag_multiplier 协同下沉
        let rim_target = hoop_pos + (carrier_pos - hoop_pos).normalize_or_zero() * 3.0;
        // 只有弱侧低位人拥有高耦合的护筐引力；高位人需保留在外线，防备三分
        let w_threat = if is_low_man {
            self.config.k_threat_base * global_threat * local_rim_proximity * 2.8 * sag_mult
        } else if is_high_man {
            self.config.k_threat_base * global_threat * local_rim_proximity * 0.15
        } else {
            self.config.k_threat_base * global_threat * local_rim_proximity * 0.4 * sag_mult
        };

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
            w_void = self.config.k_void_base * low_man_excitation * 4.0;
        }

        // 7. 多体势能场平衡点闭式求解 (Analytical Force Equilibrium)
        // \sum F = w_man * (shell_anchor - x) + w_threat * (rim_target - x) + w_void * (void_target - x) = 0
        let total_weight = (w_man + w_threat + w_void).max(0.001);
        let equilibrium_pos =
            (shell_anchor * w_man + rim_target * w_threat + void_target * w_void) / total_weight;

        // 8. 场力主导性分析与宏观战术动作涌现
        let threat_ratio = w_threat / total_weight;
        let void_ratio = w_void / total_weight;

        let (action, slot) = if threat_ratio > 0.40 && (equilibrium_pos - hoop_pos).length() < 12.0
        {
            ("ROTATE_RIM_HELP", "LowManRimHelp")
        } else if void_ratio > 0.32 {
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
        }
    }
}
