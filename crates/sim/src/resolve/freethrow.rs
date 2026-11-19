//! 罚球：球体飞行穿过筐口才计分，出手时机与瞄准误差由技能和策略参数决定。

use glam::Vec2;

use crate::action::{ActionKind, Fact, ShotKind};
use crate::draw::{purpose, Stream};
use crate::resolve::shot::{PendingShot, ShotResult};
use crate::state::BallState;
use crate::world::World;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FreeThrowFollowUp {
    Attempts {
        shooter: usize,
        total: u8,
        remaining: u8,
        made: u8,
        and_one: bool,
        retain_possession: bool,
        foul_id: u64,
    },
    DeadBall {
        team: crate::action::TeamSide,
    },
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct FreeThrowMeta {
    pub total: u8,
    pub remaining: u8,
    pub made: u8,
    pub and_one: bool,
    pub retain_possession: bool,
    pub foul_id: u64,
    pub technical_retained_team: Option<crate::action::TeamSide>,
    pub after_technical: Option<FreeThrowFollowUp>,
}

pub fn shoot(
    world: &mut World,
    shooter: usize,
    total: u8,
    remaining: u8,
    made_so_far: u8,
    and_one: bool,
    retain_possession: bool,
    foul_id: u64,
    technical_retained_team: Option<crate::action::TeamSide>,
    after_technical: Option<FreeThrowFollowUp>,
) {
    execute_free_throw(
        world,
        shooter,
        total,
        remaining,
        made_so_far,
        and_one,
        retain_possession,
        foul_id,
        technical_retained_team,
        after_technical,
    );
}

fn execute_free_throw(
    world: &mut World,
    shooter: usize,
    total: u8,
    remaining: u8,
    made_so_far: u8,
    and_one: bool,
    retain_possession: bool,
    foul_id: u64,
    technical_retained_team: Option<crate::action::TeamSide>,
    after_technical: Option<FreeThrowFollowUp>,
) {
    let hoop = world.hoop_for(world.players[shooter].team);
    let pos = world.players[shooter].pos;
    world.players[shooter].facing = (hoop - pos).normalize_or_zero();
    let model = crate::shot_quality::free_throw_model(world, shooter);
    let u1 = world
        .draw(Stream::Aim, shooter, purpose::SHOT_MAKE, 0)
        .max(1e-6);
    let u2 = world.draw(Stream::Aim, shooter, purpose::SHOT_MAKE, 1);
    let spread = model.sigma_ft * (-2.0 * u1.ln()).sqrt();
    let angle = u2 * std::f32::consts::TAU;
    let aim_offset = Vec2::new(angle.cos(), angle.sin()) * spread;
    let shot_id = world.new_shot_id();
    let meta = FreeThrowMeta {
        total,
        remaining,
        made: made_so_far,
        and_one,
        retain_possession,
        foul_id,
        technical_retained_team,
        after_technical,
    };
    world.pending_shot = Some(PendingShot {
        shooter,
        // 罚球不走出手统计：它的类型是罚球，统计口径在罚球事实里。
        kind: ShotKind::CatchShoot,
        zone: "free_throw",
        distance_ft: (pos - hoop).length(),
        contest: 0.0,
        fouled: false,
        and_one,
        free_throws: 0,
        points: 1,
        is_three: false,
        assisted_by: None,
        shot_id,
        free_throw: Some(meta),
        block_by: None,
        result: None,
        touched_backboard: false,
        released_game_clock: world.clock.game_clock(),
        // 罚球不适用节末压哨复核：没有检查点需要恢复。
        released_clock_checkpoint: None,
        released_event_id: world.last_event_id,
        released_shot_clock: world.clock.shot_clock(),
    });
    let rules = world.rules.clone();
    // 罚球人先把球拿到手里，再出手：球态的归属变化必须先于飞行开始，
    // 否则「松球直接变成投篮飞行」这条迁移会被迁移表拒绝。
    world.set_ball_held(shooter);
    // 列位点在出手时失效：球离手之后是活球篮板争夺，槽位不再是约束。
    world.clear_free_throw_spots();
    world.free_throw_started = None;
    // 罚球是一次出手：罚球人有一个真实的动作运行时，因此渲染器能画
    // 出他在罚球线上出手，事件流里也有一次可查的动作生命周期。
    start_free_throw_action(world, shooter);
    world.ball.release_shot(crate::ball::ShotRelease {
        rules: &rules,
        now: world.now(),
        shot_id,
        shooter,
        hoop,
        release_z: model.release_z_ft,
        aperture_ft: model.effective_aperture_ft,
        rim_in_probability: model.rim_in_probability,
        origin: model.origin,
        plan: model.plan,
        aim_offset,
        context: crate::state::ShotContext::FreeThrow,
    });
    world.set_flight(
        crate::state::FlightKind::FreeThrow,
        world.now() + f64::from(rules.max_shot_duration_seconds) + 2.0,
    );
}

pub fn shoot_team_technical(
    world: &mut World,
    shooting_team: crate::action::TeamSide,
    retained_team: crate::action::TeamSide,
    foul_id: u64,
    after_technical: FreeThrowFollowUp,
) {
    let shooter = world
        .on_court(shooting_team)
        .as_slice()
        .iter()
        .copied()
        .max_by(|left, right| {
            world.players[*left]
                .attribute("free_throw")
                .total_cmp(&world.players[*right].attribute("free_throw"))
        })
        .expect("each team must have a player to shoot a technical free throw");
    prepare_free_throw(
        world,
        shooter,
        1,
        1,
        0,
        false,
        true,
        foul_id,
        Some(retained_team),
        Some(after_technical),
        false,
    );
}

fn prepare_free_throw(
    world: &mut World,
    shooter: usize,
    total: u8,
    remaining: u8,
    made: u8,
    and_one: bool,
    retain_possession: bool,
    foul_id: u64,
    technical_retained_team: Option<crate::action::TeamSide>,
    after_technical: Option<FreeThrowFollowUp>,
    lane_positions: bool,
) {
    world.cancel_actions("free_throw_staged");
    let team = world.players[shooter].team;
    let attacking_right = world.attacking_right(team);
    let shooter_spot = nba_domain::court::Court::free_throw_pos_with_geometry(
        attacking_right,
        world.rules.free_throw_distance_ft,
        world.rules.court,
    );
    world.players[shooter].target = shooter_spot;
    let walk_ft = (world.players[shooter].pos - shooter_spot).length();
    let walk_seconds = f64::from(walk_ft / world.rules.max_player_speed_ftps.max(0.1)) * 1.6;
    let delay = f64::from(world.rules.free_throw_interval_seconds()).max(walk_seconds + 0.5);
    world.set_phase(crate::state::Phase::FreeThrows {
        shooter,
        total,
        remaining,
        made,
        next_at: world.now() + delay,
        and_one,
        retain_possession,
        foul_id,
        technical_retained_team,
        after_technical,
    });
    world.free_throw_started = Some(world.now());
    world.free_throw_lane = Some(crate::world::FreeThrowLane::default());
    world.set_ball_held(shooter);
    world.stage_free_throw_procedure(shooter, lane_positions);
}

/// 罚球出手的动作运行时。
///
/// 罚球程序里的出手有自己的动作身份：它是跳投族里的一种（罚球），
/// 阶段是准备、执行与收势三段。
fn start_free_throw_action(world: &mut World, shooter: usize) {
    world.next_action_id += 1;
    let action_id = world.next_action_id;
    world.next_interaction_id += 1;
    let interaction_id = world.next_interaction_id;
    let now = world.now();
    let preparation = f64::from(world.rules.jump_shot_prep_seconds);
    let execution = f64::from(world.rules.jump_shot_exec_seconds);
    let follow_through = f64::from(world.rules.jump_shot_follow_seconds);
    world.players[shooter].action = Some(crate::action::ActionRuntime::new(
        action_id,
        interaction_id,
        crate::action::ActionIntent {
            actor: shooter,
            kind: ActionKind::FreeThrow,
            target: None,
            from_ft: world.players[shooter].pos,
            to_ft: world.hoop_for(world.players[shooter].team),
            start_time: now,
            duration: preparation + execution + follow_through,
        },
        shooter,
        crate::action::ActionIntentRole::Primary,
        nba_domain::action_window::ActionPhase::Preparation,
        preparation,
        execution,
        follow_through,
    ));
    world.emit(Fact::ActionStarted {
        action_id,
        interaction_id,
        player: shooter,
        role: crate::action::ActionIntentRole::Primary,
        kind: "FreeThrow",
        target: None,
    });
}

pub fn finish(world: &mut World, pending: PendingShot, meta: FreeThrowMeta) {
    let made = matches!(pending.result, Some(ShotResult::Made));
    let index = meta.total - meta.remaining + 1;
    {
        let line = world.line_mut(pending.shooter);
        line.fta += 1;
        if made {
            line.ftm += 1;
            line.points += 1;
        }
    }
    if made {
        world.add_points(world.players[pending.shooter].team, 1);
    }
    world.emit(Fact::FreeThrow {
        shooter: pending.shooter,
        made,
        index,
        total: meta.total,
        foul_id: meta.foul_id,
    });
    let made_count = meta.made + u8::from(made);
    if meta.remaining == 1 {
        if let Some(after_technical) = meta.after_technical {
            match after_technical {
                FreeThrowFollowUp::Attempts {
                    shooter,
                    total,
                    remaining,
                    made,
                    and_one,
                    retain_possession,
                    foul_id,
                } => {
                    // 主罚球程序接管：走裁决入口重新列位（罚球人可能换人），
                    // 阶段载荷由 adjudicate 统一写入。
                    prepare_free_throw(
                        world,
                        shooter,
                        total,
                        remaining,
                        made,
                        and_one,
                        retain_possession,
                        foul_id,
                        None,
                        None,
                        true,
                    );
                }
                FreeThrowFollowUp::DeadBall { team } => {
                    world.adjudicate(crate::adjudicate::Adjudication::new(
                        crate::adjudicate::StoppageCause::FreeThrow,
                        crate::adjudicate::RestartPlan::Stoppage {
                            team,
                            ready_after: 1.2,
                        },
                    ));
                    world.ball.release(BallState::Loose {
                        pos: world.hoop_for(team),
                        vel: Vec2::ZERO,
                    });
                }
            }
            return;
        }
        let team = meta
            .technical_retained_team
            .unwrap_or_else(|| world.players[pending.shooter].team);
        let result = pending
            .result
            .expect("free throw flight must resolve before settlement");
        let rim_contact = matches!(result, ShotResult::Missed { rim_contact: true });
        // 普通罚球序列的最后一罚必须命中或碰筐。罚球未碰筐是违例；
        // 球未进入活球，球权交给对方在罚球线延长线发球。
        if !made
            && !rim_contact
            && !meta.retain_possession
            && meta.technical_retained_team.is_none()
        {
            world.clear_free_throw_spots();
            world.possession.shot_clock_start = world.clock.shot_clock();
            world.line_mut(pending.shooter).turnovers += 1;
            world.emit(Fact::Turnover {
                player: pending.shooter,
                kind: crate::action::TurnoverKind::FreeThrowMissNoRim,
            });
            let attacking_right = world.attacking_right(team);
            let spot = world
                .rules
                .court
                .sideline_at_free_throw_line_extended(world.ball.pos, attacking_right);
            world.adjudicate(crate::adjudicate::Adjudication::violation(
                crate::adjudicate::ViolationKind::FreeThrowMissNoRim,
                crate::adjudicate::RestartPlan::Inbound {
                    team: team.other(),
                    location: crate::adjudicate::InboundLocation::Spot(spot),
                    ready_after: None,
                },
            ));
            world.release_ball(BallState::Loose {
                pos: world.ball.pos,
                vel: Vec2::ZERO,
            });
            return;
        }
        let eligible_rebound = !meta.retain_possession && rim_contact;
        if eligible_rebound {
            // 罚球禁区违例：球碰筐之前不得有人提前进区。
            //
            // 判定层给出事实（谁提前进了区），裁决层发布事实，调用方给出
            // 后果：进攻方提前进区判失误，防守方提前进区重罚一次。
            let lane_incident = crate::detect::detect_free_throw_lane_violation(world);
            if let Some(incident) = lane_incident {
                let offense_early = incident.offense_early;
                crate::adjudicate_incidents::adjudicate_one(world, Some(incident));
                if offense_early {
                    let enemy = team.other();
                    world.possession.shot_clock_start = world.clock.shot_clock();
                    world.adjudicate(
                        crate::adjudicate::Adjudication::new(
                            crate::adjudicate::StoppageCause::FreeThrow,
                            crate::adjudicate::RestartPlan::Stoppage {
                                team: enemy,
                                ready_after: 1.0,
                            },
                        )
                        .ending(
                            nba_domain::event::PossessionEndCause::TurnoverViolation,
                            "free_throw_lane_violation",
                        ),
                    );
                    world.release_ball(BallState::Loose {
                        pos: world.hoop_for(enemy),
                        vel: Vec2::ZERO,
                    });
                    return;
                }
                // 防守方提前进区：重罚一次并重新执行列位程序。
                prepare_free_throw(
                    world,
                    pending.shooter,
                    meta.total,
                    1,
                    made_count,
                    meta.and_one,
                    meta.retain_possession,
                    meta.foul_id,
                    meta.technical_retained_team,
                    None,
                    true,
                );
                return;
            }
            world.clear_free_throw_spots();
            world.possession.shot_clock_start = world.clock.shot_clock();
            world.release_ball(BallState::Loose {
                pos: world.ball.pos,
                vel: world.ball.vel,
            });
            crate::resolve::rebound::resolve(world, "free_throw", 15.0, rim_contact);
            return;
        }
        if !made && !rim_contact && !meta.retain_possession {
            // 没碰筐的罚球是死球违例；罚球方有后续罚球时继续该程序，
            // 序列已结束时交给对方在罚球线延长线发球。
            if meta.remaining > 1 {
                prepare_free_throw(
                    world,
                    pending.shooter,
                    meta.total,
                    meta.remaining - 1,
                    made_count,
                    meta.and_one,
                    meta.retain_possession,
                    meta.foul_id,
                    None,
                    meta.after_technical,
                    true,
                );
                return;
            }
            let spot = world
                .rules
                .court
                .sideline_at_free_throw_line_extended(world.ball.pos, world.attacking_right(team));
            world.clear_free_throw_spots();
            world.line_mut(pending.shooter).turnovers += 1;
            world.emit(Fact::Turnover {
                player: pending.shooter,
                kind: crate::action::TurnoverKind::FreeThrowMissNoRim,
            });
            world.adjudicate(crate::adjudicate::Adjudication::violation(
                crate::adjudicate::ViolationKind::FreeThrowMissNoRim,
                crate::adjudicate::RestartPlan::Inbound {
                    team: team.other(),
                    location: crate::adjudicate::InboundLocation::Spot(spot),
                    ready_after: None,
                },
            ));
            world.release_ball(BallState::Loose {
                pos: world.ball.pos,
                vel: Vec2::ZERO,
            });
            return;
        }
        world.possession.shot_clock_start = world.clock.shot_clock();
        world.free_throw_lane = None;
        world.clear_free_throw_spots();
        if meta.technical_retained_team.is_some() {
            match meta
                .after_technical
                .expect("technical free throw has a follow-up")
            {
                FreeThrowFollowUp::Attempts {
                    shooter,
                    total,
                    remaining,
                    made,
                    and_one,
                    retain_possession,
                    foul_id,
                } => {
                    prepare_free_throw(
                        world,
                        shooter,
                        total,
                        remaining,
                        made,
                        and_one,
                        retain_possession,
                        foul_id,
                        None,
                        None,
                        true,
                    );
                }
                FreeThrowFollowUp::DeadBall { team } => {
                    let spot = world
                        .inbound_spot
                        .unwrap_or_else(|| world.rules.court.hoop_pos(world.attacking_right(team)));
                    world.adjudicate(crate::adjudicate::Adjudication::new(
                        crate::adjudicate::StoppageCause::FreeThrow,
                        crate::adjudicate::RestartPlan::Inbound {
                            team,
                            location: crate::adjudicate::InboundLocation::Spot(spot),
                            ready_after: Some(1.2),
                        },
                    ));
                    world.release_ball(BallState::Loose {
                        pos: spot,
                        vel: Vec2::ZERO,
                    });
                }
            }
            return;
        }
        let restart_team = if meta.retain_possession {
            team
        } else {
            team.other()
        };
        let mut adjudication = crate::adjudicate::Adjudication::new(
            crate::adjudicate::StoppageCause::FreeThrow,
            crate::adjudicate::RestartPlan::Inbound {
                team: restart_team,
                location: crate::adjudicate::InboundLocation::OwnBaseline,
                ready_after: Some(1.2),
            },
        );
        if !meta.retain_possession && made_count > 0 {
            adjudication = adjudication
                .ending(nba_domain::event::PossessionEndCause::Score, "free_throw")
                .scoring(made_count as u32);
        }
        world.adjudicate(adjudication);
        world.ball.release(BallState::Loose {
            pos: world.hoop_for(team),
            vel: Vec2::ZERO,
        });
        return;
    }
    world.adjudicate(crate::adjudicate::Adjudication::new(
        crate::adjudicate::StoppageCause::FreeThrow,
        crate::adjudicate::RestartPlan::FreeThrows {
            shooter: pending.shooter,
            total: meta.total,
            remaining: meta.remaining - 1,
            made: made_count,
            and_one: meta.and_one,
            retain_possession: meta.retain_possession,
            foul_id: meta.foul_id,
            technical_retained_team: meta.technical_retained_team,
            after_technical: meta.after_technical,
        },
    ));
}
