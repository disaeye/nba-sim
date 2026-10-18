//! 罚球结算：命中率由球员罚球能力与联盟基准决定。

use rand::Rng;

use crate::action::Fact;
use crate::state::{BallState, PossessionKind};
use crate::world::{Phase, World};

pub fn shoot(world: &mut World, shooter: usize, total: u8, remaining: u8, made_so_far: u8, and_one: bool) {
    let table = &world.params.freethrow;
    let attribute = world.players[shooter].data.attributes.free_throw;
    let p_make =
        (table.league_make + (attribute - 0.5) * table.skill_slope).clamp(0.30, 0.95);
    let made = world.rng.gen::<f32>() < p_make;
    let index = total - remaining + 1;
    {
        let line = world.line_mut(shooter);
        line.fta += 1;
        if made {
            line.ftm += 1;
            line.points += 1;
        }
    }
    if made {
        world.add_points(world.players[shooter].team, 1);
    }
    world.emit(Fact::FreeThrow {
        shooter,
        made,
        index,
        total,
    });

    let made_count = made_so_far + u8::from(made);
    let left = remaining - 1;
    if left == 0 {
        let team = world.players[shooter].team;
        world.possession.shot_clock_start = world.clock.shot_clock;
        world.end_possession(if made_count > 0 { "free_throw" } else { "free_throw_miss" }, made_count as u32);
        world.phase = Phase::DeadBall {
            team: team.other(),
            ready_at: world.now() + 1.2,
        };
        world.ball = BallState::Loose {
            pos: world.hoop_for(team),
            vel: glam::Vec2::ZERO,
        };
        let _ = and_one;
        return;
    }
    world.phase = Phase::FreeThrows {
        shooter,
        total,
        remaining: left,
        made: made_count,
        next_at: world.now() + 1.6,
        and_one,
    };
    world.ball = BallState::Held(shooter);
    let _ = PossessionKind::Dead;
}
