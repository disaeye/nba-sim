//! 站位几何：开场站位、推进起点、弧顶坐标与镜像。
//!
//! 站位是数据（`data/calibration/formations.json`）驱动的：这里的函数只负责
//! 把数据取出来、按进攻方向镜像，缓存与清理交给世界状态。

use glam::Vec2;

use crate::action::{ActionKind, TeamSide};
use crate::params::Formations;
use crate::state::{PlayerState, Slot};
use crate::world::World;

pub fn offense_spots(formations: &Formations, attacking_right: bool, alternate: bool) -> [Vec2; 5] {
    let source = if alternate && !formations.offense_alt.is_empty() {
        &formations.offense_alt
    } else {
        &formations.offense
    };
    let spots = [
        Vec2::from(source[0]),
        Vec2::from(source[1]),
        Vec2::from(source[2]),
        Vec2::from(source[3]),
        Vec2::from(source[4]),
    ];
    mirror(spots, attacking_right)
}

pub fn defense_spots(formations: &Formations, attacking_right: bool) -> [Vec2; 5] {
    let spots = [
        Vec2::from(formations.defense[0]),
        Vec2::from(formations.defense[1]),
        Vec2::from(formations.defense[2]),
        Vec2::from(formations.defense[3]),
        Vec2::from(formations.defense[4]),
    ];
    mirror(spots, attacking_right)
}

fn mirror(spots: [Vec2; 5], attacking_right: bool) -> [Vec2; 5] {
    if attacking_right {
        spots
    } else {
        spots.map(|point| Vec2::new(94.0 - point.x, 50.0 - point.y))
    }
}

/// 弧顶：推进的目标点，也是进攻发起的位置。
pub fn top_of_key(world: &World, team: TeamSide) -> Vec2 {
    // 弧顶持球位取自阵型的持球点：它必须在三分线内，否则持球人的每次
    // 急停跳投都是三分球，进攻的区域分布整个塌到外线。
    let spots = offense_spots(
        &world.params.formations,
        world.attacking_right(team),
        world.alternate_spacing,
    );
    spots[0]
}

/// 按阵型把双方摆到开场站位。只在构造世界时调用一次；比赛中的位置
/// 变化全部由 `body::integrate` 产生。
pub fn place_opening_positions(world: &mut World, offense: TeamSide) {
    let spots = offense_spots(
        &world.params.formations,
        world.attacking_right(offense),
        world.alternate_spacing,
    );
    let defense = defense_spots(&world.params.formations, world.attacking_right(offense));
    let mut offense_index = 0;
    let mut defense_index = 0;
    let mut placements = Vec::new();
    for handle in 0..world.players.len() {
        if !world.players[handle].on_court {
            continue;
        }
        let is_offense = world.players[handle].team == offense;
        let target = if is_offense {
            let spot = spots[offense_index.min(spots.len() - 1)];
            offense_index += 1;
            spot
        } else {
            let spot = defense[defense_index.min(defense.len() - 1)];
            defense_index += 1;
            spot
        };
        placements.push((handle, target));
    }
    for (handle, target) in placements {
        crate::body::place(world, handle, target);
    }
}

/// 给双方重新分配阵型目标点。不写位置；球员从当前位置走向新目标。
pub fn set_targets(world: &mut World, offense: TeamSide) {
    let spots = offense_spots(
        &world.params.formations,
        world.attacking_right(offense),
        world.alternate_spacing,
    );
    let defense = defense_spots(&world.params.formations, world.attacking_right(offense));
    let mut offense_index = 0;
    let mut defense_index = 0;
    for handle in 0..world.players.len() {
        if !world.players[handle].on_court {
            continue;
        }
        let target = if world.players[handle].team == offense {
            let spot = spots[offense_index.min(spots.len() - 1)];
            offense_index += 1;
            spot
        } else {
            let spot = defense[defense_index.min(defense.len() - 1)];
            defense_index += 1;
            spot
        };
        crate::assignment::force_target(world, handle, target);
        world.players[handle].home_spot = target;
    }
}

/// 替补席位置。替补在场边等待，换人后从场边走向目标点。
pub fn bench_spot(world: &World, team: TeamSide, bench_index: usize) -> Vec2 {
    let width = world.rules.court.width_ft;
    let height = world.rules.court.height_ft;
    let offset = 2.0 + bench_index as f32 * 1.5;
    match team {
        TeamSide::Home => Vec2::new(2.0, offset),
        TeamSide::Away => Vec2::new(width - 2.0, height - offset),
    }
}

pub fn assign_slots(players: &mut [PlayerState], team: TeamSide) {
    let mut used = [false; 5];
    for player in players.iter_mut().filter(|p| p.on_court && p.team == team) {
        player.slot = slot_for_position(player.data.position, &mut used);
    }
}

fn slot_for_position(position: nba_domain::PlayerPosition, used: &mut [bool; 5]) -> Slot {
    let order: [Slot; 5] = match position {
        nba_domain::PlayerPosition::Point | nba_domain::PlayerPosition::Combo => [
            Slot::Handler,
            Slot::Wing,
            Slot::Corner,
            Slot::Post,
            Slot::DunkerSpot,
        ],
        nba_domain::PlayerPosition::Wing => [
            Slot::Wing,
            Slot::Corner,
            Slot::Handler,
            Slot::Post,
            Slot::DunkerSpot,
        ],
        nba_domain::PlayerPosition::Forward => [
            Slot::Corner,
            Slot::Post,
            Slot::Wing,
            Slot::DunkerSpot,
            Slot::Handler,
        ],
        nba_domain::PlayerPosition::Big => [
            Slot::Post,
            Slot::DunkerSpot,
            Slot::Corner,
            Slot::Wing,
            Slot::Handler,
        ],
        nba_domain::PlayerPosition::Center => [
            Slot::DunkerSpot,
            Slot::Post,
            Slot::Corner,
            Slot::Wing,
            Slot::Handler,
        ],
    };
    for slot in order {
        let index = slot_index(slot);
        if !used[index] {
            used[index] = true;
            return slot;
        }
    }
    Slot::DunkerSpot
}

fn slot_index(slot: Slot) -> usize {
    match slot {
        Slot::Handler => 0,
        Slot::Wing => 1,
        Slot::Corner => 2,
        Slot::Post => 3,
        Slot::DunkerSpot => 4,
        Slot::Top => 0,
    }
}

pub fn spot_target(world: &World, player: usize) -> Vec2 {
    let team = world.players[player].team;
    let spots = offense_spots(
        &world.params.formations,
        world.attacking_right(team),
        world.alternate_spacing,
    );
    spot_target_for(world, player, &spots)
}

pub fn spot_target_for(world: &World, player: usize, spots: &[Vec2; 5]) -> Vec2 {
    let team = world.players[player].team;
    let slot = world.players[player].slot;
    let index = match slot {
        Slot::Handler => 0,
        Slot::Wing => 1,
        Slot::Corner => 2,
        Slot::Post => 3,
        Slot::DunkerSpot => 4,
        Slot::Top => 0,
    };
    let base = spots[index];
    let hoop = world.hoop_for(team);
    let offset = base - hoop;
    let distance = offset.length();
    let preference = world.players[player].data.tendencies.three_point_preference;
    let rim = world.params.policy.g("spacing_rim_ft");
    let center = world.params.policy.g("spacing_preference_center");
    let span = world.params.policy.g("spacing_preference_span");
    let arc = world.rules.league.three_point_distance_ft;
    let is_perimeter = index == 2;
    let shift = if is_perimeter && preference > center {
        (arc + 1.2 - distance).max(0.0) * (preference - center) * span
    } else if preference < center {
        -(distance - rim).max(0.0) * (center - preference) * span
    } else {
        (arc - 0.5 - distance).max(0.0) * (preference - center) * span
    };
    let max_distance = if is_perimeter && preference > center {
        arc + 1.5
    } else {
        arc - 0.5
    };
    let spaced = (distance + shift).clamp(rim, max_distance);
    hoop + offset.normalize_or_zero() * spaced
}

pub fn move_toward_targets(world: &mut World) {
    crate::defense::refresh_matchup(world);
    crate::defense::apply_defense_targets(world);
    crate::assignment::assign_offense_jobs(world);
    let offense = world.possession.team;
    let holder = world.holder();
    let spots = offense_spots(
        &world.params.formations,
        world.attacking_right(offense),
        world.alternate_spacing,
    );
    for &handle in world.on_court(offense).as_slice() {
        if Some(handle) == holder {
            continue;
        }
        if world
            .pass_in_flight
            .is_some_and(|info| info.to == handle)
        {
            continue;
        }
        if world
            .active
            .as_ref()
            .is_some_and(|screen| matches!(screen.kind, ActionKind::Screen(_)) && screen.target == Some(handle))
        {
            continue;
        }
        if world
            .active
            .as_ref()
            .is_some_and(|active| matches!(active.kind, ActionKind::Cut) && active.target == Some(handle))
        {
            continue;
        }
        let desired = spot_target_for(world, handle, &spots);
        if world.players[handle].target_kind != crate::state::TargetKind::Contact {
            crate::assignment::commit_target(world, handle, desired);
            crate::assignment::reselect_yielding(world, handle, desired);
        }
    }
    let min_separation = world.rules.min_player_separation_ft;
    let on_court_home = world.on_court_home;
    let on_court_away = world.on_court_away;
    for &handle in on_court_home.as_slice().iter().chain(on_court_away.as_slice()) {
        let player = &world.players[handle];
        let target_distance = (player.target - player.pos).length();
        let cap = target_distance + 2.0 * std::f32::consts::PI * min_separation;
        if (player.recovery_since > 0.0 || target_distance > min_separation)
            && player.recovery_path_ft > cap * 0.9
        {
            let pos = player.pos + Vec2::new(0.25, 0.0);
            crate::assignment::force_target_as(
                world,
                handle,
                pos,
                crate::state::TargetKind::Window,
                usize::MAX,
            );
            let player = &mut world.players[handle];
            player.recovery_since = 0.0;
            player.recovery_d0_ft = 0.0;
            player.recovery_path_ft = 0.0;
        }
    }
}
