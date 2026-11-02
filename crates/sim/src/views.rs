//! 决策视图：把世界状态折算成决策层只读的 [`DecisionState`]。
//!
//! 视图与规则、物理分开：决策层看不到世界对象，只能看到这里的字段，
//! 「掩护是否造成错位」「突破是否过掉人」因此必须以字段形式存在。

use crate::action::{DefenderRoute, TeamSide};
use crate::state::{DecisionState, DefenderView, HelpView, TeammateView};
use crate::world::World;

pub fn decision_state(world: &World) -> DecisionState {
    let actor = world.holder().expect("decision requires a ball holder");
    let team = world.players[actor].team;
    let hoop = world.hoop_for(team);
    let pos = world.players[actor].pos;
    let mut defenders: Vec<(usize, f32)> = world
        .players
        .iter()
        .enumerate()
        .filter(|(_, p)| p.on_court && p.team != team)
        .map(|(index, p)| (index, (p.pos - pos).length()))
        .collect();
    defenders.sort_by(|a, b| a.1.partial_cmp(&b.1).expect("finite distances"));
    let primary = defenders.first().map(|(index, distance)| {
        let defender = &world.players[*index];
        let closing =
            (defender.vel - world.players[actor].vel).dot((pos - defender.pos).normalize_or_zero());
        DefenderView {
            index: *index,
            distance_ft: *distance,
            closing_speed_ftps: closing,
            between_ball_and_hoop: crate::state::is_between(defender.pos, pos, hoop),
            beaten: *distance > 5.0,
            route: screen_read_route(world),
            reach_excess_ft: crate::physical::Physical::of(defender, &world.params.policy)
                .reach_excess_ft(&world.params.policy),
        }
    });
    let help = defenders.get(1).map(|(index, distance)| {
        let defender = &world.players[*index];
        HelpView {
            distance_ft: *distance,
            arrival_seconds: (*distance / defender.effective_speed(&world.params.policy)).max(0.05),
        }
    });
    let teammates = world
        .on_court(team)
        .into_iter()
        .filter(|handle| *handle != actor)
        .map(|handle| {
            let teammate = &world.players[handle];
            // 最近防守人的摸高与距离一起决定接球人的空位程度，
            // 与主决策里的干扰用同一个映射与同一条公式。
            let nearest = world
                .players
                .iter()
                .filter(|p| p.on_court && p.team != team)
                .min_by(|a, b| {
                    let da = (a.pos - teammate.pos).length_squared();
                    let db = (b.pos - teammate.pos).length_squared();
                    da.partial_cmp(&db).expect("finite distances")
                });
            let gap = nearest
                .map(|defender| (defender.pos - teammate.pos).length())
                .unwrap_or(f32::INFINITY);
            let reach = nearest
                .map(|defender| {
                    crate::physical::Physical::of(defender, &world.params.policy)
                        .reach_excess_ft(&world.params.policy)
                })
                .unwrap_or(f32::default());
            TeammateView {
                index: handle,
                pos: teammate.pos,
                openness: crate::state::contest_from_defender(gap, false, reach),
                defender_distance_ft: gap,
                lane_clearance: crate::state::pass_lane_clearance(
                    &world.players,
                    pos,
                    teammate.pos,
                    team,
                ),
                distance_to_hoop: (teammate.pos - hoop).length(),
            }
        })
        .collect();
    let lane = crate::state::lane_clearance(&world.players, pos, hoop, team);
    let skill_edge = skill_edge(world, actor, team);
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

/// 持球人与最近防守人的突破技能差（未截断）。
///
/// 决策层用它估突破的期望得分，结算层用同一个差抽过人结果；
/// 两处用不同式子会让「觉得能过」与「实际能过」脱钩。
pub fn skill_edge(world: &World, actor: usize, team: TeamSide) -> f32 {
    let nearest = world
        .players
        .iter()
        .filter(|p| p.on_court && p.team != team)
        .min_by(|a, b| {
            (a.pos - world.players[actor].pos)
                .length_squared()
                .partial_cmp(&(b.pos - world.players[actor].pos).length_squared())
                .expect("finite distances")
        });
    let Some(defender) = nearest else {
        return 0.0;
    };
    let offense = world.players[actor].data.attributes.ball_handling
        + world.players[actor].data.attributes.agility * 0.5;
    let defense =
        defender.data.attributes.defense_perimeter + defender.data.attributes.agility * 0.5;
    offense - defense
}

/// 决策层能看到的一名球员。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObservedPlayer {
    pub index: usize,
    pub pos: glam::Vec2,
    pub vel: glam::Vec2,
    pub distance_ft: f32,
    pub angle_rad: f32,
    pub detail: bool,
}

/// 决策时可见信息的摘要。细节球员用于传球与协防判断，其余只保留位置。
#[derive(Debug, Clone)]
pub struct Observation {
    pub actor: usize,
    pub facing: glam::Vec2,
    pub players: Vec<ObservedPlayer>,
    pub ball_pos: glam::Vec2,
    pub ball_visible: bool,
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
                    && segment_distance_sq(candidate.pos, player.pos, other.pos) < 20.25
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
    let ball_visible = ball_distance <= 55.0 && ball_angle <= 100_f32.to_radians();
    Observation {
        actor,
        facing: forward,
        players,
        ball_pos: world.ball.pos,
        ball_visible,
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
    let observation = observe(world, actor);
    let mut hash = 0xcbf29ce484222325u64;
    mix(&mut hash, observation.actor as u64);
    for player in &observation.players {
        mix(&mut hash, player.index as u64);
        mix(&mut hash, u64::from(player.pos.x.to_bits()));
        mix(&mut hash, u64::from(player.pos.y.to_bits()));
        mix(&mut hash, u64::from(player.vel.x.to_bits()));
        mix(&mut hash, u64::from(player.vel.y.to_bits()));
        mix(&mut hash, u64::from(player.detail));
    }
    mix(&mut hash, u64::from(observation.ball_pos.x.to_bits()));
    mix(&mut hash, u64::from(observation.ball_pos.y.to_bits()));
    mix(&mut hash, u64::from(observation.ball_visible));
    mix(&mut hash, observation.shot_clock.to_bits());
    mix(&mut hash, observation.game_clock.to_bits());
    mix(&mut hash, u64::from(observation.score[0]));
    mix(&mut hash, u64::from(observation.score[1]));
    mix(&mut hash, observation.possession);
    hash
}
