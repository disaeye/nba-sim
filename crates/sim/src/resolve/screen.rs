//! 掩护结算：接触、防守人路线、分离距离。

use glam::Vec2;

use crate::action::{
    ActionIntent, ActionKind, ActionOutcome, Completion, DefenderRoute, Fact, ScreenKind,
    StateDelta, TurnoverKind,
};
use crate::world::World;

pub fn resolve(world: &mut World, intent: &ActionIntent, kind: ScreenKind) -> ActionOutcome {
    let screener = intent.target.expect("screen requires a screener");
    let team = world.players[screener].team;
    let defender = world
        .screen_defender
        .expect("screen resolution requires its decision-time defender");
    // 掩护的防守人是决策时刻定下的，不是动作的参与人：死球换人可以在
    // 决策与结算之间把他换下场。这种时序不是内核错误，掩护因此没有
    // 接触也没有路线，动作按失败结束。
    if !world.players[defender].on_court || world.players[defender].team == team {
        return ActionOutcome {
            actor: intent.actor,
            kind: ActionKind::Screen(kind),
            completion: Completion::Failed,
            delta: StateDelta::default(),
            facts: Vec::new(),
        };
    }

    let defender_pos = world.players[defender].pos;

    let strength = world.players[screener].data.attributes.strength;
    let defender_skill = world.players[defender].data.attributes.defense_perimeter;
    let contact_threshold =
        world.params.policy.g("pressure_separation_ft") + world.rules.contact_margin_ft;
    let contact = (world.players[screener].pos - defender_pos).length() <= contact_threshold;

    let shooter_three = world.players[intent.actor].data.attributes.shooting_three;
    let roll = world.draw(
        crate::draw::Stream::Decision,
        defender,
        crate::draw::purpose::ACTION_CHOICE,
        0,
    );
    // 力量连续调制防守路线与推开距离；防守抵抗力经 capability 映射计算。
    let defender_attributes = &world.players[defender].data.attributes;
    let resistance = nba_domain::capability::effective_post_defense_physicality(
        &world.rules,
        defender_attributes,
    );
    let separation_before = (world.players[screener].pos - defender_pos).length();
    let contact_window = (world.params.policy.g("screen_contact_range_ft") - separation_before)
        .clamp(0.0, world.params.policy.g("screen_contact_range_ft"))
        / world.params.policy.g("screen_contact_range_ft");
    let strength_edge = strength - defender_skill;
    let push_chance = crate::resolve::sigmoid(
        world.params.policy.w("screen_push_strength_gain") * strength_edge - 1.0,
    ) * contact_window;
    let route = if roll < push_chance {
        DefenderRoute::Switch
    } else if shooter_three >= 0.70 {
        if roll < 0.45 {
            DefenderRoute::Under
        } else {
            DefenderRoute::Over
        }
    } else if roll < 0.65 {
        DefenderRoute::Under
    } else {
        DefenderRoute::Over
    };
    let strength_edge = strength - resistance;
    // 掩护的身体接触本身就会把防守人顶开一点，力量差在它上面叠加：
    // 弱掩护人也会把人撞歪，强掩护人把差距拉开。
    let displacement_ft = (world.params.policy.w("screen_base_displacement_ft")
        + strength_edge.max(0.0))
        * world.params.policy.w("screen_push_displacement_gain_ft")
        * contact_window;
    let separation = (world.players[screener].pos - world.players[defender].pos)
        .length()
        .max(0.2);
    // 掩护的身体接触把防守人顶开：位移直接作用在位置上，防守人随后自己
    // 走回对位点。单次顶开有上限，不能一记掩护把人送出几步。
    if displacement_ft > 0.0 {
        let away = (world.players[defender].pos - world.players[screener].pos).normalize_or_zero();
        let pushed = world.players[defender].pos + away * displacement_ft.min(0.6);
        world.players[defender].pos = pushed;
    }

    // 移动掩护：掩护人还没落位就撞上防守人。接触本身是掩护的常态，
    // 裁判只在其中吹罚一部分；physicality 越高越不容易被吹。
    let screener_physicality = world.players[screener].data.tendencies.physicality;
    let screener_speed = world.players[screener].vel.length();
    let illegal_speed = 1.2 - 0.5 * screener_physicality;
    // 还在走向掩护点的人没有落下脚步：站住的掩护允许接触，走着撞上去
    // 的不行。
    let unsettled = (world.players[screener].target - world.players[screener].pos).length() > 0.6;
    let moving = contact && (screener_speed > illegal_speed || unsettled);
    let call_rate = world.params.policy.w("moving_screen_foul_rate")
        * (world.params.policy.w("moving_screen_physicality_base") + 1.0 - screener_physicality);
    let called = moving
        && world.draw(
            crate::draw::Stream::Foul,
            screener,
            crate::draw::purpose::MOVING_SCREEN,
            0,
        ) < call_rate;
    if called {
        world.line_mut(screener).turnovers += 1;
        world.emit(Fact::Turnover {
            player: screener,
            kind: TurnoverKind::OffensiveFoul,
        });
        crate::resolve::foul::apply_offensive_foul(world, screener, defender);
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
    // 掩护完成后掩护人必须选择顺下或外弹：记下时刻，无球决策读它。
    world.players[screener].screened_at = world.now();
    world.emit(Fact::Screen {
        screener,
        team,
        on_ball: kind == ScreenKind::OnBall,
        contact,
        route,
        separation_ft: separation,
        strength,
        displacement_ft,
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
