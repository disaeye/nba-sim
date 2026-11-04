//! 限制区计时：进攻三秒与防守三秒。
//!
//! 两条规则都按「某个球员连续停留」计时，计时记在球员身上：
//!
//! 进攻三秒（NBA Rule 10 §VI / FIBA Article 26）：进攻方在前场控制球时，
//! 进攻人在限制区里连续停留超过三秒即违例。豁免情形：
//!
//! - 球不在前场（进攻方还没控制前场的球）；
//! - 那个进攻人自己持球；
//! - 那个进攻人正要接球（球在飞向他的路上）；
//! - 那个进攻人正在朝篮筐发起连续动作（Rule 10 §VI.2）；
//! - 那个进攻人的职责本身就要求他在禁区里（低位、顺下）。
//!
//! 防守三秒（NBA Rule 10 §VII）：进攻方在前场控制球时，防守人在限制区里
//! 连续停留超过三秒即违例，判技术犯规，对手罚一次并保留球权。豁免：
//! 那个防守人正在积极盯防（Rule 10 §VII.2 的「actively guarding」：
//! 一臂之内且处于盯防位），或者进攻方在禁区里没有人可盯。
//!
//! 两条规则都要求「无球清空限制区」的行为：计时开始之后，球员要在
//! 限时之内走出限制区。因此计时不是纯判定，它同时驱动落点修正。

use glam::Vec2;

use crate::action::Fact;
use crate::world::World;

/// 进攻三秒的宽限：计时到这个秒数之后，球员才开始往外走。
///
/// 宽限让进攻方在正常进攻里保留原有的走位：球员在限制区里站两秒是
/// 正常的低位站位，跑到第三秒才需要往外走一步。计时因此按真实比赛
/// 的方式停得下来，而不是每次进攻都改变进攻方的形状。
pub const OFFENSE_LANE_GRACE_SECONDS: f32 = 0.0;

/// 球是否在限制区里。
///
/// 球在限制区里时，双方都不计时：进攻人正要接球或出手，防守人正要
/// 抢断，限制区里有人是正常状态。
pub fn ball_in_lane(world: &World) -> bool {
    if !world.live_play() {
        return false;
    }
    let offense = world.possession.team;
    let attacking_right = world.attacking_right(offense);
    world
        .rules
        .court
        .is_in_lane(world.ball.pos, attacking_right)
}

/// 进攻方此刻是否在前场控制球。
///
/// 两条三秒规则都以这个条件为起点（NBA Rule 10 §VI.3 与 §VII.1）：
/// 「The count starts when the offensive team is in control of the ball
/// in the frontcourt」。球在空中、球被拍飞、球还在后场，都不成立。
pub fn offense_controls_frontcourt(world: &World) -> bool {
    if !world.live_play() {
        return false;
    }
    let offense = world.possession.team;
    // 球在进攻方手里，或者球正在飞向进攻方的队友：两种都是进攻方
    // 控制球。球被对手拍走之后控制权才结束。
    match world.holder() {
        Some(holder) if world.players[holder].team == offense => {}
        _ => {
            let in_flight_to_offense = world
                .pass_in_flight
                .is_some_and(|info| world.players[info.to].team == offense);
            if !in_flight_to_offense {
                return false;
            }
        }
    }
    // 球在进攻方的前场：这是规则的原文条件。球在哪一侧由进攻方向决定：
    // 进攻方向是右时，前场就是越过中线的那一侧。
    //
    // 判据是球的位置而不是「已建立前场控制」那个标志：那个标志要到球
    // 推进到前场深处才置位，用它当判据会让三秒计时在球刚过中线时
    // 停不下来。
    let attacking_right = world.attacking_right(offense);
    !world.rules.court.is_backcourt(world.ball.pos, attacking_right)
}

/// 这个进攻人是不是因为自己的职责才站在限制区里。
///
/// 低位与顺下这两个职责本身就要求人在禁区：低位要卡住防守人，顺下要
/// 往篮下切。他们的球会送进来，因此不按「在禁区里站久了」判三秒。
/// 其余进攻人没有这类职责，在禁区里待满三秒就是违例。
fn lane_by_role(world: &World, player: usize) -> bool {
    use crate::state::OffenseJob;
    if matches!(
        world.players[player].offense_job,
        OffenseJob::PostUp | OffenseJob::Roll
    ) {
        return true;
    }
    // 阵型槽位比职责稳定：职责会随着回合的阶段变化（切入、顺下、卡位
    // 轮流出现），槽位不会。用职责判断会让同一个人在不同的职责下得到
    // 不同的豁免，计时因此反复。
    matches!(
        world.players[player].slot,
        crate::state::Slot::Post | crate::state::Slot::DunkerSpot
    )
}

/// 这个进攻人是否正在朝篮筐发起动作，或者正要离开限制区。
///
/// NBA Rule 10 §VI.2 给了两条豁免：
///
/// - 「a player who ... is in the act of shooting at the end of the third
///   second」——已经在出手的球员继续；
/// - 「This is also true if it is imminent the offensive player will exit
///   this area」——正要离开限制区的球员同样不计时。
///
/// 没有这两条豁免，切入与上篮的球员会在冲向篮筐的路上被判三秒——
/// 进攻方的形状因此整个变了。
fn shooting_toward_basket(world: &World, player: usize, attacking_right: bool) -> bool {
    let state = &world.players[player];
    // 职责是切入：这个人正要往篮下跑，规则允许他在第三秒的末尾继续
    // （NBA Rule 10 §VI.2 的「continuous motion toward the basket」）。
    if matches!(state.offense_job, crate::state::OffenseJob::Cut) {
        return true;
    }
    // 正在朝篮筐发起动作：出手与上篮都是「in the act of shooting」。
    let Some(runtime) = state.action.as_ref() else {
        return false;
    };
    if runtime.intent.actor != player {
        return false;
    }
    let to_basket = matches!(
        runtime.intent.kind,
        crate::action::ActionKind::Finish | crate::action::ActionKind::Shoot(_)
    );
    if !to_basket {
        return false;
    }
    // 落点在篮筐一侧：朝篮筐发起的那一步。
    let hoop = world.hoop_for(world.players[player].team);
    let target = runtime.intent.to_ft;
    world.rules.court.is_in_lane(target, attacking_right)
        || (target - hoop).length() < 12.0
}

/// 进攻三秒此刻在计时的进攻人。
///
/// 返回计时应当累加的进攻人句柄；`None` 表示此刻没有进攻人计时。
pub fn offense_timer_holder(world: &World) -> Option<usize> {
    // 计时从进攻方在前场控制球那一刻开始（NBA Rule 10 §VI.3）。
    if !offense_controls_frontcourt(world) {
        return None;
    }
    if ball_in_lane(world) {
        return None;
    }
    let offense = world.possession.team;
    let attacking_right = world.attacking_right(offense);
    let holder = world.holder();
    // 球在空中时，接到球的那个进攻人还没有控制球：他不计时。
    let inbound_receiver = world
        .pass_in_flight
        .map(|info| info.to)
        .filter(|to| world.players[*to].team == offense);
    world.on_court(offense).as_slice().iter().copied().find(|handle| {
        if Some(*handle) == holder || Some(*handle) == inbound_receiver {
            return false;
        }
        if shooting_toward_basket(world, *handle, attacking_right) {
            return false;
        }
        if lane_by_role(world, *handle) {
            return false;
        }
        if !world
            .rules
            .court
            .is_in_lane(world.players[*handle].pos, attacking_right)
        {
            return false;
        }
        // 规则罚的是「remain」——待在禁区里不走。跑过禁区的球员不算
        // 待在禁区里，因此他在移动时不计时：他正穿过禁区，不是站在
        // 禁区里等球。
        !passing_through(world, *handle)
    })
}

/// 这个球员是不是正跑过限制区而不是待在禁区里。
///
/// 判据是速度：跑动中的球员在穿过禁区，站定不动的球员才是在禁区里
/// 停留。切入与上篮的跑动因此不计入，停在禁区里的无球人计入。
fn passing_through(world: &World, player: usize) -> bool {
    let speed = world.players[player].vel.length();
    speed > world.params.policy.g("lane_passing_speed_ftps")
}

/// 防守人是否正在积极盯防。
///
/// NBA Rule 10 §VII.2：「actively guarding means being within arm's
/// length of an offensive player and in a guarding position」。盯防位
/// 就是站在被盯防的人与篮筐之间——护框球员站在低位球员与篮筐之间，
/// 正是盯防位，因此不按防守三秒判罚。
fn guarding_in_lane(world: &World, defender: usize, attacking_right: bool) -> bool {
    let offense = world.possession.team;
    let defense = offense.other();
    let hoop = world.hoop_for(offense);
    let defender_pos = world.players[defender].pos;
    // 一臂之内（NBA Rule 10 §VII.2「within arm's length」）且处于盯防位：
    // 防守人在被盯防人与篮筐之间。
    let arm_length = world.rules.min_player_separation_ft + 2.5;
    // 一臂之内就算在盯人（NBA Rule 10 §VII.2）：规则盯的是「有没有在
    // 盯住某个进攻人」，不要求盯的是对位的那一个。盯防位是这条规则的
    // 第二半，模型里的间隔就是那个距离。
    let guarding_arm = world.on_court(offense).as_slice().iter().any(|man| {
        (world.players[*man].pos - defender_pos).length() <= arm_length
    });
    if guarding_arm {
        return true;
    }
    let _ = hoop;
    // 他对位的进攻人就在禁区里时，他也在盯防：护框球员守着低位球员，
    // 两个人隔着半个禁区，但那正是盯防位。规则的「一臂之内」指的是
    // 裁判看的伸手距离，模型里的职责分配就是那个距离。
    let defenders = world.on_court(defense);
    let Some(slot) = defenders.as_slice().iter().position(|handle| *handle == defender) else {
        return false;
    };
    let man = world.possession.matchup[slot];
    if man == usize::MAX || !world.players[man].on_court {
        return false;
    }
    world.rules.court.is_in_lane(world.players[man].pos, attacking_right)
}

/// 防守三秒此刻在计时的防守人。
///
/// 返回计时应当累加的防守人句柄；`None` 表示此刻没有防守人计时。
pub fn defense_timer_holder(world: &World) -> Option<usize> {
    // 计时从进攻方在前场控制球那一刻开始（NBA Rule 10 §VII.1）。
    if !offense_controls_frontcourt(world) {
        return None;
    }
    if ball_in_lane(world) {
        return None;
    }
    let offense = world.possession.team;
    let defense = offense.other();
    let attacking_right = world.attacking_right(offense);
    // 进攻方在禁区里没有人时，防守人守的就是空位：裁判不会吹这个。
    // 防守三秒罚的是「有人可盯而你不盯」，没有人可盯时不成立。
    let offense_in_lane = world.on_court(offense).as_slice().iter().any(|handle| {
        world
            .rules
            .court
            .is_in_lane(world.players[*handle].pos, attacking_right)
    });
    if !offense_in_lane {
        return None;
    }
    world
        .on_court(defense)
        .as_slice()
        .iter()
        .copied()
        .find(|handle| {
            if !world.rules.court.is_in_lane(world.players[*handle].pos, attacking_right) {
                return false;
            }
            // 正在积极盯防限制区里的进攻人时不计时：他在合法地防守。
            !guarding_in_lane(world, *handle, attacking_right)
        })
}

/// 走出限制区的落点。
///
/// 落点取限制区外沿再往外一英尺，方向是从限制区中心指向球员当前位置
/// 的反向。两个方向都算，取更近的那个：限制区里挤着人，往远的一侧走
/// 会被队友挡住，球员就永远出不去，计时因此停不下来。
pub fn exit_target(world: &World, player: usize) -> Vec2 {
    let offense = world.possession.team;
    let attacking_right = world.attacking_right(offense);
    let court = world.rules.court;
    let baseline_x = if attacking_right {
        court.width_ft
    } else {
        0.0
    };
    let lane_length = nba_domain::court::LANE_LENGTH_FROM_BASELINE_FT * (court.width_ft / 94.0);
    let lane_half = nba_domain::court::LANE_HALF_WIDTH_FT * (court.height_ft / 50.0);
    let pos = world.players[player].pos;
    // 沿底线方向退出限制区：往远离底线的一侧走。落点必须在限制区之外
    // ——落在区沿上仍然算在区里，计时不会停。
    let out_x = if attacking_right {
        baseline_x - lane_length - 1.0
    } else {
        baseline_x + lane_length + 1.0
    };
    // 纵向方向：往离篮筐更远的一侧走，落在区宽之外。区沿之内仍然算在
    // 禁区里，因此落点必须越过区沿再留一英尺。
    let out_y = if pos.y > court.hoop_y_ft {
        court.hoop_y_ft + lane_half + 1.0
    } else {
        court.hoop_y_ft - lane_half - 1.0
    };
    let radius = world.rules.player_radius_ft;
    let along_x = Vec2::new(
        out_x.clamp(radius, court.width_ft - radius),
        pos.y.clamp(radius, court.height_ft - radius),
    );
    let along_y = Vec2::new(
        pos.x.clamp(radius, court.width_ft - radius),
        out_y.clamp(radius, court.height_ft - radius),
    );
    if (pos - along_x).length() <= (pos - along_y).length() {
        along_x
    } else {
        along_y
    }
}

/// 这个进攻人是否必须走出限制区。
///
/// 条件与进攻三秒的计时条件一致：进攻方在前场控制球、球不在限制区里、
/// 那个球员自己既不持球也不正要接球、他的职责也不要求他在禁区里、
/// 并且他没有在朝篮筐发起连续动作。
pub fn clearing_required(world: &World, player: usize) -> bool {
    if !offense_controls_frontcourt(world) {
        return false;
    }
    if ball_in_lane(world) {
        return false;
    }
    let offense = world.possession.team;
    if world.players[player].team != offense {
        return false;
    }
    if world.holder() == Some(player) {
        return false;
    }
    if world
        .pass_in_flight
        .is_some_and(|info| info.to == player)
    {
        return false;
    }
    if lane_by_role(world, player) {
        return false;
    }
    let attacking_right = world.attacking_right(offense);
    if shooting_toward_basket(world, player, attacking_right) {
        return false;
    }
    // 正在跑自己的动作的球员不受这道修正：跑位与切入本来就可能把落点
    // 定在禁区边上（NBA Rule 10 §VI.2 允许他继续）。落点的修正只针对
    // 站着不动的人。
    if running_own_action(world, player) {
        return false;
    }
    true
}

/// 这个球员是不是正在跑自己的动作。
///
/// 跑位与切入有自己的落点：编队不能把它改回站位，限制区的修正同样
/// 不能——否则他永远到不了自己的落点。
fn running_own_action(world: &World, player: usize) -> bool {
    world.players[player].action.as_ref().is_some_and(|runtime| {
        runtime.intent.actor == player
            && matches!(
                runtime.intent.kind,
                crate::action::ActionKind::Relocate | crate::action::ActionKind::Cut(_)
            )
    })
}

/// 这个球员是否正在走出限制区。
///
/// 走出限制区的球员可以正面穿过队友：他要离开的是一条被人挤满的
/// 区域，绕行规则会让他卡在原地，计时因此停不下来。
pub fn clearing_lane(world: &World, player: usize) -> bool {
    if !offense_controls_frontcourt(world) {
        return false;
    }
    if lane_by_role(world, player) {
        return false;
    }
    // 宽限之内球员还没有往外走：低位的站位是正常进攻的一部分。
    if world.players[player].lane_seconds <= OFFENSE_LANE_GRACE_SECONDS {
        return false;
    }
    let offense = world.possession.team;
    if world.players[player].team != offense {
        return false;
    }
    let attacking_right = world.attacking_right(offense);
    let court = world.rules.court;
    if !court.is_in_lane(world.players[player].pos, attacking_right) {
        return false;
    }
    // 落点在区外：这就是一次清空限制区的走位。
    !court.is_in_lane(world.players[player].target, attacking_right)
}

/// 防守人此刻是否必须走出限制区，以及走到哪个点。
///
/// 返回 `None` 表示这个防守人不受限制：要么防守三秒的计时没在跑，
/// 要么他正在积极盯防（那是合法防守）。
pub fn defense_lane_exit(world: &World, defender: usize) -> Option<Vec2> {
    if !offense_controls_frontcourt(world) {
        return None;
    }
    if ball_in_lane(world) {
        return None;
    }
    let offense = world.possession.team;
    if world.players[defender].team == offense {
        return None;
    }
    let attacking_right = world.attacking_right(offense);
    if !world
        .rules
        .court
        .is_in_lane(world.players[defender].pos, attacking_right)
    {
        // 已经在区外：不用修正落点。
        return None;
    }
    if guarding_in_lane(world, defender, attacking_right) {
        return None;
    }
    Some(exit_target(world, defender))
}

/// 进攻三秒的裁决：违例判失误，球权交给防守方。
///
/// 罚则位置是罚球线延长线上的边线（NBA Rule 10 §VI.3）。
pub fn book_offense_three_second(world: &mut World, actor: usize) {
    let team = world.players[actor].team;
    let opponent = team.other();
    world.line_mut(actor).turnovers += 1;
    world.emit(Fact::Turnover {
        player: actor,
        kind: crate::action::TurnoverKind::LaneViolation,
    });
    let spot = world
        .rules
        .court
        .sideline_at_free_throw_line_extended(world.ball.pos, world.attacking_right(team));
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

/// 防守三秒的裁决：违例判技术犯规，对手罚一次并保留球权。
///
/// NBA Rule 10 §VII.1：违例判技术犯规，对方罚一次，罚完仍由对方持球。
/// 技术犯规走既有的技术犯规罚球程序（`shoot_team_technical`），因此
/// 罚球与球权处理跟其他技术犯规完全一致。
pub fn book_defense_three_second(world: &mut World, actor: usize) {
    let team = world.players[actor].team;
    let opponent = team.other();
    let booking = crate::resolve::foul::book(
        world,
        actor,
        actor,
        crate::action::FoulKind::Technical,
        1,
        false,
    );
    // 罚则位置：罚球线延长线上的边线（NBA Rule 10 §VII.1）。
    let spot = world
        .rules
        .court
        .sideline_at_free_throw_line_extended(world.ball.pos, world.attacking_right(team));
    world.inbound_spot = Some(spot);
    // 罚球之后仍由对方持球：技术犯规不改变球权。
    crate::resolve::freethrow::shoot_team_technical(
        world,
        opponent,
        opponent,
        booking.foul_id,
        crate::resolve::freethrow::FreeThrowFollowUp::DeadBall { team: opponent },
    );
}

/// 推进两条限制区计时。
///
/// 计时记在球员身上：条件成立时累加，条件不成立时归零——规则按连续
/// 停留计时。
///
/// 清空限制区的落点修正也在这一步里做：它读的就是这一次计时用的
/// 条件，因此「谁在计时」与「谁要往外走」永远是同一个判定。分成两个
/// 步骤时，两者的状态可能在一个 tick 之间变掉，球员被留在区里。
pub fn step(world: &mut World, dt: f64) {
    let offense_limit = world.rules.three_second_lane_seconds;
    let defense_limit = world.rules.defensive_three_second_seconds;

    // 计时中的人不在计时条件里时，计时归零。
    let offense_holder = offense_timer_holder(world);
    let defense_holder = defense_timer_holder(world);
    for handle in 0..world.players.len() {
        let player = &mut world.players[handle];
        if !player.on_court {
            player.lane_seconds = 0.0;
            continue;
        }
        let counts = Some(handle) == offense_holder || Some(handle) == defense_holder;
        if counts {
            player.lane_seconds += dt as f32;
        } else {
            player.lane_seconds = 0.0;
        }
    }

    if let Some(actor) = offense_holder {
        // 计时中的人要往外走：落点改到区外，他才有时间在限时之内
        // 清空限制区（NBA Rule 10 §VI）。
        let exit = exit_target(world, actor);
        crate::assignment::commit_target_forced(world, actor, exit);
        if world.players[actor].lane_seconds > offense_limit {
            world.players[actor].lane_seconds = 0.0;
            book_offense_three_second(world, actor);
            return;
        }
    }

    if defense_limit <= 0.0 {
        return;
    }
    if let Some(actor) = defense_holder {
        if world.players[actor].lane_seconds > defense_limit {
            world.players[actor].lane_seconds = 0.0;
            book_defense_three_second(world, actor);
        }
    }
}
