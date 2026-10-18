//! 掩护结算：接触、防守人路线、分离距离。

use glam::Vec2;
use rand::Rng;

use crate::action::{
    ActionIntent, ActionKind, ActionOutcome, Completion, DefenderRoute, Fact, ScreenKind,
    StateDelta, TurnoverKind,
};
use crate::resolve::clamp01;
use crate::state::PossessionKind;
use crate::world::World;

pub fn resolve(world: &mut World, intent: &ActionIntent, kind: ScreenKind) -> ActionOutcome {
    let screener = intent.target.expect("screen requires a screener");
    let team = world.players[screener].team;
    let handler_pos = world.players[intent.actor].pos;
    let defender = world
        .players
        .iter()
        .enumerate()
        .filter(|(_, p)| p.on_court && p.team != team)
        .min_by(|a, b| {
            (a.1.pos - handler_pos)
                .length_squared()
                .partial_cmp(&(b.1.pos - handler_pos).length_squared())
                .expect("finite distances")
        })
        .map(|(index, _)| index);

    let Some(defender) = defender else {
        return ActionOutcome {
            actor: intent.actor,
            kind: ActionKind::Screen(kind),
            completion: Completion::Failed,
            delta: StateDelta::default(),
            facts: Vec::new(),
        };
    };

    // 掩护人走向防守人，形成真实接触位置。
    let defender_pos = world.players[defender].pos;
    let handler_pos = world.players[intent.actor].pos;
    let toward_handler = (handler_pos - defender_pos).normalize_or_zero();
    let contact_point = defender_pos + toward_handler * 1.6;
    world.players[screener].pos = contact_point;
    world.players[screener].target = contact_point;

    let strength = world.players[screener].data.attributes.strength;
    let defender_skill = world.players[defender].data.attributes.defense_perimeter;
    let agility = world.players[defender].data.attributes.agility;
    let contact = true;

    let switch_pressure = clamp01(0.12 + 0.55 * (strength - defender_skill));
    let under_pressure = clamp01(0.20 + 0.45 * (strength - agility));
    let roll = world.rng.gen::<f32>();
    let route = if roll < switch_pressure {
        DefenderRoute::Switch
    } else if roll < switch_pressure + under_pressure {
        DefenderRoute::Under
    } else {
        DefenderRoute::Over
    };

    let separation = match route {
        DefenderRoute::Switch => 0.4,
        DefenderRoute::Under => 2.4 + 2.0 * strength,
        DefenderRoute::Over => 1.0 + 1.2 * strength,
        DefenderRoute::Trapped => 0.2,
    };

    // 移动掩护：掩护人没站住，吹进攻犯规，球权转换。对抗强度高的
    // 掩护人更容易推着防守人走。
    let screener_physicality = world.players[screener].data.tendencies.physicality;
    let moving = world.params.policy.w("moving_screen_foul_rate")
        * (world.params.policy.w("moving_screen_physicality_base") + screener_physicality);
    if world.rng.gen::<f32>() < moving {
        world.line_mut(screener).turnovers += 1;
        world.line_mut(screener).fouls += 1;
        let index = World::team_index(team);
        world.fouls[index].period += 1;
        world.fouls[index].total += 1;
        world.players[screener].fouls += 1;
        world.emit(Fact::Turnover {
            player: screener,
            kind: TurnoverKind::OffensiveFoul,
        });
        world.end_possession("turnover", 0);
        world.start_possession(team.other(), PossessionKind::Live);
        return ActionOutcome {
            actor: intent.actor,
            kind: ActionKind::Screen(kind),
            completion: Completion::Failed,
            delta: StateDelta {
                turnover: Some(TurnoverKind::OffensiveFoul),
                ..Default::default()
            },
            facts: Vec::new(),
        };
    }

    world.possession.screens += 1;
    world.emit(Fact::Screen {
        screener,
        on_ball: kind == ScreenKind::OnBall,
        contact,
        route,
        separation_ft: separation,
    });

    ActionOutcome {
        actor: intent.actor,
        kind: ActionKind::Screen(kind),
        completion: Completion::Completed,
        delta: StateDelta {
            separation_ft: Some(separation),
            defender_route: Some(route),
            ..Default::default()
        },
        facts: Vec::new(),
    }
}

pub fn screen_position(screener: Vec2, handler: Vec2) -> Vec2 {
    let direction = (handler - screener).normalize_or_zero();
    screener + direction * 0.5
}
