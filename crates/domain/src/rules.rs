use crate::court::CourtGeometry;
use crate::projectile::ProjectileArc;
use crate::resolve::ResolveConfig;
use serde::{Deserialize, Serialize};

mod policies;
mod validate;

pub use policies::{
    CapabilityCurveRules, DecisionRules, DefenseRules, ModulationRules,
    OnBallScreenDefenseStrategy, PotentialFieldRules, RotationRules, ScreenDefenseRules,
    SemanticRules, TacticalRules,
};

/// Match rules and timing policy shared by the simulation subsystems.
///
/// The engine consumes this value rather than embedding competition-specific
/// numbers in phase transitions, constraints, or ball execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GameRules {
    pub tick_seconds: f32,
    /// R5 持球违例执法开关（r45）：true（生产）= 收球后无移动/再运球候选
    /// （结构性合法）；false（注入测试）= 判定层抓违例。
    pub travel_enforcement: bool,
    /// 联赛规则档案（M10）：切换联赛 = 替换该档案，引擎零联赛分支。
    pub league: crate::league::LeagueProfile,
    pub estimated_possessions_per_period: u32,
    pub inbound_seconds: f32,
    pub inbound_setup_seconds: f32,
    /// 发球接应人落位点距发球员的距离（ft）：无接应位时发球只能
    /// 选择 20-60ft 高风险长传。0 关闭接应机制。
    pub inbound_safety_distance_ft: f32,
    pub backcourt_seconds: f32,
    /// 攻方三秒时限（charter §6.2）：前场控制时进攻人在限制区连续停留超过
    /// 该时限判违例。NBA 与 FIBA 同为 3 秒。
    pub three_second_lane_seconds: f32,
    pub period_break_seconds: f32,
    pub tactical_initiation_seconds: f32,
    pub decision_interval_seconds: f32,
    /// 发球决策间隔（秒）：发球是「尽快发进场」程序，不套用阵地节奏
    ///（套用曾致 37% 发球判五秒违例）。
    pub inbound_decision_interval_seconds: f32,
    pub free_throw_interval_seconds: f32,
    pub ball_max_speed_ftps: f32,
    pub min_pass_duration_seconds: f32,
    pub max_pass_duration_seconds: f32,
    pub min_shot_duration_seconds: f32,
    pub max_shot_duration_seconds: f32,
    pub shot_peak_base_ft: f32,
    pub shot_peak_distance_factor: f32,
    pub chest_height_ft: f32,
    pub rim_height_ft: f32,
    /// 筐环半径（ft，NBA 标准 9" = 0.75）：触筐点在环上的几何。
    pub rim_radius_ft: f32,
    /// 触筐点方位角扇形半径（rad）：以「从筐指向出手点」方向为中心在筐环上
    /// 采样接触点，0 表示永远正对出手点的近筐沿。
    pub rim_contact_angle_spread_radians: f32,
    /// 触筐后水平速度的恢复系数上限：正面硬碰筐沿（接触角≈0）保持
    /// 更多能量，产生长回弹尾部。
    pub rim_contact_restitution_flush: f32,
    /// 恢复系数下限：擦筐（接触角达到扇形半径）耗散更多能量。
    pub rim_contact_restitution_graze: f32,
    /// 触筐后竖直方向的弹起系数：入射下落速度 × 此系数为弹起初速。
    pub rim_contact_vertical_restitution: f32,
    /// 镜像反射方向的受控散射（rad，绕竖直轴对称均匀采样）。
    pub rim_contact_scatter_radians: f32,
    /// 自由球-人接触（ADR-017 第三步）：球撞到在场球员身体后，反射
    /// 水平速度的恢复系数。篮板飞行与地板球共用同一系数（真实篮球里
    /// 球触人后的余速量级与触板相近，约入射的一半）。
    pub loose_ball_player_restitution: f32,
    /// 篮球半径（ft，真实直径 ≈ 0.4 ft）：与球员身体碰撞的水平接触
    /// 半径 = `player_radius_ft + 本值`（主动触碰口径另计）。
    pub ball_radius_ft: f32,
    /// 人体对球的被动碰撞半径（ft，躯干半宽量级）：单一半径不区分
    /// 腿/躯干（球碰腿与碰躯干都弹开，半径差异是二阶量）。
    pub body_contact_radius_ft: f32,
    /// 地板球的「可控制速度上限」（ft/s）：超速时快球从手上弹开，
    /// 球继续飞行直到撞身体弹开或减速后可收（真实只有慢球能稳收）。
    pub loose_ball_control_speed_ftps: f32,
    /// 篮板几何（ADR-017 第三步）：板面距底线的水平距离（ft）。
    /// 篮板平面垂直于底线方向，左筐板面 x = 该值，右筐板面 x = 场宽 − 该值。
    pub backboard_offset_from_baseline_ft: f32,
    /// 篮板宽度（ft，NBA 标准 6 ft）：触板判定的 |y − hoop_y| 上限。
    pub backboard_width_ft: f32,
    /// 篮板底沿高度（ft，NBA 标准 9.5 ft）：触板 z 的下限。
    pub backboard_bottom_height_ft: f32,
    /// 篮板顶沿高度（ft，NBA 标准 13 ft）：触板 z 的上限。
    pub backboard_top_height_ft: f32,
    /// 触板后水平速度法向分量的恢复系数：镜像反射后整体乘该值。
    pub backboard_restitution: f32,
    /// 触板后竖直方向的弹起系数：入射下落速度 × 此系数为向上弹起
    /// 初速。真实板反弹球的竖直分量会损失大半但仍向上弹起（板面
    /// 给球一个离开的冲量），0 意味着球贴板滑落、篮板失去争抢窗口。
    #[serde(default = "default_backboard_vertical_restitution")]
    pub backboard_vertical_restitution: f32,
    pub pass_peak_ft: f32,
    /// 传球抛体弧顶随距离的增长率（ft/ft）：胸口短传平快，
    /// 长传弧顶抬高（第一步飞行抛体化）。
    pub pass_peak_distance_factor: f32,
    pub shot_clock_urgency_seconds: f32,
    /// Contest intensity to field-goal percentage conversion slope.
    pub shot_contest_sensitivity: f32,
    /// Shared semantic interpretation policy; no action-specific constants belong in systems.
    #[serde(default)]
    pub semantics: SemanticRules,
    /// 裁决参数嵌套组（M7 并轨）：概率与权重的唯一规则通道。
    #[serde(default)]
    pub resolve: ResolveConfig,
    /// 补篮与篮下接球的距离上限，等于 `court::RIM_ZONE_MAX_DIST_FT`。
    /// 出手分区不再读取本字段，统一走 `CourtGeometry::shot_zone`。
    pub rim_shot_distance_ft: f32,
    /// Maximum accepted distance from the court boundary for an inbound release.
    pub inbound_boundary_tolerance_ft: f32,
    pub inbound_release_depth_ft: f32,
    /// Horizontal distance from the hoop center to the free throw line.
    pub free_throw_distance_ft: f32,
    /// Clamps applied to any final field-goal probability.
    pub shot_pct_floor: f32,
    pub shot_pct_ceiling: f32,
    pub tip_off_duration_seconds: f32,
    /// 跳球点拍初速（ft/s）：须与收球门同源标定——反弹后水平速度
    /// 低于 `loose_ball_control_speed_ftps`（28 ft/s 时球滚 45 ft）。
    pub tip_off_tap_speed_ftps: f32,
    /// 点拍目标距中圈的距离（ft）：拍向本方后场方向、候在跳球圈
    /// 外的接应队友（跳球站位中本方除跳球员外离中圈最近者）。
    pub tip_off_tap_distance_ft: f32,
    /// 点拍离手高度（ft）：中锋跳到最高点时手的高度量级。
    pub tip_off_tap_height_ft: f32,
    /// 点拍出手竖直初速（ft/s）：轻拨带一点上抛弧线。
    pub tip_off_tap_vel_z_ftps: f32,
    /// Ball presentation policy while a player is dribbling.
    pub ball_bounce_base_ft: f32,
    pub ball_bounce_amplitude_ft: f32,
    pub ball_bounce_frequency_hz: f32,
    /// Court dimensions and hoop locations used by physics, tactics, and officiating.
    pub court: CourtGeometry,
    pub player_radius_ft: f32,
    /// Minimum center-to-center distance maintained by kinematic movement.
    pub min_player_separation_ft: f32,
    /// Numerical cushion used by endpoint projection before protocol rounding.
    #[serde(default = "default_separation_safety_margin_ft")]
    pub separation_safety_margin_ft: f32,
    pub max_player_speed_ftps: f32,
    pub max_player_accel_ftps2: f32,
    /// 球员纵向制动加速度上限（ft/s²），由 `acceleration` 能力调制。
    pub max_player_braking_accel_ftps2: f32,
    /// 球员横向抓地加速度上限（ft/s²），由 `agility` 能力调制。
    pub max_player_lateral_accel_ftps2: f32,
    pub turnaround_min_decel_seconds: f32,
    pub player_linear_damping: f32,
    /// 「到达」判定阈值（ft）：目标距离小于该值即视为到位，速度归零。
    /// 从 `physics::movement` 的内联常数收编而来（charter C1）。
    pub arrival_epsilon_ft: f32,
    /// 目标速度的距离缩放基准（无量纲）：`distance / (max_speed*scale + 1)`。
    pub arrival_speed_scale: f32,
    /// 目标速度缩放的下限比例（避免远距离时速度被压得过低）。
    pub arrival_speed_floor: f32,
    /// 转身减速保留速度比例基准（实际值按 `agility` 调制，§2.2）。
    pub turn_decel_retention: f32,
    /// 调制后保留比例的下限（防止低敏捷者转向失速至停）。
    pub turn_decel_retention_floor: f32,
    /// 调制后保留比例的上限（防止高敏捷者转向零损失）。
    pub turn_decel_retention_ceiling: f32,
    pub contact_margin_ft: f32,
    /// 最大转体角速度（弧度/秒，真实人体转向速率上限）
    pub max_player_turn_rate_rad_per_sec: f32,
    /// 步态模型的基准步幅（ft）：身体每越过支撑脚该距离，摆动脚落地一次；
    /// 实际步幅随速度在基准上伸展。
    pub gait_stride_length_ft: f32,
    /// 静止门（ft/秒）：速度低于此值不产生新的接地事实——走步判定需要
    /// 一个真实的推进量，站着不动时摆动脚不应反复触地。
    pub gait_stride_min_speed_ftps: f32,
    /// 步长随速度的伸展斜率分母（以最大速度的倍数表示）：步长 =
    /// 基准 × (基线 + 速度/(最大速度×本值)) 的钳制区间。
    pub gait_stride_speed_gain: f32,
    /// 步长伸展的基线比例：速度为零时步长仍有本比例，避免低速段步幅塌缩。
    pub gait_stride_speed_baseline: f32,
    /// 步长伸展比例的下限与上限：分别为 gait_stride_speed_baseline 的
    /// 钳制下界，以及其上界（防止冲刺段步幅失控）。
    pub gait_stride_stretch_min: f32,
    pub gait_stride_stretch_max: f32,
    /// 摆动脚接地点落在步长前沿的比例（0..1）：0 为落在身体正下方，
    /// 1 为恰好越过支撑脚一整步。真实步态取中间偏前。
    pub gait_stride_contact_fraction: f32,
    /// 接地健全性上限（以基准步幅的倍数表示）：双脚离身体超过该倍数时
    /// 判定为显式放置/传送，收回双脚且不发射接地事实。必须高于加速段
    /// 摆动脚旧落点的自然滞后（约两个步幅）。
    pub gait_stride_sanity_multiple: f32,
    /// 轴心脚允许微小滑动容差（ft，防止数值浮点误差触发走步）。
    pub pivot_foot_tolerance_ft: f32,
    /// 地球标准重力加速度（呎/秒^2，用于自由球与弹跳抛物线计算）
    pub ball_gravity_ftps2: f32,
    /// 地板球滚动减速度（呎/秒^2）：恒定摩擦让球滚 1-3s/10+ 呎
    ///（旧逐 tick 指数衰减 0.3s 即爬行到可收速度，争抢窗口消失）。
    pub loose_ball_rolling_decel_ftps2: f32,
    pub defender_reach_ft: f32,
    pub pass_corridor_radius_ft: f32,
    /// 接球半径基准（ft）：无技能加成时的控制圈半径，实际半径由
    /// `effective_catch_radius` 按属性调制（charter C1：曲线走规则通道）。
    pub catch_radius_base_ft: f32,
    pub catch_radius_skill_gain_ft: f32,
    /// 预估误差幅度（ft）：`off_ball_sense` 低时预估接球点的额外误差上限
    /// （0 = 完全精确；>0 = P-1「预估可能错」）。
    pub receive_estimate_noise_ft: f32,
    /// 确定性误差向量的量化分母（16 位哈希切片映射到 [-1, 1]）。
    /// 是**数值工具参数**（不是行为常数）：放在规则通道以便审计与调整，
    /// 取值 32767.5 = (2^16 − 1) / 2，使 0..=65535 映射到 [-1, 1]。
    pub estimate_offset_quantization: f32,
    /// Reference clearance used to normalize pass-lane risk scores.
    pub pass_lane_clearance_reference_ft: f32,
    /// 飞行中传球拦截判定半径（球心-防守人心距，含臂展与跨步）。
    pub flight_intercept_radius_ft: f32,
    /// 防守人主动扑向传球路线的判定半径（行为层触发距离）。
    pub intercept_lane_radius_ft: f32,
    pub teammate_density_radius_ft: f32,
    pub teammate_density_capacity: f32,
    pub court_side_margin_ratio: f32,
    pub open_shot_distance_ft: f32,
    /// Distance normalization reference for shot utility scoring.
    pub shot_distance_reference_ft: f32,
    pub rebound_outlet_fallback_distance_ft: f32,
    pub ball_holder_offset_ft: f32,
    pub ball_holder_height_ft: f32,
    pub max_ball_carrier_turn_rate_rad_per_sec: f32,
    /// 属性响应曲线下限（映射层 capability 的规则参数，attributes.md T2）。
    pub attribute_response_floor: f32,
    /// 能力映射层曲线系数（D27 六个维度，`capability.rs` 读取）。
    pub capability: CapabilityCurveRules,
    /// 球与持球者允许的最大距离（L1 BALL_WITH_HOLDER 限值，经 FrameRules 下发）。
    pub invariant_holder_leash_ft: f32,
    /// L1 速度不变量的数值容差（PLAYER_SPEED / BALL_SPEED）。
    pub invariant_speed_tolerance_ftps: f32,
    /// 球高度上限（L1 BALL_HEIGHT_BOUNDS）。
    pub ball_z_max_ft: f32,
    /// 「实质性越界」的最小超出距离（ft）：亚英尺级 clamp 差值不算越界
    ///（否则被顶在边线的持球人每场 42-52 次虚假失误）。
    pub boundary_epsilon_ft: f32,
    /// 事实/总结流的最大写入字节数（gap.md §16.4 资源治理）。
    /// 实测一场 full scope 的 facts 流约 7–16 MiB，故预算取 64 MiB
    /// 留出余量；仍是有界值，而不是无限增长。
    pub stream_max_bytes: u64,
    /// 逐 tick 帧流的最大写入字节数。帧模式是显式选择（回放/展示），
    /// 默认只够一节/片段；整场帧流请显式上调（或改用 facts 模式）。
    pub stream_frames_max_bytes: u64,
    /// 单次模拟的最大 tick 数（生命周期防护，非篮球规则）。
    pub stream_max_ticks: usize,
    /// 交接接球点相对 leash 的安全比例：接球人未能走到冻结点时，球位于
    /// 「冻结点 → 接球人」方向上距接球人 `leash × 该比例` 处，保证
    /// `BALL_WITH_HOLDER` 成立且不悬置。
    pub transfer_landing_leash_ratio: f32,
    /// 分离投影中非豁免球员承担修正量的比例（gap.md §4.3）。
    /// 两名球员都参与时为 0.5；一方是显式 placement 角色时由另一方
    /// 承担全部修正量（即 1.0 - 该比例）。
    pub separation_correction_share: f32,
    /// Player stamina model parameters, expressed in normalized stamina units.
    pub stamina_sprint_speed_ftps: f32,
    pub stamina_recovery_speed_ftps: f32,
    pub stamina_drain_per_second: f32,
    pub stamina_recovery_per_second: f32,
    pub stamina_floor: f32,
    pub stamina_exhausted_threshold: f32,
    pub jump_shot_prep_seconds: f32,
    pub jump_shot_exec_seconds: f32,
    pub jump_shot_follow_seconds: f32,
    pub pass_prep_seconds: f32,
    pub pass_exec_seconds: f32,
    pub pass_follow_seconds: f32,
    pub rebound_prep_seconds: f32,
    pub rebound_exec_seconds: f32,
    pub rebound_follow_seconds: f32,
    /// 进攻方冲抢篮板的人数（charter C1 数据通道）。全员扑落点会触发
    /// 碰撞消解把人推离落点（problem.md §23.8：指派前攻方几乎不动）。
    pub rebound_crash_offense_count: u32,
    /// 防守方卡位／收篮板的人数。守方多一人，与真实的
    /// 「守方收下约 75.5% 投失」一致（evidence/problem.md §23.7）。
    pub rebound_boxout_defense_count: u32,
    pub layup_prep_seconds: f32,
    pub layup_exec_seconds: f32,
    pub layup_follow_seconds: f32,
    pub dunk_prep_seconds: f32,
    pub dunk_exec_seconds: f32,
    pub dunk_follow_seconds: f32,
    pub screen_prep_seconds: f32,
    pub screen_exec_seconds: f32,
    pub screen_follow_seconds: f32,
    pub contest_prep_seconds: f32,
    pub contest_exec_seconds: f32,
    pub contest_follow_seconds: f32,
    /// Tactical movement policy shared by every built-in scheme.
    #[serde(default)]
    pub tactics: TacticalRules,
    /// Match-level parameters for coach and player psychological modulation.
    #[serde(default)]
    pub modulation: ModulationRules,
    /// Rotation and substitution scheduling policy（gap.md G6）。
    #[serde(default)]
    pub rotation: RotationRules,
    /// Decision utility and sampling policy used by every decision system.
    #[serde(default)]
    pub decision: DecisionRules,
}

impl Default for TacticalRules {
    fn default() -> Self {
        Self {
            initiation_distance_ratio: 0.30,
            action_duration_seconds: 9.5,
            drive_distance_ratio: 0.15,
            drive_min_duration_seconds: 0.8,
            // round-18：2.2 → 3.2。时长公式加入加速坡（v/a ≈ 0.7s）后，
            // 长.distance 突破需要更长时间才能真实到达（A/B 见 §19.3）。
            drive_max_duration_seconds: 3.2,
            drive_speed_ratio: 1.15,
            drive_early_finish_dist_ft: 6.0,
            drive_mid_range_pullup_dist_ft: 14.0,
            drive_kickout_pass_dist_ft: 22.0,
            drive_lane_offset_ft: 4.0,
            // 冲框走廊折扣：按终结能力加权（终结强者面对拥堵仍攻框）。
            // 基准 1.3 守住篮下占比 ≥0.25 与中距离占比 ≥0.08 两条带。
            drive_rim_attack_bias: 1.3,
            drive_finish_extend_ft: 6.0,
            drive_beaten_recovery_seconds: 0.6,
            // 突破停滞线：16ft 时停滞堆在 10-16ft 脏区（篮下出手仅
            // 1-2% vs 真实 30%）——收窄到 10ft，进入即直接攻框。
            drive_finish_range_ft: 16.0,
            drive_dunk_max_dist_ft: 4.0,
            drive_floater_min_dist_ft: 7.0,
            drive_dunk_min_finishing: 0.70,
            drive_dunk_max_lane_density: 0.35,
            drive_kickout_max_crowding: 0.40,
            drive_kickout_min_defender_dist_ft: 6.0,
            drive_pullup_min_crowding: 0.55,
            drive_decision_check_interval_seconds: 0.25,
            screen_distance_ratio: 0.272,
            defensive_gap_ft: 4.0,
            help_sag_ratio: 0.35,
            transition_speed_ratio: 0.91,
            carrier_speed_ratio: 0.73,
            screener_speed_ratio: 0.64,
            support_speed_ratio: 0.45,
            // r40：0.73→0.85（同速使后场纠缠：8 秒违例 8 次/场 vs ~0.3）。
            defender_speed_ratio: 0.85,
            rebound_chase_speed_ratio: 0.73,
            offensive_crash_max: 3,
            timeout_duration_seconds: 60.0,
            backcourt_support_count: 2,
            backcourt_support_offset_ft: 12.0,
            backcourt_support_y_min: 6.0,
            backcourt_support_y_max: 44.0,
            receive_approach_speed_ratio: 0.9,
            receive_stop_margin_ft: 2.5,
            receive_min_approach_speed_ratio: 0.25,
            pass_lead_gain: 0.75,
            pass_lead_max_ft: 12.0,
            apf_repulsion_radius_ft: 12.0,
            apf_teammate_repulsion_accel: 15.0,
            apf_opponent_repulsion_accel: 10.0,
            transition_defense_threshold_ratio: 0.38,
            transition_sprint_ratio: 0.88,
            transition_sprint_floor: 0.7,
            transition_sprint_span: 0.6,
            transition_sprint_window_seconds: 8.0,
            // 转换进攻篮下终结窗口（秒，G6a 链 3）：回合前段防守未落位，
            // 突破攻框的效用加成只在此窗口内生效，避免把阵地战的攻框
            // 比例一并抬高（此前校准迭代 4 的教训：强抬攻框砸穿 3P% 带）。
            transition_finish_window_seconds: 6.0,
            // 转换期篮下终结的效用加成基准。
            transition_finish_bonus: 0.35,
            screen_hold_separation_ft: 6.0,
            screen_roll_separation_ft: 8.0,
            cut_tendency_floor: 0.75,
            cut_tendency_span: 0.5,
            cut_tendency_threshold: 0.3,
            screen_tendency_floor: 0.75,
            screen_tendency_span: 0.5,
            screen_tendency_threshold: 0.3,
            // 冲筐类槽位的「进-出」曲线：三秒规则下限制区只穿越不停留，
            // 窗口 ≈2 秒，给接球后决策留三秒余量。
            rim_cut_start_seconds: 0.5,
            rim_cut_peak_seconds: 1.5,
            rim_cut_exit_seconds: 2.4,
            // 高位掩护顺下的最大顺下深度（占槽位到筐距离的比例）。
            screen_roll_peak_depth: 0.55,
            backdoor_cut_depth_ratio: 0.85,
            dip_to_rim_depth_ratio: 0.7,
            drop_coverage_depth_ft: 14.0,
            // 到位半径：跑位任务的几何完成门（validation.md §4.2）。
            // 实际位置进入目标点此半径即 `completed`；超出且到期 `timed_out`。
            arrival_radius_ft: 2.0,
            defense: DefenseRules::default(),
        }
    }
}

fn default_separation_safety_margin_ft() -> f32 {
    0.05
}

fn default_backboard_vertical_restitution() -> f32 {
    0.45
}

impl Default for GameRules {
    fn default() -> Self {
        Self {
            tick_seconds: 0.04,
            travel_enforcement: true,
            league: crate::league::LeagueProfile::nba(),
            estimated_possessions_per_period: 35,
            inbound_seconds: 5.0,
            inbound_setup_seconds: 2.2,
            inbound_safety_distance_ft: 8.0,
            backcourt_seconds: 8.0,
            three_second_lane_seconds: 3.0,
            period_break_seconds: 15.0,
            tactical_initiation_seconds: 6.5,
            decision_interval_seconds: 3.0,
            inbound_decision_interval_seconds: 0.4,
            free_throw_interval_seconds: 2.2,
            ball_max_speed_ftps: 85.0,
            min_pass_duration_seconds: 0.45,
            max_pass_duration_seconds: 1.4,
            min_shot_duration_seconds: 0.95,
            max_shot_duration_seconds: 1.45,
            decision: DecisionRules::default(),
            shot_peak_base_ft: 14.0,
            // 弧顶随距离的增长率（ft/ft）：0.04 使投篮提前 0.065s 到筐、
            // 封盖率与 3P% 双双越带且机制未明——保持 0.25，待封盖裁决与
            // 飞行时长耦合查清后再动（禁止盲目调参）。
            shot_peak_distance_factor: 0.25,
            chest_height_ft: 4.0,
            rim_height_ft: 10.0,
            rim_radius_ft: 0.75,
            // 触筐物理（ADR-017 第二步）：接触点在近筐沿受控扇形上采样，
            // 反弹初速 = 入射镜像反射 × 恢复系数 + 受控散射，落点由抛体
            // 自然产生。恢复系数量级：真实篮球碰筐后水平余速约 0.4-0.6，
            // 竖直弹起约入射下落速度的 0.3-0.4。
            rim_contact_angle_spread_radians: 1.1,
            // r45：0.62/0.35→0.45/0.25（此前长篮板散满半场，OREB 堆高）。
            rim_contact_restitution_flush: 0.45,
            rim_contact_restitution_graze: 0.25,
            rim_contact_vertical_restitution: 0.35,
            rim_contact_scatter_radians: 0.35,
            // 自由球-人接触（ADR-017 第三步）：球触人后的余速约入射一半，
            // 与触筐/触板恢复系数量级一致（0.35-0.62 的中点附近）。
            loose_ball_player_restitution: 0.5,
            ball_radius_ft: 0.4,
            body_contact_radius_ft: 1.0,
            loose_ball_control_speed_ftps: 12.0,
            // 篮板几何（ADR-017 第三步）：真实 NBA 篮板底沿 9.5 ft、顶沿
            // 13 ft、宽 6 ft、板面距底线 4 ft；触板水平恢复系数 0.55
            // （板比筐沿耗散更少，打板回弹更平直）。
            backboard_offset_from_baseline_ft: 4.0,
            backboard_width_ft: 6.0,
            backboard_bottom_height_ft: 9.5,
            backboard_top_height_ft: 13.0,
            backboard_restitution: 0.55,
            backboard_vertical_restitution: 0.45,
            pass_peak_ft: 4.0,
            // 传球弧顶随距离增长：30 ft 传球弧顶 7 ft（抛体解出 T ≈ 0.86 s，
            // 球速 ≈ 38 ft/s，真实胸口传球量级）；50 ft 长传弧顶 9 ft。
            pass_peak_distance_factor: 0.10,
            // D3.4 校准：紧逼窗口 5s→12s（5s 窗口下单场 53 次 24 秒
            // 违例 vs 真实 0-2——真实进攻 24→14s 区间已开始组织）。
            shot_clock_urgency_seconds: 12.0,
            // D3.1 校准：防守干扰惩罚从 0.22 提至 0.32，使空位/重压出手的
            // 命中率分化接近真实。0.42 实测 2P 63.8% 超带且诱发贴边几何僵局
            // （seed11 streak 714>200）；0.32 为 8 seed full 实测双入带点：
            // 3P 39.7% ∈ [30,40]、2P 58.1% ∈ [48,58]，streak 121<200。
            shot_contest_sensitivity: 0.32,
            semantics: SemanticRules::default(),
            resolve: crate::resolve::ResolveConfig::default(),
            rim_shot_distance_ft: crate::court::RIM_ZONE_MAX_DIST_FT,
            inbound_boundary_tolerance_ft: 5.5,
            inbound_release_depth_ft: 3.0,
            free_throw_distance_ft: 13.75,
            shot_pct_floor: 0.10,
            shot_pct_ceiling: 0.85,
            tip_off_duration_seconds: 1.5,
            tip_off_tap_speed_ftps: 13.0,
            tip_off_tap_distance_ft: 14.0,
            tip_off_tap_height_ft: 5.5,
            tip_off_tap_vel_z_ftps: 6.0,
            ball_bounce_base_ft: 3.25,
            ball_bounce_amplitude_ft: 0.35,
            ball_bounce_frequency_hz: 2.2,
            court: CourtGeometry::default(),
            player_radius_ft: 1.8,
            min_player_separation_ft: 3.6,
            separation_safety_margin_ft: default_separation_safety_margin_ft(),
            max_player_speed_ftps: 22.0,
            max_player_accel_ftps2: 35.0,
            max_player_braking_accel_ftps2: 24.0,
            max_player_lateral_accel_ftps2: 20.0,
            turnaround_min_decel_seconds: 0.12,
            player_linear_damping: 4.0,
            arrival_epsilon_ft: 0.15,
            arrival_speed_scale: 0.25,
            arrival_speed_floor: 0.15,
            turn_decel_retention: 0.4,
            turn_decel_retention_floor: 0.2,
            turn_decel_retention_ceiling: 0.7,
            contact_margin_ft: 0.6,
            max_player_turn_rate_rad_per_sec: 18.0,
            gait_stride_length_ft: 2.5,
            // r40：步态事实链（R5 走步/二次运球）接入时内联的四个阈值，
            // 收编进档案通道以满足 charter C1。默认值与原内联值逐位相同，
            // 行为不变（golden_hash 不变）。
            gait_stride_min_speed_ftps: 0.75,
            gait_stride_speed_gain: 1.5,
            gait_stride_speed_baseline: 0.75,
            gait_stride_stretch_min: 1.0,
            gait_stride_stretch_max: 2.0,
            gait_stride_contact_fraction: 0.5,
            gait_stride_sanity_multiple: 4.0,
            pivot_foot_tolerance_ft: 0.35,
            ball_gravity_ftps2: 32.17,
            loose_ball_rolling_decel_ftps2: 6.0,
            defender_reach_ft: 4.0,
            pass_corridor_radius_ft: 3.5,
            // r38 校准：2.0→2.8 ft（此前 30% 传球被判掉球，失误率主源）。
            catch_radius_base_ft: 2.8,
            catch_radius_skill_gain_ft: 1.2,
            receive_estimate_noise_ft: 2.5,
            estimate_offset_quantization: 32767.5,
            pass_lane_clearance_reference_ft: 8.0,
            flight_intercept_radius_ft: 3.8,
            intercept_lane_radius_ft: 6.0,
            teammate_density_radius_ft: 8.0,
            teammate_density_capacity: 4.0,
            court_side_margin_ratio: 0.16,
            open_shot_distance_ft: 4.5,
            shot_distance_reference_ft: 47.0,
            rebound_outlet_fallback_distance_ft: 10.0,
            ball_holder_offset_ft: 0.8,
            ball_holder_height_ft: 4.0,
            max_ball_carrier_turn_rate_rad_per_sec: f32::from(2u8),
            attribute_response_floor: 0.5,
            capability: CapabilityCurveRules::default(),
            invariant_holder_leash_ft: 3.0,
            invariant_speed_tolerance_ftps: 1.5,
            ball_z_max_ft: 35.0,
            boundary_epsilon_ft: 1.0,
            stream_max_bytes: 64 * 1024 * 1024,
            stream_frames_max_bytes: 512 * 1024 * 1024,
            stream_max_ticks: 250_000,
            transfer_landing_leash_ratio: 0.5,
            separation_correction_share: 0.5,
            stamina_sprint_speed_ftps: 15.0,
            stamina_recovery_speed_ftps: 6.0,
            // 体力动态标定（G6 换人链前提）：实测净耗率 ≈0.0029/s，
            // 按「每队 35-45 次换人」反推上场周期 ~5.3 分钟 → drain 0.021。
            stamina_drain_per_second: 0.021,
            stamina_recovery_per_second: 0.0005,
            stamina_floor: 0.2,
            stamina_exhausted_threshold: 0.35,
            jump_shot_prep_seconds: 0.28,
            jump_shot_exec_seconds: 0.22,
            jump_shot_follow_seconds: 0.35,
            pass_prep_seconds: 0.16,
            pass_exec_seconds: 0.12,
            pass_follow_seconds: 0.18,
            rebound_prep_seconds: 0.20,
            rebound_exec_seconds: 0.30,
            rebound_follow_seconds: 0.30,
            rebound_crash_offense_count: 2,
            rebound_boxout_defense_count: 3,
            layup_prep_seconds: 0.20,
            layup_exec_seconds: 0.25,
            layup_follow_seconds: 0.25,
            dunk_prep_seconds: 0.22,
            dunk_exec_seconds: 0.20,
            dunk_follow_seconds: 0.30,
            screen_prep_seconds: 0.25,
            screen_exec_seconds: 1.50,
            screen_follow_seconds: 0.20,
            contest_prep_seconds: 0.15,
            contest_exec_seconds: 0.35,
            contest_follow_seconds: 0.25,
            tactics: TacticalRules::default(),
            modulation: ModulationRules::default(),
            rotation: RotationRules::default(),
        }
    }
}

impl GameRules {
    /// 以指定联赛档案构造规则（多联赛唯一入口，宪章 C3）。
    pub fn with_league(league: crate::league::LeagueProfile) -> Self {
        Self {
            league,
            ..Default::default()
        }
    }

    pub fn period_duration(&self, period: u32) -> f32 {
        let league = &self.league;
        if period > league.regulation_periods {
            league.overtime_duration_seconds
        } else {
            league.period_duration_seconds
        }
    }

    /// 传球飞行时长由抛体解出，并约束三维速度不超过配置上限。
    /// 常规传球按弧顶求解时长，并在可行速度区间内调整；发球平快轨迹
    /// 按起终点三维距离计算最短时长。长距离传球允许超过偏好时长上限，
    /// 以满足球速上限。
    pub fn pass_duration(&self, distance_ft: f32, inbound: bool) -> f32 {
        assert!(
            distance_ft.is_finite() && self.ball_max_speed_ftps.is_finite(),
            "pass trajectory inputs must be finite"
        );
        assert!(
            !distance_ft.is_sign_negative() && self.ball_max_speed_ftps > f32::EPSILON,
            "pass distance and speed limit must be valid"
        );
        let distance = distance_ft;
        let max_speed = (self.ball_max_speed_ftps - self.invariant_speed_tolerance_ftps)
            .max(self.invariant_speed_tolerance_ftps);
        assert!(
            max_speed > f32::EPSILON,
            "pass speed envelope must remain positive"
        );
        let minimum_window = self.min_pass_duration_seconds;
        if inbound {
            let vertical_distance = self.rim_height_ft - self.chest_height_ft;
            let path_length = distance.hypot(vertical_distance);
            return (path_length / max_speed).max(minimum_window);
        }
        let gravity = self.ball_gravity_ftps2;
        let speed_squared = max_speed * max_speed;
        let discriminant = speed_squared * speed_squared - gravity * gravity * distance * distance;
        assert!(
            gravity > f32::EPSILON && !discriminant.is_sign_negative(),
            "pass distance cannot fit the configured three-dimensional ball-speed envelope"
        );
        let root = discriminant.sqrt();
        let two = f32::from(2u8);
        let minimum_speed_duration = (two * distance * distance / (speed_squared + root)).sqrt();
        let maximum_speed_duration = (two * (speed_squared + root) / (gravity * gravity)).sqrt();
        assert!(
            minimum_window <= maximum_speed_duration,
            "minimum pass duration exceeds the ball-speed envelope"
        );
        let chest = self.chest_height_ft;
        let peak = (self.pass_peak_ft + distance * self.pass_peak_distance_factor)
            .max(chest + f32::EPSILON);
        let projectile_duration = ProjectileArc::time_for_peak(chest, chest, peak, gravity);
        let preferred_duration = projectile_duration.clamp(
            minimum_window,
            self.max_pass_duration_seconds.max(minimum_window),
        );
        preferred_duration
            .max(minimum_speed_duration)
            .min(maximum_speed_duration)
    }

    pub fn inbound_expired(&self, elapsed: f32) -> bool {
        elapsed >= self.inbound_seconds
    }

    pub fn period_expired(&self, clock: f32) -> bool {
        clock <= 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::GameRules;

    #[test]
    fn defaults_are_valid() {
        assert!(GameRules::default().validate().is_ok());
    }

    #[test]
    fn pass_duration_keeps_long_passes_inside_the_3d_speed_envelope() {
        let rules = GameRules::default();
        for distance in [8.0_f32, 25.0, 50.0, 75.0, 90.0, 95.0] {
            let duration = rules.pass_duration(distance, false);
            let arc = crate::projectile::ProjectileArc::solve(
                rules.chest_height_ft,
                rules.chest_height_ft,
                duration,
                rules.ball_gravity_ftps2,
            );
            let horizontal_speed = distance / duration;
            let vertical_speed = arc.vz0.abs();
            let landing_vertical_speed = (arc.vz0 - rules.ball_gravity_ftps2 * duration).abs();
            let maximum_speed = horizontal_speed
                .hypot(vertical_speed)
                .max(horizontal_speed.hypot(landing_vertical_speed));
            assert!(
                maximum_speed <= rules.ball_max_speed_ftps,
                "pass distance {distance:.1} ft requires {duration:.3}s but reaches {maximum_speed:.2} ft/s"
            );
            assert!(duration >= rules.min_pass_duration_seconds);
        }
        let capped_rules = GameRules {
            max_pass_duration_seconds: 1.0,
            ..rules
        };
        assert!(capped_rules.pass_duration(95.0, false) > 1.0);
    }

    #[test]
    fn rejects_invalid_decision_policy() {
        let mut rules = GameRules::default();
        rules.decision.temperature = 0.0;
        assert!(rules.validate().is_err());
        rules.decision.temperature = 0.2;
        rules.decision.pass_lead_time_seconds = -0.1;
        assert!(rules.validate().is_err());
        rules.decision.pass_lead_time_seconds = f32::EPSILON;
        rules.decision.play_effect_weight = -f32::EPSILON;
        assert!(rules.validate().is_err());
    }

    #[test]
    fn decision_policy_round_trips_through_json_defaults() {
        let rules = GameRules::default();
        let mut value = serde_json::to_value(&rules).expect("rules serialize");
        let decision = value
            .get_mut("decision")
            .and_then(serde_json::Value::as_object_mut)
            .expect("decision rules serialize as an object");
        decision.remove("play_effect_weight");
        let decoded: GameRules = serde_json::from_value(value).expect("legacy rules deserialize");
        assert_eq!(decoded.decision.temperature, rules.decision.temperature);
        assert_eq!(
            decoded.decision.play_effect_weight,
            rules.decision.play_effect_weight
        );
    }

    #[test]
    fn rejects_invalid_numeric_envelopes() {
        let rules = GameRules {
            ball_max_speed_ftps: 0.0,
            ..GameRules::default()
        };
        assert!(rules.validate().is_err());

        let rules = GameRules {
            max_player_braking_accel_ftps2: 0.0,
            ..GameRules::default()
        };
        assert!(rules.validate().is_err());

        let rules = GameRules {
            max_player_lateral_accel_ftps2: f32::NAN,
            ..GameRules::default()
        };
        assert!(rules.validate().is_err());

        let rules = GameRules {
            court: crate::court::CourtGeometry {
                hoop_right_x_ft: GameRules::default().court.width_ft + 1.0,
                ..GameRules::default().court
            },
            ..GameRules::default()
        };
        assert!(rules.validate().is_err());
    }

    #[test]
    fn loose_ball_control_speed_is_validated_and_serialized() {
        let rules = GameRules::default();
        assert_eq!(rules.loose_ball_control_speed_ftps, 12.0);
        let value = serde_json::to_value(&rules).expect("rules serialize");
        let decoded: GameRules = serde_json::from_value(value).expect("rules deserialize");
        assert_eq!(
            decoded.loose_ball_control_speed_ftps,
            rules.loose_ball_control_speed_ftps
        );
        let invalid = GameRules {
            loose_ball_control_speed_ftps: 0.0,
            ..rules
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn rejects_inbound_tolerance_smaller_than_player_radius() {
        let rules = GameRules {
            inbound_boundary_tolerance_ft: 0.5,
            ..GameRules::default()
        };
        assert!(rules.validate().is_err());
    }

    #[test]
    fn rejects_non_finite_separation_margin() {
        let rules = GameRules {
            separation_safety_margin_ft: f32::NAN,
            ..GameRules::default()
        };
        assert!(rules.validate().is_err());
    }

    #[test]
    fn rejects_invalid_semantic_weight() {
        let mut rules = GameRules::default();
        rules.semantics.spacing_lane_weight = -0.1;
        assert!(rules.validate().is_err());
    }
}

#[cfg(test)]
mod modulation_tests {
    use super::{GameRules, ModulationRules};

    #[test]
    fn modulation_defaults_are_part_of_valid_rules() {
        assert!(GameRules::default().validate().is_ok());
    }

    #[test]
    fn modulation_rejects_zero_thresholds_and_negative_bias_policy() {
        let policy = ModulationRules {
            hot_hand_makes: 0,
            ..Default::default()
        };
        assert!(policy.validate().is_err());
        let policy = ModulationRules {
            trailing_pace_factor: -0.1,
            ..Default::default()
        };
        assert!(policy.validate().is_err());
    }

    #[test]
    fn unhandled_rule_fields_are_explicitly_documented() {
        // D27 周期实现因果闭环，未实现字段清单已清空
        assert!(super::UNIMPLEMENTED_RULE_FIELDS.is_empty());
    }
}

/// 显式标记未接入因果链的 GameRules 候选字段清单（D27 已全部闭环并清零）。
pub const UNIMPLEMENTED_RULE_FIELDS: &[&str] = &[];
