//! Play 动作词表的物理与动作窗口语义 + 谓词求值的引擎侧装配
//! （tactics.md §2.2.4、ADR-019、plan_play.md #14 / 阶段 2）。
//!
//! 动词（`PlayVerb`）映射到目标生成偏置与动作窗口类型
//! （`domain::action_window` 的 `ActionType`）；动词不直接执行动作，
//! 只改变目标与候选语境（TA8）：本模块产出的是「参考点 + 偏置」形式的
//! 候选目标与窗口类型标签，真正构造 `ActionTimeWindow`、写入
//! `TargetAssignment` 由消费方（规则评估执行器，plan_play.md #15）完成。
//!
//! 谓词装配：[`build_selection_context`] 把引擎世界快照装配成
//! [`PlaySelectionContext`]（`play_selector` 定义的求值输入端口）。
//! 几何谓词（`beyond_circle_ft` / `screen_established` /
//! `corner_occupied`）全部从位置快照派生，距离与区域口径与既有实现
//! 同源（ADR-015 单一事实源）；场输出谓词（`help_shading_off` /
//! `weak_side_vacated`）只透传 [`StableFieldOutput`] 的滞回稳定值，
//! 原始 `threat_ratio` / `void_ratio` 抖动量不得进入本模块
//! （tactics.md §2.5）。

use glam::Vec2;
use nba_domain::action_window::ActionType;
use nba_domain::court::CourtGeometry;
use nba_domain::play::PlayVerb;
use nba_domain::GameRules;

use crate::play_selector::PlaySelectionContext;
use crate::potential_field::StableFieldOutput;

// ---- 偏移量级参数（集中声明）----
//
// 以下量级是本任务的合理默认值，属于**待校准量**：动词消费方接线时
// 应把它们迁入 GameRules/DecisionRules 数据通道（charter C1），届时
// 这里只保留规则缺省值的引用。迁移之前，任何调参都必须改这里，
// 禁止在调用点内联新数值。

/// ScreenRoll 顺下深度（ft）：从掩护点朝篮筐方向的跟进步长。
const SCREEN_ROLL_DEPTH_FT: f32 = 8.0;
/// ScreenPop 外弹目标：沿「篮筐 → 掩护点」射线补齐到三分线半径
/// （缺口 = `three_point_distance_ft` − 当前距篮距离），不引入独立步长。
/// SpotUp 就地微调步长（ft）：朝篮筐方向的小步站位修正。
const SPOT_UP_NUDGE_FT: f32 = 2.0;
/// Relocate 弱侧转移步长上限（ft）：朝弱侧空档锚点方向，至多移动该距离。
const RELOCATE_STEP_FT: f32 = 10.0;
/// CutBackdoor 背切步长（ft）：朝篮筐并偏向对位防守人远侧。
const BACKDOOR_DEPTH_FT: f32 = 6.0;
/// Lift 上提步长上限（ft）：朝弧顶方向的上提是一段移动，至多移动该距离
/// （低位到弧顶全程可达 25 ft 以上，真实篮球里上提接应停在肘区/罚球线
/// 一带，不必走到弧顶）。
const LIFT_MAX_STEP_FT: f32 = 16.0;
/// 偏置量级的健全性上限（ft）：任何动词的偏置都不得超过该场上尺度。
const OFFSET_SANITY_CAP_FT: f32 = 30.0;

/// 动词解析结果：目标点偏置语义 + 动作窗口类型。
///
/// `target_offset` 用绝对偏置向量（`Vec2`）表达：偏置本身已是纯几何
/// 结果，枚举加参数只是把同一几何多包一层；向量可以直接加到参考点上，
/// 也便于方向（点积）与量级（模长）两类测试。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VerbResolution {
    /// 相对参考点（动词作用对象当前位置）的目标偏置。
    pub target_offset: Vec2,
    /// 对应的动作窗口类型（`ActionType` 子集）。`None` 表示该动词是
    /// 无球移动语义，不预置出手窗口；执行窗口由到达后的效用管线决定。
    pub window_action: Option<ActionType>,
}

/// 动词解析的语境输入（参考点与几何事实由调用方提供，本函数算纯几何）。
///
/// 朝向不单列：各动词的侧别与方向全部由下列位置互相推出（篮筐、
/// 持球人、防守人的相对方位），弱侧判定与 `potential_field` 使用同一
/// 中轴口径。
#[derive(Debug, Clone, Copy)]
pub struct VerbContext {
    /// 动词作用对象（目标槽位球员）当前位置：偏移参考点。
    pub actor_pos: Vec2,
    /// 进攻篮筐位置（`CourtGeometry::hoop_pos(is_home)`）。
    pub hoop_pos: Vec2,
    /// 持球人位置（弱侧判定参照）。
    pub carrier_pos: Vec2,
    /// 动词作用对象的对位防守人位置；`None` 表示无对位信息
    /// （CutBackdoor 退化为直指篮筐）。
    pub defender_pos: Option<Vec2>,
    /// 球场几何（clamp 与弧顶方向）。
    pub court: CourtGeometry,
    /// 目标收界边距（ft），来源：`GameRules::player_radius_ft`
    /// （与 tactics.rs 槽位目标 clamp 同一口径）。
    pub clamp_margin_ft: f32,
    /// 三分线距离（ft），来源：`GameRules::league.three_point_distance_ft`
    /// （ScreenPop 补齐半径与 Lift 弧顶方向用）。
    pub three_point_distance_ft: f32,
}

/// 解析动词：返回目标偏置与动作窗口类型（纯函数，无状态）。
///
/// 偏置语义：调用方把 `target_offset` 加到 `ctx.actor_pos` 得到候选目标
/// 点，再用 `CourtGeometry::clamp_playable` 收界（[`resolve_verb_target`]
/// 即该语义的参考实现）；本函数只产出偏置，不产出绝对坐标。
///
/// 窗口类型映射（每个动词的理由见各分支注释）：
///
/// | 动词 | 偏置方向 | 窗口类型 |
/// | --- | --- | --- |
/// | ScreenRoll | 朝篮筐 | `None`（冲筐移动，终结方式到达后由效用管线决定） |
/// | ScreenPop | 沿射线补齐到三分线 | `JumpShot`（三分接球跳投） |
/// | SpotUp | 朝篮筐微调 | `JumpShot`（接球跳投） |
/// | Relocate | 朝弱侧空档锚点 | `JumpShot`（空档接球跳投） |
/// | CutBackdoor | 朝篮筐偏防守人远侧 | `None`（冲筐移动，同 ScreenRoll） |
/// | Lift | 朝弧顶上提 | `None`（中转接应，接球后可再传或面框） |
pub fn resolve_verb(verb: PlayVerb, ctx: &VerbContext) -> VerbResolution {
    debug_assert!(
        ctx.three_point_distance_ft.is_finite() && ctx.three_point_distance_ft > f32::from(0u8),
        "three_point_distance_ft must be positive and finite"
    );
    let resolution = match verb {
        PlayVerb::ScreenRoll => {
            // 顺下：从掩护点朝篮筐方向跟进。冲筐移动，不预置出手窗口。
            let to_hoop = (ctx.hoop_pos - ctx.actor_pos).normalize_or_zero();
            VerbResolution {
                target_offset: to_hoop * SCREEN_ROLL_DEPTH_FT,
                window_action: None,
            }
        }
        PlayVerb::ScreenPop => {
            // 外弹：沿「篮筐 → 掩护点」射线补齐到三分线。外弹的语义终点
            // 是三分接球跳投，预置 JumpShot；JumperKind::CatchAndShoot
            // 的细化留给执行器（JumpShot 构造器与 JumperKind 正交）。
            // 已在弧外时偏置为零。
            let radial = ctx.actor_pos - ctx.hoop_pos;
            let gap = (ctx.three_point_distance_ft - radial.length()).max(f32::from(0u8));
            VerbResolution {
                target_offset: radial.normalize_or_zero() * gap,
                window_action: Some(ActionType::JumpShot),
            }
        }
        PlayVerb::SpotUp => {
            // 定点落位：就地微调——朝篮筐方向的小步站位修正（站定等
            // 接球出手，窗口语义与外弹相同）。
            let to_hoop = (ctx.hoop_pos - ctx.actor_pos).normalize_or_zero();
            VerbResolution {
                target_offset: to_hoop * SPOT_UP_NUDGE_FT,
                window_action: Some(ActionType::JumpShot),
            }
        }
        PlayVerb::Relocate => {
            // 弱侧转移：朝弱侧空档锚点方向移动（空档 = 翼位接球跳投位）。
            let to_anchor = weak_side_anchor(ctx) - ctx.actor_pos;
            let step = to_anchor.length().min(RELOCATE_STEP_FT);
            VerbResolution {
                target_offset: to_anchor.normalize_or_zero() * step,
                window_action: Some(ActionType::JumpShot),
            }
        }
        PlayVerb::CutBackdoor => {
            // 背切：朝篮筐切入，同时偏向对位防守人的远侧（把防守人挡在
            // 身后）；无对位信息时直指篮筐。冲筐移动，不预置出手窗口。
            let to_hoop = (ctx.hoop_pos - ctx.actor_pos).normalize_or_zero();
            let lane = match ctx.defender_pos {
                Some(defender_pos) => {
                    let perp = Vec2::new(-to_hoop.y, to_hoop.x);
                    let away = if perp.dot(defender_pos - ctx.actor_pos) > f32::from(0u8) {
                        -perp
                    } else {
                        perp
                    };
                    (to_hoop + away).normalize_or_zero()
                }
                None => to_hoop,
            };
            VerbResolution {
                target_offset: lane * BACKDOOR_DEPTH_FT,
                window_action: None,
            }
        }
        PlayVerb::Lift => {
            // 上提接应：低位球员朝弧顶方向上提（弧顶点 = 篮筐沿「朝中圈」
            // 方向退到三分线半径处），步长封顶。接应是中转动作，不预置
            // 出手窗口。
            let to_center = Vec2::new(
                (ctx.court.center().x - ctx.hoop_pos.x).signum(),
                f32::from(0u8),
            );
            let arc_top = ctx.hoop_pos + to_center * ctx.three_point_distance_ft;
            let to_arc = arc_top - ctx.actor_pos;
            let step = to_arc.length().min(LIFT_MAX_STEP_FT);
            VerbResolution {
                target_offset: to_arc.normalize_or_zero() * step,
                window_action: None,
            }
        }
    };
    debug_assert!(
        resolution.target_offset.length() <= OFFSET_SANITY_CAP_FT,
        "verb `{}` offset {:.3} ft exceeds the sanity cap {OFFSET_SANITY_CAP_FT} ft",
        verb.as_str(),
        resolution.target_offset.length()
    );
    resolution
}

/// 解析动词并给出收界后的候选目标点（参考实现：参考点 + 偏置后经
/// `clamp_playable` 收界，与 tactics.rs 槽位目标同一收界口径，目标点
/// 恒为场上可站立位置）。
pub fn resolve_verb_target(verb: PlayVerb, ctx: &VerbContext) -> Vec2 {
    debug_assert!(ctx.clamp_margin_ft.is_finite() && ctx.clamp_margin_ft >= f32::from(0u8));
    let offset = resolve_verb(verb, ctx).target_offset;
    ctx.court
        .clamp_playable(ctx.actor_pos + offset, ctx.clamp_margin_ft)
}

/// 弱侧空档锚点：篮筐正横向、弱侧四分之一场高处的翼位空档。
///
/// 弱侧判定复用 `potential_field.rs` 的中轴口径：持球人居中（距中轴
/// 不足 1 ft）时按「向 +y 让出底角侧」的同一规则处理，动词的弱侧方向
/// 与势能场的弱侧标签保持一致。
fn weak_side_anchor(ctx: &VerbContext) -> Vec2 {
    let mid_y = ctx.hoop_pos.y;
    let ref_carrier_y = if (ctx.carrier_pos.y - mid_y).abs() < f32::from(1u8) {
        mid_y + f32::from(1u8)
    } else {
        ctx.carrier_pos.y
    };
    let weak_sign = -(ref_carrier_y - mid_y).signum();
    Vec2::new(
        ctx.hoop_pos.x,
        mid_y + weak_sign * ctx.court.height_ft / f32::from(4u8),
    )
}

/// 引擎世界快照：[`build_selection_context`] 的输入。
///
/// 全部字段由调用方（引擎决策侧）从当 tick 世界状态拷贝；本模块不持有
/// 世界对象引用（ADR-015 边界不变）。
#[derive(Debug, Clone)]
pub struct EngineWorldInputs {
    /// 持球人在进攻方数组中的索引；持球人位置经 [`Self::carrier_pos`]
    /// 从位置数组派生，避免同一坐标双份拷贝失配。
    pub carrier_idx: usize,
    /// 进攻方 5 名球员的位置快照（ft）。
    pub off_positions: Vec<Vec2>,
    /// 进攻方 5 名球员的速度快照（ft/s，与位置同序；掩护人站定判定用）。
    pub off_velocities: Vec<Vec2>,
    /// 进攻篮筐位置（`CourtGeometry::hoop_pos(is_home)`）。
    pub hoop_pos: Vec2,
    /// 球场几何（底角区域判定）。
    pub court: CourtGeometry,
    /// 进攻时钟剩余秒数（比赛时钟，球态谓词 `shot_clock_urgent` 输入）。
    pub shot_clock_seconds: f32,
    /// 本队持球且评估对象是持球人（球态谓词 `carrier_possed`）。
    pub carrier_possed: bool,
    /// 前场阵地战（`SubPhase::ActionExecution`，非快攻；球态谓词
    /// `halfcourt_possession`）。
    pub halfcourt: bool,
    /// 本回合已进行 tick 数（调用方簿记透传）。
    pub possession_ticks: u32,
    /// 场输出稳定值：由调用方从 `DefenseHysteresisState` 经
    /// `observe_field` / `snapshot` 取得（滞回通道，tactics.md §2.5）。
    pub stable_field: StableFieldOutput,
}

impl EngineWorldInputs {
    /// 持球人位置（位置数组按持球人索引取值）。
    pub fn carrier_pos(&self) -> Vec2 {
        self.off_positions[self.carrier_idx]
    }
}

/// 从引擎世界快照装配选板上下文（几何谓词派生 + 场输出稳定值透传）。
///
/// - `beyond_circle_ft`：`carrier_dist_to_hoop_ft` 由持球人与篮筐位置
///   派生，谓词半径 `r` 由谓词自身携带，选板器逐谓词比较；
/// - `screen_established` / `corner_occupied`：口径见
///   [`screen_established`] / [`corner_occupancy`]；
/// - 场输出谓词：`StableFieldOutput` 两个 bool 直接透传。
///
/// fast-fail：位置与速度数组长度不一致、持球人索引越界或数组为空时
/// 就地崩溃（调用方 bug，不静默回退）。
pub fn build_selection_context(
    inputs: &EngineWorldInputs,
    rules: &GameRules,
) -> PlaySelectionContext {
    assert!(
        !inputs.off_positions.is_empty(),
        "off_positions must not be empty"
    );
    assert_eq!(
        inputs.off_positions.len(),
        inputs.off_velocities.len(),
        "off_positions and off_velocities must have the same length"
    );
    assert!(
        inputs.carrier_idx < inputs.off_positions.len(),
        "carrier_idx {} out of bounds ({})",
        inputs.carrier_idx,
        inputs.off_positions.len()
    );

    let dist_to_hoop = (inputs.carrier_pos() - inputs.hoop_pos).length();
    let (corner_left, corner_right) = corner_occupancy(inputs);

    PlaySelectionContext {
        possession_ticks: inputs.possession_ticks,
        shot_clock_seconds: inputs.shot_clock_seconds,
        tick_seconds: rules.tick_seconds,
        carrier_possed: inputs.carrier_possed,
        halfcourt: inputs.halfcourt,
        carrier_dist_to_hoop_ft: dist_to_hoop,
        screen_established: screen_established(inputs, rules),
        corner_left_occupied: corner_left,
        corner_right_occupied: corner_right,
        help_shading_off_stable: inputs.stable_field.help_pulled_off,
        weak_side_vacated_stable: inputs.stable_field.weak_side_vacant,
    }
}

/// `screen_established` 装配：存在「处于掩护判定半径内且站定」的队友。
///
/// 口径来源（复用既有实现，不新造几何）：
/// - 距离阈值：`rules.tactics.defense.potential_field.screen_detection_radius_ft`
///   （tactics.rs 的 `is_screening_action` 同源）；
/// - 「站定」：速度模长 ≤ `max_player_speed_ftps ×
///   semantics.screen_stationary_speed_ratio`（semantics/lib.rs 合法掩护
///   判定的同一口径）。
///
/// 「最近掩护人」的实现口径：站定者才可能是掩护人，因此判定为任一
/// 队友（除持球人）位于半径内且站定；最近的移动队友不会掩盖真正的
/// 站定掩护人。
fn screen_established(inputs: &EngineWorldInputs, rules: &GameRules) -> bool {
    let field_rules = &rules.tactics.defense.potential_field;
    let stationary_limit =
        rules.max_player_speed_ftps * rules.semantics.screen_stationary_speed_ratio;
    let carrier_pos = inputs.carrier_pos();
    inputs
        .off_positions
        .iter()
        .enumerate()
        .any(|(idx, mate_pos)| {
            idx != inputs.carrier_idx
                && (*mate_pos - carrier_pos).length() < field_rules.screen_detection_radius_ft
                && inputs.off_velocities[idx].length() <= stationary_limit
        })
}

/// 左右底角占用装配。底角区域完全由 court 几何定义，不新增阈值：
///
/// - 边线带：距边线不足 `CourtGeometry::corner_zone_depth_ft`（court.rs
///   的底角三分线标线深度，默认 3 ft，随场地等比缩放）；
/// - 进攻半场：与 `CourtGeometry::is_three_point_attempt` 的
///   `in_attacking_half` 同一口径（底角线只存在于前场）。
///
/// 侧别约定：`Left` = y 较低一侧，`Right` = y 较高一侧。
/// 本队球员含持球人（持球人落底角同样是空间事实）。
fn corner_occupancy(inputs: &EngineWorldInputs) -> (bool, bool) {
    let depth = inputs.court.corner_zone_depth_ft();
    let attacking_right = inputs.hoop_pos.x > inputs.court.center().x;
    let in_attacking_half = |pos: Vec2| {
        if attacking_right {
            pos.x >= inputs.court.width_ft / f32::from(2u8)
        } else {
            pos.x <= inputs.court.width_ft / f32::from(2u8)
        }
    };
    let mut left = false;
    let mut right = false;
    for pos in &inputs.off_positions {
        if !in_attacking_half(*pos) {
            continue;
        }
        if pos.y <= depth {
            left = true;
        } else if pos.y >= inputs.court.height_ft - depth {
            right = true;
        }
    }
    (left, right)
}
