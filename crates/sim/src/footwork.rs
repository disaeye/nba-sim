//! 持球人的脚步判定：轴心脚与接触状态驱动的走步与二次运球。
//!
//! 判定按「收球 → 轴心脚 → 脚步接触 → 释放」的先后走：
//!
//! - 收球：运球结束、球被双手控制的那一刻，确立轴心脚。
//! - 轴心脚：收球时按球手朝向确定，此后不再改变。移动轴心脚就是走步。
//! - 脚步接触：收球之后球手每走满一步的量程算一次接触，两步用尽。
//! - 释放：出手或传球结束这次持球段。
//!
//! 收球后的平面位移不再一次性判定全部走步。位移只是脚步接触的结果：
//! 两步的量程仍然由位移给出，但判定的是脚而不是平面距离。

use glam::Vec2;
use nba_domain::footwork::PivotState;

use crate::action::{Fact, TurnoverKind};
use crate::world::World;

/// 脚步接触的距离容差（英尺）。
///
/// 浮点加减会让「正好走满一步」算成差一点点。没有容差时脚步永远记不满，
/// 收球后的两步额度因此停不下来。
const CONTACT_EPSILON_FT: f32 = 0.05;

/// 收球时确立轴心脚与脚步状态。
///
/// 轴心脚就是收球瞬间地上的那只支撑脚：支撑脚状态在每次脚步接触时在
/// 左右之间交替，收球时读它定下轴心。脚步额度来自参数表。收球之后
/// 轴心与额度都不再改变，除非球员重新运球或球权易手。
///
/// `position` 是收球的位置：突破在触球点结束收球，从那个点开始算步。
pub fn establish_pivot(world: &mut World, actor: usize, position: Vec2) {
    let mut support = world.possession.support_foot;
    support.planted_at = position;
    support.last_contact = position;
    let foot = support.foot;
    world.possession.support_foot = support;
    let steps_allowed = world.params.policy.g("footwork_steps_allowed") as u8;
    world.possession.pivot = Some(PivotState::new(foot, position, steps_allowed));
    world.possession.pivot_holder = Some(actor);
}

/// 清除轴心状态：球权易手、重新运球或回合结束时调用。
pub fn clear_pivot(world: &mut World) {
    world.possession.pivot = None;
    world.possession.pivot_holder = None;
}

/// 当前球员一步的量程（英尺）。
pub fn step_length(world: &World, actor: usize) -> f32 {
    nba_domain::footwork::step_length_ft(
        world.players[actor].data.height_cm,
        world.params.policy.g("footwork_step_base_ft"),
        world.params.policy.g("footwork_step_per_cm_ft"),
        world.params.policy.g("footwork_step_cap_ft"),
    )
}

/// 收球后球手移动时累积脚步接触。
///
/// 每走满一步的量程算一次接触，接触记进轴心状态。支撑脚在左右之间
/// 交替：收球确立的轴心脚不动，抬起的是另一只脚。脚步用尽之后还继续
/// 走就是走步：判定的是脚，不是平面位移。
///
/// 返回是否构成走步：走步是移动当下就成立的违例，不是等到出手时才
/// 追认。
pub fn accumulate_contact(world: &mut World, actor: usize) -> bool {
    let Some(pivot) = world.possession.pivot else {
        return false;
    };
    if world.possession.pivot_holder != Some(actor) {
        return false;
    }
    let step = step_length(world, actor);
    if step <= f32::MIN_POSITIVE {
        return false;
    }
    let last = pivot.last_contact.unwrap_or(pivot.position);
    let travelled = (world.players[actor].pos - last).length();
    // 容差：浮点加减会让「正好走满一步」算成差一点点，脚步因此永远
    // 记不满，计时与走步判定都停在半步之外。
    if travelled < step - CONTACT_EPSILON_FT {
        return false;
    }
    let contact = world.players[actor].pos;
    let mut pivot = pivot;
    // 支撑脚交替：这一步是另一只脚落地的。
    let mut support = world.possession.support_foot;
    support.record_contact(contact);
    let landing_foot = support.foot;
    world.possession.support_foot = support;
    // 跳步：连续两次同一只脚触地（NBA Rule 10 §XIII.8）。
    pivot.record_contact_with_foot(contact, landing_foot);
    let hop = pivot.hop;
    // 脚步用尽之后的这一步就是走步：前面的接触已经用掉了额度。
    let exhausted = !pivot.has_step_left();
    world.possession.pivot = Some(pivot);
    exhausted || hop
}

/// 释放（出手或传球）时的走步判定。
///
/// 判定按「收球 → 轴心脚 → 脚步接触 → 释放」的顺序：收球确立轴心，
/// 移动累积脚步接触，释放时结算。收球后移动了轴心脚、或者脚步用尽，
/// 都是走步。
pub fn judge_release(world: &World, actor: usize) -> FootworkVerdict {
    let Some(pivot) = world.possession.pivot else {
        return FootworkVerdict::Legal;
    };
    if world.possession.pivot_holder != Some(actor) {
        return FootworkVerdict::Legal;
    }
    // 脚步用尽：收球之后的两步已经走完。
    if pivot.steps_taken > pivot.steps_allowed {
        return FootworkVerdict::StepsExhausted;
    }
    // 跳步：连续两次同一只脚触地（NBA Rule 10 §XIII.8）。
    if pivot.hop {
        return FootworkVerdict::Hop;
    }
    // 轴心脚被移动：收球之后球手离开了轴心脚的位置。
    let step = step_length(world, actor);
    let reach = f32::from(pivot.steps_allowed) * step;
    if (world.players[actor].pos - pivot.position).length() > reach {
        return FootworkVerdict::PivotMoved;
    }
    FootworkVerdict::Legal
}

/// 收球后再次运球：抬起轴心脚就是移动了球权。
///
/// 二次运球按「收球 → 轴心脚 → 释放」的先后判定：收球确立了轴心脚，
/// 之后球手再运球就是抬起轴心脚。
pub fn dribble_after_gather(world: &World, actor: usize) -> bool {
    if world.possession.dribble_state != crate::state::DribbleState::Gathered {
        return false;
    }
    world.possession.pivot_holder == Some(actor)
}

/// 收球步的判定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FootworkVerdict {
    /// 合法：还有脚步，且没有移动轴心脚。
    Legal,
    /// 走步：移动了轴心脚。
    PivotMoved,
    /// 走步：脚步数用尽。
    StepsExhausted,
    /// 跳步：连续两次用同一只脚触地（NBA Rule 10 §XIII.8）。
    Hop,
}

/// 判定收球步是否合法。
///
/// `gather` 是收球位置，`finish` 是收球步的落点，`extra_ft` 是额外的
/// 合法量程（欧洲步的侧向一步）。落点离收球位置超过允许的步数量程就
/// 是走步：这是脚步接触的结果，不是位移本身。
pub fn judge_gather(
    world: &World,
    actor: usize,
    gather: Vec2,
    finish: Vec2,
    extra_ft: f32,
) -> FootworkVerdict {
    let Some(pivot) = world.possession.pivot else {
        // 没有轴心状态（测试夹具或旧回合）：按位移判定。
        return FootworkVerdict::Legal;
    };
    if world.possession.pivot_holder != Some(actor) {
        return FootworkVerdict::Legal;
    }
    let step = step_length(world, actor);
    let allowed = f32::from(pivot.steps_allowed) * step + extra_ft;
    // 脚步用尽：收球之后已经用掉的步数加上了这一次落点，超过额度。
    if pivot.steps_taken >= pivot.steps_allowed {
        return FootworkVerdict::StepsExhausted;
    }
    // 轴心脚被移动：收球步的落点离轴心脚原位太远，说明抬起的那只脚
    // 带着重心走了。
    if (finish - pivot.position).length() > allowed {
        return FootworkVerdict::PivotMoved;
    }
    // 落点比允许的步数量程更远：这一步走得太远。
    if (finish - gather).length() > allowed {
        return FootworkVerdict::StepsExhausted;
    }
    FootworkVerdict::Legal
}

/// 走步的账本与裁决。
/// 跳步的账本与裁决（NBA Rule 10 §XIII.8）。
///
/// 跳步与走步是两条不同的违例：走步是脚步用尽或移动了轴心脚，跳步是
/// 连续两次用同一只脚触地。罚则相同：球权交给对方。
pub fn book_hop(world: &mut World, actor: usize, team: crate::TeamSide) {
    world.line_mut(actor).turnovers += 1;
    world.emit(Fact::Turnover {
        player: actor,
        kind: TurnoverKind::Travel,
    });
    world.adjudicate(
        crate::adjudicate::Adjudication::new(
            crate::adjudicate::StoppageCause::Violation(crate::adjudicate::ViolationKind::Travel),
            crate::adjudicate::RestartPlan::Live { team: team.other() },
        )
        .ending(nba_domain::event::PossessionEndCause::TurnoverTravel, "hop"),
    );
}

pub fn book_travel(world: &mut World, actor: usize, team: crate::TeamSide) {
    world.line_mut(actor).turnovers += 1;
    world.emit(Fact::Turnover {
        player: actor,
        kind: TurnoverKind::Travel,
    });
    world.adjudicate(
        crate::adjudicate::Adjudication::new(
            crate::adjudicate::StoppageCause::Violation(crate::adjudicate::ViolationKind::Travel),
            crate::adjudicate::RestartPlan::Live { team: team.other() },
        )
        .ending(
            nba_domain::event::PossessionEndCause::TurnoverTravel,
            "travel",
        ),
    );
}
