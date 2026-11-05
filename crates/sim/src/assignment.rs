//! 职责与目标点的提交。
//!
//! 目标点不是每 tick 重写的：候选目标先经过最小变化阈值与职责停留时间，
//! 只有越过阈值才提交。职责标签只在变化时更新，换人或球权变化等事件可
//! 以强制提交。

use glam::Vec2;

use crate::action::ActionKind;
use crate::state::{DefenseResponsibility, OffenseJob, Slot, TargetKind};
use crate::world::World;

pub fn commit_target(world: &mut World, handle: usize, desired: Vec2) -> bool {
    commit_target_as(world, handle, desired, TargetKind::Station, usize::MAX)
}

pub fn commit_target_as(
    world: &mut World,
    handle: usize,
    desired: Vec2,
    kind: TargetKind,
    partner: usize,
) -> bool {
    if kind == TargetKind::Pressure && (partner == usize::MAX || !world.players[partner].on_court) {
        return false;
    }
    let deadband = if kind == TargetKind::Pressure {
        0.2
    } else {
        world.params.policy.g("assignment_deadband_ft")
    };
    let dwell = if kind == TargetKind::Pressure {
        0.1
    } else {
        f64::from(world.params.policy.g("assignment_dwell_seconds"))
    };
    let max_step = world.params.policy.g("assignment_max_step_ft");
    let now = world.now();
    let (current, changed_at) = {
        let player = &world.players[handle];
        (player.target, player.target_changed_at)
    };
    let delta = desired - current;
    if delta.length_squared() <= deadband * deadband {
        return false;
    }
    if now - changed_at < dwell {
        return false;
    }
    let mut candidate = current + delta.clamp_length_max(max_step);
    if !validate_target(world, handle, candidate, kind, partner) {
        if validate_target(world, handle, desired, kind, partner) {
            candidate = desired;
        } else if let Some(alt) = alternative_target(world, handle, desired, kind, partner) {
            candidate = alt;
        } else {
            let player = &mut world.players[handle];
            player.invalid_commits += 1;
            player.invalid_commits_by_kind[kind.index()] += 1;
            return false;
        }
    }
    let player = &mut world.players[handle];
    player.target = candidate;
    player.target_kind = kind;
    player.target_partner = partner;
    player.target_changed_at = now;
    player.progress_best_ft = f32::MAX;
    player.nonprogress_seconds = 0.0;
    player.yielding_since = 0.0;
    player.recovery_path_ft = 0.0;
    true
}

/// 事件触发的提交：球权变化、掩护接触、换人后立即生效。
pub fn force_target(world: &mut World, handle: usize, desired: Vec2) {
    force_target_as(world, handle, desired, TargetKind::Station, usize::MAX);
}

pub fn force_target_as(
    world: &mut World,
    handle: usize,
    desired: Vec2,
    kind: TargetKind,
    partner: usize,
) -> bool {
    let mut target = desired;
    if !validate_target(world, handle, target, kind, partner) {
        if kind == TargetKind::Station {
            if let Some(alt) = alternative_target(world, handle, desired, kind, partner) {
                target = alt;
            } else {
                return false;
            }
        } else {
            return false;
        }
    }
    let now = world.now();
    let player = &mut world.players[handle];
    player.target = target;
    player.target_kind = kind;
    player.target_partner = partner;
    player.target_changed_at = now;
    player.progress_best_ft = f32::MAX;
    player.nonprogress_seconds = 0.0;
    player.yielding_since = 0.0;
    player.recovery_path_ft = 0.0;
    true
}

/// 提交一个必须生效的落点：落点被占时找一个替代点。
///
/// 走位修正需要这种提交：落点被占时如果直接放弃，球员会一直卡在
/// 原地，目标形同虚设。替代点按八个方向由近及远找。
pub fn commit_target_forced(world: &mut World, handle: usize, desired: Vec2) -> bool {
    let target = if validate_target(world, handle, desired, TargetKind::Station, usize::MAX) {
        Some(desired)
    } else {
        alternative_target(world, handle, desired, TargetKind::Station, usize::MAX)
    };
    let Some(target) = target else {
        return false;
    };
    // 落点被占时用替代点，替代点离得远，死区会把它判成「没有变化」。
    // 因此这里绕开死区：这次提交必须让球员真的朝落点走。
    let now = world.now();
    let player = &mut world.players[handle];
    let moved = (player.target - target).length() > 0.05;
    player.target = target;
    player.target_kind = TargetKind::Station;
    player.target_partner = usize::MAX;
    player.target_changed_at = now;
    if moved {
        player.progress_best_ft = f32::MAX;
        player.nonprogress_seconds = 0.0;
        player.yielding_since = 0.0;
        player.recovery_path_ft = 0.0;
    }
    true
}

pub fn pressure_separation(world: &World) -> f32 {
    world.params.policy.g("pressure_separation_ft")
}

pub fn mark_target(world: &mut World, handle: usize, kind: TargetKind, partner: usize) {
    let player = &mut world.players[handle];
    player.target_kind = kind;
    player.target_partner = partner;
}

pub fn station_target_valid(world: &World, handle: usize, target: Vec2) -> bool {
    let radius = world.rules.player_radius_ft;
    if target.x < radius
        || target.y < radius
        || target.x > world.rules.court.width_ft - radius
        || target.y > world.rules.court.height_ft - radius
    {
        return false;
    }
    let _min_separation = world.rules.min_player_separation_ft;
    let _team = world.players[handle].team;
    for (other, player) in world.players.iter().enumerate() {
        if other == handle || !player.on_court {
            continue;
        }
        // 站位目标只需要不落在别人身体里：3.6 英尺的间隔是对位压迫的
        // 要求，不是站位点的要求；站进对手身体才算不合法。
        let limit = radius * 2.0;
        if (player.pos - target).length() < limit {
            return false;
        }
    }
    true
}

fn validate_target(
    world: &World,
    handle: usize,
    target: Vec2,
    kind: TargetKind,
    partner: usize,
) -> bool {
    let radius = world.rules.player_radius_ft;
    if target.x < radius
        || target.y < radius
        || target.x > world.rules.court.width_ft - radius
        || target.y > world.rules.court.height_ft - radius
    {
        return false;
    }
    match kind {
        TargetKind::Station => station_target_valid(world, handle, target),
        TargetKind::Pressure => {
            if partner == usize::MAX || !world.players[partner].on_court {
                return false;
            }
            // 所有压力目标都由同一个间隔参数生成：对位、拒绝接球、卡位、
            // 过掩护与换防的站位都是「贴着对手一个身位」。协防目标故意
            // 离开自己的人，因此按自己的区间校验。
            let separation = pressure_separation(world);
            let distance = (target - world.players[partner].pos).length();
            if world.players[handle].defense_responsibility
                == crate::state::DefenseResponsibility::Help
            {
                (0.5..=12.0).contains(&distance)
            } else {
                distance >= separation - 0.25 && distance <= separation + 2.5
            }
        }
        TargetKind::Contact => {
            if partner == usize::MAX || !world.players[partner].on_court {
                return false;
            }
            let reach = world.rules.player_radius_ft
                + world.rules.body_contact_radius_ft
                + world.rules.contact_margin_ft;
            let distance = (target - world.players[partner].pos).length();
            distance <= reach && distance >= world.rules.min_player_separation_ft - 0.1
        }
        TargetKind::Window => true,
    }
}

pub fn pressure_target_valid(world: &World, handle: usize, partner: usize, target: Vec2) -> bool {
    validate_target(world, handle, target, TargetKind::Pressure, partner)
}

fn alternative_target(
    world: &World,
    handle: usize,
    desired: Vec2,
    kind: TargetKind,
    partner: usize,
) -> Option<Vec2> {
    const DIRECTIONS: [(f32, f32); 8] = [
        (1.0, 0.0),
        (-1.0, 0.0),
        (0.0, 1.0),
        (0.0, -1.0),
        (0.707, 0.707),
        (-0.707, 0.707),
        (0.707, -0.707),
        (-0.707, -0.707),
    ];
    let offset = world.rules.min_player_separation_ft + world.rules.separation_safety_margin_ft;
    if kind == TargetKind::Pressure && partner != usize::MAX && world.players[partner].on_court {
        let separation =
            if world.players[handle].defense_responsibility == DefenseResponsibility::Deny {
                pressure_separation(world)
            } else {
                world.rules.min_player_separation_ft
            };
        let center = world.players[partner].pos;
        for radius in [separation, separation + 0.5, separation + 1.0] {
            for (x, y) in DIRECTIONS {
                let candidate = center + Vec2::new(x, y) * radius;
                if validate_target(world, handle, candidate, kind, partner) {
                    return Some(candidate);
                }
            }
        }
    }
    for factor in [1.0_f32, 2.0, 3.0] {
        for (x, y) in DIRECTIONS {
            let candidate = desired + Vec2::new(x, y) * offset * factor;
            if validate_target(world, handle, candidate, kind, partner) {
                return Some(candidate);
            }
        }
    }
    None
}

/// 被挡住超过 0.5 s 后，由职责层在同一个职责下换一个可达点；
/// 底层运动积分只负责等待和绕行，不能自行改写职责。
pub fn reselect_yielding(world: &mut World, handle: usize, desired: Vec2) -> bool {
    let now = world.now();
    let player = &world.players[handle];
    if player.yielding_since == 0.0 || now - player.yielding_since < 0.5 {
        return false;
    }
    let blocker = player.avoidance_blocker;
    let side = if player.avoidance_side == 0 {
        1
    } else {
        player.avoidance_side
    };
    let kind = player.target_kind;
    let partner = player.target_partner;
    let lateral = if blocker != usize::MAX && world.players[blocker].on_court {
        let away = (player.pos - world.players[blocker].pos).normalize_or_zero();
        away.perp() * f32::from(side)
    } else {
        Vec2::ZERO
    };
    let width = world.rules.court.width_ft;
    let height = world.rules.court.height_ft;
    let radius = world.rules.player_radius_ft;
    let offset = world.rules.min_player_separation_ft + world.rules.separation_safety_margin_ft;
    let mut target = desired + lateral * offset;
    if !validate_target(world, handle, target, kind, partner) {
        const DIRECTIONS: [(f32, f32); 8] = [
            (1.0, 0.0),
            (-1.0, 0.0),
            (0.0, 1.0),
            (0.0, -1.0),
            (0.707, 0.707),
            (-0.707, 0.707),
            (0.707, -0.707),
            (-0.707, -0.707),
        ];
        for (x, y) in DIRECTIONS {
            let candidate = desired + Vec2::new(x, y) * offset;
            if validate_target(world, handle, candidate, kind, partner) {
                target = candidate;
                break;
            }
        }
    }
    let target = target.clamp(
        Vec2::new(radius, radius),
        Vec2::new(width - radius, height - radius),
    );
    if !force_target_as(world, handle, target, kind, partner)
        && !force_target_as(
            world,
            handle,
            world.players[handle].pos,
            crate::state::TargetKind::Window,
            usize::MAX,
        )
    {
        return false;
    }
    let actual = world.players[handle].target;
    let distance = (actual - world.players[handle].pos).length();
    let player = &mut world.players[handle];
    player.recovery_since = now;
    player.recovery_d0_ft = distance;
    player.recovery_path_ft = 0.0;
    player.yielding_since = 0.0;
    player.avoidance_blocker = usize::MAX;
    player.nonprogress_seconds = 0.0;
    player.progress_best_ft = f32::MAX;
    true
}

pub fn assign_offense_jobs(world: &mut World) {
    let offense = world.possession.team;
    let holder = world.holder();
    let now = world.now();
    let dwell = f64::from(world.params.policy.g("assignment_dwell_seconds"));
    for &handle in world.on_court(offense).as_slice() {
        let job = if Some(handle) == holder {
            OffenseJob::Handler
        } else if world.any_interaction(|active| {
            matches!(active.kind, ActionKind::Screen(_)) && active.target == Some(handle)
        }) {
            OffenseJob::Screen
        } else if world.any_interaction(|active| {
            matches!(active.kind, ActionKind::Cut(_)) && active.target == Some(handle)
        }) || world.possession.last_cut_actor == Some(handle)
        {
            OffenseJob::Cut
        } else if world
            .possession
            .last_screen
            .as_ref()
            .is_some_and(|screen| screen.screener == handle)
        {
            OffenseJob::Roll
        } else if world.ball_flight.is_some_and(|flight| {
            matches!(
                flight.kind,
                crate::state::FlightKind::Shot | crate::state::FlightKind::FreeThrow
            )
        }) && (world.players[handle].pos - world.hoop_for(offense)).length() < 12.0
        {
            OffenseJob::Rebound
        } else {
            match world.players[handle].slot {
                Slot::Post => OffenseJob::PostUp,
                Slot::DunkerSpot => OffenseJob::Roll,
                _ => OffenseJob::SpotUp,
            }
        };
        let player = &mut world.players[handle];
        if player.offense_job != job && now - player.assignment_since >= dwell {
            player.offense_job = job;
            player.assignment_since = now;
            player.recovery_since = 0.0;
            player.recovery_d0_ft = 0.0;
        }
    }
}

pub fn assign_defense_responsibilities(world: &mut World, targets: &[Vec2]) {
    let defense = world.possession.team.other();
    let defenders = world.on_court(defense);
    let carrier = world
        .holder()
        .or_else(|| world.pass_in_flight.map(|info| info.to));
    let hoop = world.hoop_for(world.possession.team);
    let now = world.now();
    let dwell = f64::from(world.params.policy.g("assignment_dwell_seconds"));
    let responsibility_dwell = dwell.max(12.0);
    for (slot, handle) in defenders.as_slice().iter().enumerate() {
        let man = world.possession.matchup[slot];
        let current = world.players[*handle].defense_responsibility;
        let help_threshold = if current == DefenseResponsibility::Help {
            9.0
        } else {
            7.5
        };
        let sink_threshold = if current == DefenseResponsibility::Sink {
            11.0
        } else {
            9.5
        };
        let responsibility = if Some(man) == carrier {
            // 掩护刚结算完：按那次结算的路线决定怎么过掩护。
            let nav_window = world.params.policy.g("screen_nav_seconds") as f64;
            match world.possession.last_screen {
                Some(read) if now - read.at < nav_window => match read.route {
                    crate::action::DefenderRoute::Over => DefenseResponsibility::GoOver,
                    crate::action::DefenderRoute::Under => DefenseResponsibility::GoUnder,
                    crate::action::DefenderRoute::Switch => DefenseResponsibility::Switch,
                    crate::action::DefenderRoute::Trapped => DefenseResponsibility::OnBall,
                },
                _ => DefenseResponsibility::OnBall,
            }
        } else if world
            .possession
            .last_offensive_rebound_at
            .map(|t| now - t < 1.5)
            .unwrap_or(false)
            && (world.players[*handle].pos - hoop).length_squared() < 144.0
        {
            DefenseResponsibility::BoxOut
        } else if world.pending_rebound.is_some_and(|rebound| {
            !rebound.turnover
                && rebound.offense != world.players[*handle].team
                && rebound.box_outs[..rebound.box_out_count as usize].contains(handle)
        }) {
            // 防守篮板：这一球上去卡位的人就是卡位责任人。
            // 没有这条职责，防守目标层每 tick 都会把卡位目标改回站位点，
            // 卡位能力因此完全不起作用。
            DefenseResponsibility::BoxOut
        } else if (targets[slot] - hoop).length_squared() < help_threshold * help_threshold {
            DefenseResponsibility::Help
        } else if man != usize::MAX
            && (world.players[man].pos - hoop).length_squared() < sink_threshold * sink_threshold
        {
            DefenseResponsibility::Sink
        } else if man != usize::MAX
            && (world.players[man].pos - world.players[*handle].pos).length()
                > world.params.policy.g("recover_distance_ft")
        {
            // 被过掉或换防后期在人不在：先回到自己的对位人身上。
            DefenseResponsibility::Recover
        } else {
            DefenseResponsibility::Deny
        };
        let player = &mut world.players[*handle];
        // 对位刚换人时立刻改职责：对位表刚被重算，负责新持球人的防守人
        // 必须马上接手，否则会出现好几个人同时是「对位防守人」。
        let pairing_changed = world.possession.pairing_changed_at > player.assignment_since
            && (responsibility == DefenseResponsibility::OnBall
                || player.defense_responsibility == DefenseResponsibility::OnBall);
        if player.defense_responsibility != responsibility
            && (pairing_changed || now - player.assignment_since >= responsibility_dwell)
        {
            let from = player.defense_responsibility.as_str();
            player.defense_responsibility = responsibility;
            player.assignment_since = now;
            player.recovery_since = 0.0;
            player.recovery_d0_ft = 0.0;
            world.emit(crate::action::Fact::DefenseResponsibilityChanged {
                player: *handle,
                from,
                to: responsibility.as_str(),
            });
        }
    }
}
