//! 突破与背身结算：过掉首要防守人、到达位置、丢球。

use glam::Vec2;

use crate::action::{
    ActionIntent, ActionKind, ActionOutcome, Completion, Fact, StateDelta, TurnoverKind,
};
use crate::draw::{purpose, Stream};
use crate::world::World;

pub fn resolve(
    world: &mut World,
    intent: &ActionIntent,
    kind: nba_domain::action_window::DriveKind,
) -> ActionOutcome {
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
                kind: ActionKind::Drive(kind),
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

    // 变体不改落点：落点是突破意图给的那个点，动了它，出手的区域分布
    // 整段跟着变。变体的区别在「怎么过去」而不是「停在哪」：变向突破
    // 从防守人外侧绕过去，过掉他的机会更大；转身突破以轴心脚为轴，球
    // 护得更稳，丢球的可能更小。这些差别由决策层读变体算出来。
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
            // 防守人站住了：他占住了合法防守位置，撞人因此判进攻方犯规
            // （NBA Rule 12B §II.1-§II.2）。
            let planted = world.players[defender].vel.length() < 3.0;
            let strength = world.players[driver].data.attributes.strength;
            let rate = world.params.policy.w("charge_rate_drive") * (0.6 + strength);
            // 正在跑「站位吃犯规」这个动作的防守人，本来就是冲着吃这次
            // 犯规去的：撞人判犯规的机会因此放大。放大倍数在策略表里。
            //
            // 站位吃犯规不改「防守人有没有站住」：站住仍然只看他的速度。
            // 它改的是「撞上了算谁的」——防守人主动占住了位置，撞人因此
            // 判进攻方犯规，频率上升而不是下降。
            let taking_charge =
                crate::action::on_ball_positioning(world, defender)
                    == Some(crate::action::OnBallPositioning::ChargeTake);
            let rate = if taking_charge {
                rate * world.params.policy.w("charge_take_rate_multiplier")
            } else {
                rate
            };
            if planted && world.draw(Stream::Foul, driver, purpose::DRIVE_CHARGE, 0) < rate {
                crate::resolve::foul::apply_offensive_foul(world, driver, defender);
                return ActionOutcome {
                    actor: driver,
                    kind: ActionKind::Drive(kind),
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
                kind: ActionKind::Drive(kind),
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
        kind: kind.as_str(),
    });
    ActionOutcome {
        actor: driver,
        kind: ActionKind::Drive(kind),
        completion: Completion::Completed,
        delta: StateDelta {
            separation_ft: Some(separation_ft),
            ..Default::default()
        },
        facts: Vec::new(),
    }
}

/// 走步的账本与裁决：收球步的两条触发路径共用这里的写入。
///
/// 判定与罚则在 [`crate::footwork`] 里：那里按轴心脚与脚步接触判定，
/// 这里只负责把它记进账本并写裁决。
fn book_gather_travel(world: &mut World, actor: usize, team: crate::TeamSide) {
    crate::footwork::book_travel(world, actor, team);
}

pub fn resolve_finish(world: &mut World, intent: &ActionIntent) -> ActionOutcome {
    let actor = intent.actor;
    let team = world.players[actor].team;
    let hoop = world.hoop_for(team);
    let pos = world.players[actor].pos;
    let direction = (hoop - pos).normalize_or_zero();
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
    }
    // 收球确立轴心：球被双手控制的那一刻定下轴心脚，之后的两步从这个
    // 点算起。球手接下来从收球点走到终结点，那段位移由移动层记成脚步
    // 接触，走步在释放（出手或传球）那一刻结算。
    crate::footwork::establish_pivot(world, actor, pos);
    // 脚步接触能抓住系统性走步，剩下的一部分由判罚频率补上：真实比赛里
    // 走步多是脚步没踩实，模型里那是收球步本身的失误。
    if world.draw(Stream::Decision, actor, purpose::GATHER_TRAVEL, 0)
        < world.params.policy.w("gather_travel_rate")
    {
        book_gather_travel(world, actor, team);
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
    let base_distance =
        5.5 + world.draw(Stream::Aim, actor, purpose::POST_TARGET, 0) * 4.0 - target_distance;
    // 每种步法都有自己的空间效果，而空间效果作用在防守人身上：上下步把
    // 防守人顶到身后，撤步转身绕到底线一侧，背身挤打把人挤开。
    //
    // 效果落在防守人身上而不是进攻人的落点上，是因为落点一改，出手的
    // 区域分布整段跟着变：低位出手的位置是标定出来的，不能被步法的
    // 落点偏移带走。顶开防守人则改的是这次单打的结果分布，出手位置
    // 仍在原来的地方。
    let lateral = Vec2::new(-direction.y, direction.x);
    let push_ft = world.params.policy.g("post_step_push_ft");
    if let Some(defender) = defender {
        let pushed = match kind_name {
            // 上下步：往底线方向顶，防守人被挤到身后。
            "up_and_under" => {
                let baseline = if baseline_side { lateral } else { -lateral };
                baseline * push_ft
            }
            // 撤步转身：往底线一侧绕过去，防守人被挤到另一侧。
            "drop_step" => -lateral * push_ft,
            // 背身挤打：朝防守人的反方向挤。
            _ => {
                let away = (world.players[defender].pos - pos).normalize_or_zero();
                Vec2::new(-away.y, away.x) * push_ft
            }
        };
        let radius = world.rules.player_radius_ft;
        let spot = world.players[defender].pos + pushed;
        let clamped = glam::Vec2::new(
            spot.x.clamp(radius, world.rules.court.width_ft - radius),
            spot.y.clamp(radius, world.rules.court.height_ft - radius),
        );
        crate::assignment::force_target_as(
            world,
            defender,
            clamped,
            crate::state::TargetKind::Station,
            actor,
        );
    }
    let target = hoop - direction * base_distance;
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
