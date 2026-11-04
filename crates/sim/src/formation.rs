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

/// 快攻跑位：活球回合开始时，速度明显快于自己防守人的进攻球员直接跑向篮下。
///
/// 阵型的每一个目标点都定义在「已经进入前场」的假设上：防守篮板和抢断之后，
/// 五名进攻球员都在本方半场，全部走回阵型位置，于是快攻传球没有接球对象，
/// 只要回合在进攻时钟里推进得久一点，就全都变成落位进攻。这里给速度占优的
/// 球员一个篮下目标，窗口结束时由 `World::expire_break_targets` 把原来的站位还回去。
pub fn assign_break_targets(world: &mut World, offense: TeamSide, handler: usize) {
    let edge_needed = world.params.policy.g("break_speed_edge_ftps");
    let max_runners = world.params.policy.g("break_max_runners") as usize;
    if max_runners == 0 {
        return;
    }
    let mut runners: Vec<(usize, f32)> = Vec::new();
    for handle in 0..world.players.len() {
        let player = &world.players[handle];
        if !player.on_court || player.team != offense || handle == handler {
            continue;
        }
        let speed = player.effective_speed(&world.params.policy);
        let mut nearest_distance = f32::INFINITY;
        let mut nearest_speed = 0.0;
        for opponent in world.players.iter() {
            if !opponent.on_court || opponent.team == offense {
                continue;
            }
            let distance = (opponent.pos - player.pos).length();
            if distance < nearest_distance {
                nearest_distance = distance;
                nearest_speed = opponent.effective_speed(&world.params.policy);
            }
        }
        // 转换冲刺倾向：愿意冲的人更容易被选为快攻跑者。它加在速度差
        // 上，因此它改变的是「谁去冲」，不是「谁跑得快」。
        let sprint = player.data.tendencies.transition_sprint;
        let edge = speed - nearest_speed
            + (sprint - 0.5) * world.params.policy.g("break_sprint_tendency_gain_ftps");
        if edge >= edge_needed {
            runners.push((handle, edge));
        }
    }
    if runners.is_empty() {
        return;
    }
    runners.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(left.0.cmp(&right.0))
    });
    let hoop = world.hoop_for(offense);
    let back = if world.attacking_right(offense) {
        -1.0
    } else {
        1.0
    };
    for (slot, (handle, _)) in runners.into_iter().take(max_runners).enumerate() {
        // 两名跑向前场的球员分列篮框两侧，站得太近会互相挡住接球路线。
        let lateral = if slot.is_multiple_of(2) { 4.0 } else { -4.0 };
        let target = hoop + Vec2::new(back * 3.0, lateral);
        let previous = world.players[handle].home_spot;
        crate::assignment::force_target(world, handle, target);
        world.players[handle].home_spot = target;
        world.possession.break_return.push((handle, previous));
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
    // 外线点按站位本身判断：原始站点已经在三分线外的槽位才按外线处理。
    // 按槽位编号判断会把弧顶与两翼压回三分线内，接球投篮就都变成中距离。
    let is_perimeter = distance > arc;
    let shift = if is_perimeter {
        // 外线站位保留手写几何，只做小幅调整：三分偏好高的球员往外推一点，
        // 偏好低的最多退到弧线外侧 0.5 ft，不再退到弧内。
        let outward = (arc + 1.2 - distance).max(0.0) * (preference - center) * span;
        let inward = -((distance - arc - 0.5).max(0.0)) * (center - preference) * span;
        (outward + inward).max(-1.0)
    } else if preference < center {
        -(distance - rim).max(0.0) * (center - preference) * span
    } else {
        (arc - 0.5 - distance).max(0.0) * (preference - center) * span
    };
    let max_distance = if is_perimeter { arc + 3.0 } else { arc - 0.5 };
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
        // 跳球程序里的参跳人与接应人持有自己的点位：编队站位不能把它覆写
        // 回去，否则他们走不到圈边，掷球永远等不到人到位。
        //
        // 这道保护只在跳球程序进行中成立：比赛开始之后点位就失效了，
        // 否则场上永远有四个人被钉在圈边。
        if let Some(spot) = world
            .jump_ball_procedure_spots()
            .iter()
            .find(|(participant, _)| *participant == handle)
            .map(|(_, spot)| *spot)
        {
            crate::assignment::force_target_as(
                world,
                handle,
                spot,
                crate::state::TargetKind::Station,
                handle,
            );
            continue;
        }
        if world.pass_in_flight.is_some_and(|info| info.to == handle) {
            continue;
        }
        if world.any_interaction(|screen| {
            matches!(screen.kind, ActionKind::Screen(_)) && screen.target == Some(handle)
        }) {
            continue;
        }
        if world.any_interaction(|active| {
            matches!(active.kind, ActionKind::Cut(_)) && active.target == Some(handle)
        }) {
            continue;
        }
        // 球员自己发起的无球动作持有自己的落点：编队站位不能把它覆写回去，
        // 否则切入人永远到不了切入点，动作只能以失败结束。
        if let Some(own) = world.players[handle].action.as_ref().filter(|action| {
            action.intent.actor == handle
                && matches!(
                    action.intent.kind,
                    ActionKind::Cut(_) | ActionKind::Relocate
                )
        }) {
            let mut spot = own.intent.to_ft;
            // 跑位动作也不能把球员拉回限制区（NBA Rule 10 §VI）。
            if crate::lane::clearing_required(world, handle)
                && world
                    .rules
                    .court
                    .is_in_lane(world.players[handle].pos, world.attacking_right(offense))
            {
                spot = crate::lane::exit_target(world, handle);
            }
            crate::assignment::force_target_as(
                world,
                handle,
                spot,
                crate::state::TargetKind::Window,
                handle,
            );
            continue;
        }
        // 无球清空限制区：进攻方在前场控制球时，进攻人不能一直待在
        // 限制区里（NBA Rule 10 §VI）。阵型站位里的低位点与顺下点都在
        // 限制区内，因此站位必须按三秒计时修正。
        //
        // 修正是持久的：只要阵型站位本身落在限制区里，落点就一直是区外
        // 的那个点。只按「球员此刻在不在区里」修正会让球员走到区沿之后
        // 立刻又被阵型拉回去，计时因此停不下来。
        let formation_spot = spot_target_for(world, handle, &spots);
        let mut desired = formation_spot;
        // 清空限制区：进攻方在前场控制球时，限制区里的进攻人要往外走
        // （NBA Rule 10 §VI）。这道修正必须写在落点提交之前：它在运动
        // 积分之前生效，球员这一 tick 才真的朝区外走。
        if crate::lane::clearing_required(world, handle)
            && world
                .rules
                .court
                .is_in_lane(world.players[handle].pos, world.attacking_right(offense))
        {
            desired = crate::lane::exit_target(world, handle);
        }
        if world.players[handle].target_kind != crate::state::TargetKind::Contact {
            if desired != formation_spot {
                // 落点变了：绕开职责层的死区，这次提交必须让球员真的
                // 朝区外走。限制区里的球员离外沿本来就不到五英尺，用
                // 死区判定就等于把他钉在区里，计时永远停不下来。
                crate::assignment::commit_target_forced(world, handle, desired);
            } else {
                crate::assignment::commit_target(world, handle, desired);
            }
            crate::assignment::reselect_yielding(world, handle, desired);
        }
    }
    if world.pending_rebound.is_some() {
        let spot = crate::resolve::rebound::rebound_spot(world);
        let on_court_home = world.on_court_home;
        let on_court_away = world.on_court_away;
        for handle in on_court_home
            .as_slice()
            .iter()
            .chain(on_court_away.as_slice())
        {
            crate::assignment::force_target_as(
                world,
                *handle,
                spot,
                crate::state::TargetKind::Window,
                usize::MAX,
            );
        }
    }
    let min_separation = world.rules.min_player_separation_ft;
    let on_court_home = world.on_court_home;
    let on_court_away = world.on_court_away;
    for &handle in on_court_home
        .as_slice()
        .iter()
        .chain(on_court_away.as_slice())
    {
        // 正在等接球的人不能被绕行路径的里程上限打断：他的目标是球飞
        // 过去的那个落点，跑到一半被改成原地小步挪动就会把球漏掉。
        if world.pass_in_flight.is_some_and(|info| info.to == handle) {
            continue;
        }
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
