//! 站位几何：开场站位、推进起点、弧顶坐标与镜像。
//!
//! 站位是数据（`data/calibration/formations.json`）驱动的：这里的函数只负责
//! 把数据取出来、按进攻方向镜像，缓存与清理交给世界状态。

use glam::Vec2;

use crate::action::TeamSide;
use crate::params::Formations;
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

/// 持球人推进的起点。
pub fn entry_point(world: &World, team: TeamSide) -> Vec2 {
    if world.attacking_right(team) {
        Vec2::new(28.0, 25.0)
    } else {
        Vec2::new(66.0, 25.0)
    }
}

/// 弧顶：推进的目标点，也是进攻发起的位置。
pub fn top_of_key(world: &World, team: TeamSide) -> Vec2 {
    if world.attacking_right(team) {
        Vec2::new(64.0, 25.0)
    } else {
        Vec2::new(30.0, 25.0)
    }
}

/// 按阵型把双方摆回起始站位，同时清掉速度与目标点。
pub fn reset_positions(world: &mut World, offense: TeamSide) {
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
        world.players[handle].pos = target;
        world.players[handle].vel = Vec2::ZERO;
        world.players[handle].target = target;
        world.players[handle].home_spot = target;
    }
}
