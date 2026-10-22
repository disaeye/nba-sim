//! 突破与背身结算：过掉首要防守人、到达位置、丢球。

use glam::Vec2;
use rand::Rng;

use crate::action::{
    ActionIntent, ActionKind, ActionOutcome, Completion, Fact, StateDelta, TurnoverKind,
};
use crate::resolve::clamp01;
use crate::state::PossessionKind;
use crate::world::World;

pub fn resolve(world: &mut World, intent: &ActionIntent) -> ActionOutcome {
    let driver = intent.actor;
    let team = world.players[driver].team;
    let hoop = world.hoop_for(team);
    let pos = world.players[driver].pos;
    let lane = crate::state::lane_clearance(&world.players, pos, hoop, team);
    let nearest = world
        .players
        .iter()
        .enumerate()
        .filter(|(_, p)| p.on_court && p.team != team)
        .min_by(|a, b| {
            (a.1.pos - pos)
                .length_squared()
                .partial_cmp(&(b.1.pos - pos).length_squared())
                .expect("finite distances")
        })
        .map(|(index, _)| index);

    let turnover_base = world.params.policy.w("drive_turnover_rate");
    let handle = world.players[driver].data.attributes.ball_handling
        + world.players[driver].data.attributes.agility * 0.5;
    let decision = world.players[driver].data.attributes.decision_iq;
    let risk = clamp01(turnover_base * (1.4 - handle - 0.3 * decision));
    if world.rng.gen::<f32>() < risk {
        world.line_mut(driver).turnovers += 1;
        if let Some(defender) = nearest {
            if world.rng.gen::<f32>() < 0.55 {
                world.line_mut(defender).steals += 1;
            }
        }
        world.emit(Fact::Turnover {
            player: driver,
            kind: TurnoverKind::LostBall,
        });
        world.end_possession("turnover", 0);
        world.start_possession(team.other(), PossessionKind::Live);
        return ActionOutcome {
            actor: driver,
            kind: ActionKind::Drive,
            completion: Completion::Failed,
            delta: StateDelta {
                turnover: Some(TurnoverKind::LostBall),
                ..Default::default()
            },
            facts: Vec::new(),
        };
    }

    let (defense_skill, help_distance) = match nearest {
        Some(defender) => {
            let skill = world.players[defender].data.attributes.defense_perimeter
                + world.players[defender].data.attributes.agility * 0.5;
            let help = world
                .players
                .iter()
                .filter(|p| p.on_court && p.team != team && p.id != world.players[defender].id)
                .map(|p| (p.pos - pos).length())
                .fold(f32::INFINITY, f32::min);
            (skill, help)
        }
        None => (0.5, 30.0),
    };

    let lane_norm = clamp01(lane / 8.0);
    // 失位前后的手部犯规：防守人用犯规阻止突破。对抗与协防倾向高的
    // 防守人更容易伸手；未到 bonus 时只记犯规，到 bonus 时给罚球。
    if let Some(defender) = nearest {
        let physicality = world.players[defender].data.tendencies.physicality;
        let help = world.players[defender].data.tendencies.help_aggressiveness;
        let policy = &world.params.policy;
        let rate = policy.w("reach_in_foul_rate")
            * (policy.w("reach_in_physicality_base") + physicality)
            * (policy.w("reach_in_help_base") + help);
        if world.rng.gen::<f32>() < rate {
            let fouling_team = world.players[defender].team;
            let free_throws = crate::resolve::foul::bonus_free_throws(world, fouling_team);
            crate::resolve::foul::apply_non_shooting_foul(world, defender, driver);
            if free_throws == 0 {
                // 非 bonus：死球后进攻方继续本次回合，只是把下一次决策
                // 推后；不重建回合，否则每吹一次犯规就多算一个回合。
                world.next_decision_at =
                    world.now() + world.params.policy.g("reach_in_dead_ball_seconds") as f64;
            }
            return ActionOutcome {
                actor: driver,
                kind: ActionKind::Drive,
                completion: Completion::Failed,
                delta: StateDelta::default(),
                facts: Vec::new(),
            };
        }
    }
    let help_term = clamp01(1.0 - help_distance / 20.0);
    let p_beat = crate::policy::blow_by_probability(
        world,
        handle - defense_skill,
        lane_norm,
        help_term,
    );
    let beaten = world.rng.gen::<f32>() < p_beat;

    let travelled = (pos - intent.from_ft).length();
    let max_travel = world.rules.max_player_speed_ftps * intent.duration as f32;
    if travelled > max_travel * 1.08 {
        world.violations.push(format!(
            "geometry: drive covered {travelled:.1} ft in {:.2} s (limit {max_travel:.1} ft)",
            intent.duration
        ));
    }

    let direction = (hoop - pos).normalize_or_zero();
    if beaten {
        // 突破落点按参考表的驱动分区份额取双峰：一部分直接到篮下，一部分在近筐带收球起抛投。
        let rim_share = world.params.shot_profile.zone_by_creation["drive"]["rim"];
        let finish_distance = if world.rng.gen::<f32>() < rim_share {
            world.params.policy.g("drive_rim_finish_ft")
                + world.rng.gen::<f32>() * world.params.policy.g("drive_rim_spread_ft")
        } else {
            world.params.policy.g("drive_finish_distance_ft")
                + world.rng.gen::<f32>() * world.params.policy.g("drive_finish_spread_ft")
        };
        let finish = hoop - direction * finish_distance;
        world.players[driver].pos = finish;
        world.players[driver].target = finish;
        // 运球到终结距离为止，触球点记在这里；收球动作从它开始。
        world.possession.gather_pos = Some(finish);
        world.possession.dribble_state = crate::state::DribbleState::Dribbling;
        world.possession.drives += 1;
        world.possession.last_drive_at = Some(world.now());
        world.possession.last_drive_actor = Some(driver);
        world.emit(Fact::Drive {
            driver,
            reached: true,
            defender_beaten: true,
        });
        ActionOutcome {
            actor: driver,
            kind: ActionKind::Drive,
            completion: Completion::Completed,
            delta: StateDelta {
                separation_ft: Some(2.0 + 2.0 * lane_norm),
                ..Default::default()
            },
            facts: Vec::new(),
        }
    } else {
        world.possession.drives += 1;
        world.possession.last_drive_at = Some(world.now());
        world.possession.last_drive_actor = Some(driver);
        world.emit(Fact::Drive {
            driver,
            reached: false,
            defender_beaten: false,
        });
        ActionOutcome {
            actor: driver,
            kind: ActionKind::Drive,
            completion: Completion::Failed,
            delta: StateDelta {
                separation_ft: Some(0.3),
                ..Default::default()
            },
            facts: Vec::new(),
        }
    }
}

/// 走步判定：收球点到终结点的位移超过容限。容限是两步加起跳的
/// 英尺量，来自 `policy.geometry`。
pub fn travel_on_drive(gather: Vec2, finish: Vec2, allowance: f32) -> bool {
    (finish - gather).length() > allowance
}

/// 收球步：从触球点向篮筐迈两步。位移超过容限就是走步，容限是
/// 两步加起跳的英尺量，来自 `policy.geometry`。
pub fn resolve_finish(world: &mut World, intent: &ActionIntent) -> ActionOutcome {
    let actor = intent.actor;
    let team = world.players[actor].team;
    let hoop = world.hoop_for(team);
    let pos = world.players[actor].pos;
    let direction = (hoop - pos).normalize_or_zero();
    let allowance = world.params.policy.g("gather_travel_allowance_ft");
    let gather = world.possession.gather_pos.unwrap_or(pos);
    // 收球步的两步量程不超过容限：超出的部分本该在运球阶段消化，
    // 否则每次从外线发起的终结都是走步。
    let min_distance = world.params.policy.g("finish_min_distance_ft");
    let span = ((hoop - gather).length() - min_distance).clamp(0.0, allowance);
    let finish = hoop - direction * (min_distance + world.rng.gen::<f32>() * span);
    if travel_on_drive(gather, finish, allowance) {
        world.line_mut(actor).turnovers += 1;
        world.emit(Fact::Turnover {
            player: actor,
            kind: TurnoverKind::Travel,
        });
        world.end_possession("turnover", 0);
        world.start_possession(team.other(), PossessionKind::Live);
        return ActionOutcome {
            actor,
            kind: ActionKind::Finish,
            completion: Completion::Failed,
            delta: StateDelta {
                turnover: Some(TurnoverKind::Travel),
                ..Default::default()
            },
            facts: Vec::new(),
        };
    }
    world.players[actor].pos = finish;
    world.players[actor].target = finish;
    world.possession.dribble_state = crate::state::DribbleState::Gathered;
    world.possession.gather_pos = Some(finish);
    // 收球步仍属于这次突破，出手语境按突破计算。
    world.possession.last_drive_at = Some(world.now());
    world.possession.last_drive_actor = Some(actor);
    ActionOutcome {
        actor,
        kind: ActionKind::Finish,
        completion: Completion::Completed,
        delta: StateDelta::default(),
        facts: Vec::new(),
    }
}

pub fn resolve_post(world: &mut World, intent: &ActionIntent) -> ActionOutcome {
    let actor = intent.actor;
    let team = world.players[actor].team;
    let hoop = world.hoop_for(team);
    let pos = world.players[actor].pos;
    let direction = (hoop - pos).normalize_or_zero();
    let target = hoop - direction * (4.0 + world.rng.gen::<f32>() * 4.0);
    world.players[actor].pos = target;
    world.players[actor].target = target;
    world.possession.drives += 1;
    world.possession.last_post_at = Some(world.now());
    world.possession.last_post_actor = Some(actor);
    world.emit(Fact::PostUp { player: actor });
    ActionOutcome {
        actor,
        kind: ActionKind::PostUp,
        completion: Completion::Completed,
        delta: StateDelta {
            separation_ft: Some(1.0),
            ..Default::default()
        },
        facts: Vec::new(),
    }
}

pub fn drive_target(hoop: Vec2, pos: Vec2) -> Vec2 {
    let direction = (hoop - pos).normalize_or_zero();
    hoop - direction * 2.0
}
