//! 决策视图：把世界状态折算成决策层只读的 [`DecisionState`]。
//!
//! 从球员位置和可用观察构建策略输入；动作执行从权威状态读取实际位置。

use crate::action::DefenderRoute;
use crate::state::{DecisionState, DefenderView, HelpView, TeammateView};
use crate::world::World;

pub fn decision_state(world: &World) -> DecisionState {
    let actor = world.holder().expect("decision requires a ball holder");
    let team = world.players[actor].team;
    let hoop = world.hoop_for(team);
    let pos = world.players[actor].pos;
    let mut defenders: Vec<_> = known_opponents(world, actor).collect();
    defenders.sort_by(|a, b| (a.1.position - pos).length_squared().total_cmp(&(b.1.position - pos).length_squared()));
    let primary = defenders.first().map(|&(index, read)| {
        let defender = &world.players[index];
        let offset = pos - read.position;
        let distance = offset.length();
        DefenderView {
            index,
            distance_ft: distance,
            closing_speed_ftps: (read.velocity - world.players[actor].vel).dot(offset.normalize_or_zero()),
            between_ball_and_hoop: crate::state::is_between(read.position, pos, hoop),
            beaten: distance > world.params.policy.g("contest_proximity_ft"),
            route: screen_read_route(world),
            reach_excess_ft: crate::physical::Physical::of(defender, &world.params.policy)
                .reach_excess_ft(&world.params.policy),
        }
    });
    let help = defenders.iter().skip(1).map(|&(index, read)| {
        let defender = crate::physical::Physical::of(&world.players[index], &world.params.policy);
        HelpView {
            distance_ft: (read.position - pos).length(),
            arrival_seconds: crate::opportunity::arrival_seconds(read, pos, world.rules.player_radius_ft, defender.speed_ftps, defender.accel_ftps2),
        }
    }).min_by(|left, right| left.arrival_seconds.total_cmp(&right.arrival_seconds));
    let shape = crate::state::ContestShape::from_policy(&world.params.policy);
    let teammates = world
        .on_court(team)
        .into_iter()
        // 候选只能指向真的在场上的队友：名单缓存与球员标志不同步时，
        // 以球员标志为准，不把不在场的人当成可以接球的人。
        .filter(|handle| *handle != actor && world.players[*handle].on_court)
        .filter_map(|handle| {
            let teammate = crate::opportunity::read(world, actor, handle)?;
            let nearest = defenders.iter().map(|&(index, read)| (index, (read.position - teammate.position).length()))
                .min_by(|left, right| left.1.total_cmp(&right.1));
            let (gap, reach) = nearest.map_or((shape.window_ft * 0.5, f32::default()), |(index, gap)| {
                let physical = crate::physical::Physical::of(&world.players[index], &world.params.policy);
                (gap, physical.reach_excess_ft(&world.params.policy))
            });
            Some(TeammateView {
                index: handle,
                pos: teammate.position,
                openness: crate::state::contest_from_defender(shape, gap, false, reach),
                defender_distance_ft: gap,
                lane_clearance: crate::opportunity::observed_lane_clearance(world, actor, pos, teammate.position),
                distance_to_hoop: (teammate.position - hoop).length(),
            })
        })
        .collect();
    let lane = crate::opportunity::observed_lane_clearance(world, actor, pos, hoop);
    let skill_edge = primary.map_or(f32::default(), |defender| skill_edge(world, actor, defender.index));
    let mismatch = skill_edge.clamp(-1.0, 1.0) * 0.5;
    let margin = world.score[World::team_index(team)] as i32
        - world.score[World::team_index(team.other())] as i32;
    let leverage =
        crate::endgame::leverage(world, margin, crate::endgame::remaining_seconds(world));
    DecisionState {
        actor,
        actor_pos: pos,
        hoop,
        attacking_right: world.attacking_right(team),
        clock: world.clock.game_clock(),
        shot_clock: world.clock.shot_clock(),
        margin,
        period: world.clock.period(),
        primary_defender: primary,
        help,
        teammates,
        lane_clearance: lane,
        skill_edge,
        mismatch,
        stamina: world.players[actor].stamina,
        possession_kind: world.possession.kind,
        elapsed_in_possession: world.now() - world.possession.started_at,
        leverage,
        momentum: world.momentum[World::team_index(team)],
        contest_shape: shape,
    }
}

/// 当前有效的掩护路线读取窗口。路线过期后不再参与决策。
pub fn screen_read_route(world: &World) -> Option<DefenderRoute> {
    let window = world.params.policy.g("screen_read_window_seconds") as f64;
    world
        .possession
        .last_screen
        .filter(|read| world.now() - read.at <= window)
        .map(|read| read.route)
}

pub(crate) fn known_opponents(world: &World, actor: usize) -> impl Iterator<Item = (usize, crate::opportunity::PlayerRead)> + '_ {
    let team = world.players[actor].team;
    world.players.iter().enumerate()
        .filter(move |(_, player)| player.on_court && player.team != team)
        .filter_map(move |(index, _)| crate::opportunity::read(world, actor, index).map(|read| (index, read)))
}

fn skill_edge(world: &World, actor: usize, defender: usize) -> f32 {
    let defender = &world.players[defender];
    let offense = world.players[actor].data.attributes.ball_handling
        + world.players[actor].data.attributes.agility * 0.5;
    let defense =
        defender.data.attributes.defense_perimeter + defender.data.attributes.agility * 0.5;
    offense - defense
}

/// 决策层能看到的一名球员。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ObservedPlayer {
    pub index: usize,
    pub pos: glam::Vec2,
    pub vel: glam::Vec2,
    pub distance_ft: f32,
    pub angle_rad: f32,
    pub detail: bool,
}

/// 决策时可见信息的摘要。细节球员用于传球与协防判断，其余只保留位置。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Observation {
    pub actor: usize,
    pub facing: glam::Vec2,
    pub players: Vec<ObservedPlayer>,
    pub ball_pos: Option<glam::Vec2>,
    pub shot_clock: f64,
    pub game_clock: f64,
    pub score: [u32; 2],
    pub possession: u64,
}

fn segment_distance_sq(point: glam::Vec2, from: glam::Vec2, to: glam::Vec2) -> f32 {
    let segment = to - from;
    let length_squared = segment.length_squared();
    if length_squared <= f32::EPSILON {
        return (point - from).length_squared();
    }
    let t = ((point - from).dot(segment) / length_squared).clamp(0.0, 1.0);
    (point - (from + segment * t)).length_squared()
}

pub fn observe(world: &World, actor: usize) -> Observation {
    let player = &world.players[actor];
    let forward = player.facing.normalize_or_zero();
    let mut players = Vec::new();
    for (index, other) in world.players.iter().enumerate() {
        if index == actor || !other.on_court {
            continue;
        }
        let offset = other.pos - player.pos;
        let distance = offset.length();
        if distance > 55.0 {
            continue;
        }
        let angle = if distance <= f32::EPSILON {
            0.0
        } else {
            (forward.dot(offset / distance)).clamp(-1.0, 1.0).acos()
        };
        if angle > 100_f32.to_radians() {
            continue;
        }
        let dist_sq = distance * distance;
        let occluded = world
            .players
            .iter()
            .enumerate()
            .any(|(blocker, candidate)| {
                blocker != actor
                    && blocker != index
                    && candidate.on_court
                    && (candidate.pos - player.pos).length_squared() < dist_sq
                    && segment_distance_sq(candidate.pos, player.pos, other.pos) < world.rules.player_radius_ft.powi(2)
            });
        if occluded {
            continue;
        }
        let detail = distance <= 30.0 && angle <= 60_f32.to_radians();
        players.push(ObservedPlayer {
            index,
            pos: other.pos,
            vel: other.vel,
            distance_ft: distance,
            angle_rad: angle,
            detail,
        });
    }
    players.sort_by(|left, right| left.distance_ft.total_cmp(&right.distance_ft));
    for (rank, observed) in players.iter_mut().enumerate() {
        observed.detail &= rank < 6;
    }
    let ball_offset = world.ball.pos - player.pos;
    let ball_distance = ball_offset.length();
    let ball_angle = if ball_distance <= f32::EPSILON {
        0.0
    } else {
        (forward.dot(ball_offset / ball_distance))
            .clamp(-1.0, 1.0)
            .acos()
    };
    let ball_visible = world.holder() == Some(actor) || (ball_distance <= 55.0 && ball_angle <= 100_f32.to_radians()
        && !world.players.iter().enumerate().any(|(index, blocker)| {
            index != actor && Some(index) != world.holder() && blocker.on_court
                && (blocker.pos - player.pos).length_squared() < ball_offset.length_squared()
                && segment_distance_sq(blocker.pos, player.pos, world.ball.pos) < world.rules.player_radius_ft.powi(2)
        }));
    Observation {
        actor,
        facing: forward,
        players,
        ball_pos: ball_visible.then_some(world.ball.pos),
        shot_clock: world.clock.shot_clock(),
        game_clock: world.clock.game_clock(),
        score: world.score,
        possession: world.possession.id,
    }
}

fn mix(hash: &mut u64, value: u64) {
    *hash ^= value;
    *hash = hash.wrapping_mul(0x100000001b3);
}

pub fn observation_digest(world: &World, actor: usize) -> u64 {
    observation_digest_of(&observe(world, actor))
}

pub fn observation_digest_of(observation: &Observation) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    mix(&mut hash, observation.actor as u64);
    mix(&mut hash, u64::from(observation.facing.x.to_bits()));
    mix(&mut hash, u64::from(observation.facing.y.to_bits()));
    for player in &observation.players {
        mix(&mut hash, player.index as u64);
        mix(&mut hash, u64::from(player.pos.x.to_bits()));
        mix(&mut hash, u64::from(player.pos.y.to_bits()));
        mix(&mut hash, u64::from(player.vel.x.to_bits()));
        mix(&mut hash, u64::from(player.vel.y.to_bits()));
        mix(&mut hash, u64::from(player.distance_ft.to_bits()));
        mix(&mut hash, u64::from(player.angle_rad.to_bits()));
        mix(&mut hash, u64::from(player.detail));
    }
    mix(&mut hash, u64::from(observation.ball_pos.is_some()));
    if let Some(ball) = observation.ball_pos {
        mix(&mut hash, u64::from(ball.x.to_bits()));
        mix(&mut hash, u64::from(ball.y.to_bits()));
    }
    mix(&mut hash, observation.shot_clock.to_bits());
    mix(&mut hash, observation.game_clock.to_bits());
    mix(&mut hash, u64::from(observation.score[0]));
    mix(&mut hash, u64::from(observation.score[1]));
    mix(&mut hash, observation.possession);
    hash
}
