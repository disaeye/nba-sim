//! 球的唯一持有者、飞行积分与接触。
//!
//! 出手与传球的初始速度由抛体数学解出；之后每 tick 按自适应子步积分，
//! 子步位移小于球的最小接触间隙，接触点用子步内的最早时刻。命中不是
//! 出手时抽出来的结果：球必须真的穿过筐口，触筐、触板、触地、触手都
//! 会产生带连续时刻的事实。

use glam::Vec2;
use nba_domain::projectile::ProjectileArc;

use crate::action::{Fact, PassKind};
use crate::state::BallState;
use crate::world::World;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassResult {
    Caught { to: usize },
    Intercepted { by: usize },
    Incomplete,
}

#[derive(Debug, Clone, Copy)]
pub struct PassInFlight {
    pub pass_id: u64,
    pub from: usize,
    pub to: usize,
    pub kind: PassKind,
}

#[derive(Debug, Clone)]
pub struct BallBody {
    pub state: BallState,
    pub holder: Option<usize>,
    pub pos: Vec2,
    pub z: f32,
    pub vel: Vec2,
    pub vz: f32,
}

/// 一次出手释放所需的全部量。
///
/// 单独成结构是因为参数太多：函数签名超过七个参数后，调用方只能靠
/// 位置记住哪个是哪个。
#[derive(Debug, Clone, Copy)]
pub struct ShotRelease<'a> {
    pub now: f64,
    pub shot_id: u64,
    pub shooter: usize,
    pub hoop: Vec2,
    pub release_z: f32,
    pub aperture_ft: f32,
    pub origin: Vec2,
    pub plan: crate::shot_quality::ShotPlan,
    pub aim_offset: Vec2,
    pub rules: &'a nba_domain::GameRules,
}

/// 一次传球释放所需的全部量。
#[derive(Debug, Clone, Copy)]
pub struct PassRelease<'a> {
    pub now: f64,
    pub pass_id: u64,
    pub from: usize,
    pub to: usize,
    pub kind: PassKind,
    pub landing: Option<Vec2>,
    pub flight: Option<f64>,
    pub from_player: &'a crate::state::PlayerState,
    pub to_player: &'a crate::state::PlayerState,
    pub rules: &'a nba_domain::GameRules,
}

impl BallBody {
    pub fn loose(pos: Vec2, vel: Vec2) -> Self {
        Self {
            state: BallState::Loose { pos, vel },
            holder: None,
            pos,
            z: 0.5,
            vel,
            vz: 0.0,
        }
    }

    pub fn held(handle: usize) -> Self {
        Self {
            state: BallState::Held,
            holder: Some(handle),
            pos: Vec2::ZERO,
            z: 0.0,
            vel: Vec2::ZERO,
            vz: 0.0,
        }
    }

    pub fn set_held(&mut self, handle: usize) {
        self.state = BallState::Held;
        self.holder = Some(handle);
        self.vel = Vec2::ZERO;
        self.vz = 0.0;
    }

    pub fn release(&mut self, state: BallState) {
        self.state = state;
        self.holder = None;
    }

    pub fn release_shot(&mut self, release: ShotRelease<'_>) {
        // 球飞向本次抽样的落点，因此弹道、触筐位置与封盖接触逐次不同。
        // 有效筐口在出手时算好并随球体一起传下去：价值层预测的命中率与
        // 执行层的穿筐判定用的是同一个值。
        //
        // 飞行时长与竖直初速按「瞄向篮筐」的基准弹道解算，球心因此正好
        // 在筐平面上穿过落点；水平方向则瞄向抽样落点，于是球心穿过筐
        // 平面的位置就是这次抽样的落点。
        let landing = release.hoop + release.aim_offset;
        let from = release.origin;
        let z0 = release.release_z.max(release.rules.ball_radius_ft);
        let duration = release.plan.duration;
        self.state = BallState::Shot {
            shot_id: release.shot_id,
            shooter: release.shooter,
            from_pos: from,
            target: landing,
            start: release.now,
            aperture_ft: release.aperture_ft,
            aim_offset: release.aim_offset,
        };
        self.holder = None;
        self.pos = from;
        self.z = z0;
        // 水平方向飞向落点，竖直方向按同一次飞行解算：球心必须在落点
        // 处正好穿过筐平面，否则穿筐点与瞄准半径不是同一个位置，模型
        // 预测的命中率与实际命中就不是同一件事。
        self.vel = (landing - from) / duration;
        self.vz = release.plan.vz0;
    }

    pub fn release_pass(&mut self, release: PassRelease<'_>) {
        let rules = release.rules;
        let from_player = release.from_player;
        let to_player = release.to_player;
        let origin = from_player.pos
            + from_player.facing.normalize_or_zero() * rules.ball_holder_offset_ft;
        let aim = release.landing.unwrap_or(to_player.pos);
        let distance = (aim - origin).length();
        let duration = release
            .flight
            .unwrap_or_else(|| f64::from(rules.pass_duration(distance, false)));
        let target = aim.clamp(
            Vec2::new(rules.ball_radius_ft, rules.ball_radius_ft),
            Vec2::new(
                rules.court.width_ft - rules.ball_radius_ft,
                rules.court.height_ft - rules.ball_radius_ft,
            ),
        );
        let z0 = rules.chest_height_ft + 0.5;
        // 高吊球走一条高弧线：它必须从防守人头顶飞过，落点才在接球人手里。
        // 落点高度仍是接球人够得到的胸口高度。
        let z1 = rules.chest_height_ft + 0.5;
        // 高吊球的飞行时间长，平直传球短；同样的抛体解算因此自然给出
        // 一条更高的弧线：球从防守人头顶飞过，落点仍是接球人够得到的
        // 胸口高度。
        let arc = ProjectileArc::solve(z0, z1, duration as f32, rules.ball_gravity_ftps2);
        self.state = BallState::Pass {
            pass_id: release.pass_id,
            from: release.from,
            to: release.to,
            kind: release.kind,
            from_pos: origin,
            to_pos: target,
            start: release.now,
            duration,
        };
        self.holder = None;
        self.pos = origin;
        self.z = z0;
        self.vel = (target - origin) / duration as f32;
        self.vz = arc.vz0;
    }
}

/// 每 tick 推进球。持球时球跟随手；飞行时积分并在最早接触处停下。
pub fn step(world: &mut World, dt: f64) {
    if world.ball.holder.is_some() {
        sync_held(world);
        return;
    }
    match world.ball.state {
        BallState::Held => sync_held(world),
        BallState::Loose { .. } => step_loose(world, dt),
        BallState::Pass { .. } | BallState::Shot { .. } => step_flight(world, dt),
    }
}

fn sync_held(world: &mut World) {
    let Some(handle) = world.ball.holder else {
        return;
    };
    let player = &world.players[handle];
    let offset = world.rules.ball_holder_offset_ft;
    world.ball.pos = player.pos + player.facing.normalize_or_zero() * offset;
    world.ball.z = world.rules.ball_holder_height_ft;
    world.ball.vel = player.vel;
    world.ball.vz = 0.0;
}

fn step_loose(world: &mut World, dt: f64) {
    let g = world.rules.ball_gravity_ftps2;
    let radius = world.rules.ball_radius_ft;
    let restitution = world.rules.loose_ball_player_restitution;
    let dt = dt as f32;
    if world.pending_rebound.is_some() {
        let spot = crate::resolve::rebound::rebound_spot(world);
        for player in world.players.iter_mut().filter(|player| player.on_court) {
            player.target = spot;
            player.target_kind = crate::state::TargetKind::Window;
            player.target_partner = usize::MAX;
        }
    }
    let mut ball = world.ball.clone();
    let on_floor = ball.z <= radius && ball.vz.abs() < 1.0;
    if !on_floor {
        ball.vz -= g * dt;
    } else {
        ball.vz = 0.0;
        ball.vel *= 0.95;
    }
    ball.pos += ball.vel * dt;
    ball.z = (ball.z + ball.vz * dt).max(radius);
    let width = world.rules.court.width_ft;
    let height = world.rules.court.height_ft;
    let bounded = Vec2::new(
        ball.pos.x.clamp(radius, width - radius),
        ball.pos.y.clamp(radius, height - radius),
    );
    if bounded != ball.pos {
        ball.pos = bounded;
        ball.vel.x = -ball.vel.x * restitution;
        ball.vel.y = -ball.vel.y * restitution;
        world.emit(Fact::BallContact {
            kind: "out_of_bounds",
            entity: None,
            time: world.now(),
            x: ball.pos.x,
            y: ball.pos.y,
            z: ball.z.max(radius),
            sub_tick: 0,
        });
    }
    if ball.z <= radius && ball.vz < -1.0 {
        ball.z = radius;
        ball.vz = -ball.vz * restitution;
        ball.vel *= 0.8;
        world.emit(Fact::BallContact {
            kind: "floor",
            entity: None,
            time: world.now(),
            x: ball.pos.x,
            y: ball.pos.y,
            z: radius,
            sub_tick: 0,
        });
    }
    world.ball = ball;
    try_control_loose(world);
}

fn try_control_loose(world: &mut World) {
    let Some(state) = world.pending_rebound else {
        return;
    };
    if world.ball.z > 8.0 {
        return;
    }
    let elapsed = world.now() - state.started_at;
    // 长时间无人控制时必须结束松球状态，正常争抢仍按能力触及半径结算。
    let force_pickup = elapsed > 2.0;
    let base = world.params.policy.w("loose_ball_catch_base_ft");
    let skill = world.params.policy.w("loose_ball_catch_skill_ft");
    let mut best: Option<(usize, f32)> = None;
    for &handle in world.on_court_home.as_slice().iter().chain(world.on_court_away.as_slice()) {
        let player = &world.players[handle];
        let is_offense = player.team == state.offense;
        let attribute = if is_offense {
            "offensive_rebound"
        } else {
            "defensive_rebound"
        };
        let base_reach = base + player.attribute(attribute) * skill;
        let reach = if force_pickup {
            if is_offense { 16.0 } else { 50.0 }
        } else if is_offense {
            base_reach - 1.15
        } else {
            base_reach + 1.75
        };
        let vertical = (world.ball.z - 4.0).max(0.0);
        let distance = (player.pos - world.ball.pos).length().hypot(vertical);
        let margin = reach - distance;
        if margin >= 0.0 && best.is_none_or(|(_, current)| margin > current) {
            best = Some((handle, margin));
        }
    }
    if let Some((handle, _)) = best {
        crate::resolve::rebound::control(world, handle);
    }
}

fn step_flight(world: &mut World, dt: f64) {
    let rules = world.rules.clone();
    let dt = dt as f32;
    let start_time = world.now() - f64::from(dt);
    let mut ball = world.ball.clone();
    let mut remaining = dt;
    let mut elapsed = 0.0f32;
    let mut sub_tick = 0u16;
    let g = rules.ball_gravity_ftps2;
    let radius = rules.ball_radius_ft;
    let clearance = radius.min(radius + rules.rim_radius_ft * 0.1);
    while remaining > f32::MIN_POSITIVE {
        let speed = (ball.vel.length_squared() + ball.vz * ball.vz)
            .sqrt()
            .max(1.0);
        let h = (0.5 * clearance / speed).min(1.0 / 240.0).min(remaining);
        let prev_z = ball.z;
        let prev_pos = ball.pos;
        let next_vz = ball.vz - g * h;
        let next_z = ball.z + ball.vz * h - 0.5 * g * h * h;
        let next_pos = ball.pos + ball.vel * h;
        // 穿过筐口：球心连续穿越筐平面时与筐心的距离不超过有效筐口。
        // 有效筐口由出手时的 ShotModel 给出，与价值层预测用的是同一个值；
        // 球飞向本次抽样的落点，因此落点就是球心穿过筐平面的位置。
        if let BallState::Shot {
            shot_id: _, shooter, aperture_ft, ..
        } = &ball.state
        {
            if prev_z > rules.rim_height_ft && next_z <= rules.rim_height_ft {
                let t = (prev_z - rules.rim_height_ft) / (prev_z - next_z).max(f32::MIN_POSITIVE);
                let crossing = prev_pos.lerp(next_pos, t);
                let hoop = world.hoop_for(world.players[*shooter].team);
                if (crossing - hoop).length() <= *aperture_ft {
                    if let Some(pending) = world.pending_shot.as_mut() {
                        pending.result = Some(crate::resolve::shot::ShotResult::Made);
                    }
                    let sub = sub_tick;
                    world.emit(Fact::BallContact {
                        kind: "rim_swish",
                        entity: None,
                        time: start_time + f64::from(elapsed + h * t),
                        x: crossing.x,
                        y: crossing.y,
                        z: rules.rim_height_ft,
                        sub_tick: sub,
                    });
                    ball.pos = crossing;
                    ball.z = rules.rim_height_ft;
                    ball.vel *= 0.2;
                    ball.vz = -3.0;
                    ball.state = BallState::Loose {
                        pos: crossing,
                        vel: ball.vel,
                    };
                    world.ball = ball;
                    return;
                }
            }
        }
        // 地板：解析求最早触地时刻。
        if next_z <= radius {
            let a = -0.5 * g;
            let b = ball.vz;
            let c = ball.z - radius;
            let disc = b * b - 4.0 * a * c;
            let t = if disc >= 0.0 {
                let root = disc.sqrt();
                let t1 = (-b + root) / (2.0 * a);
                let t2 = (-b - root) / (2.0 * a);
                [t1, t2]
                    .into_iter()
                    .filter(|value| *value >= 0.0 && *value <= h)
                    .fold(f32::INFINITY, f32::min)
            } else {
                h
            };
            if t.is_finite() {
                let contact = prev_pos + ball.vel * t;
                ball.pos = contact;
                ball.z = radius;
                ball.vel *= 0.8;
                ball.vz = -ball.vz * 0.55;
                world.emit(Fact::BallContact {
                    kind: "floor",
                    entity: None,
                    time: start_time + f64::from(elapsed + t),
                    x: contact.x,
                    y: contact.y,
                    z: radius,
                    sub_tick,
                });
                finish_flight(world, &mut ball, false);
                world.ball = ball;
                return;
            }
        }
        // 篮板：垂直于底线的平面，按 y/z 窗口截断。
        if let Some((t, normal)) = backboard_contact(&ball, h, &rules) {
            let contact = prev_pos + ball.vel * t;
            let z = ball.z + ball.vz * t - 0.5 * g * t * t;
            ball.pos = contact;
            ball.z = z;
            ball.vel.x = -ball.vel.x * rules.backboard_restitution * normal;
            ball.vz = -ball.vz * rules.backboard_vertical_restitution;
            world.emit(Fact::BallContact {
                kind: "backboard",
                entity: None,
                time: start_time + f64::from(elapsed + t),
                x: contact.x,
                y: contact.y,
                z,
                sub_tick,
            });
            elapsed += t;
            remaining -= t;
            sub_tick += 1;
            continue;
        }
        // 篮筐：球心到环管的距离。
        //
        // 筐环接触与穿筐是同一件事的两面：球心穿过筐平面时与筐心的水平
        // 距离落在有效筐口内就是干净穿筐，否则就是触筐。触筐判据用球心
        // 到环管的距离检测，并且只在球心处于筐平面附近（上下各一个球
        // 半径）时生效——否则上升途中的球会先碰到还没到的筐沿。
        if let Some(hoop) = shot_hoop(world, &ball) {
            let rho = (next_pos - hoop).length();
            let dz = next_z - rules.rim_height_ft;
            let tube = radius + rules.rim_radius_ft * 0.1;
            let near_plane = dz.abs() <= radius;
            // 已经穿过筐口的球不再与筐沿接触；其余球按球心到环管的距离判定。
            let aperture = match &ball.state {
                BallState::Shot { aperture_ft, .. } => *aperture_ft,
                _ => f32::INFINITY,
            };
            // 「是否穿筐口」必须按球心穿过筐平面的位置判断：球在筐口
            // 上方还有好几个子步，那里的水平距离比穿越点大，用它判定会
            // 把本该命中的球判成触筐。
            let inside_opening = if ball.vz < 0.0 && prev_z > rules.rim_height_ft {
                let t = (prev_z - rules.rim_height_ft)
                    / (prev_z - next_z).max(f32::MIN_POSITIVE);
                let crossing = prev_pos.lerp(next_pos, t);
                // 穿越点必须与出手时抽到的瞄准半径一致：球飞向落点，球心穿过
                // 筐平面的位置就是落点。有效筐口用出手时定价的那一个，
                // 价值层与执行层因此看同一个量。
                (crossing - hoop).length() <= aperture
            } else {
                false
            };
            if ball.vz < 0.0
                && near_plane
                && !inside_opening
                && (rho - rules.rim_radius_ft).hypot(dz) <= tube
            {
                let contact = prev_pos.lerp(next_pos, 0.5);
                ball.pos = contact;
                ball.z = next_z;
                ball.vel *= -rules.rim_contact_restitution_graze;
                ball.vz = -ball.vz * rules.rim_contact_vertical_restitution;
                world.emit(Fact::BallContact {
                    kind: "rim",
                    entity: None,
                    time: start_time + f64::from(elapsed + h),
                    x: contact.x,
                    y: contact.y,
                    z: ball.z,
                    sub_tick,
                });
                if let Some(pending) = world.pending_shot.as_mut() {
                    if pending.result.is_none() {
                        pending.result = Some(crate::resolve::shot::ShotResult::Missed {
                            rim_contact: true,
                        });
                    }
                }
                ball.state = BallState::Loose {
                    pos: contact,
                    vel: ball.vel,
                };
                world.ball = ball;
                return;
            }
        }
        // 罚球不允许防守人触球；其他出手仍由手部接触决定封盖。
        if world.pending_shot.as_ref().is_some_and(|pending| pending.free_throw.is_some()) {
        } else if let Some((handle, kind)) = player_contact(world, next_pos, next_z, &rules) {
            let contact = prev_pos.lerp(next_pos, 0.5);
            ball.pos = contact;
            ball.z = next_z;
            world.emit(Fact::BallContact {
                kind,
                entity: Some(handle),
                time: start_time + f64::from(elapsed + h * 0.5),
                x: contact.x,
                y: contact.y,
                z: next_z,
                sub_tick,
            });
            handle_player_contact(world, &mut ball, handle, kind);
            world.ball = ball;
            return;
        }
        ball.pos = next_pos;
        ball.z = next_z;
        ball.vz = next_vz;
        elapsed += h;
        remaining -= h;
        sub_tick += 1;
    }
    world.ball = ball;
}

fn finish_flight(world: &mut World, ball: &mut BallBody, rim_contact: bool) {
    match ball.state {
        BallState::Shot { .. } => {
            if let Some(pending) = world.pending_shot.as_mut() {
                if pending.result.is_none() {
                    pending.result = Some(crate::resolve::shot::ShotResult::Missed { rim_contact });
                }
            }
            ball.state = BallState::Loose {
                pos: ball.pos,
                vel: ball.vel,
            };
        }
        BallState::Pass { .. } => {
            world.pass_result = Some(PassResult::Incomplete);
            ball.state = BallState::Loose {
                pos: ball.pos,
                vel: ball.vel,
            };
        }
        _ => {}
    }
}

fn handle_player_contact(world: &mut World, ball: &mut BallBody, handle: usize, kind: &'static str) {
    match ball.state {
        BallState::Shot { .. } => {
            if let Some(pending) = world.pending_shot.as_mut() {
                if pending.result.is_none() {
                    pending.result = Some(crate::resolve::shot::ShotResult::Blocked { by: handle });
                }
            }
            ball.vel *= -world.rules.loose_ball_player_restitution;
            ball.vz = -ball.vz * world.rules.loose_ball_player_restitution;
            ball.state = BallState::Loose {
                pos: ball.pos,
                vel: ball.vel,
            };
        }
        BallState::Pass { to, .. } => {
            if kind == "player" && handle != to {
                // 躯干挡到球只是改变球路，干净抢断要手碰到球。
                ball.vel *= -world.rules.loose_ball_player_restitution;
                ball.vz = -ball.vz * world.rules.loose_ball_player_restitution;
                return;
            }
            world.pass_result = Some(if handle == to {
                PassResult::Caught { to }
            } else {
                PassResult::Intercepted { by: handle }
            });
            ball.state = BallState::Loose {
                pos: ball.pos,
                vel: ball.vel,
            };
        }
        BallState::Loose { .. } => {
            ball.vel *= -world.rules.loose_ball_player_restitution;
            ball.vz = -ball.vz * world.rules.loose_ball_player_restitution;
        }
        BallState::Held => {}
    }
    let _ = kind;
}

fn shot_hoop(world: &World, ball: &BallBody) -> Option<Vec2> {
    match &ball.state {
        BallState::Shot { shooter, .. } => Some(world.hoop_for(world.players[*shooter].team)),
        _ => None,
    }
}

fn backboard_contact(ball: &BallBody, h: f32, rules: &nba_domain::GameRules) -> Option<(f32, f32)> {
    let width = rules.court.width_ft;
    let hoop = if ball.pos.x < width * 0.5 {
        rules.court.hoop_left_x_ft
    } else {
        rules.court.hoop_right_x_ft
    };
    let board = if hoop < width * 0.5 {
        rules.backboard_offset_from_baseline_ft
    } else {
        width - rules.backboard_offset_from_baseline_ft
    };
    let plane = board + rules.ball_radius_ft;
    let (start, end) = (ball.pos.x, ball.pos.x + ball.vel.x * h);
    let crosses = (start - plane).signum() != (end - plane).signum();
    if !crosses || ball.vel.x.abs() < f32::MIN_POSITIVE {
        return None;
    }
    let t = (plane - start) / ball.vel.x;
    if !(0.0..=h).contains(&t) {
        return None;
    }
    let y = ball.pos.y + ball.vel.y * t;
    let z = ball.z + ball.vz * t - 0.5 * rules.ball_gravity_ftps2 * t * t;
    if (y - rules.court.hoop_y_ft).abs() > rules.backboard_width_ft * 0.5
        || !(rules.backboard_bottom_height_ft..=rules.backboard_top_height_ft).contains(&z)
    {
        return None;
    }
    Some((t, 1.0))
}

/// 传球接球半径：价值模型与球体接触共用这一个定义。
pub fn pass_catch_radius(
    rules: &nba_domain::GameRules,
    receiver: &crate::state::PlayerState,
) -> f32 {
    rules.catch_radius_base_ft
        + receiver.data.attributes.ball_handling * rules.catch_radius_skill_gain_ft
}

/// 传球拦截半径：价值模型与球体接触共用这一个定义。
pub fn pass_intercept_radius(
    rules: &nba_domain::GameRules,
    defender: &crate::state::PlayerState,
) -> f32 {
    // 抄球半径随抢断属性放大：抢断能力强的防守人真的能把球断下来。
    rules.ball_radius_ft + 0.92 + defender.attribute("steal") * 1.65
}

fn player_contact(
    world: &World,
    next_pos: Vec2,
    next_z: f32,
    rules: &nba_domain::GameRules,
) -> Option<(usize, &'static str)> {
    let ball_radius = rules.ball_radius_ft;
    let pass = world.pass_in_flight;
    let body_radius = rules.body_contact_radius_ft + ball_radius;
    let skip = if let Some(info) = pass {
        Some(info.from)
    } else {
        world.pending_shot.as_ref().map(|pending| pending.shooter)
    };
    let mut best: Option<(usize, &'static str, f32)> = None;
    for (handle, player) in world.players.iter().enumerate() {
        if !player.on_court || Some(handle) == skip {
            continue;
        }
        let hand_radius = if pass.is_some() {
            pass_intercept_radius(rules, player)
        } else {
            ball_radius + 0.31 + player.attribute("block") * 0.80
        };
        if let Some(info) = pass {
            if handle == info.to {
                let catch = pass_catch_radius(rules, player);
                let distance = (next_pos - player.pos).length();
                if distance <= catch && (1.5..=8.0).contains(&next_z) {
                    return Some((handle, "hand"));
                }
            }
            // 防守人挡在传球路线上就会碰到球：拦截半径与价值模型用的是
            // 同一个定义，因此「模型说会断」与「执行层真的断」是同一件事。
            //
            // 拦截用手的高度：球必须飞到防守人举得到的高度才碰得到。
            // 高吊球在上升段从人的头顶飞过，防守人伸手够不到；平直传球
            // 走胸口高度，防守人伸手就能碰到。
            // 刚出手的一瞬间球还在传球人手里，防守人贴��身就能碰到；
            // 球离开传球人一个身位之后才谈得上「被断」。
            if (next_pos - world.players[info.from].pos).length() < rules.ball_radius_ft {
                continue;
            }
            let intercept = pass_intercept_radius(rules, player);
            // 拦截用的是防守人在传球高度上伸手够到的范围：平直传球走胸口，
            // 高吊球从头顶飞过就够不到。球的高度与手的��度一起决定这次
            // 拦截是否成立。
            //
            // 抄球能力越强，手伸得越远：拦截半径按抢断能力放大，这样
            // 防守人的抢断属性才会真的改变结果。
            let hand_height = rules.chest_height_ft + 1.0;
            let distance = (next_pos - player.pos).length().hypot(next_z - hand_height);
            if distance <= intercept {
                return Some((handle, "hand"));
            }
            continue;
        }
        let hand = player.pos
            + player.facing.normalize_or_zero()
                * (rules.ball_holder_offset_ft + 0.4);
        let hand_z = if pass.is_some() {
            rules.chest_height_ft + 1.0
        } else {
            rules.chest_height_ft + 1.0 + player.attribute("block") * 2.5
        };
        let hand_distance = (next_pos - hand).length().hypot(next_z - hand_z);
        if hand_distance <= hand_radius {
            let score = hand_distance;
            if best.is_none_or(|(_, _, current)| score < current) {
                best = Some((handle, "hand", score));
            }
            continue;
        }
        if pass.is_some() {
            continue;
        }
        let horizontal = (next_pos - player.pos).length();
        if horizontal <= body_radius && (0.0..=6.8).contains(&next_z) {
            let score = horizontal + 0.1;
            if best.is_none_or(|(_, _, current)| score < current) {
                best = Some((handle, "player", score));
            }
        }
    }
    best.map(|(handle, kind, _)| (handle, kind))
}

#[cfg(test)]
mod tests {

    #[test]
    fn clearance_is_below_contact_radius() {
        let rules = nba_domain::GameRules::default();
        let clearance = rules
            .ball_radius_ft
            .min(rules.ball_radius_ft + rules.rim_radius_ft * 0.1);
        assert!(clearance < rules.ball_radius_ft + rules.rim_radius_ft);
    }

    #[test]
    fn substep_at_85_ftps_moves_less_than_ball_radius() {
        let rules = nba_domain::GameRules::default();
        let speed = rules.ball_max_speed_ftps;
        let clearance = rules.ball_radius_ft;
        let h = (0.5 * clearance / speed).min(1.0 / 240.0);
        assert!(speed * h < rules.ball_radius_ft);
    }
}
