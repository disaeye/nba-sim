//! 贴身切球：防守人伸手把持球人的球拍掉，球进入活球争夺。

use glam::Vec2;

use crate::action::Fact;
use crate::draw::{purpose, Stream};
use crate::state::BallState;
use crate::world::{Phase, World};

/// 单次伸手的成功概率：几何、能力与当前姿态（护球、失衡）全部计入。
///
/// 抽样在 [`step`] 里，概率在这里：测试与审计读同一个函数，避免
/// 「模型说的成功率」与「执行时用的成功率」分家。
pub fn poke_success(world: &World, thief: usize, holder: usize, nearest: f32, now: f64) -> f32 {
    let ball = world.ball.pos;
    let geometric_exposure = nba_domain::capability::poke_ball_exposure(
        &world.rules,
        ball.distance(world.players[holder].pos),
    );
    // 运球技术决定这一段动作里球有多暴露：贴身变向与后撤步把球收在身体一侧，
    // 原地推进则保持几何暴露。窗口到期后回到几何值。
    // 技术只能把球收得更紧，不能比几何暴露更松。
    let exposure = if world.possession.dribble_exposure_until > now {
        geometric_exposure.min(world.possession.dribble_exposure)
    } else {
        geometric_exposure
    };
    let towards_ball = (ball - world.players[thief].pos).normalize_or_zero();
    let closing = world.players[thief].vel.dot(towards_ball);
    let facing = nba_domain::capability::poke_pressure_factor(&world.rules, nearest);
    let mut success = nba_domain::capability::poke_check_success_with_context(
        &world.rules,
        &world.players[thief].data.attributes,
        &world.players[holder].data.attributes,
        world.players[holder].data.tendencies.risk_tolerance,
        exposure,
        world.ball.vel.length(),
        closing,
        facing,
    );
    // 护球姿态与持球人被假动作骗过的防守人都会改变这一下伸手的结果。
    if world
        .protection
        .is_some_and(|protection| protection.holder == holder && protection.until > now)
    {
        success *= f32::from(1u8) - world.rules.resolve.technique.protection_poke_relief;
    }
    if world.players[thief]
        .bait
        .is_some_and(|bait| bait.until > now)
    {
        success *= f32::from(1u8) - world.rules.resolve.technique.fake_bait_contest_relief;
    }
    // 主动伸手的动作在执行窗口内是赌对了时机：成功率按策略加成。
    if world.defensive_actions[thief]
        .as_ref()
        .is_some_and(|action| {
            action.kind == crate::action::DefensiveActionKind::StealAttempt
                && action.phase == nba_domain::action_window::ActionPhase::Execution
        })
    {
        success *= f32::from(1u8) + world.rules.resolve.technique.steal_attempt_exec_bonus;
    }
    success
}

/// 每个 tick 判定一次：几何筛出最近的贴身防守人，按速率抽样是否伸手，
/// 伸手后用领域层的模型判定是否拍到球。
pub fn step(world: &mut World, dt: f64) {
    if !matches!(world.phase(), Phase::Live) {
        return;
    }
    let Some(holder) = world.holder() else {
        return;
    };
    let now = world.now();
    if let Some((poked, until)) = world.poke_cooldown {
        if poked == holder && now < until {
            return;
        }
    }
    let radius = world.rules.resolve.ball_security.poke_pressure_radius_ft;
    let offense = world.possession.team;
    let ball = world.ball.pos;
    let mut thief = None;
    let mut nearest = f32::INFINITY;
    for (handle, player) in world.players.iter().enumerate() {
        if !player.on_court || player.team == offense {
            continue;
        }
        let gap = player.pos.distance(ball);
        if gap > radius || gap >= nearest {
            continue;
        }
        nearest = gap;
        thief = Some(handle);
    }
    let Some(thief) = thief else {
        return;
    };
    let security = &world.rules.resolve.ball_security;
    let attempt_rate = security.poke_attempt_rate_per_sec;
    let tendency_floor = security.poke_steal_tendency_floor;
    let tendency_span = security.poke_steal_tendency_span;
    let spread = security.poke_deflection_spread_rad;
    let poke_speed = security.poke_ball_speed_ftps;
    let recovery_seconds = security.poke_recovery_seconds;
    let gamble = world.players[thief].data.tendencies.gamble_steal;
    let rate = f64::from(attempt_rate * (tendency_floor + gamble * tendency_span));
    let attempt = 1.0 - (-rate * dt).exp();
    if f64::from(world.draw(Stream::Decision, thief, purpose::STEAL_CREDIT, 0)) >= attempt {
        return;
    }
    let success = poke_success(world, thief, holder, nearest, now);
    if world.draw(Stream::Decision, thief, purpose::STEAL_CREDIT, 1) >= success {
        if let Some(protection) = world
            .protection
            .filter(|protection| protection.holder == holder && protection.until > now)
        {
            world.emit(Fact::PokePrevented {
                player: holder,
                defender: thief,
                kind: protection.kind,
            });
        }
        return;
    }
    let axis = (world.players[thief].pos - world.players[holder].pos).normalize_or_zero();
    let side = if world.draw(Stream::Aim, thief, purpose::STEAL_CREDIT, 2) < 0.5 {
        -1.0
    } else {
        1.0
    };
    let wobble = (world.draw(Stream::Aim, thief, purpose::STEAL_CREDIT, 3) - 0.5) * 2.0 * spread;
    let lateral = Vec2::new(-axis.y * side, axis.x * side);
    let dir = Vec2::new(
        lateral.x * wobble.cos() - lateral.y * wobble.sin(),
        lateral.x * wobble.sin() + lateral.y * wobble.cos(),
    );
    let radius_ft = world.rules.ball_radius_ft;
    let loose = world.players[holder].pos + dir * 0.8;
    world.ball.pos = loose;
    world.ball.z = radius_ft + 0.6;
    world.ball.vel = dir * poke_speed;
    world.ball.vz = poke_speed * 0.6;
    world.release_ball(BallState::Loose {
        pos: loose,
        vel: dir * poke_speed,
    });
    world.emit(Fact::BallContact {
        kind: "poke",
        entity: Some(thief),
        time: now,
        x: loose.x,
        y: loose.y,
        z: radius_ft + 0.6,
        sub_tick: 0,
    });
    world.poke_cooldown = Some((holder, now + f64::from(recovery_seconds)));
    world.cancel_actions("ball_poked");
    crate::resolve::rebound::resolve_turnover(world);
    if let Some(pending) = world.pending_rebound.as_mut() {
        pending.poke_by = Some((thief, holder));
    }
}
