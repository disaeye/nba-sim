//! 篮板结算：前场篮板率来自校准表，抢板人由能力与位置决定。

use glam::Vec2;
use rand::Rng;

use crate::action::Fact;
use crate::resolve::clamp01;
use crate::state::{BallState, PossessionKind};
use crate::world::World;

pub fn resolve(world: &mut World, zone: &'static str, shot_distance: f32) {
    world.rebound_attempts += 1;
    let offense = world.possession.team;
    let defense = offense.other();
    let table_rate = world
        .params
        .rebound
        .oreb_rate_by_zone
        .get(zone)
        .copied()
        .unwrap_or(world.params.rebound.league_oreb_per_miss);
    let offense_rebounding = average(world, offense, "offensive_rebound");
    let defense_rebounding = average(world, defense, "defensive_rebound");
    let strength_edge = average(world, offense, "strength") - average(world, defense, "strength");
    let p_offensive = clamp01(
        table_rate
            * (1.0 + 0.9 * (offense_rebounding - 0.5) - 0.6 * (defense_rebounding - 0.5)
                + 0.8 * strength_edge),
    );
    let offensive = world.rng.gen::<f32>() < p_offensive;
    let team = if offensive { offense } else { defense };
    let hoop = world.hoop_for(offense);
    let rebounder = pick_rebounder(world, team, hoop, offensive);
    let landing = if offensive {
        let angle = world.rng.gen::<f32>() * std::f32::consts::TAU;
        let radius = world.rng.gen::<f32>() * 4.0;
        hoop + Vec2::new(angle.cos(), angle.sin()) * radius
    } else {
        rebound_landing(world, hoop, shot_distance)
    };

    world.players[rebounder].pos = landing;
    world.players[rebounder].target = landing;
    world.ball = BallState::Held(rebounder);
    let distance_ft = (landing - hoop).length();
    world.emit(Fact::Rebound {
        player: rebounder,
        offensive,
        distance_ft,
    });
    if offensive {
        world.line_mut(rebounder).oreb += 1;
        world.possession.had_offensive_rebound = true;
        world.end_possession("offensive_rebound", 0);
        world.start_second_chance(offense, rebounder);
    } else {
        world.line_mut(rebounder).dreb += 1;
        world.end_possession("defensive_rebound", 0);
        world.start_possession(defense, PossessionKind::Live);
    }
}

fn average(world: &World, team: crate::action::TeamSide, attribute: &str) -> f32 {
    let players: Vec<f32> = world
        .players
        .iter()
        .filter(|p| p.on_court && p.team == team)
        .map(|p| p.attribute(attribute))
        .collect();
    players.iter().sum::<f32>() / players.len() as f32
}

fn pick_rebounder(
    world: &mut World,
    team: crate::action::TeamSide,
    hoop: Vec2,
    offensive: bool,
) -> usize {
    let attribute = if offensive {
        "offensive_rebound"
    } else {
        "defensive_rebound"
    };
    let candidates: Vec<(usize, f32)> = world
        .players
        .iter()
        .enumerate()
        .filter(|(_, p)| p.on_court && p.team == team)
        .map(|(index, p)| {
            let proximity = 1.0 / (1.0 + (p.pos - hoop).length() / 8.0);
            let strength = p.attribute("strength");
            let weight = 0.4 + 1.6 * p.attribute(attribute) + 0.6 * strength + proximity;
            (index, weight)
        })
        .collect();
    let weights: Vec<f32> = candidates.iter().map(|(_, weight)| *weight).collect();
    let pick = crate::resolve::weighted_pick(&mut world.rng, &weights);
    candidates[pick].0
}

fn rebound_landing(world: &mut World, hoop: Vec2, shot_distance: f32) -> Vec2 {
    let spread = (4.0 + shot_distance * 0.35).min(18.0);
    let angle = world.rng.gen::<f32>() * std::f32::consts::TAU;
    let radius = world.rng.gen::<f32>().sqrt() * spread;
    hoop + Vec2::new(angle.cos(), angle.sin()) * radius
}
