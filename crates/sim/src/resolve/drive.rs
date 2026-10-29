//! 突破与背身结算：过掉首要防守人、到达位置、丢球。

use glam::Vec2;

use crate::action::{
    ActionIntent, ActionKind, ActionOutcome, Completion, Fact, StateDelta, TurnoverKind,
};
use crate::draw::{purpose, Stream};
use crate::state::PossessionKind;
use crate::world::World;

pub fn resolve(world: &mut World, intent: &ActionIntent) -> ActionOutcome {
    let driver = intent.actor;
    let team = world.players[driver].team;
    let hoop = world.hoop_for(team);
    let pos = world.players[driver].pos;
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

    // 失位前后的手部犯规：防守人用犯规阻止突破。对抗与协防倾向高的
    // 防守人更容易伸手；未到 bonus 时只记犯规，到 bonus 时给罚球。
    if let Some(defender) = nearest {
        let physicality = world.players[defender].data.tendencies.physicality;
        let help = world.players[defender].data.tendencies.help_aggressiveness;
        let policy = &world.params.policy;
        let rate = policy.w("reach_in_foul_rate")
            * (policy.w("reach_in_physicality_base") + physicality)
            * (policy.w("reach_in_help_base") + help);
        if world.draw(Stream::Foul, defender, purpose::REACH_IN_FOUL, 0) < rate {
            let fouling_team = world.players[defender].team;
            let free_throws = crate::resolve::foul::bonus_free_throws(world, fouling_team);
            crate::resolve::foul::apply_non_shooting_foul(world, defender, driver);
            if free_throws == 0 {
                // 非 bonus：死球后进攻方继续本次回合，只是把下一次决策
                // 推后；不重建回合，否则每吹一次犯规就多算一个回合。
                world.players[driver].schedule.next_redecide_at =
                    world.now() + world.params.policy.g("reach_in_dead_ball_seconds") as f64;
                world.players[driver].schedule.pending =
                    Some(crate::state::DecisionTrigger::ActionComplete);
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
    let travelled = (pos - intent.from_ft).length();
    let max_travel = world.rules.max_player_speed_ftps * intent.duration as f32;
    if travelled > max_travel * 1.08 {
        world.violations.push(format!(
            "geometry: drive covered {travelled:.1} ft in {:.2} s (limit {max_travel:.1} ft)",
            intent.duration
        ));
    }

    let separation_ft = nearest
        .map(|defender| (world.players[defender].pos - pos).length())
        .unwrap_or(30.0);
    let distance_ft = (pos - hoop).length();
    let reached = distance_ft <= world.params.policy.g("finish_zone_ft");
    let defender_beaten = separation_ft > world.rules.min_player_separation_ft + 1.0;
    if !defender_beaten && separation_ft <= world.rules.min_player_separation_ft + 0.5 {
        let handle_skill = world.players[driver].data.attributes.ball_handling;
        let steal_skill = nearest
            .map(|d| world.players[d].data.attributes.defense_perimeter)
            .unwrap_or(0.5);
        let lost_rate = (0.12 + 0.25 * (steal_skill - handle_skill)).clamp(0.03, 0.20);
        if world.draw(Stream::Decision, driver, purpose::DRIVE_BEAT, 0) < lost_rate {
            world.line_mut(driver).turnovers += 1;
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
    }
    world.possession.gather_pos = Some(pos);
    world.possession.dribble_state = crate::state::DribbleState::Dribbling;
    world.possession.ball_phase = crate::state::BallPhase::Drive;
    world.possession.drives += 1;
    world.possession.last_drive_at = Some(world.now());
    world.possession.last_drive_actor = Some(driver);
    world.emit(Fact::Drive {
        driver,
        reached,
        defender_beaten,
        distance_ft,
        separation_ft,
    });
    ActionOutcome {
        actor: driver,
        kind: ActionKind::Drive,
        completion: Completion::Completed,
        delta: StateDelta {
            separation_ft: Some(separation_ft),
            ..Default::default()
        },
        facts: Vec::new(),
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
    let gather = world.possession.gather_pos.unwrap_or(intent.from_ft);
    // 收球步的终结位置：突破上篮进入篮下完成，抛投在近筐区域完成。
    let min_distance = world.params.policy.g("finish_min_distance_ft");
    let is_floater = world.draw(Stream::Aim, actor, purpose::FINISH_STEP, 1) < 0.56;
    let finish_dist = if is_floater {
        5.2 + world.draw(Stream::Aim, actor, purpose::FINISH_STEP, 0) * 5.2
    } else {
        min_distance + world.draw(Stream::Aim, actor, purpose::FINISH_STEP, 0) * 0.8
    };
    let finish = hoop - direction * finish_dist;
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
    world.players[actor].target = finish;
    world.players[actor].target_kind = crate::state::TargetKind::Window;
    world.players[actor].target_partner = actor;
    if let Some(slot) = world.possession.matchup.iter().position(|man| *man == actor) {
        let defense = world.possession.team.other();
        let defenders = world.on_court(defense);
        if let Some(defender) = defenders.as_slice().get(slot) {
            let pressure = crate::assignment::pressure_separation(world);
            world.players[*defender].target = finish + direction * (pressure + 0.3);
        }
    }
    world.possession.dribble_state = crate::state::DribbleState::Gathered;
    world.possession.gather_pos = Some(pos);
    world.possession.ball_phase = crate::state::BallPhase::Gather;
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
    let defender = world
        .players
        .iter()
        .enumerate()
        .filter(|(_, player)| player.on_court && player.team != team)
        .min_by(|left, right| {
            (left.1.pos - pos)
                .length_squared()
                .total_cmp(&(right.1.pos - pos).length_squared())
        })
        .map(|(handle, _)| handle);
    let target_distance = defender.map_or(0.0, |handle| {
        let offense_strength = world.players[actor].data.attributes.strength;
        let defense = &world.players[handle].data.attributes;
        let defense_resistance = nba_domain::capability::effective_post_defense_physicality(
            &world.rules,
            defense,
        );
        let offense_pressure = offense_strength * world.rules.capability.post_defense_secondary_gain;
        (offense_pressure - defense_resistance).clamp(-1.5, 1.5)
    });
    let target = hoop
        - direction
            * (5.5 + world.draw(Stream::Aim, actor, purpose::POST_TARGET, 0) * 4.0 - target_distance);
    world.players[actor].target = target;
    world.players[actor].target_kind = crate::state::TargetKind::Window;
    world.players[actor].target_partner = actor;
    world.possession.drives += 1;
    world.possession.last_post_at = Some(world.now());
    world.possession.last_post_actor = Some(actor);
    world.possession.ball_phase = crate::state::BallPhase::Post;
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
