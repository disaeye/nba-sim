//! 篮板：投失后球保持活球，卡位、落点与控球都由位置和接触决定。

use glam::Vec2;

use crate::action::{Fact, TeamSide, TurnoverKind};
use crate::state::{TargetKind, MAX_ON_COURT};
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
    /// 这一球上真正执行了卡位的防守人：只有他们拿卡位加成。
    /// 「卡位」是一个动作，而不是一个几何巧合：出手时离球近的防守人
    /// 才会去卡位，别人去抢自己的位置。
    ///
    /// 定长数组而不是 `Vec`：这个结构每 tick 都会被读，稳态路径不能分配。
    pub box_outs: [usize; MAX_ON_COURT],
    /// 实际执行卡位的人数。
    pub box_out_count: u8,
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
    let mut box_outs = [usize::MAX; MAX_ON_COURT];
    let mut box_out_count = 0u8;
    // 所有场上球员都向球移动；防守人先占篮下卡位，随后再抢球。
    for handle in 0..world.players.len() {
        if !world.players[handle].on_court {
            continue;
        }
        let distance = (world.players[handle].pos - ball_pos).length();
        // 冲抢篮板倾向决定进攻人冲不冲这一球：愿意冲的去抢球，不愿意的
        // 守在原位，因此自然抢不到。倾向改的是落点，不是伸手范围——伸手
        // 范围里再加一项会与防守篮板属性在同一场争抢里直接抵消，把防守
        // 篮板属性的效应量压掉一半。
        let crashes = world.players[handle].team != offense
            || world.players[handle].data.tendencies.offensive_rebound_frequency
                >= world.params.policy.g("oreb_crash_tendency_floor");
        world.players[handle].target = if crashes {
            ball_pos
        } else {
            world.players[handle].pos
        };
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
            // 卡位是一件事：好的卡位者往球落点的方向挤，把进攻球员
            // 挤到身后。只有伸手半径一项的话，卡位能力不会真的改变
            // 谁先碰到球，防守篮板属性因此几乎不产生可见效果。
            let boxout = nba_domain::capability::effective_defensive_boxout_bonus(
                &world.rules,
                &world.players[handle].data.attributes,
            );
            let to_ball = (ball_pos - world.players[handle].pos).normalize_or_zero();
            // 卡位强度直接读属性：好的卡位者把进攻球员挤到身后更多。
            // 属性与曲线分开：曲线是「卡位能力加成」，属性是「卡位意愿」，
            // 两者相加才是他实际挤开的距离。
            let willingness = world.players[handle].attribute("defensive_rebound");
            let push = world.params.policy.g("boxout_position_push_ft")
                * (boxout / world.params.policy.g("boxout_position_push_reference")
                    + (willingness - 0.5) * world.params.policy.g("boxout_willingness_gain"));
            world.players[handle].target = ball_pos - to_ball * push;
            world.players[handle].target_kind = TargetKind::Window;
            world.players[handle].target_partner = handle;
            if (box_out_count as usize) < box_outs.len() {
                box_outs[box_out_count as usize] = handle;
                box_out_count += 1;
            }
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
        box_outs,
        box_out_count,
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
        box_outs: [usize::MAX; MAX_ON_COURT],
        box_out_count: 0,
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
    // 回合已经结束（例如时钟违例或出界）：这次拿球只把球交给控制者，
    // 不再记篮板或失误。
    if world.possession.id == 0 {
        take_live_ball(world, handle);
        return;
    }
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
            let end = if state.pass_by.is_some() {
                (
                    nba_domain::event::PossessionEndCause::TurnoverPassDropped,
                    "pass_incomplete",
                )
            } else {
                (
                    nba_domain::event::PossessionEndCause::TurnoverLooseBall,
                    "loose_ball_recovered",
                )
            };
            world.adjudicate(
                crate::adjudicate::Adjudication::new(
                    crate::adjudicate::StoppageCause::PossessionChange,
                    crate::adjudicate::RestartPlan::Live { team },
                )
                .ending(end.0, end.1),
            );
        }
        take_live_ball(world, handle);
        return;
    }
    let offensive = world.players[handle].team == state.offense;
    let defense = state.offense.other();
    // 抢到篮板是一次起跳争抢：动作账上记一对开始与结束，结束窗口用联赛的
    // 篮板跟随时长，抢到球的人要等落地收势之后才能再动作。
    world.next_action_id += 1;
    let jump_action = world.next_action_id;
    world.emit(Fact::ActionStarted {
        action_id: jump_action,
        interaction_id: 0,
        player: handle,
        kind: "ReboundJump",
        target: None,
    });
    world.emit(Fact::ActionCompleted {
        action_id: jump_action,
        player: handle,
        kind: "ReboundJump",
    });
    let settle_until = world.now() + f64::from(world.rules.rebound_follow_seconds);
    world.players[handle].schedule.next_redecide_at = settle_until;
    let distance_ft = (world.ball.pos - world.hoop_for(state.offense)).length();
    world.emit(Fact::Rebound {
        player: handle,
        offensive,
        distance_ft,
    });
    // 有对抗的篮板：对手就在卡位半径内。卡位加成只在这种篮板里生效，
    // 机制实验也用它度量 defensive_rebound 的效果。
    let contest_radius = world.params.policy.g("boxout_contest_radius_ft");
    let winner_team = world.players[handle].team;
    let contested = world
        .on_court(winner_team.other())
        .as_slice()
        .iter()
        .any(|opponent| {
            (world.players[*opponent].pos - world.players[handle].pos).length() <= contest_radius
        });
    if contested {
        world.contested_rebounds[World::team_index(winner_team)] += 1;
    }
    if offensive {
        world.line_mut(handle).oreb += 1;
        world.possession.had_offensive_rebound = true;
        world.start_second_chance(state.offense, handle, state.rim_contact);
    } else {
        world.line_mut(handle).dreb += 1;
        world.adjudicate(
            crate::adjudicate::Adjudication::new(
                crate::adjudicate::StoppageCause::PossessionChange,
                crate::adjudicate::RestartPlan::Live { team: defense },
            )
            .ending(
                nba_domain::event::PossessionEndCause::DefensiveRebound,
                "defensive_rebound",
            ),
        );
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
