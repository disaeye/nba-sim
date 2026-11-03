//! 罚球：球体飞行穿过筐口才计分，出手时机与瞄准误差由技能和策略参数决定。

use glam::Vec2;

use crate::action::{Fact, ShotKind};
use crate::draw::{purpose, Stream};
use crate::resolve::shot::{PendingShot, ShotResult};
use crate::state::{BallState, Phase};
use crate::world::World;

#[derive(Debug, Clone, Copy, PartialEq)]
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

#[derive(Debug, Clone, Copy)]
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
) {
    shoot_with_retained_team(
        world,
        shooter,
        total,
        remaining,
        made_so_far,
        and_one,
        retain_possession,
        foul_id,
        None,
        None,
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
    shoot_with_retained_team(
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
    );
}

fn shoot_with_retained_team(
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
    });
    let rules = world.rules.clone();
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
    });
    world.set_flight(
        crate::state::FlightKind::FreeThrow,
        world.now() + f64::from(rules.max_shot_duration_seconds) + 2.0,
    );
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
                } => shoot(
                    world,
                    shooter,
                    total,
                    remaining,
                    made,
                    and_one,
                    retain_possession,
                    foul_id,
                ),
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
        let eligible_rebound =
            !meta.retain_possession && matches!(result, ShotResult::Missed { rim_contact: true });
        if eligible_rebound {
            // 罚球禁区违例：球碰筐之前不得有人提前进区。
            let lane = world.free_throw_lane.take().unwrap_or_default();
            if lane.offense_early {
                world.emit(Fact::FreeThrowLaneViolation { side: "offense" });
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
            if lane.defense_early {
                // 防守方提前进区：重罚一次。
                world.emit(Fact::FreeThrowLaneViolation { side: "defense" });
                world.free_throw_lane = Some(crate::world::FreeThrowLane::default());
                world.adjudicate(crate::adjudicate::Adjudication::new(
                    crate::adjudicate::StoppageCause::FreeThrow,
                    crate::adjudicate::RestartPlan::FreeThrows {
                        shooter: pending.shooter,
                        total: meta.total,
                        remaining: 1,
                        made: made_count,
                        and_one: meta.and_one,
                        retain_possession: meta.retain_possession,
                        foul_id: meta.foul_id,
                        technical_retained_team: meta.technical_retained_team,
                        after_technical: None,
                    },
                ));
                return;
            }
            let rim_contact = matches!(result, ShotResult::Missed { rim_contact: true });
            world.possession.shot_clock_start = world.clock.shot_clock();
            world.release_ball(BallState::Loose {
                pos: world.ball.pos,
                vel: world.ball.vel,
            });
            crate::resolve::rebound::resolve(world, "free_throw", 15.0, rim_contact);
            return;
        }
        world.possession.shot_clock_start = world.clock.shot_clock();
        world.free_throw_lane = None;
        let restart_team = if meta.technical_retained_team.is_some() || meta.retain_possession {
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
        if meta.technical_retained_team.is_none() && !meta.retain_possession {
            adjudication = if made_count > 0 {
                adjudication
                    .ending(nba_domain::event::PossessionEndCause::Score, "free_throw")
                    .scoring(made_count as u32)
            } else {
                adjudication.ending(
                    nba_domain::event::PossessionEndCause::TurnoverViolation,
                    "free_throw_miss_without_rim_rebound",
                )
            };
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
