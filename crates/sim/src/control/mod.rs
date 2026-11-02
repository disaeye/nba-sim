//! 球的唯一持有者、飞行积分与接触。
//!
//! 出手与传球的初始速度由抛体数学解出；之后每 tick 按自适应子步积分，
//! 子步位移小于球的最小接触间隙，接触点用子步内的最早时刻。命中不是
//! 出手时抽出来的结果：球必须真的穿过筐口，触筐、触板、触地、触手都
//! 会产生带连续时刻的事实。

use glam::Vec2;
use nba_domain::projectile::ProjectileArc;

use crate::action::{Fact, PassKind, TurnoverKind};
use crate::draw::{purpose, Stream};
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
    /// 击地传球已经触过地：触地只算一次，之后按普通飞行处理。
    pub bounced: bool,
}

#[derive(Debug, Clone)]
pub struct BallBody {
    state: BallState,
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
    /// 碰到筐沿之后仍能补进的概率。与模型里那一项是同一个数。
    pub rim_in_probability: f32,
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
            pos,
            z: 0.5,
            vel,
            vz: 0.0,
        }
    }

    pub fn held(handle: usize) -> Self {
        Self {
            state: BallState::Held { holder: handle },
            pos: Vec2::ZERO,
            z: 0.0,
            vel: Vec2::ZERO,
            vz: 0.0,
        }
    }

    pub fn set_held(&mut self, handle: usize) {
        self.state = BallState::Held { holder: handle };
        self.vel = Vec2::ZERO;
        self.vz = 0.0;
    }

    pub fn release(&mut self, state: BallState) {
        self.state = state;
    }

    pub fn state(&self) -> BallState {
        self.state
    }

    pub fn holder(&self) -> Option<usize> {
        match self.state {
            BallState::Held { holder } => Some(holder),
            BallState::Pass { .. } | BallState::Shot { .. } | BallState::Loose { .. } => None,
        }
    }

    pub(crate) fn release_shot(&mut self, release: ShotRelease<'_>) {
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
            rim_in_probability: release.rim_in_probability,
            aim_offset: release.aim_offset,
        };
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
        let origin =
            from_player.pos + from_player.facing.normalize_or_zero() * rules.ball_holder_offset_ft;
        // 接球点由调用方的 `catch_point` 算好（提前量已包含在内），这里只用它。
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
        self.pos = origin;
        self.z = z0;
        self.vel = (target - origin) / duration as f32;
        // 击地传球：球在飞行过半时触地，弹起后继续飞向接球人。
        self.vz = if release.kind == PassKind::Bounce {
            let t1 = duration as f32 * 0.55;
            (rules.ball_radius_ft - z0 + 0.5 * rules.ball_gravity_ftps2 * t1 * t1) / t1
        } else {
            arc.vz0
        };
    }
}

/// 每 tick 推进球。持球时球跟随手；飞行时积分并在最早接触处停下。
pub fn step(world: &mut World, dt: f64) {
    if world.ball.holder().is_some() {
        sync_held(world);
        return;
    }
    match world.ball.state() {
        BallState::Held { .. } => sync_held(world),
        BallState::Loose { .. } => step_loose(world, dt),
        BallState::Pass { .. } | BallState::Shot { .. } => step_flight(world, dt),
    }
}

fn sync_held(world: &mut World) {
    let Some(handle) = world.ball.holder() else {
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
    if matches!(world.phase(), crate::state::Phase::JumpBall { .. }) {
        step_jump_ball_toss(world, dt);
        return;
    }
    if matches!(world.phase(), crate::state::Phase::JumpBallFlight) {
        step_jump_ball_flight(world, dt);
        return;
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
    // 球越过边线就死球，不弹回来：球场没有墙。最后触球的队伍交出
    // 球权，对方在出界点发球。
    let outside = ball.pos.x < radius
        || ball.pos.x > width - radius
        || ball.pos.y < radius
        || ball.pos.y > height - radius;
    if outside {
        // 慢球停在边线上、由球员捡起；只有真正带着向外速度的球才会出界。
        let inward = if ball.pos.x < radius {
            Vec2::X
        } else if ball.pos.x > width - radius {
            Vec2::NEG_X
        } else if ball.pos.y < radius {
            Vec2::Y
        } else {
            Vec2::NEG_Y
        };
        let threshold = world.params.policy.w("loose_ball_boundary_speed_ftps");
        let outward_speed = -ball.vel.dot(inward);
        if outward_speed <= threshold && ball.vz.abs() <= threshold {
            ball.pos = Vec2::new(
                ball.pos.x.clamp(radius, width - radius),
                ball.pos.y.clamp(radius, height - radius),
            );
            ball.vel = inward * threshold * 0.1;
            ball.z = ball.z.max(radius);
            world.ball = ball;
            try_control_loose(world);
            return;
        }
        let spot = Vec2::new(
            ball.pos.x.clamp(radius, width - radius),
            ball.pos.y.clamp(radius, height - radius),
        );
        ball.pos = spot;
        ball.vel = Vec2::ZERO;
        ball.vz = 0.0;
        ball.z = radius;
        world.ball = ball;
        out_of_bounds(world, spot);
        return;
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

/// 出界：球停在边线上，球权给最后触球方的对手。投篮出手后球无人
/// 控制地飞出界是投篮不中，不记失误；持球人或后场球员碰出界才是
/// 违例。
fn out_of_bounds(world: &mut World, spot: Vec2) {
    let touched = world.last_touch;
    let offense = world.possession.team;
    // 发球地点就是出界点：球从哪条线出去，就从哪里发回来。
    world.inbound_spot = Some(spot);
    world.emit(Fact::BallContact {
        kind: "out_of_bounds",
        entity: touched,
        time: world.now(),
        x: spot.x,
        y: spot.y,
        z: world.rules.ball_radius_ft,
        sub_tick: 0,
    });
    let touched_team = touched.map(|handle| world.players[handle].team);
    let recipient = match touched_team {
        Some(team) => team.other(),
        None => offense.other(),
    };
    // 最后触球的是进攻方就是进攻方违例；防守方碰出界则进攻方保留球权。
    let violation = touched_team.is_none_or(|team| team == offense);
    let shot_in_air = world.pending_shot.is_some();
    if violation && !shot_in_air {
        if let Some(handle) = touched {
            world.line_mut(handle).turnovers += 1;
            world.emit(Fact::Turnover {
                player: handle,
                kind: TurnoverKind::OutOfBounds,
            });
        }
    }
    // 停表后果统一走裁决提交入口：结束回合、记录发球地点与恢复方式。
    // 这条裁定可复核：丢球一方在复核窗口内可以申请教练挑战。
    world.record_ruling(recipient, "out_of_bounds");
    world.adjudicate(crate::adjudicate::Adjudication::violation(
        crate::adjudicate::ViolationKind::OutOfBounds,
        crate::adjudicate::RestartPlan::Inbound {
            team: recipient,
            location: crate::adjudicate::InboundLocation::Spot(spot),
            ready_after: None,
        },
    ));
    // 球必须在死球期间离手：否则下一 tick 的活球争抢会在回合已关闭
    //（possession.id == 0）的情况下接管球。发球流程稍后自己把球交给发球人。
    let handler = world
        .players
        .iter()
        .enumerate()
        .filter(|(_, player)| player.on_court && player.team == recipient)
        .min_by(|(left_handle, left), (right_handle, right)| {
            (left.pos - spot)
                .length()
                .total_cmp(&(right.pos - spot).length())
                .then(left_handle.cmp(right_handle))
        })
        .map(|(handle, _)| handle);
    if let Some(handle) = handler {
        world.set_ball_held(handle);
    }
    world.apply_dead_ball_window();
}

fn step_jump_ball_toss(world: &mut World, dt: f32) {
    let mut ball = world.ball.clone();
    let gravity = world.rules.ball_gravity_ftps2;
    let previous_vz = ball.vz;
    let previous_z = ball.z;
    let next_z = ball.z + ball.vz * dt - 0.5 * gravity * dt * dt;
    let next_vz = ball.vz - gravity * dt;
    let reached_peak = previous_vz > 0.0 && next_vz <= 0.0;
    if reached_peak {
        let peak_time = previous_vz / gravity;
        ball.pos += ball.vel * peak_time;
        ball.z = previous_z + previous_vz * peak_time - 0.5 * gravity * peak_time * peak_time;
        ball.vz = 0.0;
        world.emit(Fact::BallContact {
            kind: "jump_ball_peak",
            entity: None,
            time: world.now() - f64::from(dt) + f64::from(peak_time),
            x: ball.pos.x,
            y: ball.pos.y,
            z: ball.z,
            sub_tick: 0,
        });
    } else {
        ball.pos += ball.vel * dt;
        ball.z = next_z.max(world.rules.ball_radius_ft);
        ball.vz = next_vz;
    }
    world.ball = ball;
}

fn step_jump_ball_flight(world: &mut World, dt: f32) {
    let mut ball = world.ball.clone();
    let gravity = world.rules.ball_gravity_ftps2;
    let radius = world.rules.ball_radius_ft;
    let mut remaining = dt;
    let mut sub_tick = 0u16;
    while remaining > f32::MIN_POSITIVE {
        let vertical_speed = ball.vz.abs().max(1.0);
        let step = (0.5 * radius / vertical_speed)
            .min(1.0 / 240.0)
            .min(remaining);
        let previous_z = ball.z;
        let previous_pos = ball.pos;
        let next_z = ball.z + ball.vz * step - 0.5 * gravity * step * step;
        let next_vz = ball.vz - gravity * step;
        let next_pos = ball.pos + ball.vel * step;
        let crossed_peak = ball.vz > 0.0 && next_vz <= 0.0;
        let crossed_floor = next_z <= radius;
        let contact_time = if crossed_peak {
            ball.vz / gravity
        } else if crossed_floor {
            let a = -0.5 * gravity;
            let b = ball.vz;
            let c = ball.z - radius;
            let discriminant = b * b - 4.0 * a * c;
            let root = discriminant.max(0.0).sqrt();
            [(-b + root) / (2.0 * a), (-b - root) / (2.0 * a)]
                .into_iter()
                .filter(|time| *time >= 0.0 && *time <= step)
                .fold(f32::INFINITY, f32::min)
        } else {
            f32::INFINITY
        };
        if contact_time.is_finite() {
            ball.pos = previous_pos + ball.vel * contact_time;
            ball.z = if crossed_floor {
                radius
            } else {
                previous_z + ball.vz * contact_time - 0.5 * gravity * contact_time * contact_time
            };
            ball.vz = if crossed_floor {
                (ball.vz - gravity * contact_time).abs() * 0.45
            } else {
                0.0
            };
            let remaining_step = step - contact_time;
            let bounded = Vec2::new(
                ball.pos
                    .x
                    .clamp(radius, world.rules.court.width_ft - radius),
                ball.pos
                    .y
                    .clamp(radius, world.rules.court.height_ft - radius),
            );
            if bounded != ball.pos {
                ball.pos = bounded;
                ball.vel.x = -ball.vel.x * world.rules.loose_ball_player_restitution;
                ball.vel.y = -ball.vel.y * world.rules.loose_ball_player_restitution;
            }
            world.emit(Fact::BallContact {
                kind: if crossed_floor {
                    "jump_ball_floor_bounce"
                } else {
                    "jump_ball_peak"
                },
                entity: None,
                time: world.now() - f64::from(remaining) + f64::from(contact_time),
                x: ball.pos.x,
                y: ball.pos.y,
                z: ball.z,
                sub_tick,
            });
            world.ball = ball.clone();
            remaining -= contact_time;
            if remaining_step <= f32::MIN_POSITIVE {
                continue;
            }
            continue;
        }
        ball.pos = next_pos;
        ball.z = next_z;
        ball.vz = next_vz;
        let bounded = Vec2::new(
            ball.pos
                .x
                .clamp(radius, world.rules.court.width_ft - radius),
            ball.pos
                .y
                .clamp(radius, world.rules.court.height_ft - radius),
        );
        if bounded != ball.pos {
            ball.pos = bounded;
            ball.vel.x = -ball.vel.x * world.rules.loose_ball_player_restitution;
            ball.vel.y = -ball.vel.y * world.rules.loose_ball_player_restitution;
        }
        remaining -= step;
        sub_tick = sub_tick.saturating_add(1);
    }
    world.ball = ball;
}

fn try_control_loose(world: &mut World) {
    let Some(state) = world.pending_rebound else {
        return;
    };
    if world.ball.z > 8.0 {
        return;
    }
    let elapsed = world.now() - state.started_at;
    // 被碰歪的传球要先飞一段再进入可控阶段：碰球的人就在球边，延迟为
    // 零的话下一个 tick 他就把球收走，除了他没人有机会反应。
    if state.pass_by.is_some()
        && elapsed < f64::from(world.params.policy.w("loose_ball_pickup_delay_seconds"))
    {
        return;
    }
    // 争抢能力先按可伸手高度折算到球心所在高度，再与平面距离合成。
    let force_pickup = elapsed > 2.0;
    let base = world.params.policy.w("loose_ball_catch_base_ft");
    let skill = world.params.policy.w("loose_ball_catch_skill_ft");
    let mut best: Option<(usize, f32)> = None;
    let mut best_home: Option<(usize, f32)> = None;
    let mut best_away: Option<(usize, f32)> = None;
    for &handle in world
        .on_court_home
        .as_slice()
        .iter()
        .chain(world.on_court_away.as_slice())
    {
        let player = &world.players[handle];
        let is_offense = player.team == state.offense;
        let attribute = if is_offense {
            "offensive_rebound"
        } else {
            "defensive_rebound"
        };
        let base_reach = base + player.attribute(attribute) * skill;
        let penalty = world.params.policy.g("rebound_offense_reach_penalty_ft");
        let bonus = world.params.policy.g("rebound_defense_reach_bonus_ft");
        let reach = if force_pickup {
            if is_offense {
                world.params.policy.g("rebound_offense_late_reach_ft")
            } else {
                world.params.policy.g("rebound_defense_late_reach_ft")
            }
        } else if state.turnover {
            base_reach
        } else if is_offense {
            base_reach - penalty
        } else {
            // 防守卡位加成来自 defensive_rebound 与 strength：这条能力曲线
            // 之前没有任何消费者，防守篮板因此对属性不敏感。
            let boxout = nba_domain::capability::effective_defensive_boxout_bonus(
                &world.rules,
                &player.data.attributes,
            );
            base_reach
                + bonus
                + boxout * world.params.policy.w("rebound_boxout_weight")
        };
        let vertical = (world.ball.z - 4.0).max(0.0);
        let distance = (player.pos - world.ball.pos).length().hypot(vertical);
        let margin = reach - distance;
        if margin >= 0.0 {
            if best.is_none_or(|(_, current)| margin > current) {
                best = Some((handle, margin));
            }
            let per_team = if player.team == crate::action::TeamSide::Home {
                &mut best_home
            } else {
                &mut best_away
            };
            if per_team.is_none_or(|(_, current)| distance < current) {
                *per_team = Some((handle, distance));
            }
        }
    }
    // 双方同时够到球：停表跳球，而不是让先算到的人直接拿走。
    if let (Some((home, home_distance)), Some((away, away_distance))) = (best_home, best_away) {
        let tie_margin = world.params.policy.g("held_ball_tie_margin_ft");
        if (home_distance - away_distance).abs() <= tie_margin {
            let position = world.ball.pos;
            let z = world.ball.z;
            world.pending_rebound = None;
            world.emit(Fact::BallContact {
                kind: "held_ball",
                entity: None,
                time: world.now(),
                x: position.x,
                y: position.y,
                z,
                sub_tick: 0,
            });
            world.end_possession(
                nba_domain::event::PossessionEndCause::HeldBall,
                "held_ball",
                0,
            );
            world.stage_held_ball(home, away);
            return;
        }
    }
    if let Some((handle, _)) = best {
        if std::env::var_os("NBA_TRACE_LOOSE").is_some() {
            eprintln!(
                "loose-control elapsed={elapsed:.2} winner={handle} team={:?} offense={:?} ball=({:.1},{:.1},\n {:.1}) holder_before={:?}",
                world.players[handle].team,
                state.offense,
                world.ball.pos.x,
                world.ball.pos.y,
                world.ball.z,
                world.ball.holder(),
            );
        }
        crate::resolve::rebound::control(world, handle);
    } else if std::env::var_os("NBA_TRACE_LOOSE").is_some() {
        eprintln!(
            "loose-wait elapsed={elapsed:.2} ball=({:.1},{:.1},{:.1}) offense={:?}",
            world.ball.pos.x, world.ball.pos.y, world.ball.z, state.offense,
        );
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
    let coefficients = crate::physical::Coefficients::from_policy(&world.params.policy);
    let reach_share = world.params.policy.w("shot_release_vertical_share");
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
            shot_id: _,
            shooter,
            aperture_ft,
            ..
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
                let bounces_now = matches!(
                    &ball.state,
                    BallState::Pass {
                        kind: PassKind::Bounce,
                        ..
                    }
                ) && world
                    .pass_in_flight
                    .as_ref()
                    .is_some_and(|info| !info.bounced);
                if bounces_now {
                    if let Some(info) = world.pass_in_flight.as_mut() {
                        info.bounced = true;
                    }
                    // 击地传球：触地后弹起继续飞向接球人，而不是判为传球失败。
                    ball.vel *= 0.85;
                    ball.vz = -ball.vz * 0.62;
                    world.emit(Fact::BallContact {
                        kind: "floor",
                        entity: None,
                        time: start_time + f64::from(elapsed + t),
                        x: contact.x,
                        y: contact.y,
                        z: radius,
                        sub_tick,
                    });
                    elapsed += t;
                    remaining -= t;
                    sub_tick += 1;
                    continue;
                }
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
            let rim_in = match &ball.state {
                BallState::Shot {
                    rim_in_probability, ..
                } => *rim_in_probability,
                _ => 0.0,
            };
            // 「是否穿筐口」必须按球心穿过筐平面的位置判断：球在筐口
            // 上方还有好几个子步，那里的水平距离比穿越点大，用它判定会
            // 把本该命中的球判成触筐。
            let inside_opening = if ball.vz < 0.0 && prev_z > rules.rim_height_ft {
                let t = (prev_z - rules.rim_height_ft) / (prev_z - next_z).max(f32::MIN_POSITIVE);
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
                // 触筐不等于不中：球子打在筐沿内侧后仍然可能落进筐心。
                // 这一步与模型里的 p_make_cond 是同一项，抽样命中就把球搬到
                // 筐心正上方竖直下落，下一个子步穿筐平面时距筐心为 0，按
                // 穿筐结算。
                let shooter = match &ball.state {
                    BallState::Shot { shooter, .. } => *shooter,
                    _ => usize::MAX,
                };
                let bounce_in = rim_in > 0.0
                    && shooter != usize::MAX
                    && world.draw(Stream::Aim, shooter, purpose::SHOT_MAKE, 2) < rim_in;
                if bounce_in {
                    world.emit(Fact::BallContact {
                        kind: "rim_bounce_in",
                        entity: None,
                        time: start_time + f64::from(elapsed + h),
                        x: hoop.x,
                        y: hoop.y,
                        z: rules.rim_height_ft,
                        sub_tick,
                    });
                    ball.pos = hoop;
                    ball.z = rules.rim_height_ft + 0.4;
                    ball.vel = Vec2::ZERO;
                    ball.vz = -3.0;
                    world.ball = ball;
                    return;
                }
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
                        pending.result =
                            Some(crate::resolve::shot::ShotResult::Missed { rim_contact: true });
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
        if world
            .pending_shot
            .as_ref()
            .is_some_and(|pending| pending.free_throw.is_some())
        {
        } else if let Some((handle, kind)) = shot_swat(world, next_pos, next_z)
            .or_else(|| player_contact(world, next_pos, next_z, &rules, &coefficients, reach_share))
        {
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
            if std::env::var("NBA_TRACE_MISS").is_ok() {
                if let Some(info) = world.pass_in_flight {
                    let spot = crate::resolve::pass::cut_target(world, info.to);
                    let receiver = &world.players[info.to];
                    let action = receiver.action.as_ref().map(|runtime| {
                        (
                            format!("{:?}", runtime.role),
                            (runtime.intent.to_ft - spot).length(),
                            runtime.phase,
                        )
                    });
                    eprintln!(
                        "FLIGHTEND to={} kind={:?} dist={:.2} to_spot={:.2} to_target={:.2} spot_vs_target={:.2} action={action:?} vel={:.2}",
                        info.to,
                        info.kind,
                        (ball.pos - receiver.pos).length(),
                        (receiver.pos - spot).length(),
                        (receiver.target - receiver.pos).length(),
                        (receiver.target - spot).length(),
                        receiver.vel.length(),
                    );
                }
            }
            world.pass_result = Some(PassResult::Incomplete);
            ball.state = BallState::Loose {
                pos: ball.pos,
                vel: ball.vel,
            };
        }
        _ => {}
    }
}

fn handle_player_contact(
    world: &mut World,
    ball: &mut BallBody,
    handle: usize,
    kind: &'static str,
) {
    world.last_touch = Some(handle);
    match ball.state {
        BallState::Shot { .. } => {
            if let Some(pending) = world.pending_shot.as_mut() {
                if pending.result.is_none() {
                    pending.result = Some(if kind == "hand" {
                        crate::resolve::shot::ShotResult::Blocked { by: handle }
                    } else {
                        crate::resolve::shot::ShotResult::Missed { rim_contact: false }
                    });
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
                // 防守球员拍到传球：手是横着扫过来的，球被抽离原来的线路，
                // 带着一部分前进速度斜着飞出去。沿法线镜像会把球原速打回
                // 出球人，结果是所有人挤在原点互相弹球，进攻方永远抢不到。
                world.pass_result = Some(PassResult::Incomplete);
                let speed = ball.vel.length();
                if speed > 1.0e-3 {
                    let forward = ball.vel / speed;
                    let side = if forward.y * (ball.pos.x - world.players[handle].pos.x)
                        - forward.x * (ball.pos.y - world.players[handle].pos.y)
                        >= 0.0
                    {
                        1.0
                    } else {
                        -1.0
                    };
                    let lateral = crate::ball::Vec2::new(-forward.y, forward.x) * side;
                    let deflected = (forward * 0.55 + lateral * 0.85).normalize();
                    let retained = 0.45 * speed;
                    ball.vel = deflected * retained;
                }
                ball.vz = -ball.vz * world.rules.loose_ball_player_restitution;
                ball.state = BallState::Loose {
                    pos: ball.pos,
                    vel: ball.vel,
                };
                return;
            }
            let teammate = world
                .pass_in_flight
                .is_some_and(|info| world.players[info.from].team == world.players[handle].team);
            world.pass_result = Some(if handle == to || teammate {
                PassResult::Caught { to: handle }
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
        BallState::Held { .. } => {}
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
pub fn pass_intercept_radius(world: &World, defender: &crate::state::PlayerState) -> f32 {
    // 半径是防守人的手掌在身侧能够到的范围：只有手臂正好在传球
    // 线路上的防守人才碰得到球，站得离开线路半个身位就碰不到。
    // 抄球半径随抢断属性放大：抢断能力强的防守人真的能把球断下来。
    let base = world.rules.ball_radius_ft
        + world.params.policy.g("pass_intercept_base_ft")
        + defender.attribute("steal") * world.params.policy.g("pass_intercept_steal_ft");
    // 被传球假动作骗到的防守人重心已偏向假目标，真实线路上的手位收窄。
    let bait_relief = match defender.bait {
        Some(bait)
            if bait.until > world.now() && matches!(bait.kind, crate::action::FakeKind::Pass) =>
        {
            world.rules.resolve.technique.fake_pass_lane_ft
        }
        _ => f32::default(),
    };
    (base - bait_relief).max(world.rules.ball_radius_ft)
}

/// 出手时已经抽出「这一球被封盖」，这里只等球飞到那只手的球体里。
/// 封盖概率与手位几何都来自投篮模型，因此模型说的那次封盖就是这一次。
fn shot_swat(world: &World, point: Vec2, z: f32) -> Option<(usize, &'static str)> {
    let pending = world.pending_shot.as_ref()?;
    if pending.result.is_some() || pending.free_throw.is_some() {
        return None;
    }
    let blocker = pending.block_by?;
    let hand = crate::shot_quality::blocking_hands(world, pending.shooter)
        .into_iter()
        .find(|hand| hand.handle == blocker)?;
    ((point - hand.pos).length().hypot(z - hand.z) <= hand.radius).then_some((blocker, "hand"))
}

fn player_contact(
    world: &World,
    next_pos: Vec2,
    next_z: f32,
    rules: &nba_domain::GameRules,
    coefficients: &crate::physical::Coefficients,
    reach_share: f32,
) -> Option<(usize, &'static str)> {
    if matches!(world.phase(), crate::state::Phase::JumpBallFlight) {
        return None;
    }
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
            pass_intercept_radius(world, player)
        } else {
            ball_radius
                + world.params.policy.w("block_hand_radius_base_ft")
                + player.attribute("block") * world.params.policy.w("block_hand_radius_skill_ft")
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
            let intercept = pass_intercept_radius(world, player);
            // 拦截用的是防守人在传球高度上伸手够到的范围：平直传球走胸口，
            // 高吊球从头顶飞过就够不到。球的高度与手的��度一起决定这次
            // 拦截是否成立。
            //
            // 抄球能力越强，手伸得越远：拦截半径按抢断能力放大，这样
            // 防守人的抢断属性才会真的改变结果。
            let hand_height = rules.chest_height_ft + 1.0;
            let distance = (next_pos - player.pos).length().hypot(next_z - hand_height);
            if distance <= intercept {
                if std::env::var("NBA_TRACE_CONTACT").is_ok() {
                    let start = world.players[info.from].pos;
                    let end = world.players[info.to].pos;
                    let segment = end - start;
                    let along = ((next_pos - start).dot(segment) / segment.length_squared())
                        .clamp(0.0, 1.0);
                    eprintln!(
                        "CONTACT defender={handle} along={along:.3} to_end={:.2} to_start={:.2} ball_z={next_z:.2} dist={distance:.2}",
                        (next_pos - end).length(),
                        (next_pos - start).length()
                    );
                }
                return Some((handle, "hand"));
            }
            continue;
        }
        let hand =
            player.pos + player.facing.normalize_or_zero() * (rules.ball_holder_offset_ft + 0.4);
        let hand_z = if pass.is_some() {
            rules.chest_height_ft + 1.0
        } else {
            // 封盖手的高度：站着防守的人手只在站立摸高，只有真正起跳的
            // 干扰动作（防守动作处于执行阶段）才把手举到起跳摸高。
            let jumping = world
                .defensive_actions
                .get(handle)
                .and_then(Option::as_ref)
                .is_some_and(|action| {
                    action.phase == nba_domain::action_window::ActionPhase::Execution
                });
            let share = if jumping { reach_share } else { 0.0 };
            crate::physical::Physical::with(player, coefficients).contest_reach_ft_with(share)
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
