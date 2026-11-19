//! 掩护结算：接触、防守人路线、分离距离。

use glam::Vec2;

use crate::action::{
    ActionIntent, ActionKind, ActionOutcome, Completion, DefenderRoute, Fact, ScreenKind,
    StateDelta, TurnoverKind,
};
use crate::world::World;

pub fn resolve(world: &mut World, intent: &ActionIntent, kind: ScreenKind) -> ActionOutcome {
    let target = intent.target.expect("screen requires a partner");
    let (handler, screener) = if world.holder() == Some(intent.actor) {
        (intent.actor, target)
    } else if world.holder() == Some(target) {
        (target, intent.actor)
    } else {
        return ActionOutcome {
            actor: intent.actor, kind: ActionKind::Screen(kind), completion: Completion::Failed,
            delta: StateDelta::default(), facts: Vec::new(),
        };
    };
    if world.holder() != Some(handler) {
        return ActionOutcome {
            actor: intent.actor, kind: ActionKind::Screen(kind), completion: Completion::Failed,
            delta: StateDelta::default(), facts: Vec::new(),
        };
    }
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
    // 接触阈值：两个身体能靠到多近才算接触。人的身体中心距就是最小间隔，
    // 因此接触判据是「最小间隔 + 一点余量」。
    let contact_threshold = world.rules.min_player_separation_ft + world.rules.contact_margin_ft;
    // 接触：掩护人与对位防守人真的靠到了一起。
    let contact = (world.players[screener].pos - defender_pos).length() <= contact_threshold;

    let shooter_three = world.players[handler].data.attributes.shooting_three;
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
    let route_contact = contact;
    let strength_edge = strength - defender_skill;
    let push_chance = crate::resolve::sigmoid(
        world.params.policy.w("screen_push_strength_gain") * strength_edge - 1.0,
    ) * contact_window;
    use nba_domain::rules::OnBallScreenDefenseStrategy;
    // 掩护防守方案属于守方：谁在守，就用谁的方案。
    let scheme = world.defense_scheme(world.possession.team.other());
    let cover = scheme.screen_defense;
    let can_switch = route_contact && crate::defense::screen_partner(world, screener, defender)
        .is_some_and(|partner| (world.players[partner].pos - world.players[handler].pos).length() <= cover.switch_trigger_distance_ft);
    let route = match cover.strategy {
        // 逢掩护必换：没有可换的人就挤过。
        OnBallScreenDefenseStrategy::Switch if can_switch => DefenderRoute::Switch,
        OnBallScreenDefenseStrategy::Switch => DefenderRoute::Over,
        OnBallScreenDefenseStrategy::GoUnder => DefenderRoute::Under,
        OnBallScreenDefenseStrategy::FightThrough => DefenderRoute::Over,
        // 标准对位：防守人自己看对位事实决定。掩护人明显更强时可以换防，
        // 换防的机会随力量差增大；方案的激进程度再往上加。
        OnBallScreenDefenseStrategy::StandardContest => {
            let under = if shooter_three >= world.params.policy.g("screen_pop_rating") {
                world.params.policy.g("screen_under_prob_shooter")
            } else {
                world.params.policy.g("screen_under_prob_base")
            };
            let switch_chance = (push_chance + scheme.switch_aggressiveness * 0.5).clamp(0.0, 0.95);
            if can_switch && roll < switch_chance {
                DefenderRoute::Switch
            } else if roll < under {
                DefenderRoute::Under
            } else {
                DefenderRoute::Over
            }
        }
    };
    let strength_edge = strength - resistance;
    // 掩护的身体接触本身就会把防守人顶开一点，力量差在它上面叠加：
    // 弱掩护人也会把人撞歪，强掩护人把差距拉开。
    let displacement_ft = (world.params.policy.w("screen_base_displacement_ft")
        + strength_edge.max(0.0))
        * world.params.policy.w("screen_push_displacement_gain_ft")
        * contact_window;
    let separation = (world.players[screener].pos - world.players[defender].pos).length();
    // 掩护的身体接触把防守人顶开：位移直接作用在位置上，防守人随后自己
    // 走回对位点。单次顶开有上限，不能一记掩护把人送出几步。
    let displacement_ft = if route_contact && displacement_ft > 0.0 {
        let away = (defender_pos - world.players[screener].pos).normalize_or_zero();
        crate::body::apply_contact_displacement(world, defender, away * displacement_ft.min(0.6));
        (world.players[defender].pos - defender_pos).length()
    } else {
        f32::default()
    };

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
        // 非法掩护：判定层给出事实，裁决层给后果。
        let incident = crate::detect::detect_illegal_screen(world, screener, defender);
        crate::adjudicate_incidents::adjudicate_one(world, Some(incident));
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

    let handler_gap = (world.players[handler].pos - world.players[screener].pos).length();
    let handler_approached = handler_gap <= contact_threshold;
    let used = handler_approached && contact;
    if used {
        world.possession.screens += 1;
        world.players[screener].screened_at = world.now();
    }
    // 掩护的技术变体决定掩护人往哪走：顺下滚向篮下，外弹弹向外线，
    // 提前滑出则在接触之前先离开。变体因此改变防守人追谁。
    match kind {
        ScreenKind::Roll => {
            let hoop = world.hoop_for(team);
            let direction = (hoop - world.players[screener].pos).normalize_or_zero();
            let roll = world.players[screener].pos
                + direction * world.params.policy.g("screen_roll_distance_ft");
            crate::assignment::force_target_as(
                world,
                screener,
                roll,
                crate::state::TargetKind::Window,
                handler,
            );
        }
        ScreenKind::Pop => {
            let direction =
                (world.players[screener].pos - world.hoop_for(team)).normalize_or_zero();
            let pop = world.players[screener].pos
                + direction * world.params.policy.g("screen_pop_distance_ft");
            crate::assignment::force_target_as(
                world,
                screener,
                pop,
                crate::state::TargetKind::Window,
                handler,
            );
        }
        ScreenKind::Slip => {
            // 提前滑出：掩护人在接触之前先离开，防守人被晃过。
            let direction =
                (world.players[screener].pos - world.players[defender].pos).normalize_or_zero();
            let slip = world.players[screener].pos
                + direction * world.params.policy.g("screen_slip_distance_ft");
            crate::assignment::force_target_as(
                world,
                screener,
                slip,
                crate::state::TargetKind::Window,
                handler,
            );
        }
        ScreenKind::OnBall | ScreenKind::OffBall | ScreenKind::HandOff => {}
    }
    world.emit(Fact::Screen {
        action_id: world.action_turn.map(|(id, _)| id),
        handler,
        screener,
        defender,
        team,
        on_ball: kind != ScreenKind::OffBall,
        contact,
        used,
        handler_approached,
        handler_screener_gap_ft: handler_gap,
        route,
        separation_ft: separation,
        strength,
        displacement_ft,
    });

    ActionOutcome {
        actor: intent.actor,
        kind: ActionKind::Screen(kind),
        completion: if used || kind == ScreenKind::Slip { Completion::Completed } else { Completion::Failed },
        delta: StateDelta {
            separation_ft: used.then_some((world.players[handler].pos - world.players[defender].pos).length()),
            defender_route: used.then_some(route),
            ..Default::default()
        },
        facts: Vec::new(),
    }
}

pub fn screen_position(screener: Vec2, handler: Vec2) -> Vec2 {
    let direction = (handler - screener).normalize_or_zero();
    screener + direction * 0.5
}

/// 掩护人该站的位置：防守人朝持球人的方向让出一步。
///
/// 这是掩护落点的唯一来源：候选生成、动作时长与结算都用它，因此「掩护人
/// 该站在哪」在三处不会各算各的。
pub fn set_point(
    defender_pos: Vec2,
    handler_pos: Vec2,
    rules: &nba_domain::GameRules,
) -> Vec2 {
    let axis = (handler_pos - defender_pos).normalize_or_zero();
    defender_pos + axis * (rules.min_player_separation_ft + rules.contact_margin_ft * 0.5)
}
