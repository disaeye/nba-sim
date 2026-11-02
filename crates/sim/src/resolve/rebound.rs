//! 篮板：投失后球保持活球，卡位、落点与控球都由位置和接触决定。

use glam::Vec2;

use crate::action::{Fact, TeamSide, TurnoverKind};
use crate::state::{PossessionKind, TargetKind};
use crate::world::{Phase, World};

#[derive(Debug, Clone, Copy)]
pub struct PendingRebound {
    pub offense: TeamSide,
    pub zone: &'static str,
    pub shot_distance: f32,
    pub rim_contact: bool,
    pub started_at: f64,
    /// 松球来自失误而非投失：没有篮板统计与卡位流程。
    pub turnover: bool,
    /// 这次失误来自切球时，记录（拍球人，被拍人），抢回球权时据此统计。
    pub poke_by: Option<(usize, usize)>,
    /// 这次松球来自一次没接住的传球时，记录传球人；球落到对方手里才
    /// 记失误与抢断，回到自己人手里则进攻继续。
    pub pass_by: Option<usize>,
}

pub fn resolve(world: &mut World, zone: &'static str, shot_distance: f32, rim_contact: bool) {
    // 回合已经结束（节末、终场）：这次投失不再产生活球篮板。
    if world.possession.id == 0 {
        return;
    }
    world.rebound_attempts += 1;
    if rim_contact {
        world.clock.set_shot_clock_at_least(14.0);
    }
    let offense = world.possession.team;
    let ball_pos = world.ball.pos;
    // 所有场上球员都向球移动；防守人先占篮下卡位，随后再抢球。
    for handle in 0..world.players.len() {
        if !world.players[handle].on_court {
            continue;
        }
        let distance = (world.players[handle].pos - ball_pos).length();
        world.players[handle].target = ball_pos;
        world.players[handle].target_kind = TargetKind::Window;
        world.players[handle].target_partner = handle;
        // 跳起封盖的人还在空中：他没有落脚点去卡位，这一球他不参与 box-out。
        let airborne = world.defensive_actions[handle]
            .as_ref()
            .is_some_and(|action| action.kind == crate::action::DefensiveActionKind::BlockJump);
        if world.players[handle].team != offense && distance < 12.0 && !airborne {
            world.emit(Fact::BoxOut {
                player: handle,
                distance_ft: distance,
            });
            world.players[handle].defense_responsibility =
                crate::state::DefenseResponsibility::BoxOut;
            world.players[handle].assignment_since = world.now();
        }
    }
    // 真实篮板机会：场上人数与当前球位置、控球半径都满足的人数。
    // 这项记录与控球结算使用同一套半径，避免审计与执行各说一套。
    let base = world.params.policy.w("loose_ball_catch_base_ft");
    let skill = world.params.policy.w("loose_ball_catch_skill_ft");
    let mut players = 0u8;
    let mut contestable = 0u8;
    for player in world.players.iter().filter(|player| player.on_court) {
        players = players.saturating_add(1);
        let attribute = if player.team == offense {
            "offensive_rebound"
        } else {
            "defensive_rebound"
        };
        let reach = base
            + player.attribute(attribute) * skill
            + if player.team == offense { -1.15 } else { 1.75 };
        let vertical = (world.ball.z - 4.0).max(0.0);
        let distance = (player.pos - world.ball.pos).length().hypot(vertical);
        if distance <= reach {
            contestable = contestable.saturating_add(1);
        }
    }
    world.emit(Fact::ReboundOpportunity {
        players,
        contestable,
    });
    world.pending_rebound = Some(PendingRebound {
        offense,
        zone,
        shot_distance,
        rim_contact,
        started_at: world.now(),
        turnover: false,
        poke_by: None,
        pass_by: None,
    });
    world.set_phase(Phase::Live);
}

/// 失误造成的松球：球权已经转给接球方，谁先控制谁开始进攻。
pub fn resolve_turnover(world: &mut World) {
    // 回合已经结束：松球不建立活球争夺。
    if world.possession.id == 0 {
        return;
    }
    let ball_pos = world.ball.pos;
    for handle in 0..world.players.len() {
        if !world.players[handle].on_court {
            continue;
        }
        world.players[handle].target = ball_pos;
        world.players[handle].target_kind = TargetKind::Window;
        world.players[handle].target_partner = handle;
    }
    world.pending_rebound = Some(PendingRebound {
        offense: world.possession.team,
        zone: "turnover",
        shot_distance: 0.0,
        rim_contact: false,
        started_at: world.now(),
        turnover: true,
        poke_by: None,
        pass_by: None,
    });
    world.set_phase(Phase::Live);
}

/// 球员接管活球：持球、进入推进段。
///
/// 调度和推进目标由 `World::begin_live_push` 统一写入：活球回合要先把球
/// 推进到前场再读场面，推进入手位置由实际控球的人决定。
fn take_live_ball(world: &mut World, handle: usize) {
    world.set_ball_held(handle);
    world.possession.ball_phase = crate::state::BallPhase::Dribble;
    world.begin_live_push(handle);
}

/// 球被场上球员控制：控制权先由球与球员的接触决定，再结算前场/后场篮板。
pub fn control(world: &mut World, handle: usize) {
    let Some(state) = world.pending_rebound.take() else {
        return;
    };
    assert_ne!(
        world.possession.id, 0,
        "a rebound control requires an active possession"
    );
    if state.turnover {
        let team = world.players[handle].team;
        world.emit(Fact::LooseBallRecovered {
            player: handle,
            kind: if state.pass_by.is_some() {
                "pass"
            } else {
                "turnover"
            },
        });
        if team != world.possession.team {
            let charge = state
                .poke_by
                .map(|(_, victim)| (victim, TurnoverKind::LostBall))
                .or_else(|| state.pass_by.map(|passer| (passer, TurnoverKind::BadPass)));
            if let Some((victim, kind)) = charge {
                world.line_mut(handle).steals += 1;
                world.line_mut(victim).turnovers += 1;
                world.emit(Fact::Steal {
                    player: handle,
                    victim,
                });
                world.emit(Fact::Turnover {
                    player: victim,
                    kind,
                });
            }
            if state.pass_by.is_some() {
                world.end_possession(
                    nba_domain::event::PossessionEndCause::TurnoverPassDropped,
                    "pass_incomplete",
                    0,
                );
            } else {
                world.end_possession(
                    nba_domain::event::PossessionEndCause::TurnoverLooseBall,
                    "loose_ball_recovered",
                    0,
                );
            }
            world.start_possession(team, PossessionKind::Live);
        }
        take_live_ball(world, handle);
        return;
    }
    let offensive = world.players[handle].team == state.offense;
    let defense = state.offense.other();
    let distance_ft = (world.ball.pos - world.hoop_for(state.offense)).length();
    world.emit(Fact::Rebound {
        player: handle,
        offensive,
        distance_ft,
    });
    if offensive {
        world.line_mut(handle).oreb += 1;
        world.possession.had_offensive_rebound = true;
        world.start_second_chance(state.offense, handle, state.rim_contact);
    } else {
        world.line_mut(handle).dreb += 1;
        world.end_possession(
            nba_domain::event::PossessionEndCause::DefensiveRebound,
            "defensive_rebound",
            0,
        );
        world.start_possession(defense, PossessionKind::Live);
        take_live_ball(world, handle);
    }
}

pub fn rebound_spot(world: &World) -> Vec2 {
    let ball = &world.ball;
    let lead = 0.45;
    let mut spot = ball.pos + ball.vel * lead;
    if ball.z + ball.vz * lead - 0.5 * world.rules.ball_gravity_ftps2 * lead * lead
        <= world.rules.ball_radius_ft
    {
        spot = ball.pos + ball.vel * lead;
    }
    spot.clamp(
        Vec2::new(world.rules.ball_radius_ft, world.rules.ball_radius_ft),
        Vec2::new(
            world.rules.court.width_ft - world.rules.ball_radius_ft,
            world.rules.court.height_ft - world.rules.ball_radius_ft,
        ),
    )
}
