//! 传球结算：拦截判定、接球质量、接球人控制权。

use glam::Vec2;
use rand::Rng;

use crate::action::{
    ActionIntent, ActionKind, Completion, Fact, PassKind, StateDelta, TurnoverKind,
};
use crate::resolve::{clamp01, sigmoid};
use crate::state::PossessionKind;
use crate::world::World;

pub fn resolve(
    world: &mut World,
    intent: &ActionIntent,
    kind: PassKind,
) -> crate::action::ActionOutcome {
    let from = intent.actor;
    let to = intent.target.expect("pass requires a receiver");
    let pass_id = world.new_pass_id();
    let passer = &world.players[from];
    let receiver = &world.players[to];
    let team = passer.team;
    let lane = crate::state::lane_clearance(&world.players, passer.pos, receiver.pos, team);
    let passer_skill = passer.data.attributes.passing;
    let decision = passer.data.attributes.decision_iq;
    let base = world.params.policy.w("pass_turnover_rate");
    let lane_risk = (1.0 - clamp01(lane / 6.0)).max(0.0);
    let skill_relief = 0.5 + passer_skill + 0.8 * decision;
    // 抢断归因于卡在传球路线上的那个人：自己的 steal 与冒险倾向决定
    // 拦截机会，传球人的视野与传球能力决定风险被压低多少。
    let defender = lane_defender(world, from, to);
    let steal_scale = world.params.policy.w("steal_skill_scale");
    let skill_base = world.params.policy.w("steal_skill_base");
    let gamble_base = world.params.policy.w("steal_gamble_base");
    let gamble_slope = world.params.policy.w("steal_gamble_slope");
    let steal_credit = defender.map(|(index, _, _)| index);
    let p_intercept = match lane_defender(world, from, to) {
        Some((_, skill, gamble)) => clamp01(
            base * lane_risk / skill_relief
                * steal_scale
                * (skill_base + skill)
                * (gamble_base + gamble_slope * gamble),
        ),
        None => f32::MIN_POSITIVE,
    };

    if world.rng.gen::<f32>() < p_intercept {
        world.line_mut(from).turnovers += 1;
        if let Some(defender) = steal_credit {
            world.line_mut(defender).steals += 1;
        }
        world.emit(Fact::Turnover {
            player: from,
            kind: TurnoverKind::BadPass,
        });
        world.end_possession("turnover", 0);
        world.start_possession(team.other(), PossessionKind::Live);
        return crate::action::ActionOutcome {
            actor: from,
            kind: ActionKind::Pass(kind),
            completion: Completion::Failed,
            delta: StateDelta {
                turnover: Some(TurnoverKind::BadPass),
                ..Default::default()
            },
            facts: Vec::new(),
        };
    }

    // 非抢断失误：传球直接出界。这类回合计入失误但不给任何人记抢断；
    // 传球技巧决定出界的概率，判断力已经作用在上面的拦截风险里。
    let bad_pass = clamp01(
        world.params.policy.w("bad_pass_rate")
            / (world.params.policy.w("bad_pass_skill_base") + passer_skill),
    );
    if world.rng.gen::<f32>() < bad_pass {
        world.line_mut(from).turnovers += 1;
        world.emit(Fact::Turnover {
            player: from,
            kind: TurnoverKind::BadPass,
        });
        world.end_possession("turnover", 0);
        world.start_possession(team.other(), PossessionKind::Live);
        return crate::action::ActionOutcome {
            actor: from,
            kind: ActionKind::Pass(kind),
            completion: Completion::Failed,
            delta: StateDelta {
                turnover: Some(TurnoverKind::BadPass),
                ..Default::default()
            },
            facts: Vec::new(),
        };
    }

    let catch_quality = clamp01(
        0.55 + 0.6 * receiver.data.attributes.ball_handling - 0.25 * (1.0 - clamp01(lane / 8.0))
            + 0.2 * passer_skill,
    );
    let mut facts = vec![Fact::Pass {
        pass_id,
        from,
        to,
        kind,
        completed: true,
    }];
    if catch_quality < 0.35 {
        facts.push(Fact::Turnover {
            player: to,
            kind: TurnoverKind::LostBall,
        });
        world.line_mut(to).turnovers += 1;
        world.end_possession("turnover", 0);
        world.start_possession(team.other(), PossessionKind::Live);
        return crate::action::ActionOutcome {
            actor: from,
            kind: ActionKind::Pass(kind),
            completion: Completion::Failed,
            delta: StateDelta {
                turnover: Some(TurnoverKind::LostBall),
                ..Default::default()
            },
            facts,
        };
    }

    world.possession.passes += 1;
    world.possession.last_pass_at = Some(world.now());
    world.possession.last_pass_from = Some(from);
    world.possession.assist_candidate = Some((from, world.now()));
    world.last_pass_kind = Some(kind);
    world.emit(Fact::Pass {
        pass_id,
        from,
        to,
        kind,
        completed: true,
    });
    crate::action::ActionOutcome {
        actor: from,
        kind: ActionKind::Pass(kind),
        completion: Completion::Completed,
        delta: StateDelta {
            control: None,
            ..Default::default()
        },
        facts,
    }
}

pub fn resolve_cut(world: &mut World, intent: &ActionIntent) -> crate::action::ActionOutcome {
    let cutter = intent.target.expect("cut requires a cutter");
    let team = world.players[cutter].team;
    let hoop = world.hoop_for(team);
    let defender_distance = world
        .players
        .iter()
        .filter(|p| p.on_court && p.team != team)
        .map(|p| (p.pos - hoop).length())
        .fold(f32::INFINITY, f32::min);
    let open = clamp01(defender_distance / 8.0);
    let completion =
        clamp01(0.35 + 0.6 * open + 0.3 * world.players[cutter].data.attributes.off_ball_sense);
    if world.rng.gen::<f32>() > completion {
        return crate::action::ActionOutcome {
            actor: intent.actor,
            kind: ActionKind::Cut,
            completion: Completion::Failed,
            delta: StateDelta::default(),
            facts: Vec::new(),
        };
    }
    world.players[cutter].pos = hoop
        + (Vec2::new(-1.0, 0.0)
            * if world.attacking_right(team) {
                1.0
            } else {
                -1.0
            })
            * 2.5;
    world.players[cutter].target = world.players[cutter].pos;
    world.ball = crate::state::BallState::Held(cutter);
    world.possession.last_pass_at = Some(world.now());
    world.possession.last_pass_from = Some(intent.actor);
    world.possession.assist_candidate = Some((intent.actor, world.now()));
    world.possession.last_cut_at = Some(world.now());
    world.possession.last_cut_actor = Some(cutter);
    let pass_id = world.new_pass_id();
    world.emit(Fact::Pass {
        pass_id,
        from: intent.actor,
        to: cutter,
        kind: PassKind::Lob,
        completed: true,
    });
    crate::action::ActionOutcome {
        actor: intent.actor,
        kind: ActionKind::Cut,
        completion: Completion::Completed,
        delta: StateDelta {
            control: Some(cutter),
            ..Default::default()
        },
        facts: Vec::new(),
    }
}

/// 卡在传球路线上的防守人，连同他自己的抢断与冒险倾向。返回球员在
/// 在场球员里的下标，用来记抢断。
fn lane_defender(world: &World, from: usize, to: usize) -> Option<(usize, f32, f32)> {
    let start = world.players[from].pos;
    let end = world.players[to].pos;
    let team = world.players[from].team;
    world
        .players
        .iter()
        .enumerate()
        .filter(|(_, p)| p.on_court && p.team != team)
        .min_by(|a, b| {
            let left = segment_distance(start, end, a.1.pos);
            let right = segment_distance(start, end, b.1.pos);
            left.partial_cmp(&right).expect("finite lane distances")
        })
        .map(|(index, p)| {
            (
                index,
                p.data.attributes.steal,
                p.data.tendencies.gamble_steal,
            )
        })
}

fn segment_distance(start: Vec2, end: Vec2, point: Vec2) -> f32 {
    let span = end - start;
    let length_squared = span.length_squared();
    if length_squared <= f32::EPSILON {
        return (point - start).length();
    }
    let t = (point - start).dot(span) / length_squared;
    (point - (start + span * t)).length()
}

pub fn pass_speed_reference() -> f32 {
    let _ = sigmoid(0.0);
    28.0
}
