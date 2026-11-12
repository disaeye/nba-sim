//! 裁决层：把原始事件按联赛档案变成判罚后果。
//!
//! 裁决读 [`RawIncident`] 与权威状态，产出裁决批次。批次里的每一条后果
//! 都经统一提交入口落下，因此球状态、控制、程序、时钟、罚则与事件一起
//! 生效。
//!
//! 同一份原始事件在 NBA 与 FIBA 下得到不同的后果：FIBA 把破坏性犯规
//! 单独成类，NBA 把它归入恶意犯规；FIBA 没有守方三秒与背身五秒。

use crate::incident::{AdjudicationBatch, IncidentKind, PenaltyAfter, PenaltyEntry, RawIncident};
use crate::world::World;

/// 裁决一批原始事件并提交后果。
///
/// 批次按事件顺序处理：同一时刻的多条事件按规则定义的先后提交，因此
/// 事件集合的顺序不依赖容器遍历顺序。
pub fn adjudicate(world: &mut World, incidents: &[RawIncident]) -> Option<AdjudicationBatch> {
    if incidents.is_empty() {
        return None;
    }
    let mut batch = AdjudicationBatch::new(world.now());
    for incident in incidents {
        // 检出计数是审计账：它只从裁决入口写入，因此计数为正就说明这个
        // 类别的判定函数在主循环里被调用过。
        world.record_incident_kind(incident.kind);
        batch = batch.with_incident(incident.clone());
        apply_incident(world, incident, &mut batch);
    }
    Some(batch)
}

/// 裁决一条可选的原始事件。
///
/// 判定函数大多数时候不成立：它们返回 `None`。这个入口把“可能成立”
/// 变成“成立则裁决”，因此调用点不需要处理空值。
pub fn adjudicate_one(world: &mut World, incident: Option<RawIncident>) -> Option<AdjudicationBatch> {
    let incident = incident?;
    adjudicate(world, std::slice::from_ref(&incident))
}

/// 把一条原始事件的来源事件交给世界。
///
/// 裁决产生的事件要能说清它依据的是哪些事实：来源事件就是原始事件读到
/// 的那些接触与动作。
fn publish_incident_sources(world: &mut World, incident: &RawIncident) {
    let mut sources = incident.source_event_ids.clone();
    if sources.is_empty() {
        sources = incident
            .actor
            .map(|player| world.causal_events_for(player, 4))
            .unwrap_or_default();
    }
    if !sources.is_empty() {
        world.pending_review_source_events = Some(sources);
    }
}

/// 事件来源身份：原始事件依据的事实。
///
/// 来源为空时回退到责任方最近的因果事件，再回退到当前事件流末尾：复核
/// 结论因此总能指回一组事实。
fn incident_sources(world: &World, incident: &RawIncident) -> Vec<u64> {
    let mut sources = incident.source_event_ids.clone();
    if sources.is_empty() {
        sources = incident
            .actor
            .map(|player| world.causal_events_for(player, 4))
            .unwrap_or_default();
    }
    if sources.is_empty() {
        sources.push(world.last_event_id);
    }
    sources
}

/// 违例的共同后果：失误、发球地点与球权。
///
/// 违例一律把球权交给对方，并由对方在中断点发球。发球地点按违例类别
/// 给出：背身五秒在罚球线延长线，八秒与回场在中断点就近边线。
fn apply_violation(
    world: &mut World,
    holder: usize,
    turnover: crate::action::TurnoverKind,
    violation: crate::adjudicate::ViolationKind,
    spot: glam::Vec2,
) {
    let team = world.players[holder].team;
    world.line_mut(holder).turnovers += 1;
    world.emit(crate::action::Fact::Turnover {
        player: holder,
        kind: turnover,
    });
    world.adjudicate(crate::adjudicate::Adjudication::violation(
        violation,
        crate::adjudicate::RestartPlan::Inbound {
            team: team.other(),
            location: crate::adjudicate::InboundLocation::Spot(spot),
            ready_after: None,
        },
    ));
}

/// 按一条原始事件给出后果。
fn apply_incident(world: &mut World, incident: &RawIncident, batch: &mut AdjudicationBatch) {
    // 判罚要求责任方与受影响方都还在场上：检测与裁决之间可能隔着一个
    // 换人窗口，被换下场的人不参与这次判罚。
    let parties_on_court = |world: &World| -> bool {
        [incident.actor, incident.subject]
            .into_iter()
            .flatten()
            .all(|handle| world.players[handle].on_court)
    };
    if incident.kind.foul_kind().is_some() && !parties_on_court(world) {
        return;
    }
    publish_incident_sources(world, incident);
    match incident.kind {
        IncidentKind::OutOfBounds => {
            // 出界：球权给最后触球方的对手。投篮出手后球无人控制地飞出
            // 界是投篮不中，不记失误；持球人或后场球员碰出界才是违例。
            let touched = incident.actor;
            let recipient = incident.team.unwrap_or(world.possession.team);
            let measurement = |name: &str| -> f32 {
                incident
                    .measurements
                    .iter()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| *value)
                    .unwrap_or(f32::default())
            };
            let violation = incident.offensive_touch;
            let shot_in_air = incident.shot_in_air;
            if violation && !shot_in_air {
                if let Some(handle) = touched {
                    world.line_mut(handle).turnovers += 1;
                    world.emit(crate::action::Fact::Turnover {
                        player: handle,
                        kind: crate::action::TurnoverKind::OutOfBounds,
                    });
                }
            }
            // 停表后果统一走裁决提交入口：结束回合、记录发球地点与恢复方式。
            // 这条裁定可复核：丢球一方在复核窗口内可以申请教练挑战。
            //
            // 出界登记两个复核问题：球权给谁（归属），以及最后触球是谁。
            // 最后触球是归属的依据，因此它才是可以被更正的那一个：归属随
            // 触球结论一起确定。归属本身由最后触球直接读出，没有认定误差。
            let touched_team = touched.map(|handle| world.players[handle].team);
            let sources = incident_sources(world, incident);
            let address = sources.first().copied().unwrap_or_default();
            let misjudged = world.judge_with_error(touched.unwrap_or_default(), address);
            world.record_ruling_with_observation(
                recipient,
                "last_touch",
                touched,
                touched_team.map_or(
                    crate::review_program::Observation::None,
                    crate::review_program::Observation::Touch,
                ),
                crate::review_program::ReviewCorrection::Possession,
                crate::review_program::ReviewBasis::Judgement { misjudged },
                crate::review_program::ReviewTrigger::CoachChallenge,
                sources,
            );
            let spot = glam::Vec2::new(measurement("spot_x"), measurement("spot_y"));
            world.adjudicate(crate::adjudicate::Adjudication::violation(
                crate::adjudicate::ViolationKind::OutOfBounds,
                crate::adjudicate::RestartPlan::Inbound {
                    team: recipient,
                    location: crate::adjudicate::InboundLocation::Spot(spot),
                    ready_after: None,
                },
            ));
        }
        IncidentKind::BackToBasket => {
            let Some(holder) = incident.actor else {
                return;
            };
            let team = world.players[holder].team;
            let attacking_right = world.attacking_right(team);
            let spot = world.rules.court.sideline_at_free_throw_line_extended(
                world.players[holder].pos,
                attacking_right,
            );
            apply_violation(
                world,
                holder,
                crate::action::TurnoverKind::BackToBasket,
                crate::adjudicate::ViolationKind::BackToBasket,
                spot,
            );
        }
        IncidentKind::Backcourt => {
            let Some(holder) = incident.actor else {
                return;
            };
            let spot = world.inbound_point_of_interruption();
            apply_violation(
                world,
                holder,
                crate::action::TurnoverKind::Backcourt,
                crate::adjudicate::ViolationKind::Backcourt,
                spot,
            );
        }
        IncidentKind::EightSecond => {
            let Some(holder) = incident.actor else {
                return;
            };
            let spot = world.inbound_point_of_interruption();
            apply_violation(
                world,
                holder,
                crate::action::TurnoverKind::EightSecond,
                crate::adjudicate::ViolationKind::EightSecond,
                spot,
            );
        }
        IncidentKind::CloselyGuarded => {
            let Some(holder) = incident.actor else {
                return;
            };
            let team = world.players[holder].team;
            world.line_mut(holder).turnovers += 1;
            world.emit(crate::action::Fact::Turnover {
                player: holder,
                kind: crate::action::TurnoverKind::CloselyGuarded,
            });
            world.adjudicate(crate::adjudicate::Adjudication::violation(
                crate::adjudicate::ViolationKind::CloselyGuarded,
                crate::adjudicate::RestartPlan::Live { team: team.other() },
            ));
        }
        IncidentKind::ShotClock => {
            let team = world.possession.team;
            let actor = world.holder().unwrap_or_else(|| world.primary_handler(team));
            world.line_mut(actor).turnovers += 1;
            world.emit(crate::action::Fact::Turnover {
                player: actor,
                kind: crate::action::TurnoverKind::ShotClock,
            });
            // 进攻钟违例：球权直接交给对方，比赛继续（NBA Rule 5 §II：
            // 进攻钟走完是活球中的球权转移，不是死球）。
            world.adjudicate(crate::adjudicate::Adjudication::violation(
                crate::adjudicate::ViolationKind::ShotClock,
                crate::adjudicate::RestartPlan::Live { team: team.other() },
            ));
        }
        IncidentKind::Travel => {
            let Some(actor) = incident.actor else {
                return;
            };
            let team = world.players[actor].team;
            world.line_mut(actor).turnovers += 1;
            world.emit(crate::action::Fact::Turnover {
                player: actor,
                kind: crate::action::TurnoverKind::Travel,
            });
            world.adjudicate(
                crate::adjudicate::Adjudication::new(
                    crate::adjudicate::StoppageCause::Violation(
                        crate::adjudicate::ViolationKind::Travel,
                    ),
                    crate::adjudicate::RestartPlan::Live { team: team.other() },
                )
                .ending(
                    nba_domain::event::PossessionEndCause::TurnoverTravel,
                    "travel",
                ),
            );
        }
        IncidentKind::DoubleDribble => {
            let Some(actor) = incident.actor else {
                return;
            };
            let team = world.players[actor].team;
            world.line_mut(actor).turnovers += 1;
            world.emit(crate::action::Fact::Turnover {
                player: actor,
                kind: crate::action::TurnoverKind::DoubleDribble,
            });
            world.adjudicate(crate::adjudicate::Adjudication::violation(
                crate::adjudicate::ViolationKind::DoubleDribble,
                crate::adjudicate::RestartPlan::Live { team: team.other() },
            ));
        }
        IncidentKind::OffensiveThreeSecond => {
            let Some(actor) = incident.actor else {
                return;
            };
            let team = world.players[actor].team;
            let opponent = team.other();
            world.line_mut(actor).turnovers += 1;
            world.emit(crate::action::Fact::Turnover {
                player: actor,
                kind: crate::action::TurnoverKind::LaneViolation,
            });
            let spot = world.rules.court.sideline_at_free_throw_line_extended(
                world.ball.pos,
                world.attacking_right(team),
            );
            world.adjudicate(
                crate::adjudicate::Adjudication::violation(
                    crate::adjudicate::ViolationKind::OffensiveThreeSecond,
                    crate::adjudicate::RestartPlan::Inbound {
                        team: opponent,
                        location: crate::adjudicate::InboundLocation::Spot(spot),
                        ready_after: None,
                    },
                )
                .ending(
                    nba_domain::event::PossessionEndCause::TurnoverViolation,
                    "offensive_three_second",
                ),
            );
        }
        IncidentKind::DefensiveThreeSecond => {
            // 守方三秒判技术犯规：对手罚一次并保留球权
            // （NBA Rule 10 §VII.1）。
            let Some(actor) = incident.actor else {
                return;
            };
            crate::resolve::foul::apply_defensive_three_second(world, actor);
        }
        IncidentKind::InboundTimeout => {
            let team = incident.team.unwrap_or(world.possession.team);
            let opponent = team.other();
            world.adjudicate(
                crate::adjudicate::Adjudication::new(
                    crate::adjudicate::StoppageCause::Violation(
                        crate::adjudicate::ViolationKind::InboundTimeout,
                    ),
                    crate::adjudicate::RestartPlan::Inbound {
                        team: opponent,
                        location: crate::adjudicate::InboundLocation::OwnBaseline,
                        ready_after: None,
                    },
                )
                .ending(
                    nba_domain::event::PossessionEndCause::TurnoverViolation,
                    "inbound_timeout",
                ),
            );
        }
        IncidentKind::FreeThrowTimeout => {
            let team = incident.team.unwrap_or(world.possession.team);
            let opponent = team.other();
            world.adjudicate(
                crate::adjudicate::Adjudication::new(
                    crate::adjudicate::StoppageCause::Violation(
                        crate::adjudicate::ViolationKind::FreeThrowTimeout,
                    ),
                    crate::adjudicate::RestartPlan::Inbound {
                        team: opponent,
                        location: crate::adjudicate::InboundLocation::OwnBaseline,
                        ready_after: None,
                    },
                )
                .ending(
                    nba_domain::event::PossessionEndCause::TurnoverViolation,
                    "free_throw_timeout",
                ),
            );
        }
        IncidentKind::FreeThrowLaneViolation => {
            // 罚球禁区违例的事实由这里发布：谁提前进了区写在事实里，
            // 后果由调用方（罚球结算）给出。
            let side = if incident.offense_early {
                "offense"
            } else {
                "defense"
            };
            world.emit(crate::action::Fact::FreeThrowLaneViolation { side });
        }
        IncidentKind::PersonalFoul => {
            let (Some(fouler), Some(fouled)) = (incident.actor, incident.subject) else {
                return;
            };
            crate::resolve::foul::apply_non_shooting_foul(world, fouler, fouled);
        }
        IncidentKind::ShootingFoul => {
            let (Some(fouler), Some(shooter)) = (incident.actor, incident.subject) else {
                return;
            };
            crate::resolve::foul::apply_shooting_foul(
                world,
                fouler,
                shooter,
                incident.free_throws,
                incident.made,
            );
        }
        IncidentKind::OffensiveFoul => {
            let (Some(fouler), Some(fouled)) = (incident.actor, incident.subject) else {
                return;
            };
            crate::resolve::foul::apply_offensive_foul(world, fouler, fouled);
        }
        IncidentKind::IllegalScreen => {
            let (Some(screener), Some(defender)) = (incident.actor, incident.subject) else {
                return;
            };
            crate::resolve::foul::apply_offensive_foul(world, screener, defender);
        }
        IncidentKind::DoubleFoul => {
            let (Some(fouler), Some(fouled)) = (incident.actor, incident.subject) else {
                return;
            };
            crate::resolve::foul::apply_double_foul(world, fouler, fouled);
        }
        IncidentKind::TechnicalFoul => {
            let Some(actor) = incident.actor else {
                return;
            };
            crate::resolve::foul::apply_technical_foul(world, actor);
        }
        IncidentKind::FlagrantFoul => {
            let (Some(fouler), Some(fouled)) = (incident.actor, incident.subject) else {
                return;
            };
            // 恶意犯规的两级罚则由原始事件给出：一级两罚，二级两罚加被逐。
            let excessive = incident.excessive;
            crate::resolve::foul::apply_flagrant_foul(world, fouler, fouled, excessive);
            // 犯规升级是一个复核问题：档案承认哪一类，就登记哪一类。
            if let Some(call) = crate::review_program::foul_upgrade_call(&world.rules.league, excessive)
            {
                let address = world.last_event_id;
                let misjudged = world.judge_with_error(fouler, address);
                world.record_ruling_with_observation(
                    world.players[fouled].team,
                    call,
                    Some(fouler),
                    crate::review_program::Observation::Contact,
                    crate::review_program::ReviewCorrection::FoulUpgrade,
                    crate::review_program::ReviewBasis::Judgement { misjudged },
                    crate::review_program::ReviewTrigger::CoachChallenge,
                    vec![address],
                );
            }
            let foul_id = world.last_foul_id();
            batch.penalties.push(PenaltyEntry {
                foul_id,
                kind: crate::action::FoulKind::Flagrant,
                free_throws: 2,
                shooter: Some(fouled),
                retain_possession: excessive,
                after: PenaltyAfter::ResumeGame,
            });
        }
        IncidentKind::ClearPathFoul => {
            let (Some(fouler), Some(fouled)) = (incident.actor, incident.subject) else {
                return;
            };
            // 清晰路径的归类由档案决定：FIBA 把它单独成类（破坏性犯规），
            // NBA 归入普通个人犯规。因此档案说「这一类单独成类」时用破坏性
            // 犯规的罚则，否则用普通个人犯规的罚则。
            if world.rules.league.disruptive_foul_is_separate() {
                crate::resolve::foul::apply_disruptive_foul(world, fouler, fouled);
                // 破坏性犯规是不体育道德犯规：FIBA 把它单独成类，复核问题
                // 也因此与 NBA 的恶意犯规升级不同。
                if world.rules.league.reviews("unsportsmanlike_upgrade") {
                    let address = world.last_event_id;
                    let misjudged = world.judge_with_error(fouler, address);
                    world.record_ruling_with_observation(
                        world.players[fouled].team,
                        "unsportsmanlike_upgrade",
                        Some(fouler),
                        crate::review_program::Observation::Contact,
                        crate::review_program::ReviewCorrection::FoulUpgrade,
                        crate::review_program::ReviewBasis::Judgement { misjudged },
                        crate::review_program::ReviewTrigger::CoachChallenge,
                        vec![address],
                    );
                }
            } else {
                crate::resolve::foul::apply_clear_path_foul(world, fouler, fouled);
            }
            let foul_id = world.last_foul_id();
            batch.penalties.push(PenaltyEntry {
                foul_id,
                kind: crate::action::FoulKind::LooseBall,
                free_throws: 2,
                shooter: Some(fouled),
                retain_possession: true,
                after: PenaltyAfter::ResumeGame,
            });
        }
        IncidentKind::TransitionTakeFoul => {
            let (Some(fouler), Some(fouled)) = (incident.actor, incident.subject) else {
                return;
            };
            // 转换阻断：判给进攻方罚球并保留球权（NBA Rule 12A §XIII）。
            crate::resolve::foul::apply_clear_path_foul(world, fouler, fouled);
            let foul_id = world.last_foul_id();
            batch.penalties.push(PenaltyEntry {
                foul_id,
                kind: crate::action::FoulKind::LooseBall,
                free_throws: 2,
                shooter: Some(fouled),
                retain_possession: true,
                after: PenaltyAfter::ResumeGame,
            });
        }
        IncidentKind::TeamTechnicalFoul => {
            // 球队技术犯规：责任主体是教练与替补席，因此不计入任何场上
            // 球员的个人犯规，也不计入球队单节犯规。对方罚一次罚球，罚完
            // 球权不变（NBA Rule 12A §II）。
            //
            // 罚球之后的死球重启必须保留球权方：技术犯规不改变球权，
            // 因此保留的是当时持球的那一方。
            let Some(team) = incident.team else {
                return;
            };
            let opponent = team.other();
            let retained = world.possession.team;
            let foul_id = world.new_foul_id();
            world.fouls[World::team_index(team)].technical += 1;
            world.emit(crate::action::Fact::TeamTechnicalFoul {
                team,
                reason: "coach_protest",
                foul_id,
            });
            crate::resolve::freethrow::shoot_team_technical(
                world,
                opponent,
                retained,
                foul_id,
                crate::resolve::freethrow::FreeThrowFollowUp::DeadBall { team: retained },
            );
            batch.penalties.push(PenaltyEntry {
                foul_id,
                kind: crate::action::FoulKind::Technical,
                free_throws: 1,
                shooter: None,
                retain_possession: true,
                after: PenaltyAfter::ResumeGame,
            });
        }
        IncidentKind::Injury => {
            let Some(struck) = incident.actor else {
                return;
            };
            // 伤病：被撞伤的球员不能继续上场，替补接替他。
            world.trigger_injury(struck, "contact_injury");
        }
    }
}
