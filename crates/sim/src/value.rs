//! 进攻价值：所有候选共享同一视界与净得分口径。
//!
//! 这里不产生结果，只把当前状态换算成预期净得分。投篮、传球、突破和
//! 保留球权都从同一状态出发：立即得分、投失后的进攻篮板、被犯规价值、
//! 接球后的继续进攻与失误成本分别进入同一条链。每次传球只消耗一次
//! 视界，不能把完整的保留球权价值再复制一遍。

use glam::Vec2;

use crate::action::PassKind;
use crate::resolve::{contest_for, shooter_zone};
use crate::state::DecisionState;
use crate::world::World;

/// 保留一次球权的非负价值。
pub fn retained_value(world: &World) -> f32 {
    world
        .params
        .policy
        .w("retained_possession_value")
        .max(0.0)
}

fn shot_points(world: &World, pos: Vec2, shooter: usize) -> (f32, &'static str) {
    let zone = shooter_zone(world, pos, world.players[shooter].team);
    (if zone == "three" { 3.0 } else { 2.0 }, zone)
}

/// 投失后由进攻方拿到篮板的概率。只用当前场上位置和篮板属性，不读结果表。
pub fn offensive_rebound_probability(world: &World, shooter: usize) -> f32 {
    let offense = world.players[shooter].team;
    let hoop = world.hoop_for(offense);
    let mut offensive = 0.0_f32;
    let mut defensive = 0.0_f32;
    for player in world.players.iter().filter(|player| player.on_court) {
        let distance = (player.pos - hoop).length();
        let near = (1.0 - distance / 30.0).clamp(0.0, 1.0);
        let rebound = if player.team == offense {
            "offensive_rebound"
        } else {
            "defensive_rebound"
        };
        let weight = near * (0.4 + player.attribute(rebound));
        if player.team == offense {
            offensive += weight;
        } else {
            defensive += weight;
        }
    }
    (0.35 * offensive / (offensive + defensive + f32::MIN_POSITIVE)).clamp(0.0, 0.35)
}

/// 一次出手被犯规带来的净值：罚球期望加加罚期望，使用当前规则与策略表。
pub fn foul_value(world: &World, points: f32) -> f32 {
    let policy = &world.params.policy;
    let p_foul = (policy.w("shooting_foul_rate") * 0.75).clamp(0.0, 1.0);
    let free_throw_points = points * 0.75;
    p_foul * free_throw_points
        + p_foul
            * policy.w("and_one_rate")
            * (if points >= 3.0 { 1.0 } else { 0.5 })
}

/// 给定命中概率和分值，计算一次出手从当前状态开始的预期净得分。
pub fn shot_value_for(
    world: &World,
    actor: usize,
    _pos: Vec2,
    p_make: f32,
    points: f32,
) -> f32 {
    let offensive_rebound = offensive_rebound_probability(world, actor);
    let after_miss = (1.0 - p_make) * offensive_rebound * points * 0.5;
    p_make * points + after_miss + foul_value(world, points)
}

/// 出手价值：使用该候选自己的投篮类型、动作阶段与预计释放点。
///
/// 不再用「区域 → 通用 PullUp/Layup」代替所有估值：同一个距离在接球
/// 投、急停跳投和低位勾手下的瞄准误差不同，价值必须跟着候选本身走。
pub fn shot_value(
    world: &World,
    actor: usize,
    pos: Vec2,
    kind: crate::action::ShotKind,
    phase: crate::state::BallPhase,
) -> f32 {
    let (points, _zone) = shot_points(world, pos, actor);
    let model = crate::shot_quality::evaluate(world, actor, kind, phase, pos);
    shot_value_for(world, actor, pos, model.p_make, points)
}

/// 接球后的价值：接球人自己在这条视界上能拿到的最好的结果。
///
/// 接球的价值就是接球人的出手价值。出手价值本身已经包含「投失后的进攻
/// 篮板」与「被犯规的罚球价值」，这里不再额外加一次保留球权价值：那一次
/// 保留已经算在出手价值里，再加一次就是同一份球权被计两遍，传球会永远
/// 压过出手。
pub fn catch_value(world: &World, pos: Vec2, shooter: usize, contest: f32) -> f32 {
    let (points, _zone) = shot_points(world, pos, shooter);
    let model = crate::shot_quality::evaluate(
        world,
        shooter,
        crate::action::ShotKind::CatchShoot,
        crate::state::BallPhase::Catch,
        pos,
    );
    let _ = contest;
    shot_value_for(world, shooter, pos, model.p_make, points)
}

fn point_segment_distance(point: Vec2, start: Vec2, end: Vec2) -> f32 {
    let segment = end - start;
    let length_squared = segment.length_squared();
    if length_squared <= f32::MIN_POSITIVE {
        return (point - start).length();
    }
    let t = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    (point - (start + segment * t)).length()
}

/// 传球完成概率的几何模型。
///
/// 接球半径读取 `catch_radius_base_ft + ball_handling × catch_radius_skill_gain_ft`；
/// 拦截半径读取球体手部接触的同一组常量。分离越大，概率越高；距离越远，
/// 接球窗口越小。策略层与球体接触使用同一组几何量，不再用独立线性权重。
///
/// 防守人不是静止的：球在空中飞的那段时间里他们会移动。因此走廊的空隙
/// 按「预计接球时刻」的位置算，而不是出手那一刻的位置；否则传球几乎必
/// ���，进攻方会一直传球直到进攻时钟走完。
/// 传球完成概率的几何模型。
///
/// 接球半径读取 `catch_radius_base_ft + ball_handling × catch_radius_skill_gain_ft`；
/// 拦截半径读取球体手部接触的同一组常量。分离越大，概率越高；距离越远，
/// 接球窗口越小。策略层与球体接触使用同一组几何量，不再用独立线性权重。
///
/// 防守人不是静止的：球在空中飞的那段时间里他们会移动。因此走廊的空隙
/// 按「预计接球时刻」的位置算，而不是出手那一刻的位置；否则传球几乎必
/// 成，进攻方会一直传球直到进攻时钟走完。
///
/// 目标点的选取也必须与执行层一致：球飞向接球人将要到达的位置，而不是
/// 他此刻站的地方。瞄向他此刻的位置会让接球人往回跑，球落在他原来的
/// 位置上，进攻于是离筐越来越远。
pub fn pass_completion_probability(
    world: &World,
    from: usize,
    to: usize,
    kind: crate::action::PassKind,
) -> f32 {
    let rules = &world.rules;
    let passer = &world.players[from];
    let receiver = &world.players[to];
    let start = passer.pos;
    // 高吊球的接球点是切入人跑到的位置，不是他此刻的位置：价值层与执行层
    // 必须用同一个落点，否则模型会认为这次传球根本传不到。
    let end = if kind == crate::action::PassKind::Lob {
        crate::resolve::pass::cut_plan(world, to).0
    } else {
        catch_point(world, to)
    };
    let distance = (end - start).length();
    if distance <= f32::MIN_POSITIVE {
        return 0.0;
    }
    // 球到达接球点所需的时间：高吊球按切入人跑到接球点的时间算，与执行
    // 层使用同一个时长，防守人的预测位置才落在同一时刻。
    let flight = if kind == crate::action::PassKind::Lob {
        crate::resolve::pass::cut_plan(world, to).1
    } else {
        f64::from(rules.pass_duration(distance, false))
    };
    let catch_radius = crate::ball::pass_catch_radius(rules, receiver);
    // 拦截高度与执行层一致：高吊球从人的头顶飞过，平直传球走胸口。
    let _intercept_height = if kind == crate::action::PassKind::Lob {
        rules.rim_height_ft + 0.5
    } else {
        rules.chest_height_ft + 1.0
    };
    let mut clearance = f32::INFINITY;
    for defender in world.players.iter().filter(|player| player.on_court) {
        if defender.team == passer.team {
            continue;
        }
        // 高吊球的落点是切入人跑到的位置，他自己的防守人就在落点旁边；
        // 那个防守人不是这次传球的拦截者——高吊球从他头顶飞过。
        if kind == crate::action::PassKind::Lob {
            let near_cutter = (defender.pos - world.players[to].pos).length() < 3.0;
            if near_cutter {
                continue;
            }
        }
        let intercept_radius = crate::ball::pass_intercept_radius(rules, defender);
        // 预计接球时刻防守人的位置。
        let predicted = defender.pos + defender.vel * flight as f32;
        let separation = point_segment_distance(predicted, start, end);
        // 拦截高度与执行层一致：高吊球从人的头顶飞过，只有手能伸到那么高
        // 的人才碰得到。因此高吊球的走廊按更小的有效半径算。
        // 高吊球从人的头顶飞过：球在空中时防守人伸手够不到，因此走廊
        // 按更小的有效半径算。球的飞行时间越久，防守人越有时间跑到球
        // 的落点旁边，所以这个折扣随飞行时间减弱。
        let lob = kind == crate::action::PassKind::Lob;
        let effective_radius = if lob {
            intercept_radius * (0.35 + 0.3 * (flight as f32).min(1.5))
        } else {
            intercept_radius
        };
        clearance = clearance.min(separation - effective_radius);
    }
    let lane = clearance.clamp(-3.0, 6.0);
    let catch_window = catch_radius / (catch_radius + distance * 0.06);
    // 走廊被防守人占住时，完成概率必须真的低下去：接球窗口只在走廊干净
    // 时才起作用。执行层是硬几何相交——球从防守人身边过去就会被碰到，
    // 因此模型里的走廊项也必须是同一个硬条件。
    let lane_factor = if lane <= 0.0 {
        0.0
    } else {
        crate::resolve::sigmoid(2.0 * lane - 0.5)
    };
    // 接球窗口：距离越远、接球人手感越差，越容易接不住。干净传球的上限
    // 接近联盟水平（约 85%），而不是 100%。
    let catch_factor = crate::resolve::sigmoid(2.5 * catch_window + 0.35);
    lane_factor * catch_factor
}

/// 接球点：接球人将要到达的位置。
///
/// 球飞向接球人正在跑去的接球点（他的站位），而不是他此刻站的地方。
/// 瞄向他此刻的位置会让接球人往回跑，球落在他原来的位置上，进攻于是
/// 离筐越来越远。价值层与执行层必须用同一个接球点。
pub fn catch_point(world: &World, receiver: usize) -> Vec2 {
    let player = &world.players[receiver];
    let target = player.target;
    // 目标点不可达时（太远或还在变化）用当前位置，避免把球传到空地。
    if (target - player.pos).length() > 12.0 {
        player.pos
    } else {
        target
    }
}

/// 传球价值：接球后的完整后果乘以完成概率，失败按失误成本计入。
pub fn pass_value(world: &World, from: usize, to: usize) -> f32 {
    let probability = pass_completion_probability(world, from, to, crate::action::PassKind::Chest);
    let point = catch_point(world, to);
    let contest = contest_for(world, to, point);
    let caught = catch_value(world, point, to, contest);
    let turnover = world.params.policy.w("epv_turnover_cost")
        * world.params.policy.w("possession_value");
    probability * caught - (1.0 - probability) * turnover
}

/// 突破的后续选择分布。
///
/// 成功推进后只剩一条后续：到达终结距离就终结，没到就急停跳投。传球是
/// 决策点上的另一个候选，不在突破价值里重复计入——否则「突破 + 终结」
/// 与「传球 + 接球人终结」会各自拿到一次完整的回合收益，突破就会被
/// 永远选中而球权时间被耗尽。
///
/// 受阻时只保留非负的保留球权价值；失误成本由突破候选单独计入。
pub fn drive_value(
    world: &World,
    state: &DecisionState,
    p_success: f32,
) -> f32 {
    let policy = &world.params.policy;
    // 突破的整组后果：成功时挤过去在篮下终结，受阻时只剩下保留球权的价值。
    let finish_pos = state.hoop
        + (state.actor_pos - state.hoop).normalize_or_zero()
            * policy.g("finish_min_distance_ft");
    let finish = shot_value(
        world,
        state.actor,
        finish_pos,
        crate::action::ShotKind::DrivingLayup,
        crate::state::BallPhase::Drive,
    );
    let failure = retained_value(world);
    let p_success = p_success.clamp(0.0, 1.0);
    p_success * finish + (1.0 - p_success) * failure
}

/// 保留球权的价值：非负的保留价值，加上「把剩下的进攻时钟用掉」期权。
///
/// 进攻时钟上的每一秒都对应一份回合价值：把回合价值除以一个回合的时长
/// 就是每秒的价格。手里还有 14 秒的进攻时钟，就还值大约一个回合的
/// 选择权。这一项让进攻方在时间充裕时愿意多传一次、多跑一次，而不是
/// 看到第一个能投的机会就出手——这正是真实比赛里回合时长接近 14 秒的
/// 原因。没有它，持球候选只有 0.22 的保留价值，出手永远压过持球，
/// 回合会在 9 秒内结束。
pub fn dribble_value(world: &World, shot_clock: f64) -> f32 {
    let policy = &world.params.policy;
    let per_second =
        policy.w("possession_value") / policy.w("seconds_per_possession").max(1.0);
    let remaining = shot_clock.max(0.0) as f32;
    retained_value(world)
        + per_second * remaining * policy.w("hold_continuation_scale")
}

pub fn pass_kind(_kind: PassKind) -> PassKind {
    _kind
}
