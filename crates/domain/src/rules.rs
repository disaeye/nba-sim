use crate::court::CourtGeometry;
use crate::projectile::ProjectileArc;
use crate::resolve::ResolveConfig;
use serde::{Deserialize, Serialize};

mod policies;

pub use policies::{
    CapabilityCurveRules, DecisionRules, DefenseRules, ModulationRules, PotentialFieldRules,
    RotationRules, ScreenDefenseRules, SemanticRules, TacticalRules,
};

/// Match rules and timing policy shared by the simulation subsystems.
///
/// The engine consumes this value rather than embedding competition-specific
/// numbers in phase transitions, constraints, or ball execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GameRules {
    pub tick_seconds: f32,
    /// 联赛规则档案（M10）：计时结构、进攻时钟、犯规政策、几何。
    /// 切换联赛 = 替换该档案，引擎零联赛分支（architecture.md 6.3）。
    pub league: crate::league::LeagueProfile,
    /// Estimated possessions per period used by abbreviated simulations.
    pub estimated_possessions_per_period: u32,
    pub inbound_seconds: f32,
    pub inbound_setup_seconds: f32,
    /// 底线/边线发球时接应人（safety）落位点距发球员的距离（ft）。
    /// 真实篮球发球必有球队安排一名球员回到发球员身边接球：没有
    /// 接应位时全队背对发球员拉开前场落位，发球只能选择 20~60ft
    /// 的高风险长传。0 关闭接应机制。
    pub inbound_safety_distance_ft: f32,
    pub backcourt_seconds: f32,
    pub period_break_seconds: f32,
    pub tactical_initiation_seconds: f32,
    pub decision_interval_seconds: f32,
    /// 发球阶段的决策间隔（秒）。发球受 5 秒规则约束，若沿用阵地进攻的
    /// `decision_interval_seconds`（2.4s），首次决策若被 Dwell 消耗，第二次
    /// 要等到 4.8s，加上取整余量即越过 5.0s 阈值——实测 37% 的发球因此被判
    /// 五秒违例。发球是「尽快把球发进场」的程序，不该套用阵地节奏。
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
    /// 篮球半径（ft，真实 9.4-9.5 英寸直径 ≈ 0.4 ft）：自由球与球员
    /// 身体碰撞的水平接触半径 = `player_radius_ft + 本值`。伸手接球的
    /// 判定半径（`defender_reach_ft`）是主动触碰（抢断/封盖）的口径，
    /// 不属于被动身体碰撞。
    pub ball_radius_ft: f32,
    /// 人体对球的被动碰撞半径（ft，真实躯干半宽约 1 ft）：区别于
    /// `player_radius_ft`（人-人分离半径 1.8 ft，含臂展）。被动碰撞
    /// 用单一人体半径，不区分腿/躯干（真实篮球里球碰腿与碰躯干都弹开，
    /// 半径差异是二阶量）。
    pub body_contact_radius_ft: f32,
    /// 地板球的「可控制速度上限」（ft/s）：球速超过该值时，5.8 ft
    /// 可及半径内的球员无法立即收下球（快球会从手上弹开），球继续
    /// 按物理飞行，直到撞到身体弹开或被减速后重新可收。真实篮球里
    /// 快速地板球几乎都会被拍/被碰弹开，只有慢球能被稳稳收下。
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
    /// 跳球点拍初速（ft/s）：中锋把球轻拨给接应队友的量级。
    /// 必须与收球门同源标定：球落到接应人身前反弹（×地面反弹
    /// 系数）后的水平速度须低于 `loose_ball_control_speed_ftps`，
    /// 否则球穿过接应人一路滚向底线（实测 28 ft/s 时球滚 45 ft）。
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
    /// 转身减速期间保留的速度比例基准（round-10 从内联 0.4 收编）。
    ///
    /// 实际值经 `capability::effective_turn_decel_retention` 按 `agility` 调制
    /// （attributes.md §2.2：「变向减速代价」是 agility 的法定消费链）。
    pub turn_decel_retention: f32,
    /// 调制后保留比例的下限（防止低敏捷者转向失速至停）。
    pub turn_decel_retention_floor: f32,
    /// 调制后保留比例的上限（防止高敏捷者转向零损失）。
    pub turn_decel_retention_ceiling: f32,
    pub contact_margin_ft: f32,
    /// 最大转体角速度（弧度/秒，真实人体转向速率上限）
    pub max_player_turn_rate_rad_per_sec: f32,
    /// 轴心脚允许微小滑动容差（呎，防止数值浮点误差触发走步）
    pub pivot_foot_tolerance_ft: f32,
    /// 地球标准重力加速度（呎/秒^2，用于自由球与弹跳抛物线计算）
    pub ball_gravity_ftps2: f32,
    /// 地板球滚动减速度（呎/秒^2）：木地板滚动的近似恒定摩擦。
    /// 旧实现用逐 tick 指数衰减（每 tick × retention），等效于
    /// 空气阻力模型——球被拍落后 0.3s 内就从 14 ft/s 爬行到
    /// 可收速度，争抢窗口消失（「自动送球到对方手上」的物理根源）。
    /// 恒定减速度让球滚 1~3 秒、10+ 呎，多人追抢成为可能。
    pub loose_ball_rolling_decel_ftps2: f32,
    pub defender_reach_ft: f32,
    pub pass_corridor_radius_ft: f32,
    /// 接球半径基准（ft）：接球人"控制圈"在**无技能加成**时的半径。
    ///
    /// 用于判断球到达时接球人是否**在物理上可能接到**（层 A，P-1）。
    /// 实际半径由 `capability::effective_catch_radius` 按
    /// `ball_handling` / `off_ball_sense` / 体格调制——本值仅作曲线基准，
    /// 不直接参与判定（charter C1：曲线形状参数走规则通道）。
    pub catch_radius_base_ft: f32,
    /// 接球技能对接球半径的调制幅度（ft）：`ball_handling` 从 0→1 的增量。
    pub catch_radius_skill_gain_ft: f32,
    /// 预估误差幅度（ft）：`off_ball_sense` 从 1→0 时接球人预估接球点的
    /// 额外误差上限。0 表示完全精确（当前行为）；>0 即实现 P-1「预估可能错」。
    pub receive_estimate_noise_ft: f32,
    /// 确定性误差向量的量化分母（用于把 16 位哈希切片映射到 [-1, 1]）。
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
    /// 「实质性越界」的最小超出距离（ft）。
    ///
    /// 球员目标点贴边时，物理 clamp 会每 tick 产生零点几英尺的差值；
    /// 把这种亚英尺级钳制当成越界事实，会让被顶在边线的持球人反复
    /// 被判出界（实测每场 42–52 次虚假失误）。只有超出该阈值的位移
    /// 才产生 `BoundaryCross`。
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
    /// 进攻方冲抢篮板的人数（charter C1：行为参数走数据通道，不内联在引擎里）。
    ///
    /// 真实篮球里不是全队都冲抢：全员扑向球的落点会立即触发
    /// `min_player_separation_ft` 碰撞消解，反而把所有人推离球的落点。
    /// evidence/problem.md §23.8 记录了指派前攻方几乎无人向球移动
    /// （0.0001 ft/tick，而守方 0.0042）。
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
            drive_early_finish_dist_ft: 4.5,
            drive_mid_range_pullup_dist_ft: 14.0,
            drive_kickout_pass_dist_ft: 22.0,
            drive_lane_offset_ft: 4.0,
            // 冲框走廊折扣：congestion 是全防守人的走廊投影和（篮下走廊
            // 通常 2–3），原 1.2 的折扣不足以让篮筐候选胜出 —— 实测 88%
            // 突破停在 14–18ft，篮下出手仅 1–2%（真实 30%）。提到 3.0
            // 使终结强者面对一般拥堵仍会攻框。
            drive_rim_attack_bias: 1.2,
            drive_finish_extend_ft: 6.0,
            drive_beaten_recovery_seconds: 0.6,
            // 突破停滞线（finish_range）：16ft 时实测 78 次/场的突破停滞在
            // 10–16ft 的脏区重新组织，篮下出手仅 1–2%（真实 NBA 30%）。
            // 收窄到 10ft：突破推进到 10ft 内就直接攻框（上篮/抛投），
            // 停滞只发生在防守把人卡在脏区之外的场合。
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
            defender_speed_ratio: 0.73,
            rebound_chase_speed_ratio: 0.73,
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
            // 转换进攻篮下终结窗口（秒，G6a 链 3）：回合前段防守未落位，
            // 突破攻框的效用加成只在此窗口内生效，避免把阵地战的攻框
            // 比例一并抬高（此前校准迭代 4 的教训：强抬攻框砸穿 3P% 带）。
            transition_finish_window_seconds: 6.0,
            // 转换期篮下终结的效用加成基准。
            transition_finish_bonus: 0.35,
            screen_hold_separation_ft: 6.0,
            screen_roll_separation_ft: 8.0,
            backdoor_cut_depth_ratio: 0.85,
            dip_to_rim_depth_ratio: 0.7,
            drop_coverage_depth_ft: 14.0,
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
            league: crate::league::LeagueProfile::nba(),
            estimated_possessions_per_period: 35,
            inbound_seconds: 5.0,
            inbound_setup_seconds: 2.2,
            inbound_safety_distance_ft: 8.0,
            backcourt_seconds: 8.0,
            period_break_seconds: 15.0,
            tactical_initiation_seconds: 6.5,
            decision_interval_seconds: 2.4,
            inbound_decision_interval_seconds: 0.4,
            free_throw_interval_seconds: 2.2,
            ball_max_speed_ftps: 85.0,
            min_pass_duration_seconds: 0.45,
            max_pass_duration_seconds: 1.4,
            min_shot_duration_seconds: 0.95,
            max_shot_duration_seconds: 1.45,
            decision: DecisionRules::default(),
            shot_peak_base_ft: 14.0,
            // 弧顶随距离的增长率（ft/ft）：ADR-017 曾计划校准为 0.04，
            // 实测后暂缓：0.04 使全部投篮提前约 0.065 s 到筐，8-seed
            // 三分封盖率 3.7%→7.3%（真实 NBA 约 2–3%）、3P% 中位数
            // 34.0→27.0 越出 [30,40] 带；出手时平均 make_probability
            // 三种配置完全一致（0.307/0.308），封盖概率的高度惩罚归零
            // 实验也不改变封盖数——机制未明，禁止盲目调参补救，
            // 保持 0.25 待封盖裁决与飞行时长的耦合查清后再动。
            shot_peak_distance_factor: 0.25,
            chest_height_ft: 4.0,
            rim_height_ft: 10.0,
            rim_radius_ft: 0.75,
            // 触筐物理（ADR-017 第二步）：接触点在近筐沿受控扇形上采样，
            // 反弹初速 = 入射镜像反射 × 恢复系数 + 受控散射，落点由抛体
            // 自然产生。恢复系数量级：真实篮球碰筐后水平余速约 0.4-0.6，
            // 竖直弹起约入射下落速度的 0.3-0.4。
            rim_contact_angle_spread_radians: 1.1,
            rim_contact_restitution_flush: 0.62,
            rim_contact_restitution_graze: 0.35,
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
            // D3.4 校准（dev 方案 §6.2）：紧逼窗口从最后 5s 扩到 12s。
            // 实测：5s 窗口下进攻方长期 Dwell 到 24s 违例（单场 53 次
            // SHOT_CLOCK_VIOLATION，真实 NBA ≈ 0–2 次），回合以违例而非
            // 出手告终。真实进攻在 24→14s 区间就已开始组织。
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
            pivot_foot_tolerance_ft: 0.35,
            ball_gravity_ftps2: 32.17,
            loose_ball_rolling_decel_ftps2: 6.0,
            defender_reach_ft: 4.0,
            pass_corridor_radius_ft: 3.5,
            catch_radius_base_ft: 2.0,
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
            // 体力动态标定（G6 换人链的前提）：实测全场速度档分布为
            // 冲刺 4% / 中速 31% / 慢速 63%（seed 42，在场逐 tick），且
            // 持球人冲刺占比显著高于均值。实测净耗率 ≈0.0029/s（0.033 时
            // 198s 即首换、70–88 次/队/场），按「每队 35–45 次」反推
            // 上场周期 ~5.3 分钟 → drain 0.021。
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

    /// 传球飞行时长由抛体解出（第一步飞行抛体化）：
    /// 弧顶 = `pass_peak_ft + dist × pass_peak_distance_factor`，
    /// 时长 = 升段 + 降段闭式解，再夹在动作窗口区间。
    /// 速度包络（`ball_max_speed_ftps`）作为校验上限：超限时削峰重解。
    pub fn pass_duration(&self, distance_ft: f32, inbound: bool) -> f32 {
        let distance = distance_ft.max(0.0);
        let g = self.ball_gravity_ftps2;
        let chest = self.chest_height_ft;
        // 速度包络下限：水平速度 = dist/T 不得超过球速包络。
        // 这条下限优先于弧顶解（实测回归：95 ft 发球长传在平抛 T=0.45 s 下
        // 水平速度 213 ft/s，是 BALL_SPEED Hard 的直接来源）；抬高弧顶只能
        // 减垂直分量，唯一能压水平速度的是延长时长。
        let t_envelope = distance / self.ball_max_speed_ftps.max(f32::EPSILON);
        let peak_base = if inbound {
            // 发球平快：弧顶贴胸口（入场传球不挑高弧），时长由包络下限抬。
            chest
        } else {
            (self.pass_peak_ft + distance * self.pass_peak_distance_factor).max(chest)
        };
        let peak = peak_base.max(chest + f32::EPSILON);
        let t_projectile = ProjectileArc::time_for_peak(chest, chest, peak, g);
        let t = t_projectile.max(t_envelope);
        t.clamp(
            self.min_pass_duration_seconds,
            self.max_pass_duration_seconds,
        )
    }

    pub fn inbound_expired(&self, elapsed: f32) -> bool {
        elapsed >= self.inbound_seconds
    }

    pub fn period_expired(&self, clock: f32) -> bool {
        clock <= 0.0
    }

    /// Validate the timing and geometry envelope before a run starts.
    ///
    /// Keeping this check next to the policy object prevents each caller from
    /// inventing a partial validation list. Deserialization callers (such as
    /// the debug server) can reject invalid policy explicitly, while the
    /// simulation constructor remains a cheap value constructor.
    pub fn validate(&self) -> Result<(), String> {
        self.resolve.validate()?;

        let finite = [
            self.tick_seconds,
            self.league.period_duration_seconds,
            self.league.overtime_duration_seconds,
            self.league.shot_clock_seconds,
            self.league.offensive_rebound_shot_clock_seconds,
            self.inbound_seconds,
            self.inbound_setup_seconds,
            self.backcourt_seconds,
            self.period_break_seconds,
            self.tactical_initiation_seconds,
            self.decision_interval_seconds,
            self.free_throw_interval_seconds,
            self.min_pass_duration_seconds,
            self.max_pass_duration_seconds,
            self.min_shot_duration_seconds,
            self.max_shot_duration_seconds,
            self.shot_peak_base_ft,
            self.shot_peak_distance_factor,
            self.chest_height_ft,
            self.rim_height_ft,
            self.rim_radius_ft,
            self.rim_contact_angle_spread_radians,
            self.rim_contact_restitution_flush,
            self.rim_contact_restitution_graze,
            self.rim_contact_vertical_restitution,
            self.rim_contact_scatter_radians,
            self.loose_ball_player_restitution,
            self.ball_radius_ft,
            self.body_contact_radius_ft,
            self.loose_ball_control_speed_ftps,
            self.tip_off_tap_speed_ftps,
            self.tip_off_tap_distance_ft,
            self.tip_off_tap_height_ft,
            self.tip_off_tap_vel_z_ftps,
            self.backboard_offset_from_baseline_ft,
            self.backboard_width_ft,
            self.backboard_bottom_height_ft,
            self.backboard_top_height_ft,
            self.backboard_restitution,
            self.pass_peak_ft,
            self.shot_clock_urgency_seconds,
            self.shot_contest_sensitivity,
            self.open_shot_distance_ft,
            self.shot_distance_reference_ft,
            self.rim_shot_distance_ft,
            self.inbound_boundary_tolerance_ft,
            self.inbound_release_depth_ft,
            self.free_throw_distance_ft,
            self.shot_pct_floor,
            self.shot_pct_ceiling,
            self.tip_off_duration_seconds,
            self.ball_bounce_base_ft,
            self.ball_bounce_amplitude_ft,
            self.ball_bounce_frequency_hz,
            self.contact_margin_ft,
            self.court.width_ft,
            self.court.height_ft,
            self.court.hoop_left_x_ft,
            self.court.hoop_right_x_ft,
            self.court.hoop_y_ft,
            self.player_radius_ft,
            self.min_player_separation_ft,
            self.separation_safety_margin_ft,
            self.max_player_speed_ftps,
            self.max_player_accel_ftps2,
            self.max_player_braking_accel_ftps2,
            self.max_player_lateral_accel_ftps2,
            self.turnaround_min_decel_seconds,
            self.player_linear_damping,
            self.ball_gravity_ftps2,
            self.ball_max_speed_ftps,
            self.defender_reach_ft,
            self.pass_corridor_radius_ft,
            self.pass_lane_clearance_reference_ft,
            self.flight_intercept_radius_ft,
            self.intercept_lane_radius_ft,
            self.rebound_outlet_fallback_distance_ft,
            self.teammate_density_radius_ft,
            self.teammate_density_capacity,
            self.court_side_margin_ratio,
            self.league.three_point_distance_ft,
            self.ball_holder_offset_ft,
            self.ball_holder_height_ft,
            self.stamina_sprint_speed_ftps,
            self.stamina_recovery_speed_ftps,
            self.stamina_drain_per_second,
            self.stamina_recovery_per_second,
            self.stamina_floor,
            self.stamina_exhausted_threshold,
            self.jump_shot_prep_seconds,
            self.jump_shot_exec_seconds,
            self.jump_shot_follow_seconds,
            self.pass_prep_seconds,
            self.pass_exec_seconds,
            self.pass_follow_seconds,
            self.rebound_prep_seconds,
            self.rebound_exec_seconds,
            self.rebound_follow_seconds,
            self.semantics.contact_minor_speed_ratio,
            self.semantics.contact_positional_speed_ratio,
            self.semantics.contact_foul_candidate_speed_ratio,
            self.semantics.screen_stationary_speed_ratio,
            self.semantics.spacing_corner_weight,
            self.semantics.spacing_weak_side_weight,
            self.semantics.spacing_lane_weight,
            self.semantics.spacing_paint_penalty,
            self.semantics.opponent_density_capacity,
            self.semantics.pass_base_probability,
            self.semantics.pass_openness_weight,
            self.semantics.pass_lane_risk_weight,
            self.semantics.density_distance_weight,
            self.semantics.minimum_entity_distance_ft,
            self.semantics.perimeter_side_margin_ratio,
            self.semantics.perimeter_depth_ratio,
            self.semantics.perimeter_density_capacity,
            self.semantics.strong_side_ball_weight,
            self.semantics.strong_side_defender_weight,
            self.semantics.weak_side_defender_weight,
            self.semantics.crowded_zone_threshold,
            self.semantics.dense_zone_threshold,
            self.semantics.crowded_zone_utility_penalty,
            self.semantics.crowded_zone_risk,
            self.semantics.dense_zone_utility_penalty,
            self.semantics.dense_zone_risk,
            self.semantics.heavily_contested_threshold,
            self.semantics.heavily_contested_utility_penalty,
            self.semantics.heavily_contested_risk,
            self.semantics.contested_threshold,
            self.semantics.contested_utility_penalty,
            self.semantics.contested_risk,
            self.semantics.open_shot_contest_threshold,
        ];
        if finite.iter().any(|value| !value.is_finite()) {
            return Err("rules contain a non-finite number".to_string());
        }
        if self.tick_seconds <= 0.0
            || self.league.period_duration_seconds <= 0.0
            || self.league.overtime_duration_seconds <= 0.0
            || self.league.shot_clock_seconds <= 0.0
            || self.league.offensive_rebound_shot_clock_seconds <= 0.0
            || self.inbound_seconds <= 0.0
            || self.tip_off_tap_speed_ftps <= 0.0
            || self.tip_off_tap_distance_ft <= 0.0
            || self.tip_off_tap_height_ft <= 0.0
            || self.tip_off_tap_vel_z_ftps <= 0.0
            || self.inbound_setup_seconds < 0.0
            || self.backcourt_seconds <= 0.0
            || self.period_break_seconds < 0.0
            || self.tactical_initiation_seconds < 0.0
            || self.decision_interval_seconds <= 0.0
            || self.free_throw_interval_seconds <= 0.0
            || self.league.regulation_periods == 0
            || self.estimated_possessions_per_period == 0
        {
            return Err("clock and period policy contains an invalid value".to_string());
        }
        if self.min_pass_duration_seconds <= 0.0
            || self.max_pass_duration_seconds < self.min_pass_duration_seconds
            || self.min_shot_duration_seconds <= 0.0
            || self.max_shot_duration_seconds < self.min_shot_duration_seconds
            || self.ball_max_speed_ftps <= 0.0
            || self.pass_peak_ft < 0.0
            || self.pass_peak_distance_factor < 0.0
            || self.chest_height_ft < 0.0
            || self.rim_height_ft <= self.chest_height_ft
        {
            return Err("ball timing policy contains an invalid value".to_string());
        }
        if self.court.width_ft <= 0.0
            || self.court.height_ft <= 0.0
            || self.court.hoop_left_x_ft <= 0.0
            || self.court.hoop_left_x_ft >= self.court.hoop_right_x_ft
            || self.court.hoop_right_x_ft >= self.court.width_ft
            || self.court.hoop_y_ft <= 0.0
            || self.court.hoop_y_ft >= self.court.height_ft
            || self.free_throw_distance_ft <= 0.0
            || self.free_throw_distance_ft >= self.court.width_ft
        {
            return Err("court geometry policy contains an invalid value".to_string());
        }
        if self.court.hoop_y_ft + self.player_radius_ft > self.court.height_ft
            || self.court.hoop_y_ft < self.player_radius_ft
        {
            return Err("hoop geometry leaves no playable margin".to_string());
        }
        if self.free_throw_distance_ft >= self.court.hoop_right_x_ft
            || self.free_throw_distance_ft >= self.court.width_ft - self.court.hoop_left_x_ft
        {
            return Err("free-throw line lies outside the court geometry".to_string());
        }
        if self.player_radius_ft <= 0.0
            || self.min_player_separation_ft < self.player_radius_ft * 2.0
            || self.separation_safety_margin_ft < 0.0
            || self.max_player_speed_ftps <= 0.0
            || self.max_player_accel_ftps2 <= 0.0
            || self.max_player_braking_accel_ftps2 <= 0.0
            || self.max_player_lateral_accel_ftps2 <= 0.0
            || self.player_linear_damping < 0.0
            || self.defender_reach_ft < 0.0
            || self.pass_corridor_radius_ft < 0.0
            || self.catch_radius_base_ft <= 0.0
            || self.catch_radius_skill_gain_ft < 0.0
            || self.receive_estimate_noise_ft < 0.0
            || self.pass_lane_clearance_reference_ft <= 0.0
            || self.flight_intercept_radius_ft <= 0.0
            || self.intercept_lane_radius_ft < self.flight_intercept_radius_ft
            || self.rebound_outlet_fallback_distance_ft < 0.0
            || self.teammate_density_radius_ft <= 0.0
            || self.teammate_density_capacity <= 0.0
            || !(0.0..=0.5).contains(&self.court_side_margin_ratio)
        {
            return Err("physical policy contains an invalid value".to_string());
        }
        if !(0.0..=1.0).contains(&self.semantics.contact_minor_speed_ratio)
            || !(0.0..=1.0).contains(&self.semantics.contact_positional_speed_ratio)
            || !(0.0..=1.0).contains(&self.semantics.contact_foul_candidate_speed_ratio)
            || !(0.0..=1.0).contains(&self.semantics.screen_stationary_speed_ratio)
            || self.semantics.spacing_corner_weight < 0.0
            || self.semantics.spacing_weak_side_weight < 0.0
            || self.semantics.spacing_lane_weight < 0.0
            || self.semantics.spacing_paint_penalty < 0.0
            || self.semantics.opponent_density_capacity <= 0.0
            || self.semantics.pass_base_probability < 0.0
            || self.semantics.pass_base_probability > 1.0
            || self.semantics.pass_openness_weight < 0.0
            || self.semantics.pass_lane_risk_weight < 0.0
            || self.semantics.density_distance_weight < 0.0
            || self.semantics.minimum_entity_distance_ft < 0.0
            || !(0.0..=0.5).contains(&self.semantics.perimeter_side_margin_ratio)
            || !(0.0..=1.0).contains(&self.semantics.perimeter_depth_ratio)
            || self.semantics.perimeter_density_capacity <= 0.0
            || self.semantics.strong_side_ball_weight < 0.0
            || self.semantics.strong_side_defender_weight < 0.0
            || self.semantics.weak_side_defender_weight < 0.0
            || !(0.0..=1.0).contains(&self.semantics.dense_zone_threshold)
            || !(0.0..=1.0).contains(&self.semantics.crowded_zone_threshold)
            || self.semantics.crowded_zone_threshold < self.semantics.dense_zone_threshold
            || self.semantics.crowded_zone_utility_penalty < 0.0
            || self.semantics.crowded_zone_risk < 0.0
            || self.semantics.dense_zone_utility_penalty < 0.0
            || self.semantics.dense_zone_risk < 0.0
            || !(0.0..=1.0).contains(&self.semantics.heavily_contested_threshold)
            || self.semantics.heavily_contested_utility_penalty < 0.0
            || self.semantics.heavily_contested_risk < 0.0
            || !(0.0..=1.0).contains(&self.semantics.contested_threshold)
            || self.semantics.contested_utility_penalty < 0.0
            || self.semantics.contested_risk < 0.0
            || !(0.0..=1.0).contains(&self.semantics.open_shot_contest_threshold)
        {
            return Err("semantic policy contains an invalid value".to_string());
        }
        if !(0.0..=1.0).contains(&self.shot_pct_floor)
            || !(0.0..=1.0).contains(&self.shot_pct_ceiling)
            || self.shot_pct_floor > self.shot_pct_ceiling
            || self.shot_contest_sensitivity < 0.0
            || self.open_shot_distance_ft < 0.0
            || self.shot_distance_reference_ft <= 0.0
            || self.rim_shot_distance_ft < 0.0
            || self.league.three_point_distance_ft <= self.rim_shot_distance_ft
        {
            return Err("shot percentage and distance bounds are invalid".to_string());
        }
        if self.stream_max_bytes == 0
            || self.stream_frames_max_bytes == 0
            || self.stream_max_ticks == 0
        {
            return Err("stream byte/tick budgets must be positive".to_string());
        }
        if !(0.0..=1.0).contains(&self.separation_correction_share)
            || self.separation_correction_share == 0.0
        {
            return Err("separation correction share must be in (0, 1]".to_string());
        }
        if !(0.0..=1.0).contains(&self.transfer_landing_leash_ratio)
            || self.transfer_landing_leash_ratio == 0.0
        {
            return Err("transfer landing leash ratio must be in (0, 1]".to_string());
        }
        if !(0.0..=1.0).contains(&self.tactics.initiation_distance_ratio)
            || self.tactics.action_duration_seconds <= 0.0
            || !(0.0..=1.0).contains(&self.tactics.drive_distance_ratio)
            || self.tactics.drive_min_duration_seconds <= 0.0
            || self.tactics.drive_max_duration_seconds < self.tactics.drive_min_duration_seconds
            || self.tactics.drive_speed_ratio <= 0.0
            || self.tactics.drive_early_finish_dist_ft <= 0.0
            || self.tactics.drive_mid_range_pullup_dist_ft
                <= self.tactics.drive_early_finish_dist_ft
            || self.tactics.drive_lane_offset_ft < 0.0
            || self.tactics.drive_rim_attack_bias < 0.0
            || self.tactics.drive_beaten_recovery_seconds < 0.0
            || self.tactics.drive_finish_range_ft <= 0.0
            || self.tactics.drive_dunk_max_dist_ft <= 0.0
            || self.tactics.drive_floater_min_dist_ft <= self.tactics.drive_dunk_max_dist_ft
            || !(0.0..=1.0).contains(&self.tactics.drive_dunk_min_finishing)
            || !(0.0..=1.0).contains(&self.tactics.drive_dunk_max_lane_density)
            || !(0.0..=1.0).contains(&self.tactics.drive_kickout_max_crowding)
            || self.tactics.drive_kickout_min_defender_dist_ft <= 0.0
            || !(0.0..=1.0).contains(&self.tactics.drive_pullup_min_crowding)
            || self.tactics.drive_decision_check_interval_seconds <= 0.0
            || !(0.0..=1.0).contains(&self.tactics.screen_distance_ratio)
            || self.tactics.defensive_gap_ft < 0.0
            || !(0.0..=1.0).contains(&self.tactics.help_sag_ratio)
            || !(0.0..=1.0).contains(&self.tactics.transition_speed_ratio)
            || !(0.0..=1.0).contains(&self.tactics.carrier_speed_ratio)
            || !(0.0..=1.0).contains(&self.tactics.screener_speed_ratio)
            || !(0.0..=1.0).contains(&self.tactics.support_speed_ratio)
            || !(0.0..=1.0).contains(&self.tactics.defender_speed_ratio)
            || !(0.0..=1.0).contains(&self.tactics.rebound_chase_speed_ratio)
            || !(0.0..=1.0).contains(&self.tactics.transition_defense_threshold_ratio)
            || !(0.0..=1.0).contains(&self.tactics.transition_sprint_ratio)
            || self.tactics.screen_hold_separation_ft <= 0.0
            || self.tactics.screen_roll_separation_ft <= 0.0
            || self.tactics.drop_coverage_depth_ft <= 0.0
        {
            return Err("tactical movement policy contains an invalid value".to_string());
        }
        if self.rim_radius_ft <= 0.0
            || self.rim_contact_angle_spread_radians < 0.0
            || self.rim_contact_angle_spread_radians > std::f32::consts::PI
            || self.rim_contact_restitution_flush < 0.0
            || self.rim_contact_restitution_flush > 1.0
            || self.rim_contact_restitution_graze < 0.0
            || self.rim_contact_restitution_graze > self.rim_contact_restitution_flush
            || self.rim_contact_vertical_restitution < 0.0
            || self.rim_contact_vertical_restitution > 1.0
            || self.rim_contact_scatter_radians < 0.0
            || self.rim_contact_scatter_radians > std::f32::consts::PI
            || self.loose_ball_player_restitution < 0.0
            || self.loose_ball_player_restitution > 1.0
            || self.loose_ball_control_speed_ftps <= 0.0
            || !self.loose_ball_control_speed_ftps.is_finite()
            || self.backboard_width_ft <= 0.0
            || self.backboard_bottom_height_ft >= self.backboard_top_height_ft
            || self.backboard_offset_from_baseline_ft <= 0.0
            || self.backboard_offset_from_baseline_ft
                >= self.court.width_ft - self.backboard_offset_from_baseline_ft
            || !(0.0..=1.0).contains(&self.backboard_restitution)
            || self.inbound_boundary_tolerance_ft < self.player_radius_ft
            || self.inbound_release_depth_ft <= 0.0
            || self.stamina_floor < 0.0
            || self.stamina_floor > 1.0
            || self.stamina_exhausted_threshold < self.stamina_floor
            || self.stamina_exhausted_threshold > 1.0
            || self.stamina_sprint_speed_ftps < 0.0
            || self.stamina_recovery_speed_ftps < 0.0
            || self.stamina_drain_per_second < 0.0
            || self.stamina_recovery_per_second < 0.0
            || self.ball_bounce_base_ft < 0.0
            || self.ball_bounce_amplitude_ft < 0.0
            || self.ball_bounce_frequency_hz < 0.0
            || self.ball_holder_offset_ft < 0.0
            || self.ball_holder_height_ft < 0.0
            || self.jump_shot_prep_seconds < 0.0
            || self.jump_shot_exec_seconds < 0.0
            || self.jump_shot_follow_seconds < 0.0
            || self.pass_prep_seconds < 0.0
            || self.pass_exec_seconds < 0.0
            || self.pass_follow_seconds < 0.0
            || self.rebound_prep_seconds < 0.0
            || self.rebound_exec_seconds < 0.0
            || self.rebound_follow_seconds < 0.0
            || self.rebound_crash_offense_count == 0
            || self.rebound_boxout_defense_count == 0
        {
            return Err(
                "stamina, rebound, and action policy contains an invalid value".to_string(),
            );
        }
        self.modulation.validate()?;
        self.rotation.validate()?;
        self.decision.validate()?;
        Ok(())
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
