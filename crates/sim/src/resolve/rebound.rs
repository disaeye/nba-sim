//! 篮板：投失后球保持活球，卡位、落点与控球都由位置和接触决定。

use glam::Vec2;

use crate::action::{ActionKind, Fact, TeamSide, TurnoverKind};
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
    // 篮板活球建立后解除程序站位，所有人开始追球。
    world.clear_free_throw_spots();
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
            || world.players[handle]
                .data
                .tendencies
                .offensive_rebound_frequency
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
            // 职责变了，上一段职责因此结束。
            world.end_posture(handle, crate::state::PostureSlot::OffBall);
            let from = world.players[handle].defense_responsibility;
            world.players[handle].defense_responsibility =
                crate::state::DefenseResponsibility::BoxOut;
            world.players[handle].assignment_since = world.now();
            // 卡位姿态有身份：协议投影与事件流都指到它。职责变化那条事实
            // 带着它的身份落下，因此这个姿态在事件流里也有起止。
            let action_id = world.posture_action_id(
                handle,
                crate::state::PostureSlot::OffBall,
                crate::state::DefenseResponsibility::BoxOut.as_str(),
            );
            world.emit(Fact::DefenseResponsibilityChanged {
                player: handle,
                from: from.as_str(),
                to: crate::state::DefenseResponsibility::BoxOut.as_str(),
                action_id,
            });
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
    // 争抢篮板的人各自有一个起跳动作：动作运行时因此带着动作身份，
    // 渲染器能画出谁在抢球。起跳的人不只是够得着球的人：所有冲向球落点
    // 的人都跳，够得着球的���是最后碰到球的那一个。
    let mut jumpers: [usize; MAX_ON_COURT] = [usize::MAX; MAX_ON_COURT];
    let mut jumper_count = 0usize;
    for (index, player) in world.players.iter().enumerate() {
        if !player.on_court {
            continue;
        }
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
        // 起跳：冲向球落点的人。冲向球的人就是抢球的人。
        if distance <= world.params.policy.g("rebound_jump_radius_ft")
            && jumper_count < jumpers.len()
        {
            jumpers[jumper_count] = index;
            jumper_count += 1;
        }
    }
    world.emit(Fact::ReboundOpportunity {
        players,
        contestable,
    });
    // 起跳争抢是一个动作：跳起来的人各自进入执行窗口，抢到球的人的
    // 动作在控制成立的那一刻结算。
    for index in 0..jumper_count {
        let handle = jumpers[index];
        if handle == usize::MAX || world.players[handle].action.is_some() {
            continue;
        }
        start_rebound_jump(world, handle);
    }
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

/// 起跳争抢：跳起来够球的人进入一个动作运行时。
///
/// 动作身份让渲染器能画出谁在抢球，也让这一次的篮板有一个可查的动作
/// 生命周期：开始、阶段变化与终态都在事件流里。
fn start_rebound_jump(world: &mut World, handle: usize) {
    world.next_action_id += 1;
    world.next_interaction_id += 1;
    let action_id = world.next_action_id;
    let interaction_id = world.next_interaction_id;
    let now = world.now();
    let preparation = 0.15;
    let execution = 0.25;
    let follow_through = f64::from(world.rules.rebound_follow_seconds);
    let ball_pos = world.ball.pos;
    world.players[handle].action = Some(crate::action::ActionRuntime::new(
        action_id,
        interaction_id,
        crate::action::ActionIntent {
            actor: handle,
            kind: ActionKind::Rebound(crate::action::ReboundKind::JumpRebound),
            target: None,
            from_ft: world.players[handle].pos,
            to_ft: ball_pos,
            start_time: now,
            duration: preparation + execution + follow_through,
        },
        handle,
        crate::action::ActionIntentRole::Primary,
        nba_domain::action_window::ActionPhase::Preparation,
        preparation,
        execution,
        follow_through,
    ));
    world.emit(Fact::ActionStarted {
        action_id,
        interaction_id,
        player: handle,
        kind: "ReboundJump",
        target: None,
    });
}

/// 双方同时控制球：两名球员各自有一个地板球争抢动作。
///
/// 争抢动作在跳球程序开始时结算：原判是「球被两人同时控制」，这个事实
/// 必须留在事件流里，因此动作有一个可查的终态。
pub fn start_held_ball_actions(world: &mut World, home: usize, away: usize) {
    for handle in [home, away] {
        if handle >= world.players.len() || world.players[handle].action.is_some() {
            continue;
        }
        world.next_action_id += 1;
        let action_id = world.next_action_id;
        world.next_interaction_id += 1;
        let interaction_id = world.next_interaction_id;
        let now = world.now();
        let duration = f64::from(world.rules.rebound_follow_seconds);
        world.players[handle].action = Some(crate::action::ActionRuntime::new(
            action_id,
            interaction_id,
            crate::action::ActionIntent {
                actor: handle,
                kind: ActionKind::Rebound(crate::action::ReboundKind::HeldBall),
                target: None,
                from_ft: world.players[handle].pos,
                to_ft: world.ball.pos,
                start_time: now,
                duration,
            },
            handle,
            crate::action::ActionIntentRole::Primary,
            nba_domain::action_window::ActionPhase::Preparation,
            0.0,
            0.0,
            duration,
        ));
        world.emit(Fact::ActionStarted {
            action_id,
            interaction_id,
            player: handle,
            kind: "HeldBallDive",
            target: None,
        });
    }
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
    // 地板球争抢：松球在地上，离球最近的两名球员（每队一名）趴下去抢。
    // 十个人一起扑向球不是地板球争抢，而是十个人一起丢了自己的位置：
    // 争抢动作因此只给真正够得到球的那两个人。
    let divers = loose_ball_divers(world, ball_pos);
    for handle in divers {
        start_loose_ball_dive(world, handle);
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

/// 地板球争抢的人选：每队离球最近的那名在场球员。
///
/// 争抢是两个人的事：每队一个。离球最近的人才是真正够得到球的人。
fn loose_ball_divers(world: &World, ball_pos: glam::Vec2) -> Vec<usize> {
    let mut divers = Vec::with_capacity(2);
    for team in [crate::action::TeamSide::Home, crate::action::TeamSide::Away] {
        let nearest = world
            .players
            .iter()
            .enumerate()
            .filter(|(_, player)| player.on_court && player.team == team)
            .min_by(|left, right| {
                (left.1.pos - ball_pos)
                    .length_squared()
                    .total_cmp(&(right.1.pos - ball_pos).length_squared())
                    .then_with(|| left.0.cmp(&right.0))
            })
            .map(|(handle, _)| handle);
        if let Some(handle) = nearest {
            if world.players[handle].action.is_none() {
                divers.push(handle);
            }
        }
    }
    divers
}

/// 地板球争抢的动作运行时。
///
/// 松球在地上的时候每个人都趴下去抢。这个动作在球被控制的那一刻
/// 结算。
fn start_loose_ball_dive(world: &mut World, handle: usize) {
    world.next_action_id += 1;
    let action_id = world.next_action_id;
    world.next_interaction_id += 1;
    let interaction_id = world.next_interaction_id;
    let now = world.now();
    let duration = f64::from(world.rules.rebound_follow_seconds) * 2.0;
    world.players[handle].action = Some(crate::action::ActionRuntime::new(
        action_id,
        interaction_id,
        crate::action::ActionIntent {
            actor: handle,
            kind: ActionKind::Rebound(crate::action::ReboundKind::LooseBallDive),
            target: None,
            from_ft: world.players[handle].pos,
            to_ft: world.ball.pos,
            start_time: now,
            duration,
        },
        handle,
        crate::action::ActionIntentRole::Primary,
        nba_domain::action_window::ActionPhase::Preparation,
        0.0,
        0.0,
        duration,
    ));
    world.emit(Fact::ActionStarted {
        action_id,
        interaction_id,
        player: handle,
        kind: "LooseBallDive",
        target: None,
    });
}

/// 球员接管活球：持球、进入推进段。
///
/// 调度和推进目标由 `World::begin_live_push` 统一写入：活球回合要先把球
/// 推进到前场再读场面，推进入手位置由实际控球的人决定。
fn take_live_ball(world: &mut World, handle: usize) {
    // 球被控制：地板球争抢的动作在此结算。
    if let Some(runtime) = world.players[handle].action.take() {
        if matches!(
            runtime.intent.kind,
            ActionKind::Rebound(crate::action::ReboundKind::LooseBallDive)
        ) {
            world.emit(Fact::ActionCompleted {
                action_id: runtime.action_id,
                player: handle,
                kind: runtime.intent.kind.as_str(),
            });
        } else {
            world.players[handle].action = Some(runtime);
        }
    }
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
    // 篮板归属：抢到球的人属于进攻方还是防守方，决定它是不是进攻篮板。
    let offensive = world.players[handle].team == state.offense;
    let defense = state.offense.other();
    // 点球：球在落点附近被碰了一下但没有被控制。点球之后球仍然是活的，
    // 双方继续争抢。
    //
    // 点球的三种后续：球被往上拨给队友是点传，球直接被拨进是补篮，
    // 球被平着拨出去、双方继续争抢是点球。判据是球在控制成立那一刻
    // 的高度与速度：够到篮筐高度但没有往上传，是点球。
    let ball_z = world.ball.z;
    let ball_vz = world.ball.vz;
    let rim_height = world.rules.rim_height_ft;
    let secure_kind = if ball_z < world.rules.ball_radius_ft + 0.6 {
        // 球已经落地：控制篮板。
        crate::action::ReboundKind::SecureRebound
    } else if ball_vz > 2.0 {
        // 球被明显往上拨：点传，队友接球。
        crate::action::ReboundKind::TipPass
    } else if ball_z < rim_height {
        // 球还在筐口以下被拨进去：补篮。
        crate::action::ReboundKind::TipIn
    } else {
        // 球被平着拨出去，双方继续争抢：点球。
        crate::action::ReboundKind::Tap
    };
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
    // 球被两个人同时碰到但谁也没完全控制住：这也是一次点球，球还在
    // 争抢之中。
    let secure_kind = if contested {
        crate::action::ReboundKind::Tap
    } else {
        secure_kind
    };
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
    // 抢到篮板的人把起跳动作结算成这次篮板动作：控制成立的那一刻，篮板
    // 动作有一个唯一的终态。这个动作在裁决之后建立，因此不会被裁决
    // 的停表取消。
    if let Some(runtime) = world.players[handle].action.take() {
        world.emit(Fact::ActionCompleted {
            action_id: runtime.action_id,
            player: handle,
            kind: runtime.intent.kind.as_str(),
        });
    }
    world.next_action_id += 1;
    let secure_action = world.next_action_id;
    world.next_interaction_id += 1;
    let secure_interaction = world.next_interaction_id;
    let now = world.now();
    let follow_through = f64::from(world.rules.rebound_follow_seconds);
    world.players[handle].action = Some(crate::action::ActionRuntime::new(
        secure_action,
        secure_interaction,
        crate::action::ActionIntent {
            actor: handle,
            kind: ActionKind::Rebound(secure_kind),
            target: None,
            from_ft: world.players[handle].pos,
            to_ft: world.ball.pos,
            start_time: now,
            duration: follow_through,
        },
        handle,
        crate::action::ActionIntentRole::Primary,
        nba_domain::action_window::ActionPhase::Preparation,
        0.0,
        0.0,
        follow_through,
    ));
    world.emit(Fact::ActionStarted {
        action_id: secure_action,
        interaction_id: secure_interaction,
        player: handle,
        kind: secure_kind.as_str(),
        target: None,
    });
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
