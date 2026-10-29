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

#[derive(Debug, Clone, Copy)]
pub struct ShotModel {
    pub p_make: f32,
    pub sigma_ft: f32,
    pub effective_aperture_ft: f32,
    pub release_z_ft: f32,
    pub release_delay_seconds: f32,
    pub contest: f32,
    pub block_risk: f32,
    /// 出手点到篮筐的平面距离（ft）。
    pub distance_ft: f32,
    /// 球心轨迹穿过筐平面时的入射角：1 表示垂直下落，0 表示平飞。
    pub incidence: f32,
    /// 出手计划：释放点、抛体参数与落点偏移。执行层直接用它飞行。
    pub plan: ShotPlan,
    /// 出手释放点。
    pub origin: Vec2,
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
    let mut duration =
        ProjectileArc::time_for_peak(release_z, rules.rim_height_ft, peak, rules.ball_gravity_ftps2)
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

/// 某一类出手的基准干扰强度。
///
/// 实测曲线按出手类型给出，篮下出手的曲线里已经含有贴身的防守人，外线
/// 出手的基准干扰则低得多。干扰修正必须相对这个基准，否则同一份实测数据
/// 会被扣两次干扰。
fn contest_reference(world: &World, kind: ShotKind) -> f32 {
    let policy = &world.params.policy;
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
        | ShotKind::AlleyOop => policy.w("shot_contest_reference_rim"),
        ShotKind::Hook | ShotKind::Turnaround | ShotKind::Floater => {
            policy.w("shot_contest_reference_near")
        }
        _ => policy.w("shot_contest_reference_perimeter"),
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

fn release_height(world: &World, kind: ShotKind) -> f32 {
    let base = world.rules.chest_height_ft + 0.5;
    let bonus = match kind {
        ShotKind::Dunk
        | ShotKind::DrivingDunk
        | ShotKind::CuttingDunk
        | ShotKind::RunningDunk => 1.0,
        ShotKind::Layup
        | ShotKind::DrivingLayup
        | ShotKind::CuttingLayup
        | ShotKind::RunningLayup
        | ShotKind::ReverseLayup
        | ShotKind::Putback => 0.45,
        ShotKind::Floater | ShotKind::Hook => 0.3,
        _ => 0.0,
    };
    (base + bonus).min(world.rules.rim_height_ft + 1.0)
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
    kind_delay + if phase == BallPhase::Gather { 0.08 } else { 0.0 }
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
    let rules = &world.rules;
    let mut best_contest = 0.0_f32;
    let mut best_distance = f32::INFINITY;
    for defender in world.players.iter().filter(|player| player.on_court) {
        if defender.team == team {
            continue;
        }
        let predicted = defender.pos + defender.vel * delay;
        let physical = crate::physical::Physical::of(defender, policy);
        let hand_z = rules.chest_height_ft + 1.0 + defender.attribute("block") * 2.5;
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
/// 物理上「手在轨迹上」是确定事件，但防守人��手也在动，接触是持续的
/// 过程而不是一次瞬时判定。因此模型用「手到轨迹的最近距离与拦截半径
/// 的比值」给出连续的风险：轨迹穿过手的拦截体积时风险为 1，完全在
/// 拦截体积外时为 0，中间线性过渡。
pub fn block_risk_for_path(
    world: &World,
    shooter: usize,
    origin: Vec2,
    landing: Vec2,
    plan: &ShotPlan,
    release_z: f32,
) -> f32 {
    let team = world.players[shooter].team;
    let rules = &world.rules;
    let mut best = 0.0_f32;
    for defender in world.players.iter().filter(|player| player.on_court) {
        if defender.team == team {
            continue;
        }
        let hand = defender.pos
            + defender.facing.normalize_or_zero() * (rules.ball_holder_offset_ft + 0.4);
        let hand_z = rules.chest_height_ft + 1.0 + defender.attribute("block") * 2.5;
        let hand_radius = rules.ball_radius_ft + 0.25 + defender.attribute("block") * 0.70;
        let (distance, _point, _lateral, _z) = trajectory_closest(
            plan, origin, landing, release_z, hand, hand_z, rules.ball_gravity_ftps2,
        );
        // 手到轨迹的最近距离落在拦截半径内就是确定封盖；在外就是确定
        // 不封盖；中间按距离线性过渡。
        let risk = (1.0 - (distance - hand_radius) / hand_radius.max(0.1)).clamp(0.0, 1.0);
        if risk > best {
            best = risk;
        }
    }
    best
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
    let release_z = release_height(world, kind);
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
    let measured = world
        .params
        .shot_distance
        .rate_at(kind.as_str(), zone == "three", release_distance);
    let mut target = match measured {
        Some(rate) => rate + (skill - 0.5) * spread,
        None => reference,
    };
    // 三分线外的衰减：实测曲线在最后一个距离档之后不再有数据，超远距离的
    // 命中率必须连续趋近于零，因此这里从最后一个实测档继续衰减。
    if zone == "three" {
        let arc = world.rules.league.three_point_distance_ft;
        if distance > arc {
            let decay = three_distance_decay(world, distance);
            let edge = three_distance_decay(world, arc);
            target *= decay / edge.max(MIN_LOG_ARGUMENT);
        }
    }
    let target = target.clamp(0.0, 1.0);
    // 参考条件下的瞄准误差：把目标命中率换算成「落点离筐多远」。
    let denominator = (-2.0 * (1.0 - target).ln()).max(MIN_LOG_ARGUMENT);
    let sigma_reference = aperture / denominator.sqrt();
    let movement = world.players[shooter].vel.length() / rules.max_player_speed_ftps;
    // 干扰对瞄准误差的作用：实测曲线本身已经包含联盟平均干扰，因此只有
    // 比平均更强的干扰才扩大瞄准误差。干扰比平均弱时不能缩小误差——否则
    // 模型的命中率会高于实测曲线，三分球会显得比真实更划算。
    // 每类出手的基准干扰不同：篮下出手本来就有防守人贴身，实测曲线里已经
    // 包含这份干扰；外线出手的基准干扰低得多。干扰修正按「相对本类出手的
    // 基准干扰」计算，否则篮下出手会被重复扣一次干扰，突破终结反而比
    // 原地跳投更不值钱。
    let contest_reference = contest_reference(world, kind);
    let contest_factor_of = |contest: f32| {
        1.0 + (contest - contest_reference).max(0.0)
            * policy.w("shot_quality_contest_sensitivity")
    };
    let movement_factor =
        1.0 + movement * policy.w("shot_aim_movement_sensitivity");
    let gather_factor = if phase == BallPhase::Gather {
        policy.w("shot_gather_sigma_scale")
    } else {
        1.0
    };
    // 出手类型与动作阶段的倍率是「相对实测曲线的修正」：实测曲线已经是
    // 这个出手类型在这个距离上的真实命中率，倍率只表达「同样的距离下，
    // 这种出手方式与这个动作阶段的额外折扣或加成」。
    let (contest, _block_distance) = release_contest(world, shooter, pos, release_z, delay);
    let sigma = (sigma_reference
        * kind_factor(kind)
        * phase_factor(world, phase)
        * movement_factor
        * contest_factor_of(contest)
        * gather_factor
        * policy.w("shot_aim_sigma_scale"))
        .max(MIN_LOG_ARGUMENT);
    let block_risk = block_risk_for_path(
        world,
        shooter,
        origin,
        hoop,
        &plan,
        release_z,
    )
    .clamp(0.0, 0.95);
    // 命中概率就是瞄准误差落进有效筐口的概率；执行层用同一个 sigma 抽样、
    // 同一个有效筐口判定，两者不会漂移。
    let p_make = if aperture <= MIN_APERTURE_FT {
        0.0
    } else {
        (1.0 - (-(aperture * aperture) / (2.0 * sigma * sigma)).exp()).clamp(0.0, 1.0)
    };
    ShotModel {
        p_make,
        sigma_ft: sigma,
        effective_aperture_ft: aperture,
        release_z_ft: release_z,
        release_delay_seconds: delay,
        contest,
        block_risk,
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
    let release_z = rules.chest_height_ft + 0.5;
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
        sigma_ft: sigma,
        effective_aperture_ft: aperture,
        release_z_ft: release_z,
        release_delay_seconds: 0.0,
        contest: 0.0,
        block_risk: 0.0,
        distance_ft: distance,
        incidence: plan.incidence,
        plan,
        origin,
    }
}

/// 补篮只属于开启二次进攻的那个抢板人，并且只在二次进攻的当前阶段成立。
pub fn putback_available(world: &World, actor: usize) -> bool {
    world.possession.ball_phase == BallPhase::Putback
        && world.possession.last_offensive_rebound_actor == Some(actor)
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
            } else if phase == BallPhase::Drive {
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
                Some((if distance > 8.0 {
                    ShotKind::Turnaround
                } else {
                    ShotKind::Hook
                }, zone))
            } else if transition {
                Some((ShotKind::RunningLayup, zone))
            } else if matches!(phase, BallPhase::Drive | BallPhase::Gather) {
                Some((ShotKind::Floater, zone))
            } else if phase == BallPhase::Catch {
                Some((
                    if distance > 10.0 {
                        ShotKind::PullUp
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
            } else if phase == BallPhase::Catch {
                let curling_or_contested = state
                    .primary_defender
                    .map(|d| d.distance_ft < 10.5)
                    .unwrap_or(false)
                    || world
                        .possession
                        .last_screen
                        .map(|read| world.now() - read.at < 5.0)
                        .unwrap_or(false);
                if curling_or_contested {
                    Some((ShotKind::PullUp, zone))
                } else {
                    Some((ShotKind::CatchShoot, zone))
                }
            } else {
                Some((ShotKind::PullUp, zone))
            }
        }
        "three" => {
            if transition && phase == BallPhase::Dribble {
                Some((ShotKind::RunningJump, zone))
            } else if phase == BallPhase::Catch {
                let curling_or_contested = state
                    .primary_defender
                    .map(|d| d.distance_ft < 10.5)
                    .unwrap_or(false)
                    || world
                        .possession
                        .last_screen
                        .map(|read| world.now() - read.at < 5.0)
                        .unwrap_or(false);
                if curling_or_contested {
                    Some((ShotKind::PullUp, zone))
                } else {
                    Some((ShotKind::CatchShoot, zone))
                }
            } else {
                Some((ShotKind::PullUp, zone))
            }
        }
        _ => None,
    }
}
