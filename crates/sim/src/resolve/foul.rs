//! 犯规与罚则：投篮犯规、进攻犯规、bonus、罚球程序入口。
//!
//! 犯规总是停表。差别只在停表之后是谁的球：投篮犯规与 bonus 犯规进入
//! 罚球，其余犯规由被犯规方发边线球。这条「一律停表」是死球换人窗口的
//! 来源——主力身背 4 犯之所以能下场，是因为对手的每一次犯规都把比赛
//! 停下来一次。

use crate::action::{Fact, FoulKind};
use crate::state::TeamSide;
use crate::world::World;

/// 犯规的公共计账：个人累计、球队单节犯规、bonus 判定与犯规事实。
///
/// 个人累计与球队单节犯规是两套账：进攻犯规计入个人累计（满 6 犯仍离场），
/// 但不计入球队单节犯规，因此不会把对手送进 bonus。`award` 是该犯规类型
/// 自带的罚球数（投篮犯规 2/3 罚），为 0 时按球队单节犯规判定 bonus。
#[derive(Debug, Clone, Copy)]
pub struct FoulBooking {
    pub free_throws: u8,
    pub foul_id: u64,
    pub team_technical_id: Option<u64>,
    pub retained_team: TeamSide,
}

pub fn book(
    world: &mut World,
    fouler: usize,
    fouled: usize,
    kind: FoulKind,
    award: u8,
    and_one: bool,
) -> FoulBooking {
    let team = world.players[fouler].team;
    assert!(
        world.players[fouler].on_court,
        "only an on-court player can foul"
    );
    let was_disqualified = world.players[fouler].disqualified;
    let foul_limit = world.rules.league.fouls.max_personal_fouls;
    let exhausted_bench = world.rules.league.id == nba_domain::league::LeagueId::Nba
        && !(0..world.players.len()).any(|handle| {
            let player = &world.players[handle];
            player.team == team
                && !player.on_court
                && !player.disqualified
                && player.fouls < foul_limit
        });
    // 技术犯规不计入个人犯规累计（NBA Rule 12A §II）：它不是一次个人
    // 犯规，也不计入球队单节犯规。账本的个人犯规计数按同一条规则。
    let personal = nba_domain::rules::counts_toward_personal_limit(kind);
    if personal {
        world.line_mut(fouler).fouls += 1;
    }
    let disqualified = if personal {
        crate::rotation::record_foul(world, fouler)
    } else {
        false
    };
    let index = World::team_index(world.players[fouler].team);
    let counts = nba_domain::rules::counts_toward_team_period_fouls(kind);
    if counts {
        world.fouls[index].period += 1;
    }
    world.fouls[index].total += 1;
    if disqualified {
        world.rotation.pending[index] = true;
    }
    let free_throws = if award > 0 {
        award
    } else if counts {
        bonus_free_throws(world, world.players[fouler].team)
    } else {
        0
    };
    let foul_id = world.new_foul_id();
    let retained_team = if kind == FoulKind::Offensive {
        team.other()
    } else {
        world.possession.team
    };
    let team_technical_id = if disqualified && exhausted_bench {
        world.rotation.pending[index] = false;
        world.players[fouler].retained_after_foul_limit = true;
        world.fouls[index].technical += 1;
        Some(world.new_foul_id())
    } else {
        if disqualified {
            world.rotation.pending[index] = true;
        }
        None
    };
    world.emit(Fact::Foul {
        fouler,
        fouled,
        kind,
        free_throws,
        and_one,
        foul_id,
    });
    if let Some(technical_id) = team_technical_id {
        if !was_disqualified {
            world.emit(Fact::BenchDepleted {
                team,
                player: fouler,
            });
        }
        world.emit(Fact::FoulOutRetention {
            team,
            player: fouler,
            foul_id,
        });
        world.emit(Fact::TeamTechnicalFoul {
            team,
            reason: "foul_out_with_depleted_bench",
            foul_id: technical_id,
        });
    }
    FoulBooking {
        free_throws,
        foul_id,
        team_technical_id,
        retained_team,
    }
}

pub fn start_roster_technical(
    world: &mut World,
    booking: FoulBooking,
    beneficiary: TeamSide,
    follow_up: crate::resolve::freethrow::FreeThrowFollowUp,
) {
    let Some(technical_id) = booking.team_technical_id else {
        return;
    };
    crate::resolve::freethrow::shoot_team_technical(
        world,
        beneficiary,
        booking.retained_team,
        technical_id,
        follow_up,
    );
}

/// 球队是否已经进入 bonus。
///
/// 必须在球队单节犯规已经加上本次犯规之后调用：第 5 犯本身就在 bonus 内。
pub fn bonus_free_throws(world: &World, fouling_team: TeamSide) -> u8 {
    let index = World::team_index(fouling_team);
    world.rules.league.bonus_free_throws_for(
        world.fouls[index].period as u32,
        world.clock.period(),
        world.clock.game_clock(),
    )
}

/// 最后时段的无球犯规（一罚一掷）是否适用。
///
/// 这条规则本身就是「不许用犯规阻断无球跑动」：如果罚球后不保留球权，
/// 落后方就能靠抓无球人无限换回球权。
fn is_away_from_play(world: &World, fouled: usize) -> bool {
    world
        .rules
        .league
        .in_final_window(world.clock.period(), world.clock.game_clock())
        && world.holder() != Some(fouled)
}

pub fn apply_shooting_foul(
    world: &mut World,
    fouler: usize,
    shooter: usize,
    free_throws: u8,
    made: bool,
) {
    world.line_mut(fouler).shooting_fouls += 1;
    if made {
        world.line_mut(shooter).and_one += 1;
    }
    let booking = book(
        world,
        fouler,
        shooter,
        FoulKind::Shooting,
        free_throws,
        made,
    );
    world.pending_rebound = None;
    // 罚则计划：一次犯规判几次罚球、有没有加罚、之后谁拿球。
    let penalty = crate::adjudicate::PenaltyPlan {
        free_throws: booking.free_throws,
        and_one: made,
        retain_possession: false,
        possession_after: None,
        technical: booking.team_technical_id.is_some(),
    };
    // 新罚球程序：清空上一轮的禁区记录。
    world.free_throw_lane = Some(crate::world::FreeThrowLane::default());
    world.adjudicate(crate::adjudicate::Adjudication::new(
        crate::adjudicate::StoppageCause::Foul,
        crate::adjudicate::RestartPlan::FreeThrows {
            shooter,
            total: penalty.free_throws,
            remaining: penalty.free_throws,
            made: 0,
            and_one: penalty.and_one,
            retain_possession: penalty.retain_possession,
            foul_id: booking.foul_id,
            technical_retained_team: None,
            after_technical: None,
        },
    ));
    world.apply_dead_ball_window();
    if booking.team_technical_id.is_some() {
        start_roster_technical(
            world,
            booking,
            world.players[shooter].team,
            crate::resolve::freethrow::FreeThrowFollowUp::Attempts {
                shooter,
                total: booking.free_throws,
                remaining: booking.free_throws,
                made: 0,
                and_one: made,
                retain_possession: false,
                foul_id: booking.foul_id,
            },
        );
    }
}

/// 非投篮普通犯规：bonus 之内两罚，最后时段对无球人犯规，
/// 其余情况由被犯规方发边线球。
pub fn apply_non_shooting_foul(world: &mut World, fouler: usize, fouled: usize) {
    let away_from_play = is_away_from_play(world, fouled);
    let award = if away_from_play {
        world.rules.league.fouls.away_from_play_free_throws
    } else {
        0
    };
    let booking = book(world, fouler, fouled, FoulKind::Personal, award, false);
    // 哨响就停表：场上正在进行的掩护、切入、突破都作废，否则那些动作的
    // 结算会用上一个状态的防守人或站位去判一个已经不存在的回合。
    world.cancel_actions("foul");
    if booking.free_throws > 0 {
        world.pending_rebound = None;
        world.adjudicate(crate::adjudicate::Adjudication::new(
            crate::adjudicate::StoppageCause::Foul,
            crate::adjudicate::RestartPlan::FreeThrows {
                shooter: fouled,
                total: booking.free_throws,
                remaining: booking.free_throws,
                made: 0,
                and_one: false,
                retain_possession: away_from_play,
                foul_id: booking.foul_id,
                technical_retained_team: None,
                after_technical: None,
            },
        ));
    } else {
        // 无罚球的犯规同样停表换边（保留球权方为被犯规方）：
        // 阶段写入走同一个提交入口，发球位置取中断点的就近边线。
        // 被犯规方重新从后场发球，八秒计时随之重新开始。
        world.possession.backcourt_started_game_clock = None;
        let spot = world.inbound_point_of_interruption();
        world.adjudicate(crate::adjudicate::Adjudication::new(
            crate::adjudicate::StoppageCause::Foul,
            crate::adjudicate::RestartPlan::Inbound {
                team: world.players[fouled].team,
                location: crate::adjudicate::InboundLocation::Spot(spot),
                ready_after: None,
            },
        ));
    }
    world.apply_dead_ball_window();
    let follow_up = if booking.free_throws > 0 {
        crate::resolve::freethrow::FreeThrowFollowUp::Attempts {
            shooter: fouled,
            total: booking.free_throws,
            remaining: booking.free_throws,
            made: 0,
            and_one: false,
            retain_possession: away_from_play,
            foul_id: booking.foul_id,
        }
    } else {
        crate::resolve::freethrow::FreeThrowFollowUp::DeadBall {
            team: world.players[fouled].team,
        }
    };
    if booking.team_technical_id.is_some() {
        start_roster_technical(
            world,
            booking,
            world.players[fouler].team.other(),
            follow_up,
        );
    }
}

/// 双方犯规：双方各记一次个人犯规，但不计入球队单节犯规，也不罚球。
///
/// 球权按规则留给当时持球的一方（NBA Rule 12B §VI.3）：双方犯规时球权
/// 留在进攻方，从靠近中断点的边线发球；进攻钟在前场发球时保持不变或
/// 补到十四秒，在后场发球时回到二十四秒。
pub fn apply_double_foul(world: &mut World, fouler: usize, fouled: usize) {
    let team = world.players[fouler].team;
    let other = world.players[fouled].team;
    // 双方各记一次个人犯规：两边都进个人累计。
    book(world, fouler, fouled, FoulKind::Double, 0, false);
    book(world, fouled, fouler, FoulKind::Double, 0, false);
    // 球权留在持球的一方：进攻方犯规时球权给对方，防守方犯规时球权留在
    // 进攻方。两边都在场上，因此双方犯规的球权就是双方犯规发生时控球的
    // 那一方。
    let retained = world
        .holder()
        .map(|handle| world.players[handle].team)
        .unwrap_or_else(|| world.possession.team);
    let retained = if retained == team && retained == other {
        world.possession.team
    } else {
        retained
    };
    world.cancel_actions("double_foul");
    let spot = world.inbound_point_of_interruption();
    world.adjudicate(crate::adjudicate::Adjudication::new(
        crate::adjudicate::StoppageCause::Foul,
        crate::adjudicate::RestartPlan::Inbound {
            team: retained,
            location: crate::adjudicate::InboundLocation::Spot(spot),
            ready_after: None,
        },
    ));
    world.apply_dead_ball_window();
}

/// 清晰路径犯规（NBA Rule 12B §I.5.f）。
///
/// 罚则是两次罚球加上球权（Rule 12B §I.5.f.6）：被犯规方在罚球之后
/// 拿到球权。犯规计入个人累计与球队单节犯规：它是一次个人犯规。
///
/// 成立条件由调用方判断：球在对方半场且在进攻机会中，防守人从背后
/// 犯规导致这次进攻机会丢失。
pub fn apply_clear_path_foul(world: &mut World, fouler: usize, fouled: usize) {
    let team = world.players[fouler].team;
    let beneficiary = world.players[fouled].team;
    let booking = book(world, fouler, fouled, FoulKind::Personal, 2, false);
    world.pending_rebound = None;
    // 新罚球程序：清空上一轮的禁区记录。
    world.free_throw_lane = Some(crate::world::FreeThrowLane::default());
    // 罚球之后由被犯规方在罚球线延长线发球（Rule 12B §I.5.f.6）。
    let spot = world.rules.court.sideline_at_free_throw_line_extended(
        world.players[fouled].pos,
        world.attacking_right(beneficiary),
    );
    world.adjudicate(crate::adjudicate::Adjudication::new(
        crate::adjudicate::StoppageCause::Foul,
        crate::adjudicate::RestartPlan::FreeThrows {
            shooter: fouled,
            total: booking.free_throws,
            remaining: booking.free_throws,
            made: 0,
            and_one: false,
            retain_possession: true,
            foul_id: booking.foul_id,
            technical_retained_team: None,
            after_technical: None,
        },
    ));
    world.apply_dead_ball_window();
    world.inbound_spot = Some(spot);
    let _ = team;
}

/// 恶意犯规（NBA Rule 12B §IV）。
///
/// 罚则分两级：
///
/// - 一级（penalty 1）：两次罚球，被犯规方在罚球线延长线的任意一侧
///   发球。
/// - 二级（penalty 2）：两次罚球加球权，而且犯规者被逐（Rule 12B
///   §IV.2）。
///
/// 恶意犯规计入个人犯规累计与球队单节犯规：它是一���个人犯规。
pub fn apply_flagrant_foul(world: &mut World, fouler: usize, fouled: usize, excessive: bool) {
    let team = world.players[fouler].team;
    let beneficiary = world.players[fouled].team;
    let booking = book(world, fouler, fouled, FoulKind::Flagrant, 2, false);
    world.pending_rebound = None;
    // 新罚球程序：清空上一轮的禁区记录。
    world.free_throw_lane = Some(crate::world::FreeThrowLane::default());
    // 恶意犯规计入个人犯规累计：被逐之前先看他的个人犯规是否已经到顶
    // （NBA Rule 3 §I 的个人犯规离场）。
    crate::rotation::record_foul(world, fouler);
    if excessive {
        // 二级恶意犯规：犯规者被逐（NBA Rule 12B §IV.2）。
        world.players[fouler].disqualified = true;
        world.rotation.pending[World::team_index(team)] = true;
        world.emit(Fact::Ejected {
            team,
            player: fouler,
            reason: "flagrant_penalty_2",
        });
    }
    // 罚球：两次。罚完之后，一级在罚球线延长线发球（Rule 12B §IV.1），
    // 二级由被犯规方直接拿到球权（Rule 12B §IV.2）。
    world.adjudicate(crate::adjudicate::Adjudication::new(
        crate::adjudicate::StoppageCause::Foul,
        crate::adjudicate::RestartPlan::FreeThrows {
            shooter: fouled,
            total: booking.free_throws,
            remaining: booking.free_throws,
            made: 0,
            and_one: false,
            // 二级恶意犯规：被犯规方在罚球之后拿到球权。
            retain_possession: excessive,
            foul_id: booking.foul_id,
            technical_retained_team: None,
            after_technical: None,
        },
    ));
    world.apply_dead_ball_window();
    let _ = beneficiary;
}

/// 进攻犯规：计入个人累计与球队全场犯规，不计入球队单节犯规，
/// 不判给对手罚球，只把球权交给对方。
pub fn apply_offensive_foul(world: &mut World, fouler: usize, fouled: usize) {
    let team = world.players[fouler].team;
    let booking = book(world, fouler, fouled, FoulKind::Offensive, 0, false);
    // 进攻犯规停表换边：取消动作、结束回合与恢复方式都经同一个提交入口，
    // 发球位置取中断点的就近边线。
    let spot = world.inbound_point_of_interruption();
    world.adjudicate(
        crate::adjudicate::Adjudication::new(
            crate::adjudicate::StoppageCause::Foul,
            crate::adjudicate::RestartPlan::Inbound {
                team: team.other(),
                location: crate::adjudicate::InboundLocation::Spot(spot),
                ready_after: None,
            },
        )
        .ending(
            nba_domain::event::PossessionEndCause::TurnoverOffensiveFoul,
            "offensive_foul",
        ),
    );
    world.apply_dead_ball_window();
    let retained_team = team.other();
    start_roster_technical(
        world,
        FoulBooking {
            retained_team,
            ..booking
        },
        retained_team,
        crate::resolve::freethrow::FreeThrowFollowUp::DeadBall {
            team: retained_team,
        },
    );
}
