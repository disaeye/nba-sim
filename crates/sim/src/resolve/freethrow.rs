//! 罚球：球体飞行穿过筐口才计分，出手时机与瞄准误差由技能和策略参数决定。

use glam::Vec2;

use crate::action::{Fact, ShotKind};
use crate::draw::{purpose, Stream};
use crate::resolve::shot::{PendingShot, ShotResult};
use crate::state::{BallState, Phase};
use crate::world::World;

#[derive(Debug, Clone, Copy)]
pub struct FreeThrowMeta {
    pub total: u8,
    pub remaining: u8,
    pub made: u8,
    pub and_one: bool,
    pub retain_possession: bool,
}

pub fn shoot(
    world: &mut World,
    shooter: usize,
    total: u8,
    remaining: u8,
    made_so_far: u8,
    and_one: bool,
    retain_possession: bool,
) {
    let hoop = world.hoop_for(world.players[shooter].team);
    let pos = world.players[shooter].pos;
    world.players[shooter].facing = (hoop - pos).normalize_or_zero();
    let model = crate::shot_quality::free_throw_model(world, shooter);
    let u1 = world
        .draw(Stream::Aim, shooter, purpose::SHOT_MAKE, 0)
        .max(1e-6);
    let u2 = world.draw(Stream::Aim, shooter, purpose::SHOT_MAKE, 1);
    let spread = model.sigma_ft * (-2.0 * u1.ln()).sqrt();
    let angle = u2 * std::f32::consts::TAU;
    let aim_offset = Vec2::new(angle.cos(), angle.sin()) * spread;
    let shot_id = world.new_shot_id();
    let meta = FreeThrowMeta {
        total,
        remaining,
        made: made_so_far,
        and_one,
        retain_possession,
    };
    world.pending_shot = Some(PendingShot {
        shooter,
        kind: ShotKind::CatchShoot,
        zone: "free_throw",
        distance_ft: (pos - hoop).length(),
        contest: 0.0,
        fouled: false,
        and_one,
        free_throws: 0,
        points: 1,
        is_three: false,
        assisted_by: None,
        shot_id,
        free_throw: Some(meta),
        result: None,
    });
    let rules = world.rules.clone();
    world.ball.release_shot(crate::ball::ShotRelease {
        rules: &rules,
        now: world.now(),
        shot_id,
        shooter,
        hoop,
        release_z: model.release_z_ft,
        aperture_ft: model.effective_aperture_ft,
        origin: model.origin,
        plan: model.plan,
        aim_offset,
    });
    world.phase = Phase::ShotFlight {
        resolve_at: world.now() + f64::from(rules.max_shot_duration_seconds) + 2.0,
    };
}

pub fn finish(world: &mut World, pending: PendingShot, meta: FreeThrowMeta) {
    let made = matches!(pending.result, Some(ShotResult::Made));
    let index = meta.total - meta.remaining + 1;
    {
        let line = world.line_mut(pending.shooter);
        line.fta += 1;
        if made {
            line.ftm += 1;
            line.points += 1;
        }
    }
    if made {
        world.add_points(world.players[pending.shooter].team, 1);
    }
    world.emit(Fact::FreeThrow {
        shooter: pending.shooter,
        made,
        index,
        total: meta.total,
    });
    let made_count = meta.made + u8::from(made);
    if meta.remaining == 1 {
        let team = world.players[pending.shooter].team;
        world.possession.shot_clock_start = world.clock.shot_clock;
        world.end_possession(
            if made_count > 0 {
                "free_throw"
            } else {
                "free_throw_miss"
            },
            made_count as u32,
        );
        world.phase = Phase::DeadBall {
            team: if meta.retain_possession {
                team
            } else {
                team.other()
            },
            ready_at: world.now() + 1.2,
        };
        world.ball.release(BallState::Loose {
            pos: world.hoop_for(team),
            vel: Vec2::ZERO,
        });
        return;
    }
    world.phase = Phase::FreeThrows {
        shooter: pending.shooter,
        total: meta.total,
        remaining: meta.remaining - 1,
        made: made_count,
        next_at: world.now() + world.rules.free_throw_interval_seconds as f64,
        and_one: meta.and_one,
        retain_possession: meta.retain_possession,
    };
    world.ball.set_held(pending.shooter);
}
