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
        let closing = (defender.vel - world.players[actor].vel)
            .dot((pos - defender.pos).normalize_or_zero());
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
    let leverage = crate::endgame::leverage(world, margin, crate::endgame::remaining_seconds(world));
    DecisionState {
        actor,
        actor_pos: pos,
        hoop,
        attacking_right: world.attacking_right(team),
        clock: world.clock.game_clock,
        shot_clock: world.clock.shot_clock,
        margin,
        period: world.clock.period,
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
