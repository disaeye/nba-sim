//! 犯规与罚则：投篮犯规、bonus、罚球程序入口。

use crate::action::Fact;
use crate::state::TeamSide;
use crate::world::{Phase, World};

pub fn apply_shooting_foul(
    world: &mut World,
    fouler: usize,
    shooter: usize,
    free_throws: u8,
    made: bool,
) {
    let team = world.players[shooter].team;
    world.line_mut(fouler).fouls += 1;
    world.line_mut(fouler).shooting_fouls += 1;
    if made {
        world.line_mut(shooter).and_one += 1;
    }
    let index = World::team_index(world.players[fouler].team);
    world.fouls[index].period += 1;
    world.fouls[index].total += 1;
    world.players[fouler].fouls += 1;
    world.emit(Fact::Foul {
        fouler,
        fouled: shooter,
        shooting: true,
        free_throws,
        and_one: made,
    });
    world.phase = Phase::FreeThrows {
        shooter,
        total: free_throws,
        remaining: free_throws,
        made: 0,
        next_at: world.now() + 1.6,
        and_one: made,
    };
    world.substitute_if_needed();
    let _ = team;
}

pub fn bonus_free_throws(world: &World, fouling_team: TeamSide) -> u8 {
    let index = World::team_index(fouling_team);
    let threshold = world.rules.league.bonus_fouls_per_period;
    if world.fouls[index].period as u32 >= threshold {
        2
    } else {
        0
    }
}

pub fn apply_non_shooting_foul(world: &mut World, fouler: usize, fouled: usize) {
    let team = world.players[fouler].team;
    let index = World::team_index(team);
    world.line_mut(fouler).fouls += 1;
    world.fouls[index].period += 1;
    world.fouls[index].total += 1;
    world.players[fouler].fouls += 1;
    let free_throws = bonus_free_throws(world, team);
    world.emit(Fact::Foul {
        fouler,
        fouled,
        shooting: false,
        free_throws,
        and_one: false,
    });
    if free_throws > 0 {
        world.phase = Phase::FreeThrows {
            shooter: fouled,
            total: free_throws,
            remaining: free_throws,
            made: 0,
            next_at: world.now() + 1.6,
            and_one: false,
        };
    }
    world.substitute_if_needed();
}
