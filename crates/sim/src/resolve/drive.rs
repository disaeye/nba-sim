//! 突破与背身结算：过掉首要防守人、到达位置、丢球。

use glam::Vec2;

use crate::action::{
    ActionIntent, ActionKind, ActionOutcome, Completion, Fact, StateDelta, TurnoverKind,
};
use crate::draw::{purpose, Stream};
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
        let fouling_team = world.players[defender].team;
        let free_throws = crate::resolve::foul::bonus_free_throws(world, fouling_team);
        let policy = &world.params.policy;
        // 到 bonus 之后防守人收手：同样的对位，伸手犯规明显变少。
        let discipline = if free_throws > 0 {
            policy.w("reach_in_penalty_scale")
        } else {
            1.0
        };
        let rate = policy.w("reach_in_foul_rate")
            * (policy.w("reach_in_physicality_base") + physicality)
            * (policy.w("reach_in_help_base") + help)
            * discipline;
        if world.draw(Stream::Foul, defender, purpose::REACH_IN_FOUL, 0) < rate {
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
    // 带球撞人：防守人在接触前已经站定（双脚落地、不再滑步），碰撞
    // 由进攻方发起。裁判看的就是防守人有没有先占住位置，所以判定挂在
    // 防守人的速度上，而不是进攻人的速度。
    if separation_ft <= world.rules.min_player_separation_ft + 0.7 {
        if let Some(defender) = nearest {
            let planted = world.players[defender].vel.length() < 3.0;
            let strength = world.players[driver].data.attributes.strength;
            let rate = world.params.policy.w("charge_rate_drive") * (0.6 + strength);
            if planted && world.draw(Stream::Foul, driver, purpose::DRIVE_CHARGE, 0) < rate {
                crate::resolve::foul::apply_offensive_foul(world, driver, defender);
                return ActionOutcome {
                    actor: driver,
                    kind: ActionKind::Drive,
                    completion: Completion::Failed,
                    delta: StateDelta {
                        turnover: Some(TurnoverKind::OffensiveFoul),
                        ..Default::default()
                    },
                    facts: Vec::new(),
                };
            }
        }
    }
    if separation_ft <= world.rules.min_player_separation_ft + 2.0 {
        let handle_skill = world.players[driver].data.attributes.ball_handling;
        let steal_skill = nearest
            .map(|d| world.players[d].data.attributes.defense_perimeter)
            .unwrap_or(0.5);
        let lost_rate = (0.035 + 0.12 * (steal_skill - handle_skill)).clamp(0.01, 0.07);
        if world.draw(Stream::Decision, driver, purpose::DRIVE_BEAT, 0) < lost_rate {
            world.line_mut(driver).turnovers += 1;
            if let Some(thief) = nearest {
                world.line_mut(thief).steals += 1;
                world.emit(Fact::Steal {
                    player: thief,
                    victim: driver,
                });
            }
            world.emit(Fact::Turnover {
                player: driver,
                kind: TurnoverKind::LostBall,
            });
            world.adjudicate(
                crate::adjudicate::Adjudication::new(
                    crate::adjudicate::StoppageCause::PossessionChange,
                    crate::adjudicate::RestartPlan::Live { team: team.other() },
                )
                .ending(
                    nba_domain::event::PossessionEndCause::TurnoverSteal,
                    "lost_ball",
                ),
            );
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
    // 突破成功后球员留在落点。动作结束到下一次决策之间有半秒空档，
    // 如果 target 还是弧顶，球员会从篮下走回弧顶，下一次决策时已经在
    // 中距离，突破的出手全部落进近筐区，近筐比例整体偏高。
    world.players[driver].target = intent.to_ft;
    world.players[driver].target_kind = crate::state::TargetKind::Window;
    world.players[driver].target_partner = driver;
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
/// 走步的账本与裁决：两条触发路径共用这里的写入。
fn book_travel(world: &mut World, actor: usize, team: crate::TeamSide) {
    world.line_mut(actor).turnovers += 1;
    world.emit(Fact::Turnover {
        player: actor,
        kind: TurnoverKind::Travel,
    });
    world.adjudicate(
        crate::adjudicate::Adjudication::new(
            crate::adjudicate::StoppageCause::Violation(crate::adjudicate::ViolationKind::Travel),
            crate::adjudicate::RestartPlan::Live { team: team.other() },
        )
        .ending(
            nba_domain::event::PossessionEndCause::TurnoverTravel,
            "travel",
        ),
    );
}

pub fn resolve_finish(world: &mut World, intent: &ActionIntent) -> ActionOutcome {
    let actor = intent.actor;
    let team = world.players[actor].team;
    let hoop = world.hoop_for(team);
    let pos = world.players[actor].pos;
    let direction = (hoop - pos).normalize_or_zero();
    let mut allowance = world.params.policy.g("gather_travel_allowance_ft");
    let gather = world.possession.gather_pos.unwrap_or(intent.from_ft);
    // 收球步的终结位置由决策意图给出：突破上篮进入篮下完成，抛投在近筐
    // 区域完成，低位保持低位位置。
    let mut finish = intent.to_ft;
    // 欧洲步：防守人横在直线路上时，收球步要往侧面绕一步再起，
    // 绕的那一步是第二步，仍然是合法收球步。
    let blocker = world
        .players
        .iter()
        .filter(|player| player.on_court && player.team != team)
        .map(|player| {
            let segment = finish - pos;
            let length = segment.length_squared();
            let t = if length > f32::MIN_POSITIVE {
                ((player.pos - pos).dot(segment) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let closest = pos + segment * t;
            ((player.pos - closest).length(), t)
        })
        .filter(|(distance, t)| {
            *distance <= world.params.policy.g("euro_step_defender_ft") && (0.2..=0.8).contains(t)
        })
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .map(|(_, _)| ())
        .is_some();
    if blocker {
        let lateral = Vec2::new(-direction.y, direction.x);
        let side = world
            .players
            .iter()
            .filter(|player| player.on_court && player.team != team)
            .map(|player| lateral.dot(player.pos - pos))
            .fold(0.0f32, |acc, value| acc + value.signum());
        let away = if side >= 0.0 { -1.0 } else { 1.0 };
        finish += lateral * (away * world.params.policy.g("euro_step_lateral_ft"));
        allowance += world.params.policy.g("euro_step_allowance_ft");
    }
    if travel_on_drive(gather, finish, allowance) {
        book_travel(world, actor, team);
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
    // 走步是步数违规，与收球位移是同一件事的两种尺度：容限检查抓到
    // 迈得太远的那几次，剩下的由判罚频率补上。
    if world.draw(Stream::Decision, actor, purpose::GATHER_TRAVEL, 0)
        < world.params.policy.w("gather_travel_rate")
    {
        book_travel(world, actor, team);
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
    if let Some(slot) = world
        .possession
        .matchup
        .iter()
        .position(|man| *man == actor)
    {
        let defense = world.possession.team.other();
        let defenders = world.on_court(defense);
        if let Some(defender) = defenders.as_slice().get(slot) {
            let pressure = crate::assignment::pressure_separation(world);
            world.players[*defender].target = finish + direction * (pressure + 0.3);
        }
    }
    world.possession.dribble_state = crate::state::DribbleState::Gathered;
    world.possession.gather_pos = Some(pos);
    let was_post = world.possession.ball_phase == crate::state::BallPhase::Post
        || (world.possession.last_post_actor == Some(actor)
            && world
                .possession
                .last_post_at
                .map(|t| world.now() - t < 4.0)
                .unwrap_or(false));
    if was_post {
        world.possession.ball_phase = crate::state::BallPhase::Post;
        world.possession.last_post_at = Some(world.now());
        world.possession.last_post_actor = Some(actor);
    } else {
        world.possession.ball_phase = crate::state::BallPhase::Gather;
        world.possession.last_drive_at = Some(world.now());
        world.possession.last_drive_actor = Some(actor);
    }
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
        let defense_resistance =
            nba_domain::capability::effective_post_defense_physicality(&world.rules, defense);
        let offense_pressure =
            offense_strength * world.rules.capability.post_defense_secondary_gain;
        (offense_pressure - defense_resistance).clamp(-1.5, 1.5)
    });
    // 背身步法由防守人的站位决定：站定的防守人用上下步，靠底线的用撤步转身，
    // 其余情况用背身挤打。步法写入事实，落点仍由力量对抗与抽样决定。
    let planted = defender.is_some_and(|handle| world.players[handle].vel.length() < 3.0);
    let baseline_side = (pos.y - hoop.y).abs() > 6.0;
    let kind_name = if planted {
        "up_and_under"
    } else if baseline_side {
        "drop_step"
    } else {
        "backdown"
    };
    let target = hoop
        - direction
            * (5.5 + world.draw(Stream::Aim, actor, purpose::POST_TARGET, 0) * 4.0
                - target_distance);
    world.players[actor].target = target;
    world.players[actor].target_kind = crate::state::TargetKind::Window;
    world.players[actor].target_partner = actor;
    world.possession.drives += 1;
    world.possession.last_post_at = Some(world.now());
    world.possession.last_post_actor = Some(actor);
    world.possession.ball_phase = crate::state::BallPhase::Post;
    world.emit(Fact::PostUp {
        player: actor,
        kind: kind_name,
    });
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
