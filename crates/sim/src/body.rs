//! 十名球员的运动积分。
//!
//! 位置只在 `body` 模块内写。球员朝目标点加速，速度与加速度受规则包络
//! 限制；身体重叠按每 tick 上限分离；位置被夹在球场内。其他模块只写
//! `target`，不写 `pos`。

use glam::Vec2;
use nba_domain::rules::GameRules;

use crate::state::PlayerState;
use crate::world::World;

/// 只用于开场与替补席布置。比赛进行中的位置变化全部由 [`integrate`] 产生。
pub(crate) fn place(world: &mut World, handle: usize, pos: Vec2) {
    let now = world.now();
    let player = &mut world.players[handle];
    player.pos = pos;
    player.vel = Vec2::ZERO;
    player.target = pos;
    player.target_kind = crate::state::TargetKind::Station;
    player.target_partner = usize::MAX;
    player.target_changed_at = now;
    player.home_spot = pos;
}

/// 推进所有在场球员一步。
pub fn integrate(world: &mut World, dt: f64) {
    let dt = dt as f32;
    let rules = &world.rules;
    let max_speed = rules.max_player_speed_ftps.max(f32::MIN_POSITIVE);
    let max_accel = rules.max_player_accel_ftps2.max(f32::MIN_POSITIVE);
    let radius = rules.player_radius_ft;
    let width = rules.court.width_ft;
    let height = rules.court.height_ft;
    let min_separation = rules.min_player_separation_ft;
    let policy = &world.params.policy;
    let now = world.now();
    let holder = world.holder();
    let live = world.live_play();
    let inbound_passer = if matches!(world.phase(), crate::state::Phase::Inbound { .. }) {
        world.inbound_passer
    } else {
        None
    };
    let screen_screener = world
        .players
        .iter()
        .enumerate()
        .find_map(|(handle, player)| {
            player.action.as_ref().and_then(|runtime| {
                (runtime.role == crate::action::ActionIntentRole::ScreenPartner).then_some(handle)
            })
        });
    // 空切的人：高吊球的接球人必须往切点跑，且不参与绕行判定。
    // 传球在空中的接球人同样适用：球正飞向他，他停下等人传球就落地了。
    let cut_cutter = world.pass_in_flight.map(|info| info.to).or_else(|| {
        world
            .players
            .iter()
            .enumerate()
            .find_map(|(handle, player)| {
                player.action.as_ref().and_then(|runtime| {
                    (runtime.role == crate::action::ActionIntentRole::CutPartner).then_some(handle)
                })
            })
    });
    for (i, p) in world.players.iter().enumerate() {
        world.body_before[i] = p.pos;
    }
    let on_court_home = world.on_court_home;
    let on_court_away = world.on_court_away;
    // 贴身间隔每个 tick 只查一次校准表：它在绕行求解里按每对球员被读，
    // 放在循环内查表会让运动积分成为全场最热的路径。
    let pressure = crate::assignment::pressure_separation(world);
    // 映射系数同样每 tick 解析一次：每名球员都要读速度与加速度。
    let coefficients = crate::physical::Coefficients::from_policy(policy);
    let motion = Motion {
        dt,
        max_speed,
        max_accel,
        min_separation,
        pressure,
    };
    for &handle in on_court_home
        .as_slice()
        .iter()
        .chain(on_court_away.as_slice())
    {
        // 发球队员要走到发球点：发球点在他自己的底线，所以他走的是短距离，
        // 但他必须走——瞬移与原地不动都会让进攻在远离篮筐的地方开始。
        let is_inbound_passer = inbound_passer == Some(handle);
        let runtime = world.players[handle].action.as_ref();
        let has_action = runtime.is_some();
        let has_defensive_action = world
            .defensive_actions
            .get(handle)
            .and_then(Option::as_ref)
            .is_some_and(|runtime| {
                runtime.phase == nba_domain::action_window::ActionPhase::Execution
                    && !runtime.completed
            });
        let mut target =
            runtime.map_or(world.players[handle].target, |runtime| match runtime.role {
                crate::action::ActionIntentRole::Primary => runtime.intent.to_ft,
                crate::action::ActionIntentRole::CutPartner
                | crate::action::ActionIntentRole::ScreenPartner => world.players[handle].target,
            });
        if let Some(closeout_target) = world
            .defensive_actions
            .get(handle)
            .and_then(Option::as_ref)
            .filter(|runtime| {
                runtime.phase == nba_domain::action_window::ActionPhase::Execution
                    && !runtime.completed
            })
            .map(|runtime| runtime.target_position)
        {
            target = closeout_target;
        }
        if is_inbound_passer {
            target = world.players[handle].target;
        }
        let before = world.players[handle].pos;
        let player_side = world.players[handle].avoidance_side;
        // 在飞传球的接球人也不绕行：球正飞向他，绕路就接不到。
        let holder_or_actor = has_action
            || has_defensive_action
            || holder == Some(handle)
            || cut_cutter == Some(handle);
        let allow_forward = has_action
            || has_defensive_action
            || holder == Some(handle)
            || cut_cutter == Some(handle);
        let (steered, blocker, side, blocked) = if holder_or_actor {
            (target, usize::MAX, player_side, false)
        } else {
            steer_target(world, handle, target, motion, allow_forward)
        };
        {
            let player = &mut world.players[handle];
            player.avoidance_blocker = blocker;
            player.avoidance_side = side;
            let distance = (target - player.pos).length();
            if distance <= rules.arrival_epsilon_ft {
                // 已经站在目标点上：这不是「没有进展」，不该累积停滞时间。
                // 否则开场跳球这种全员静止的阶段会让每个人带着停滞计时进入回合，
                // 一拿到新目标就立即进入让路状态并原地停住。
                player.progress_best_ft = distance;
                player.nonprogress_seconds = 0.0;
            } else if distance + 0.1 < player.progress_best_ft {
                player.progress_best_ft = distance;
                player.nonprogress_seconds = 0.0;
            } else {
                player.nonprogress_seconds += dt as f64;
            }
            if player.nonprogress_seconds >= 0.5 && player.yielding_since == 0.0 {
                player.yielding_since = now;
            }
            if !blocked && player.nonprogress_seconds == 0.0 {
                player.yielding_since = 0.0;
            }
            let active_actor = has_action;
            let is_holder = holder == Some(handle);
            if active_actor
                || has_defensive_action
                || is_holder
                || screen_screener == Some(handle)
                || cut_cutter == Some(handle)
                || !live
            {
                player.yielding_since = 0.0;
                target = steered;
            } else if player.yielding_since > 0.0 {
                target = player.pos;
            } else {
                target = steered;
            }
        }
        let vel_before = world.players[handle].vel;
        let physical = crate::physical::Physical::with(&world.players[handle], &coefficients);
        let effective_speed = physical.speed_ftps.min(max_speed);
        let effective_accel = physical.accel_ftps2.min(max_accel);
        step(
            &mut world.players[handle],
            target,
            effective_speed,
            effective_accel,
            motion,
            rules,
        );
        {
            let player = &mut world.players[handle];
            let change = player.vel - vel_before;
            let limit = effective_accel * dt;
            let change_sq = change.length_squared();
            if change_sq > limit * limit {
                player.vel = vel_before + change / change_sq.sqrt() * limit;
            }
        }
        let after = world.players[handle].pos;
        let player = &mut world.players[handle];
        player.last_motion_ft = (after - before).length();
        player.last_separation_ft = 0.0;
        if player.recovery_since > 0.0 || (player.target - player.pos).length() > min_separation {
            player.recovery_path_ft += player.last_motion_ft;
        }
    }
    separate(world, dt, max_speed, min_separation);
    for &handle in on_court_home
        .as_slice()
        .iter()
        .chain(on_court_away.as_slice())
    {
        let player = &mut world.players[handle];
        player.pos.x = player.pos.x.clamp(radius, width - radius);
        player.pos.y = player.pos.y.clamp(radius, height - radius);
        let delta = player.pos - world.body_before[handle];
        let limit = max_speed * dt;
        if delta.length() > limit {
            player.pos = world.body_before[handle] + delta.normalize_or_zero() * limit;
        }
    }
}

/// 一次运动积分所需的全部量。
///
/// 单独成结构是因为参数太多：签名超过七个参数后，调用方只能靠位置
/// 记住哪个是哪个。
#[derive(Debug, Clone, Copy)]
pub struct Motion {
    pub dt: f32,
    pub max_speed: f32,
    pub max_accel: f32,
    pub min_separation: f32,
    pub pressure: f32,
}

fn step(
    player: &mut PlayerState,
    target: Vec2,
    speed: f32,
    accel: f32,
    motion: Motion,
    rules: &GameRules,
) {
    let delta = target - player.pos;
    let distance = delta.length();
    if distance <= rules.arrival_epsilon_ft {
        player.vel = Vec2::ZERO;
        player.progress_best_ft = 0.0;
        player.nonprogress_seconds = 0.0;
        player.recovery_since = 0.0;
        player.recovery_d0_ft = 0.0;
        player.recovery_path_ft = 0.0;
        return;
    }
    let braking = rules.max_player_braking_accel_ftps2.max(f32::MIN_POSITIVE);
    let brake_limit = (2.0 * braking * distance).sqrt();
    let scale = (distance / (motion.max_speed * rules.arrival_speed_scale + 1.0))
        .clamp(rules.arrival_speed_floor, 1.0);
    let desired_speed = speed.min(brake_limit) * scale;
    let direction = rotate_toward(
        player.facing,
        delta / distance,
        rules.max_player_turn_rate_rad_per_sec * motion.dt,
    );
    let desired = direction * desired_speed;
    let change = desired - player.vel;
    let change_sq = change.length_squared();
    let decelerating = desired_speed * desired_speed < player.vel.length_squared();
    let max_change = if decelerating { braking } else { accel } * motion.dt;
    if change_sq > max_change * max_change {
        player.vel += change / change_sq.sqrt() * max_change;
    } else {
        player.vel = desired;
    }
    let vel_sq = player.vel.length_squared();
    if vel_sq > speed * speed {
        player.vel *= speed / vel_sq.sqrt();
    }
    player.pos += player.vel * motion.dt;
    player.facing = direction;
}

fn rotate_toward(from: Vec2, to: Vec2, max_angle: f32) -> Vec2 {
    let cross = from.perp_dot(to);
    let dot = from.dot(to).clamp(-1.0, 1.0);
    let angle = cross.atan2(dot);
    if angle.abs() <= max_angle {
        return to;
    }
    let limited = angle.clamp(-max_angle, max_angle);
    let (sin, cos) = limited.sin_cos();
    Vec2::new(from.x * cos - from.y * sin, from.x * sin + from.y * cos).normalize_or_zero()
}

fn steer_target(
    world: &World,
    handle: usize,
    target: Vec2,
    motion: Motion,
    allow_forward: bool,
) -> (Vec2, usize, i8, bool) {
    let player = &world.players[handle];
    let delta = target - player.pos;
    let distance = delta.length();
    if distance <= f32::MIN_POSITIVE {
        return (target, usize::MAX, 0, false);
    }
    let direction = delta / distance;
    let mut blocker = usize::MAX;
    let mut blocker_distance = f32::MAX;
    for &other in world
        .on_court_home
        .as_slice()
        .iter()
        .chain(world.on_court_away.as_slice())
    {
        if other == handle {
            continue;
        }
        let candidate = &world.players[other];
        let offset = candidate.pos - player.pos;
        if offset.dot(direction) <= 0.0 {
            continue;
        }
        let pair_separation =
            pair_separation_fast(world, handle, other, motion.pressure, motion.min_separation);
        let avoid_range = pair_separation + motion.max_speed * motion.dt * 4.0 + 1.0;
        let sep_sq = offset.length_squared();
        if sep_sq <= f32::MIN_POSITIVE
            || sep_sq > avoid_range * avoid_range
            || sep_sq >= distance * distance
        {
            continue;
        }
        let separation = sep_sq.sqrt();
        if separation < blocker_distance {
            blocker_distance = separation;
            blocker = other;
        }
    }
    if blocker == usize::MAX {
        return (target, usize::MAX, 0, false);
    }
    let offset = world.players[blocker].pos - player.pos;
    let normal = offset / blocker_distance;
    let mut side = player.avoidance_side;
    if side == 0 {
        side = if direction.perp_dot(normal) >= 0.0 {
            1
        } else {
            -1
        };
    }
    let tangent = normal.perp() * f32::from(side);
    // 绕行点必须保留朝目标方向的分量：只沿着切向走，防守人永远关不到
    // 投篮人面前，进攻方就会得到没有干扰的出手。
    let radius = world.rules.player_radius_ft;
    let width = world.rules.court.width_ft;
    let height = world.rules.court.height_ft;
    let clear = motion.min_separation + world.rules.separation_safety_margin_ft;
    // 回避方向一经选定就不该改变：来回切换会让球员在两名防守人之间抖动。
    // 切向与目标方向相反时，允许前进的球员改为直接朝目标推进（间隔分离
    // 会把他推开），不允许前进的球员原地等待职责层换目标。
    let advance = if tangent.dot(direction) < 0.0 {
        if !allow_forward {
            return (player.pos, blocker, side, true);
        }
        direction
    } else {
        tangent
    };
    // 绕行点必须本身合法：不能落进场地外，也不能压在别人身上。场上十个人
    // 的站位很挤，随便往旁边挪一步常常正好踩在别人身上，于是球手被判定
    // 为「一直绕不出去」，整场比赛几乎没有一次成功出手。
    let mut chosen = player.pos + advance * distance.min(6.0);
    let mut fallback: Option<Vec2> = None;
    for step in [6.0f32, 4.0, 2.0, 1.0] {
        let candidate = player.pos + advance * step.min(distance);
        if candidate.x < radius
            || candidate.x > width - radius
            || candidate.y < radius
            || candidate.y > height - radius
        {
            continue;
        }
        let mut clear_of_others = true;
        let clear_sq = clear * clear;
        for &other in world
            .on_court_home
            .as_slice()
            .iter()
            .chain(world.on_court_away.as_slice())
        {
            if other == handle {
                continue;
            }
            if (world.players[other].pos - candidate).length_squared() < clear_sq {
                clear_of_others = false;
                break;
            }
        }
        if !clear_of_others {
            continue;
        }
        fallback = Some(candidate);
        break;
    }
    if let Some(candidate) = fallback {
        chosen = candidate;
    }
    (chosen, blocker, side, true)
}

/// 贴身的间隔值：只有对位或领防责任的跨队球员会被压到贴身距离。
///
/// `pressure` 由调用方传入，因为贴身间隔每个 tick 只查一次校准表，
/// 而这里的调用点在每对球员上，重复查表会让运动积分成为全场最热的路径。
fn pair_separation_fast(world: &World, a: usize, b: usize, pressure: f32, default: f32) -> f32 {
    let diff_teams = world.players[a].team != world.players[b].team;
    let on_ball = matches!(
        world.players[a].defense_responsibility,
        crate::state::DefenseResponsibility::Deny | crate::state::DefenseResponsibility::OnBall
    ) || matches!(
        world.players[b].defense_responsibility,
        crate::state::DefenseResponsibility::Deny | crate::state::DefenseResponsibility::OnBall
    );
    if diff_teams && on_ball {
        pressure
    } else {
        default
    }
}

fn separate(world: &mut World, dt: f32, max_speed: f32, min_separation: f32) {
    let limit = max_speed * dt * 0.5;
    let pressure = crate::assignment::pressure_separation(world);
    let mut handles = [0usize; 10];
    let home = world.on_court_home.as_slice();
    let away = world.on_court_away.as_slice();
    let total = home.len() + away.len();
    handles[..home.len()].copy_from_slice(home);
    handles[home.len()..total].copy_from_slice(away);
    for left in 0..total {
        for right in (left + 1)..total {
            let a = handles[left];
            let b = handles[right];
            let delta = world.players[b].pos - world.players[a].pos;
            let dist_sq = delta.length_squared();
            let separation = pair_separation_fast(world, a, b, pressure, min_separation);
            if dist_sq >= separation * separation {
                continue;
            }
            let distance = dist_sq.sqrt();
            let direction = if distance > f32::MIN_POSITIVE {
                delta / distance
            } else {
                Vec2::X
            };
            let correction = ((separation - distance) * 0.5).min(limit);
            let shift = direction * correction;
            world.players[a].pos -= shift;
            world.players[b].pos += shift;
            world.players[a].last_separation_ft += correction;
            world.players[b].last_separation_ft += correction;
        }
    }
}
