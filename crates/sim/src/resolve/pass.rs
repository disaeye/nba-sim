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
    let team = world.players[from].team;
    let lane = crate::state::pass_lane_clearance(
        &world.players,
        world.players[from].pos,
        world.players[to].pos,
        team,
    );
    let passer_skill = world.players[from].data.attributes.passing;
    let decision = world.players[from].data.attributes.decision_iq;
    let receiver_handling = world.players[to].data.attributes.ball_handling;
    // 传球终结了持球状态：收球标志在出手时清掉。
    world.possession.dribble_state = crate::state::DribbleState::Dribbling;
    world.possession.gather_pos = None;
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
    let (steal_credit, p_intercept) = match defender {
        Some((index, skill, gamble)) => (
            Some(index),
            clamp01(
                base * lane_risk / skill_relief
                    * steal_scale
                    * (skill_base + skill)
                    * (gamble_base + gamble_slope * gamble),
            ),
        ),
        None => (None, f32::MIN_POSITIVE),
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
        0.55 + 0.6 * receiver_handling - 0.25 * (1.0 - clamp01(lane / 8.0)) + 0.2 * passer_skill,
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
    world.pass_samples.push(crate::state::PassSample {
        possession: world.possession.id,
        from,
        to,
        from_pos: intent.from_ft,
        to_pos: intent.to_ft,
        at: intent.start_time,
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
    world.possession.dribble_state = crate::state::DribbleState::Dribbling;
    world.possession.gather_pos = None;
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

/// 谁去做这次抢断：位置与能力一起参与。距离越近权重越高，同时
/// `steal` 与 `gamble_steal` 高的防守人即使离得稍远也可能扑上来。
fn lane_defender(world: &mut World, from: usize, to: usize) -> Option<(usize, f32, f32)> {
    let start = world.players[from].pos;
    let end = world.players[to].pos;
    let team = world.players[from].team;
    let policy = &world.params.policy;
    let skill_base = policy.w("steal_skill_base");
    let gamble_base = policy.w("steal_gamble_base");
    let gamble_slope = policy.w("steal_gamble_slope");
    let distance_offset = policy.w("steal_distance_offset");
    let distance_scale = policy.w("steal_distance_scale");
    let candidates: Vec<(usize, f32, f32, f32)> = world
        .players
        .iter()
        .enumerate()
        .filter(|(_, p)| p.on_court && p.team != team)
        .map(|(index, p)| {
            let distance = segment_distance(start, end, p.pos);
            let skill = p.data.attributes.steal;
            let gamble = p.data.tendencies.gamble_steal;
            let weight = (distance_scale / (distance + distance_offset))
                * (skill_base + skill)
                * (gamble_base + gamble_slope * gamble);
            (index, skill, gamble, weight.max(f32::MIN_POSITIVE))
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let weights: Vec<f32> = candidates.iter().map(|entry| entry.3).collect();
    let pick = crate::resolve::weighted_pick(&mut world.rng, &weights);
    let (index, skill, gamble, _) = candidates[pick];
    Some((index, skill, gamble))
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
