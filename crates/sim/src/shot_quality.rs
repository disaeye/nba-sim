//! 投篮模型：价值层与执行层共享同一套几何命中定义。
//!
//! `ShotModel` 从投篮类型、持球阶段、出手位置、距离、移动、收球、防守人
//! 预计到达时刻和三维手部关系出发，给出瞄准误差、有效筐口、命中概率与
//! 封盖风险。命中概率由瞄准误差落进有效筐口的概率积分得到；执行层用同一
//! 个标准差抽样瞄准点，并用同一个有效筐口判定穿筐，因此估值和实际飞行
//! 不再各有一套命中率。
//!
//! 参数按职责分组，各组独立拟合、互不串写：
//!
//! - 瞄准误差：`shot_quality_base_*`、`shot_quality_skill_*`、
//!   `shot_aim_sigma_scale`、`shot_aim_movement_sensitivity`
//! - 距离衰减：`shot_distance_decay_three_scale_ft`、
//!   `shot_distance_decay_three_power`、`shot_aim_distance_scale`
//! - 防守干扰：`shot_quality_contest_sensitivity`
//! - 动作阶段：`shot_phase_sigma_*`、`shot_gather_sigma_scale`
//! - 决策温度：`temperature_min`、`temperature_max`、`temperature_fatigue_discount`
//! - 行为偏好：其余 policy 权重

use glam::Vec2;
use nba_domain::projectile::ProjectileArc;

use crate::action::ShotKind;
use crate::resolve::shooter_zone;
use crate::state::{BallPhase, DecisionState, PossessionKind};
use crate::world::World;

/// 对数与除法使用的最小量。命中目标趋近于零时瞄准误差发散，但不发散成
/// `NaN`；概率本身取零，不在这里顶一个下界。
const MIN_LOG_ARGUMENT: f32 = 1.0e-6;

/// 有效筐口小于这个值时不存在干净穿筐的可能。
const MIN_APERTURE_FT: f32 = 1.0e-4;

/// 瞄准误差乘子的下界：任何加成都不能把偏度压到零，否则一次出手就变成必进。
const MIN_SIGMA_FACTOR: f32 = 0.25;

/// 条件命中率的上界。净目标除以存活概率后可能略高于 1，飞行的瞄准
/// 误差不能因此变成零。
const MAX_ARC_PROBABILITY: f32 = 0.995;

/// 存活概率的下界，只在封盖风险与犯规风险同时逼近 1 时起作用。
const MIN_SURVIVE: f32 = 1.0e-3;

/// 干净穿筐概率的下界：触筐补进已经解释掉目标里的大部分，剩下的干净
/// 穿筐部分不能反向落到零，否则瞄准误差会发散。
const MIN_ARC_PROBABILITY: f32 = 1.0e-4;

/// 给定有效筐口与穿筐概率，反解落点分散程度：球心的落点偏移服从
/// Rayleigh 分布，半径小于筐口的概率就是 1 - exp(-r²/(2σ²))。
fn solve_sigma(aperture: f32, swish: f32) -> f32 {
    let log_argument = (-2.0 * (1.0 - swish.clamp(MIN_ARC_PROBABILITY, 1.0 - MIN_ARC_PROBABILITY)).ln())
        .max(MIN_LOG_ARGUMENT);
    (aperture / log_argument.sqrt()).max(MIN_LOG_ARGUMENT)
}

#[derive(Debug, Clone, Copy)]
pub struct ShotModel {
    /// 净命中率：扣掉封盖与「犯规不中」两种分支之后的命中率，价值层与
    /// 决策层都用它。
    pub p_make: f32,
    /// 条件命中率：球已出手、没有封盖也没有被吹掉时的穿筐概率，也就是
    /// 瞄准误差的积分结果。执行层用它抽样瞄准点。
    pub p_make_cond: f32,
    pub sigma_ft: f32,
    pub effective_aperture_ft: f32,
    /// 筐口外一个球径带：落点落在这个环带里才算碰到筐沿。
    pub contact_radius_ft: f32,
    /// 碰到筐沿之后仍能补进的概率。
    pub rim_in_probability: f32,
    pub release_z_ft: f32,
    pub release_delay_seconds: f32,
    pub contest: f32,
    /// 出手瞬间的移动程度：球员速度除以最大速度，0 表示站定、1 表示全速。
    pub movement: f32,
    /// 出手人的区域投篮属性，原样带出来供标定统计使用。
    pub skill: f32,
    pub block_risk: f32,
    /// 这次出手最可能被谁封盖。概率由 `block_risk` 给出，出手时抽一次，
    /// 执行层只负责把这次封盖演成手碰到球。
    pub block_hand: Option<usize>,
    /// 出手点到篮筐的平面距离（ft）。
    pub distance_ft: f32,
    /// 球心轨迹穿过筐平面时的入射角：1 表示垂直下落，0 表示平飞。
    pub incidence: f32,
    /// 出手计划：释放点、抛体参数与落点偏移。执行层直接用它飞行。
    pub plan: ShotPlan,
    /// 出手释放点。
    pub origin: Vec2,
}

/// 实测曲线对应的射手水平：该区域出手权的平均属性。
///
/// 实测曲线给的是联盟平均射手在这个区域能投进多少，因此「这名射手比
/// 平均水平好多少」必须以这份名册在该区域的平均属性为原点。属性的量纲
/// 上 0.5 并不是联盟平均（这份名册的三分均值约 0.65），把 0.5 当原点会
/// 让每一投都带上一个固定的加成。
fn skill_reference(world: &World, zone: &str) -> f32 {
    let policy = &world.params.policy;
    policy.w(match zone {
        "rim" => "shot_skill_reference_rim",
        "near" => "shot_skill_reference_near",
        "mid" => "shot_skill_reference_mid",
        "three" => "shot_skill_reference_three",
        other => panic!("unknown shot zone `{other}`"),
    })
}

/// 一次出手的平面几何：飞行时长、竖直初速与穿过筐平面的入射角。
///
/// 价值层与执行层用同一个函数解算抛体，因此「模型预测的入射角」与
/// 「球实际飞行时的入射角」是同一件事，而不是两套近似。
#[derive(Debug, Clone, Copy)]
pub struct ShotPlan {
    pub duration: f32,
    pub vz0: f32,
    pub horizontal_speed: f32,
    pub incidence: f32,
}

/// 解算一次出手的抛体：释放点到落点的水平直线，加上穿过筐平面的竖直分量。
///
/// 落点含瞄准误差，因此入射角随这一次出手的落点变化；模型用同一个函数
/// 解算它，执行层直接用解出的时长与竖直初速飞行。
pub fn plan_shot(
    rules: &nba_domain::GameRules,
    from: Vec2,
    landing: Vec2,
    release_z: f32,
) -> ShotPlan {
    let distance = (landing - from).length();
    let peak = (rules.shot_peak_base_ft + distance * rules.shot_peak_distance_factor)
        .max(rules.rim_height_ft + 0.5);
    let mut duration = ProjectileArc::time_for_peak(
        release_z,
        rules.rim_height_ft,
        peak,
        rules.ball_gravity_ftps2,
    )
    .clamp(
        rules.min_shot_duration_seconds,
        rules.max_shot_duration_seconds,
    );
    if distance / duration > rules.ball_max_speed_ftps {
        duration = distance / (rules.ball_max_speed_ftps * 0.9);
    }
    let arc = ProjectileArc::solve(
        release_z,
        rules.rim_height_ft,
        duration,
        rules.ball_gravity_ftps2,
    );
    let horizontal_speed = distance / duration;
    let vertical_at_rim = arc.vz0 - rules.ball_gravity_ftps2 * duration;
    ShotPlan {
        duration,
        vz0: arc.vz0,
        horizontal_speed,
        incidence: velocity_incidence(horizontal_speed, vertical_at_rim),
    }
}

/// 速度相对水平面的下降正弦：垂直下落越陡，球体在筐平面的投影越完整。
pub fn velocity_incidence(horizontal_speed: f32, vertical_speed: f32) -> f32 {
    let drop = (-vertical_speed).max(0.0);
    drop / horizontal_speed.hypot(drop).max(MIN_LOG_ARGUMENT)
}

/// 有效筐口：球心轨迹与筐环的最小间距换算成的水平中心裕量。
///
/// 筐环是一根半径 `rim_radius` 的管；球心轨迹与筐环的最小距离是
/// `(rim_radius - d) × incidence`（`d` 是球心穿过筐平面时到筐心的水平
/// 距离，`incidence` 是球心轨迹的竖直分量）。球要干净穿过，���距离必须
/// 大于球半径加管半径，于是水平裕量是
///
/// `rim_radius - (ball_radius + 筐环管半径) / incidence`。
///
/// 入射角为 1（垂直下落）时它就是
/// `rim_radius - 筐环管半径 - ball_radius`；入射角越平，球在筐平面上的
/// 投影越长，可用裕量越窄，接近零时不存在干净穿筐。
pub fn effective_aperture(rules: &nba_domain::GameRules, incidence: f32) -> f32 {
    let rim_tube = rules.rim_radius_ft * 0.1;
    let clearance = rules.ball_radius_ft + rim_tube;
    let incidence = incidence.clamp(MIN_LOG_ARGUMENT, 1.0);
    (rules.rim_radius_ft - clearance / incidence).max(0.0)
}

/// 出手释放点：出手点在朝向篮筐方向上的持球偏移。
///
/// 出手时球总是面向篮筐释放，因此这个点只由出手点与篮筐位置决定，
/// 价值层与执行层拿到的是同一个点。
pub fn release_origin(world: &World, pos: Vec2, hoop: Vec2) -> Vec2 {
    let rules = &world.rules;
    let facing = (hoop - pos).normalize_or_zero();
    let distance = (hoop - pos).length();
    let offset = rules.ball_holder_offset_ft.min(distance * 0.5);
    pos + facing * offset
}

/// 出手类型所属的参考分区。
///
/// 实测曲线按出手类型给出，同一分区里的出手在干扰和移动上的统计特征
/// 接近，因此基准值按分区配置，而不是按每一种出手类型各配一份。
fn reference_zone(kind: ShotKind) -> &'static str {
    match kind {
        ShotKind::Dunk
        | ShotKind::DrivingDunk
        | ShotKind::CuttingDunk
        | ShotKind::RunningDunk
        | ShotKind::Layup
        | ShotKind::DrivingLayup
        | ShotKind::CuttingLayup
        | ShotKind::RunningLayup
        | ShotKind::ReverseLayup
        | ShotKind::Putback
        | ShotKind::AlleyOop => "rim",
        ShotKind::Hook | ShotKind::Turnaround | ShotKind::Floater => "near",
        _ => "perimeter",
    }
}

/// 某一类出手的基准干扰强度。
///
/// 实测曲线按出手类型给出，篮下出手的曲线里已经含有贴身的防守人，外线
/// 出手的基准干扰则低得多。干扰修正必须相对这个基准，否则同一份实测数据
/// 会被扣两次干扰。
fn contest_reference(world: &World, kind: ShotKind) -> f32 {
    let policy = &world.params.policy;
    match reference_zone(kind) {
        "rim" => policy.w("shot_contest_reference_rim"),
        "near" => policy.w("shot_contest_reference_near"),
        _ => policy.w("shot_contest_reference_perimeter"),
    }
}

/// 某一类出手的基准移动速度（占最快速度的比例）。
///
/// 实测曲线里同一类出手的移动是混合的：有原地接球出手，也有全速突破中
/// 的出手。曲线给出的是这些情况加起来的命中率，所以修正必须以该类出手
/// 的移动均值为中心，均值处不改变命中率。
fn movement_reference(world: &World, kind: ShotKind) -> f32 {
    let policy = &world.params.policy;
    match reference_zone(kind) {
        "rim" => policy.w("shot_movement_reference_rim"),
        "near" => policy.w("shot_movement_reference_near"),
        _ => policy.w("shot_movement_reference_perimeter"),
    }
}

/// 出手类型对瞄准误差的倍率。
///
/// 实测曲线已经包含了「这种出手方式本身有多难」（上篮、勾手、跳投、
/// 三分各自的真实命中率），因此这里的倍率是相对实测曲线的修正，默认
/// 为 1：模型不重复计入出手类型的难度。
fn kind_factor(_kind: ShotKind) -> f32 {
    1.0
}

/// 动作阶段对瞄准误差的倍率。
///
/// 实测曲线按出手类型给出，不区分动作阶段；因此阶段倍率默认是 1，只表达
/// 「同样的出手类型在不同的持球阶段下的额外折扣或加成」。
fn phase_factor(world: &World, phase: BallPhase) -> f32 {
    let policy = &world.params.policy;
    match phase {
        BallPhase::Catch => policy.w("shot_phase_sigma_catch"),
        BallPhase::Dribble => policy.w("shot_phase_sigma_dribble"),
        BallPhase::Drive => policy.w("shot_phase_sigma_drive"),
        BallPhase::Post => policy.w("shot_phase_sigma_post"),
        BallPhase::Cut => policy.w("shot_phase_sigma_cut"),
        BallPhase::Gather => policy.w("shot_phase_sigma_gather"),
        BallPhase::Putback => policy.w("shot_phase_sigma_putback"),
    }
}

fn release_height(world: &World, shooter: usize, kind: ShotKind) -> f32 {
    let policy = &world.params.policy;
    let rules = &world.rules;
    let physical = crate::physical::Physical::of(&world.players[shooter], policy);
    // 出手高度就是持球手的摸高：站立摸高加上起跳带来的那一段。跳投把这段
    // 摸高全部用上，上篮与抛投在球离手的瞬间还没完全伸展，篮下终结则贴着
    // 篮筐。球员身高与弹跳因此直接进入出手高度，也直接进入封盖判定。
    let extended =
        physical.standing_reach_ft + physical.vertical_ft * policy.w("shot_release_vertical_share");
    let height = match kind {
        ShotKind::Dunk | ShotKind::DrivingDunk | ShotKind::CuttingDunk | ShotKind::RunningDunk => {
            rules.rim_height_ft + 0.5
        }
        ShotKind::AlleyOop => rules.rim_height_ft + 0.3,
        ShotKind::Layup
        | ShotKind::DrivingLayup
        | ShotKind::CuttingLayup
        | ShotKind::RunningLayup
        | ShotKind::ReverseLayup
        | ShotKind::Putback => extended - 0.55,
        ShotKind::Floater | ShotKind::Hook | ShotKind::Turnaround => extended - 0.25,
        _ => extended,
    };
    height.clamp(rules.chest_height_ft + 1.0, rules.rim_height_ft + 1.5)
}

fn release_delay(kind: ShotKind, phase: BallPhase) -> f32 {
    let kind_delay = match kind {
        ShotKind::Dunk
        | ShotKind::DrivingDunk
        | ShotKind::CuttingDunk
        | ShotKind::RunningDunk
        | ShotKind::Layup
        | ShotKind::DrivingLayup
        | ShotKind::CuttingLayup
        | ShotKind::RunningLayup
        | ShotKind::ReverseLayup
        | ShotKind::Putback => 0.18,
        ShotKind::CatchShoot => 0.24,
        ShotKind::PullUp | ShotKind::StepBack => 0.32,
        ShotKind::Floater | ShotKind::Hook | ShotKind::Turnaround => 0.34,
        ShotKind::AlleyOop => 0.15,
        ShotKind::RunningJump => 0.26,
    };
    kind_delay
        + if phase == BallPhase::Gather {
            0.08
        } else {
            0.0
        }
}

/// 手心到球心中心轨迹的最近距离，以及该最近点的横向方向。
///
/// 返回（距离，轨迹上最近点，垂直于轨迹的单位方向）。垂直方向在平面内
/// （瞄准误差在平面上抽样），因此瞄准误差在这个方向上的投影是标准差
/// `sigma / √2` 的正态分布。
fn trajectory_closest(
    plan: &ShotPlan,
    origin: Vec2,
    landing: Vec2,
    release_z: f32,
    hand: Vec2,
    hand_z: f32,
    gravity: f32,
) -> (f32, Vec2, Vec2, f32) {
    let coarse = 32usize;
    let mut best = f32::INFINITY;
    let mut best_point = origin;
    let mut best_z = release_z;
    for step in 0..=coarse {
        let t = plan.duration * step as f32 / coarse as f32;
        let point = origin.lerp(landing, t / plan.duration.max(MIN_LOG_ARGUMENT));
        let z = release_z + plan.vz0 * t - 0.5 * gravity * t * t;
        let distance = (point - hand).length().hypot(z - hand_z);
        if distance < best {
            best = distance;
            best_point = point;
            best_z = z;
        }
    }
    // 细扫：最近距离常常落在两个粗采样点之间。
    let low = (best_t_fraction(plan, best_point, origin, landing) - 1.0 / coarse as f32).max(0.0);
    let high = (best_t_fraction(plan, best_point, origin, landing) + 1.0 / coarse as f32).min(1.0);
    for step in 0..=16 {
        let fraction = low + (high - low) * step as f32 / 16.0;
        let t = plan.duration * fraction;
        let point = origin.lerp(landing, fraction);
        let z = release_z + plan.vz0 * t - 0.5 * gravity * t * t;
        let distance = (point - hand).length().hypot(z - hand_z);
        if distance < best {
            best = distance;
            best_point = point;
            best_z = z;
        }
    }
    // 轨迹在最近点的水平方向（球从释放点飞向落点的方向）。
    let direction = (landing - origin).normalize_or_zero();
    let lateral = Vec2::new(-direction.y, direction.x);
    (best, best_point, lateral, best_z)
}

/// 最近点对应的飞行时间比例。
fn best_t_fraction(_plan: &ShotPlan, point: Vec2, origin: Vec2, landing: Vec2) -> f32 {
    let span = (landing - origin).length_squared();
    if span <= f32::MIN_POSITIVE {
        return 0.0;
    }
    ((point - origin).dot(landing - origin) / span).clamp(0.0, 1.0)
}

/// 预计释放时刻的三维防守压力。防守人先按当前速度到达预计位置，再用
/// 手部高度、起跳能力和是否在投篮线与篮筐之间计算干扰强度。
///
/// 返回（干扰强度，最近防守人的三维距离）。
fn release_contest(
    world: &World,
    shooter: usize,
    pos: Vec2,
    release_z: f32,
    delay: f32,
) -> (f32, f32) {
    let team = world.players[shooter].team;
    let hoop = world.hoop_for(team);
    let policy = &world.params.policy;
    let mut best_contest = 0.0_f32;
    let mut best_distance = f32::INFINITY;
    for defender in world.players.iter().filter(|player| player.on_court) {
        if defender.team == team {
            continue;
        }
        let predicted = defender.pos + defender.vel * delay;
        let physical = crate::physical::Physical::of(defender, policy);
        let hand_z = world.rules.chest_height_ft + 1.0 + defender.attribute("block") * 2.5;
        let horizontal = (predicted - pos).length();
        let distance = horizontal.hypot(hand_z - release_z);
        let between = crate::state::is_between(predicted, pos, hoop);
        let contest = crate::state::contest_from_defender(
            distance,
            between,
            physical.reach_excess_ft(policy),
        );
        if contest > best_contest {
            best_contest = contest;
        }
        best_distance = best_distance.min(distance);
    }
    (best_contest, best_distance)
}

/// 封盖风险：这一手真实落点的轨迹与防守人手的拦截体积相交的概率。
///
/// 执行层在飞行途中用手部体积与球心轨迹的相交判定封盖，因此风险必须
/// 针对这一次出手的真实轨迹来算：球的落点由瞄准误差抽样决定，轨迹就是
/// 释放点到落点的那条线。
///
/// 执行层的判据是硬判据：球心到手的距离进入拦截半径就是封盖，否则不是。
/// 模型要预测的是这一条判据在瞄准误差下的概率，因此不能自己另立一套
/// 软带宽。落点横向偏移 s 会让整条轨迹跟着偏移 s，手到轨迹的最近距离
/// 就是 |h0 - s|，于是封盖等价于 s 落在 [h0 - r, h0 + r] 之内。瞄准误差
/// 的横向分量服从 N(0, sigma)，两个端点之差就是一个正态区间概率。
/// 执行层用的正是同一个 sigma 抽样、同一个拦截半径，因此模型给出的概率
/// 就是执行层在大量同几何出手上的封盖比例。
/// 防守人伸手后的手球。模型估封盖概率与执行层判封盖接触都从这里取
/// 几何，两处的手位、高度与半径因此不会漂移。
pub struct BlockingHand {
    pub handle: usize,
    pub pos: Vec2,
    pub z: f32,
    pub radius: f32,
}

pub fn blocking_hands(world: &World, shooter: usize) -> Vec<BlockingHand> {
    let team = world.players[shooter].team;
    let policy = &world.params.policy;
    let rules = &world.rules;
    world
        .players
        .iter()
        .enumerate()
        .filter(|(_, player)| player.on_court && player.team != team)
        .map(|(handle, defender)| {
            // 垂直起跳封盖在执行窗口内真的把手举得更高：摸高按策略增加。
            let jumping = world.defensive_actions[handle].as_ref().is_some_and(|action| {
                action.kind == crate::action::DefensiveActionKind::BlockJump
                    && action.phase == nba_domain::action_window::ActionPhase::Execution
            });
            let jump_bonus = if jumping {
                rules.resolve.technique.block_jump_extra_reach_ft
            } else {
                f32::default()
            };
            BlockingHand {
            handle,
            pos: defender.pos
                + defender.facing.normalize_or_zero() * (rules.ball_holder_offset_ft + 0.4),
            // 手的高度就是防守人的摸高：站立摸高加上起跳。身材与弹跳因此
            // 决定了他能不能把手伸到球飞的轨迹上。
            z: crate::physical::Physical::of(defender, policy).contest_reach_ft(policy) + jump_bonus,
            radius: rules.ball_radius_ft
                + policy.w("block_hand_radius_base_ft")
                + defender.attribute("block") * policy.w("block_hand_radius_skill_ft"),
            }
        })
        .collect()
}

/// 球心落在哪个防守人的手球里。按三维距离判定，与
/// `block_risk_for_path` 里「轨迹离手的最近距离」是同一套口径。
pub fn blocking_hand(world: &World, shooter: usize, point: Vec2, z: f32) -> Option<usize> {
    blocking_hands(world, shooter)
        .into_iter()
        .find(|hand| (point - hand.pos).length().hypot(z - hand.z) <= hand.radius)
        .map(|hand| hand.handle)
}

pub fn block_risk_for_path(
    world: &World,
    shooter: usize,
    origin: Vec2,
    landing: Vec2,
    plan: &ShotPlan,
    release_z: f32,
    sigma: f32,
) -> (f32, Option<usize>) {
    let rules = &world.rules;
    let mut best = (0.0_f32, None);
    for hand in blocking_hands(world, shooter) {
        let (distance, _point, _lateral, _z) = trajectory_closest(
            plan,
            origin,
            landing,
            release_z,
            hand.pos,
            hand.z,
            rules.ball_gravity_ftps2,
        );
        let sigma = sigma.max(MIN_LOG_ARGUMENT);
        let near = standard_normal_cdf((distance - hand.radius) / sigma);
        let far = standard_normal_cdf((distance + hand.radius) / sigma);
        let risk = (far - near).clamp(0.0, 1.0);
        if risk > best.0 {
            best = (risk, Some(hand.handle));
        }
    }
    best
}

/// 标准正态分布的累积分布函数，用 erf 的 Abramowitz-Stegun 近似。
fn standard_normal_cdf(x: f32) -> f32 {
    let z = x / std::f32::consts::SQRT_2;
    let sign = if z < 0.0 { -1.0 } else { 1.0 };
    let z = z.abs();
    let t = 1.0 / (1.0 + 0.3275911 * z);
    let poly = t
        * (0.254_829_6
            + t * (-0.284_496_72 + t * (1.421_413_8 + t * (-1.453_152_1 + t * 1.061_405_4))));
    let erf = 1.0 - poly * (-z * z).exp();
    0.5 * (1.0 + sign * erf)
}

/// 三分距离衰减：三分线外连续衰减，远处趋近于零。
///
/// 衰减作用在命中目标上，不作用在瞄准误差上，因此距离只被计入一次。
/// 形状由「特征长度」与「幂次」两个独立参数决定。
pub fn three_distance_decay(world: &World, distance: f32) -> f32 {
    let policy = &world.params.policy;
    let arc = world.rules.league.three_point_distance_ft;
    if distance <= arc {
        return 1.0;
    }
    let scale = policy.w("shot_distance_decay_three_scale_ft");
    if scale <= 0.0 {
        return 1.0;
    }
    let power = policy.w("shot_distance_decay_three_power");
    let excess = (distance - arc) / scale;
    (-excess.powf(power)).exp()
}

/// 唯一投篮模型：估值、候选选择和球体执行都从这里取命中参数。
pub fn evaluate(
    world: &World,
    shooter: usize,
    kind: ShotKind,
    phase: BallPhase,
    pos: Vec2,
) -> ShotModel {
    let team = world.players[shooter].team;
    let hoop = world.hoop_for(team);
    let zone = crate::resolve::shooter_zone(world, pos, team);
    let distance = (pos - hoop).length();
    let policy = &world.params.policy;
    let rules = &world.rules;
    let release_z = release_height(world, shooter, kind);
    let delay = release_delay(kind, phase);
    let origin = release_origin(world, pos, hoop);
    let plan = plan_shot(rules, origin, hoop, release_z);
    let aperture = effective_aperture(rules, plan.incidence);
    let base = policy.w(match zone {
        "rim" => "shot_quality_base_rim",
        "near" => "shot_quality_base_near",
        "mid" => "shot_quality_base_mid",
        "three" => "shot_quality_base_three",
        other => panic!("unknown shot zone `{other}`"),
    });
    let spread = policy.w(match zone {
        "rim" => "shot_quality_skill_rim",
        "near" => "shot_quality_skill_near",
        "mid" => "shot_quality_skill_mid",
        "three" => "shot_quality_skill_three",
        other => panic!("unknown shot zone `{other}`"),
    });
    let skill = world.players[shooter].shooting_attribute(zone);
    let reference = (base + (skill - 0.5) * spread).max(0.0);
    // 目标命中率优先取实测的距离曲线：出手类型在���个距离上真的能投进多少，
    // 就是这个距离上的目标命中率。没有这一段曲线时用区域基准值兜底。
    // 实测曲线的距离是出手点到篮筐的距离，也就是释放点的距离；因此查表
    // 用释放点而不是球员站位点，两者相差一个持球偏移。
    let release_distance = (origin - hoop).length();
    let measured =
        world
            .params
            .shot_distance
            .rate_at(kind.as_str(), zone == "three", release_distance);
    let mut target = match measured {
        Some(rate) => rate + (skill - skill_reference(world, zone)) * spread,
        None => reference,
    };
    // 三分线外的衰减：实测曲线在最后一个距离档之后不再有数据，超远距离的
    // 命中率必须连续趋近于零，因此这里从最后一个实测档继续衰减。实测曲线
    // 已经包含距离带来的命中率下降，在曲线覆盖的范围内不能再衰减一次，
    // 否则同一段距离衰减会被计入两遍。衰减在实测档边界归一，保证曲线在
    // 边界处连续。
    if zone == "three" {
        let arc = world.rules.league.three_point_distance_ft;
        let last_upper = world
            .params
            .shot_distance
            .last_upper_ft(kind.as_str(), true)
            .unwrap_or(arc);
        if release_distance > last_upper {
            let decay = three_distance_decay(world, release_distance);
            let edge = three_distance_decay(world, last_upper);
            target *= decay / edge.max(MIN_LOG_ARGUMENT);
        }
    }
    let target = target.clamp(0.0, 1.0);
    if std::env::var_os("NBA_TRACE_QUAL").is_some() {
        eprintln!(
            "QUAL kind={} zone={} dist={:.1} measured={} target={:.4} skill={:.3} spread={:.3}",
            kind.as_str(),
            zone,
            release_distance,
            measured.map(|rate| format!("{rate:.4}")).unwrap_or_else(|| "none".to_string()),
            target,
            skill,
            spread
        );
    }
    // 封盖与「投篮犯规不中」是球出手之后才发生的分支，实测曲线给的是
    // 扣掉这两种分支之后的净命中率。瞄准误差决定的是球自己能不能穿过
    // 筐口，所以先把净目标反解成条件命中率：飞行层的封盖与犯规判定再把
    // 条件概率折回净命中率，两次口径不会重复扣减。
    let (contest, _block_distance) = release_contest(world, shooter, pos, release_z, delay);
    let rim_in = policy.w("shot_rim_in_probability").clamp(0.0, 0.9);
    let contact_band = policy.w("shot_rim_contact_band").max(0.0);
    let contact_radius = aperture + contact_band;
    let movement = world.players[shooter].vel.length() / rules.max_player_speed_ftps;
    // 实测曲线本身就是本类出手在本距离上的总体命中率，已经包含联盟平均的
    // 干扰与移动。两个乘子都以本类出手的总体均值为中心：比均值更紧的防守
    // 扩大瞄准误差，比均值更干净的出手缩小瞄准误差，两侧对称、均值不变。
    // 基准值由多种子运行的 world.shot_samples 统计得出，写在 policy 表里，
    // 每一类出手单独一份。移动基准也是以全速的比例计量，间隔基准是该类
    // 出手在 release_contest 口径下的均值，两者都取自同一次多种子运行。
    // 乘子有下界 MIN_SIGMA_FACTOR，任何加成都不会让偏差变成零。
    // 干扰越强，命中率越低：乘子在命中目标上，所以因子必须随干扰下降。
    // 基准值是该类出手在 release_contest 口径下的平均干扰，基准处因子为 1；
    // 比基准更干净的出手不额外加成，因为实测曲线已经含有联盟平均干扰。
    let contest_reference = contest_reference(world, kind);
    let contest_factor_of = |contest: f32| {
        (1.0 - (contest - contest_reference).max(0.0) * policy.w("shot_quality_contest_sensitivity"))
            .max(MIN_SIGMA_FACTOR)
    };
    // 出手时的移动同样压低命中率：移动基准是全速的比例，基准处因子为 1。
    let movement_factor = (1.0
        - (movement - movement_reference(world, kind)).max(0.0)
            * policy.w("shot_aim_movement_sensitivity"))
        .max(MIN_SIGMA_FACTOR);
    let gather_factor = if phase == BallPhase::Gather {
        policy.w("shot_gather_sigma_scale")
    } else {
        1.0
    };
    // 出手类型与动作阶段的倍率是「相对实测曲线的修正」：实测曲线已经是
    // 这个出手类型在这个距离上的真实命中率，倍率只表达「同样的距离下，
    // 这种出手方式与这个动作阶段的额外折扣或加成」。
    // 乘子作用在命中目标上：目标对乘子是线性的，以本类出手的总体均值为
    // 中心时两侧对称、均值不变。作用在瞄准误差上则会系统性放大命中率，
    // 因为命中率对瞄准偏差是凸函数，乘子的方差会抬高均值。
    let adjust = kind_factor(kind)
        * phase_factor(world, phase)
        * movement_factor
        * contest_factor_of(contest)
        * gather_factor
        * policy.w("shot_aim_sigma_scale");
    let target = (target * adjust).clamp(0.0, 1.0);
    // 参考条件下的瞄准误差：把条件命中率换算成「落点离筐多远」。
    let sigma_for = |survive: f32| {
        let conditional = (target / survive).clamp(0.0, MAX_ARC_PROBABILITY);
        // 目标命中率包含「球打到筐沿之后仍然补进」的部分，瞄准误差只决定
        // 干净穿筐的概率。触筐补进这一项自己依赖瞄准误差的宽度（偏离越大
        // 越碰不到筐沿），因此用不动点迭代反解：先按只算穿筐解一次，再把
        // 触筐项扣掉重解，几次之后两边一致。
        let mut sigma_reference = solve_sigma(aperture, conditional);
        for _ in 0..4 {
            let contact = 1.0
                - (-(contact_radius * contact_radius) /
                    (2.0 * sigma_reference * sigma_reference))
                    .exp();
            let arc_target = ((conditional - rim_in * contact) / (1.0 - rim_in).max(MIN_SURVIVE))
                .clamp(MIN_ARC_PROBABILITY, MAX_ARC_PROBABILITY);
            sigma_reference = solve_sigma(aperture, arc_target);
        }
        sigma_reference.max(MIN_LOG_ARGUMENT)
    };
    let sigma_prior = sigma_for(1.0);
    let (block_risk, block_hand) =
        block_risk_for_path(world, shooter, origin, hoop, &plan, release_z, sigma_prior);
    let block_risk = block_risk.clamp(0.0, 0.95);
    let survive = 1.0 - block_risk;
    let sigma = sigma_prior;
    // 命中概率就是瞄准误差落进有效筐口的概率；执行层用同一个 sigma 抽样、
    // 同一个有效筐口判定，两者不会漂移。这一概率是条件命中率，乘以存活
    // 概率才是实测口径的净命中率。
    let p_make_cond = if aperture <= MIN_APERTURE_FT {
        0.0
    } else {
        // 干净穿筐，加上「碰到筐沿之后补进」：只有落点落在筐口外一个
        // 球径带以内的出手才碰得到筐沿，偏离更远的球直接飞出。执行层
        // 用同一个筐口、同一个接触带、同一个补进概率抽样，两边同账。
        let swish = 1.0 - (-(aperture * aperture) / (2.0 * sigma * sigma)).exp();
        let contact = 1.0
            - (-(contact_radius * contact_radius) / (2.0 * sigma * sigma)).exp();
        (swish + (contact - swish) * rim_in).clamp(0.0, 1.0)
    };
    let p_make = (p_make_cond * survive).clamp(0.0, 1.0);
    // 体能直接压低出手概率：累了的人脚步与手腕都跟不上，落点误差更大。
    // 耐力属性把同一次疲劳放大，均值保持不变。
    let fatigue = world.players[shooter].stamina.clamp(0.0, 1.0);
    let endurance = world.players[shooter].data.attributes.stamina.clamp(0.0, 1.0);
    let fatigue_scale = 1.0 + 1.2 * (0.5 - endurance);
    let p_make = (p_make
        * (1.0 - policy.w("shot_fatigue_penalty") * (1.0 - fatigue) * fatigue_scale))
        .clamp(0.0, 1.0);
    if std::env::var_os("NBA_TRACE_RATE").is_some() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static SEEN: AtomicU32 = AtomicU32::new(0);
        if SEEN.fetch_add(1, Ordering::Relaxed) < 80 {
            eprintln!(
                "RATE kind={} three={} dist={:.1} measured={:?} target={:.3} contest={:.2} move={:.2} sigma={:.3} aper={:.3} cond={:.3} p={:.3} survive={:.3}",
                kind.as_str(),
                zone == "three",
                release_distance,
                measured,
                target,
                contest,
                movement,
                sigma,
                aperture,
                p_make_cond,
                p_make,
                survive
            );
        }
    }
    ShotModel {
        p_make,
        p_make_cond,
        sigma_ft: sigma,
        effective_aperture_ft: aperture,
        contact_radius_ft: contact_radius,
        rim_in_probability: rim_in,
        release_z_ft: release_z,
        release_delay_seconds: delay,
        contest,
        movement,
        skill,
        block_risk,
        block_hand,
        distance_ft: distance,
        incidence: plan.incidence,
        plan,
        origin,
    }
}

/// 罚球使用与投篮相同的筐口、释放高度和抛体几何；目标命中率只由
/// 罚球基准与技能决定，瞄准误差不再来自独立的固定值。
pub fn free_throw_model(world: &World, shooter: usize) -> ShotModel {
    let team = world.players[shooter].team;
    let pos = world.players[shooter].pos;
    let rules = &world.rules;
    let policy = &world.params.policy;
    let hoop = world.hoop_for(team);
    let distance = (pos - hoop).length();
    let release_z =
        crate::physical::Physical::of(&world.players[shooter], policy).contest_reach_ft(policy);
    let origin = release_origin(world, pos, hoop);
    let plan = plan_shot(rules, origin, hoop, release_z);
    let aperture = effective_aperture(rules, plan.incidence);
    let skill = world.players[shooter].data.attributes.free_throw;
    let base = policy.w("shot_quality_base_free_throw");
    let spread = policy.w("shot_quality_skill_free_throw");
    let target = (base + (skill - 0.5) * spread).clamp(0.0, 1.0);
    let denominator = (-2.0 * (1.0 - target).ln()).max(MIN_LOG_ARGUMENT);
    let sigma = (aperture / denominator.sqrt()).max(MIN_LOG_ARGUMENT);
    ShotModel {
        p_make: target,
        p_make_cond: target,
        sigma_ft: sigma,
        effective_aperture_ft: aperture,
        contact_radius_ft: aperture,
        rim_in_probability: 0.0,
        release_z_ft: release_z,
        release_delay_seconds: 0.0,
        contest: 0.0,
        movement: 0.0,
        skill,
        block_risk: 0.0,
        block_hand: None,
        distance_ft: distance,
        incidence: plan.incidence,
        plan,
        origin,
    }
}

/// 补篮只属于开启二次进攻的那个抢板人，并且只在抢板后的窗口内成立。
///
/// 用时间窗口而不是精确阶段：抢到板后先做一个收球步再起跳仍是补篮，
/// 而那个动作会把阶段从 `Putback` 改成 `Gather`。
pub fn putback_available(world: &World, actor: usize) -> bool {
    if world.possession.last_offensive_rebound_actor != Some(actor) {
        return false;
    }
    let window = f64::from(world.params.policy.g("putback_window_seconds"));
    world
        .possession
        .last_offensive_rebound_at
        .map(|at| world.now() - at <= window)
        .unwrap_or(false)
}

pub fn shot_choice(world: &World, state: &DecisionState) -> Option<(ShotKind, &'static str)> {
    let zone = shooter_zone(world, state.actor_pos, world.players[state.actor].team);
    // 出手类型只读当前阶段与实际位置：阶段由动作结算写入，不在这里猜测。
    let phase = world.possession.ball_phase;
    let transition = world.possession.kind == PossessionKind::Live
        && world.now() - world.possession.started_at
            < world.params.policy.g("transition_window_seconds") as f64;
    let finishing = world.players[state.actor].data.attributes.finishing;
    let putback = putback_available(world, state.actor);
    let distance = state.distance_to_hoop();
    let is_catch_shoot = phase == BallPhase::Cut || phase == BallPhase::Catch;

    // 投篮类型只读当前持球阶段与实际位置，不读「最近 2.5 秒做过什么」
    // 这类宽泛标签。阶段由动作结算写入，位置由球员当前坐标给出。
    match zone {
        "rim" => {
            if putback {
                Some((ShotKind::Putback, zone))
            } else if phase == BallPhase::Cut {
                Some((
                    if finishing > 0.6 {
                        ShotKind::CuttingDunk
                    } else {
                        ShotKind::CuttingLayup
                    },
                    zone,
                ))
            } else if phase == BallPhase::Post {
                Some((ShotKind::Hook, zone))
            } else if transition {
                Some((
                    if finishing > 0.6 {
                        ShotKind::RunningDunk
                    } else {
                        ShotKind::RunningLayup
                    },
                    zone,
                ))
            } else if phase == BallPhase::Drive
                || (phase == BallPhase::Gather
                    && world.possession.last_drive_actor == Some(state.actor)
                    && world
                        .possession
                        .last_drive_at
                        .map(|t| world.now() - t < 2.0)
                        .unwrap_or(false))
            {
                // 突破收球步之后的篮下完成属于突破进攻：收球步把阶段写成
                // Gather，出手时读到的阶段是 Gather 而不是 Drive。
                Some((
                    if finishing > 0.6 {
                        ShotKind::DrivingDunk
                    } else {
                        ShotKind::DrivingLayup
                    },
                    zone,
                ))
            } else {
                Some((
                    if finishing > 0.6 {
                        ShotKind::Dunk
                    } else {
                        ShotKind::Layup
                    },
                    zone,
                ))
            }
        }
        "near" => {
            if putback {
                Some((ShotKind::Putback, zone))
            } else if phase == BallPhase::Post {
                // 低位出手的类型按实际距离定：贴身是勾手，稍远是转身跳投。
                Some((
                    if distance > 8.0 {
                        ShotKind::Turnaround
                    } else {
                        ShotKind::Hook
                    },
                    zone,
                ))
            } else if transition {
                Some((ShotKind::RunningLayup, zone))
            } else if matches!(phase, BallPhase::Drive | BallPhase::Gather) {
                Some((ShotKind::Floater, zone))
            } else if is_catch_shoot {
                Some((
                    if distance > 10.0 {
                        ShotKind::CatchShoot
                    } else {
                        ShotKind::Floater
                    },
                    zone,
                ))
            } else {
                Some((
                    if distance > 10.0 {
                        ShotKind::PullUp
                    } else if finishing > 0.6 {
                        ShotKind::Layup
                    } else {
                        ShotKind::Floater
                    },
                    zone,
                ))
            }
        }
        "mid" => {
            if phase == BallPhase::Post {
                Some((ShotKind::Turnaround, zone))
            } else if transition && phase == BallPhase::Dribble {
                Some((ShotKind::RunningJump, zone))
            } else if is_catch_shoot {
                // 零运球出手：接球或切出后直接起跳，无论防守贴得多紧都算接球投篮。
                Some((ShotKind::CatchShoot, zone))
            } else {
                Some((ShotKind::PullUp, zone))
            }
        }
        "three" => {
            if transition && phase == BallPhase::Dribble {
                Some((ShotKind::RunningJump, zone))
            } else if is_catch_shoot {
                Some((ShotKind::CatchShoot, zone))
            } else {
                Some((ShotKind::PullUp, zone))
            }
        }
        _ => None,
    }
}
