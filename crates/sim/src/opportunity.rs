//! 根据球员当时能够掌握的信息，估计进攻窗口。

use glam::Vec2;
use serde::{Deserialize, Serialize};

use crate::world::World;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlayerRead {
    pub tick: u64,
    pub position: Vec2,
    pub velocity: Vec2,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct KnownPlayer {
    pub player: usize,
    pub sample: PlayerRead,
}

pub(crate) fn known_players(world: &World, actor: usize) -> Vec<KnownPlayer> {
    world.players.iter().enumerate().filter(|&(player, state)| player != actor && state.on_court)
        .filter_map(|(player, _)| read(world, actor, player).map(|sample| KnownPlayer { player, sample }))
        .collect()
}

pub(crate) fn unknown_players(world: &World, actor: usize) -> Vec<usize> {
    world.players.iter().enumerate()
        .filter(|(player, state)| *player != actor && state.on_court && read(world, actor, *player).is_none())
        .map(|(player, _)| player)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub delay_seconds: f32,
    pub defender_seconds: Option<f32>,
}

impl Window {
    pub fn margin(self) -> Option<f32> {
        self.defender_seconds.map(|arrival| arrival - self.delay_seconds)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionWindow {
    pub candidate: usize,
    pub window: Window,
}

pub(crate) fn decision_windows(
    world: &World,
    actor: usize,
    candidates: &[crate::state::CandidateTrace],
) -> Vec<DecisionWindow> {
    use crate::action::ActionKind;
    let origin = world.players[actor].pos;
    candidates.iter().enumerate().filter_map(|(candidate, action)| {
        let window = match action.kind {
            ActionKind::Shoot(kind) => {
                let phase = world.possession.ball_phase;
                let model = crate::shot_quality::evaluate_observed(world, actor, actor, kind, phase, origin);
                shot_window(world, actor, origin, model.release_delay_seconds)
            }
            ActionKind::Pass(kind) => {
                let flight = crate::value::pass_flight_seconds(world, actor, action.to_ft, kind);
                pass_window(world, actor, origin, action.to_ft, flight as f32)
            }
            _ => return None,
        };
        Some(DecisionWindow { candidate, window })
    }).collect()
}

pub(crate) fn refresh_schedule(world: &mut World, actor: usize) {
    let observation = crate::views::observe(world, actor);
    let memory = &world.players[actor].reads;
    if memory.is_empty() {
        return;
    }
    let threshold = world.rules.player_radius_ft;
    let changed = observation.players.iter().any(|player| {
        match read(world, actor, player.index) {
            Some(previous) => (player.pos - previous.position).length() >= threshold,
            None => true,
        }
    });
    if changed {
        let now = world.now();
        let reaction = f64::from(world.params.policy.g("defense_reaction_delay_seconds"))
            .max(f64::from(world.rules.redecide_interval_seconds));
        let schedule = &mut world.players[actor].schedule;
        if schedule.pending.is_none() {
            schedule.next_redecide_at = schedule.next_redecide_at.min(now + reaction);
            schedule.pending = Some(crate::state::DecisionTrigger::NewObservation);
        }
    }
}

pub fn remember(world: &mut World, actor: usize) {
    let observation = crate::views::observe(world, actor);
    let tick = world.clock.tick();
    let roster_size = world.players.len();
    let reads = &mut world.players[actor].reads;
    reads.resize(roster_size, None);
    for player in observation.players {
        reads[player.index] = Some(PlayerRead { tick, position: player.pos, velocity: player.vel });
    }
}

pub fn read(world: &World, actor: usize, target: usize) -> Option<PlayerRead> {
    let memory = &world.players[actor].reads;
    if memory.is_empty() || !world.players[actor].on_court {
        return crate::views::observe(world, actor).players.into_iter()
            .find(|player| player.index == target)
            .map(|player| PlayerRead { tick: world.clock.tick(), position: player.pos, velocity: player.vel });
    }
    let sample = memory.get(target).copied().flatten()?;
    let age = world.clock.tick().checked_sub(sample.tick)? as f64 * crate::world::TICK_SECONDS;
    let horizon = f64::from(world.params.policy.g("perception_memory_seconds"));
    if age > horizon {
        return None;
    }
    let reliability = (f64::from(1_u8) - age / horizon).max(f64::default()) as f32;
    Some(PlayerRead { position: sample.position + sample.velocity * age as f32 * reliability, velocity: sample.velocity * reliability, ..sample })
}

fn earliest_arrival(world: &World, actor: usize, point: Vec2, reach: f32) -> Option<f32> {
    let side = world.players[actor].team;
    world.players.iter().enumerate()
        .filter(|(_, player)| player.on_court && player.team != side)
        .filter_map(|(target, player)| {
            let seen = read(world, actor, target)?;
            let body = crate::physical::Physical::of(player, &world.params.policy);
            Some(arrival_seconds(seen, point, reach, body.speed_ftps, body.accel_ftps2))
        })
        .min_by(f32::total_cmp)
}

pub fn arrival_seconds(read: PlayerRead, point: Vec2, reach: f32, top_speed: f32, accel: f32) -> f32 {
    let offset = point - read.position;
    let distance = (offset.length() - reach).max(f32::default());
    let speed = top_speed.max(f32::EPSILON);
    let accel = accel.max(f32::EPSILON);
    if distance == f32::default() {
        return f32::default();
    }
    let along = read.velocity.dot(offset.normalize_or_zero()).clamp(-speed, speed);
    let acceleration_time = (speed - along) / accel;
    let acceleration_distance = (speed + along) * acceleration_time / f32::from(2_u8);
    if distance > acceleration_distance {
        acceleration_time + (distance - acceleration_distance) / speed
    } else {
        (along.mul_add(along, f32::from(2_u8) * accel * distance).sqrt() - along) / accel
    }
}

pub fn observed_lane_clearance(world: &World, actor: usize, origin: Vec2, target: Vec2) -> f32 {
    let lane = target - origin;
    let span = lane.length_squared();
    if span <= f32::EPSILON {
        return f32::default();
    }
    let team = world.players[actor].team;
    world.players.iter().enumerate()
        .filter(|(_, player)| player.on_court && player.team != team)
        .filter_map(|(handle, _)| read(world, actor, handle))
        .filter_map(|seen| {
            let offset = seen.position - origin;
            let fraction = offset.dot(lane) / span;
            (fraction > f32::default() && fraction < f32::from(1_u8))
                .then(|| (offset - lane * fraction).length())
        })
        .min_by(f32::total_cmp)
        .unwrap_or(f32::INFINITY)
}

pub(crate) fn observed_trap(world: &World, actor: usize) -> bool {
    let pos = world.players[actor].pos;
    let radius = world.params.policy.g("trap_radius_ft");
    crate::views::known_opponents(world, actor)
        .filter(|(_, read)| (read.position - pos).length() <= radius)
        .take(2).count() == 2
}

pub fn shot_window(world: &World, observer: usize, position: Vec2, delay_seconds: f32) -> Window {
    let reach = world.params.policy.g("contest_proximity_ft");
    Window { delay_seconds, defender_seconds: earliest_arrival(world, observer, position, reach) }
}

pub fn pass_window(world: &World, actor: usize, origin: Vec2, destination: Vec2, flight: f32) -> Window {
    let reach = world.rules.ball_radius_ft + world.rules.player_radius_ft;
    let earliest = [(origin + destination) / f32::from(2_u8), destination].into_iter().enumerate()
        .filter_map(|(index, point)| {
            let fraction = (index + 1) as f32 / f32::from(2_u8);
            earliest_arrival(world, actor, point, reach).map(|arrival| arrival + flight * (f32::from(1_u8) - fraction))
        })
        .min_by(f32::total_cmp);
    Window { delay_seconds: flight, defender_seconds: earliest }
}
