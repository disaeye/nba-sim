//! 篮板：投失后球保持活球，卡位、落点与控球都由位置和接触决定。

use glam::Vec2;

use crate::action::{Fact, TeamSide};
use crate::state::{PossessionKind, TargetKind};
use crate::world::{Phase, World};

#[derive(Debug, Clone, Copy)]
pub struct PendingRebound {
    pub offense: TeamSide,
    pub zone: &'static str,
    pub shot_distance: f32,
    pub rim_contact: bool,
    pub started_at: f64,
}

pub fn resolve(world: &mut World, zone: &'static str, shot_distance: f32, rim_contact: bool) {
    world.rebound_attempts += 1;
    if rim_contact {
        world.clock.shot_clock = 14.0;
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
        if world.players[handle].team != offense && distance < 12.0 {
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
    });
    world.phase = Phase::Live;
    world.active = None;
}

/// 球被场上球员控制：控制权先由球与球员的接触决定，再结算前场/后场篮板。
pub fn control(world: &mut World, handle: usize) {
    let Some(state) = world.pending_rebound.take() else {
        return;
    };
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
        world.end_possession("defensive_rebound", 0);
        world.start_possession(defense, PossessionKind::Live);
        world.ball.set_held(handle);
        world.players[handle].schedule.next_redecide_at =
            world.now() + f64::from(world.rules.redecide_interval_seconds);
        world.players[handle].schedule.pending =
            Some(crate::state::DecisionTrigger::PossessionStart);
        world.players[handle].target = world.players[handle].pos;
        world.players[handle].target_kind = TargetKind::Station;
        world.players[handle].target_partner = usize::MAX;
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
        Vec2::new(
            world.rules.ball_radius_ft,
            world.rules.ball_radius_ft,
        ),
        Vec2::new(
            world.rules.court.width_ft - world.rules.ball_radius_ft,
            world.rules.court.height_ft - world.rules.ball_radius_ft,
        ),
    )
}
