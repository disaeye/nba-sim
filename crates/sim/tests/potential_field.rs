use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_sim_core::defense::{refresh_matchup, solve_defense_targets, UNASSIGNED};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::{Phase, MAX_ON_COURT};
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};

fn world(seed: u64) -> World {
    let mut params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    // 这些测试用瞬时摆位探测势能场本身，关掉读球延迟：延迟下的
    // 球位与摆位不一致会把几何断言变成时延断言。
    params
        .policy
        .geometry
        .insert("defense_reaction_delay_seconds".to_string(), 0.0);
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

/// 把进攻与防守摆成固定的五外站位。势能场测试探测的是几何响应，
/// 不该受采样回合恰好落在哪个瞬时队形影响。
fn spread_five_out(world: &mut World, carrier: usize) {
    let offense = world.players[carrier].team;
    let defense = offense.other();
    let hoop = world.hoop_for(offense);
    let forward = if world.attacking_right(offense) {
        1.0
    } else {
        -1.0
    };
    let on_court = world.on_court(offense).as_slice().to_vec();
    let mut ordered = vec![carrier];
    ordered.extend(on_court.iter().copied().filter(|handle| *handle != carrier));
    let offsets = [
        Vec2::new(-22.0 * forward, 1.0),
        Vec2::new(-4.0 * forward, 23.0),
        Vec2::new(-4.0 * forward, -23.0),
        Vec2::new(-20.0 * forward, 16.0),
        Vec2::new(-20.0 * forward, -16.0),
    ];
    for (rank, attacker) in ordered.iter().enumerate() {
        world.players[*attacker].pos = hoop + offsets[rank % offsets.len()];
        world.players[*attacker].vel = Vec2::ZERO;
    }
    for (slot, defender) in world.on_court(defense).as_slice().iter().enumerate() {
        let man = ordered[slot % ordered.len()];
        world.players[*defender].pos = world.players[man].pos + Vec2::new(2.0 * forward, 0.0);
        world.players[*defender].vel = Vec2::ZERO;
        world.possession.matchup[slot] = man;
    }
}

/// 推进到有持球人的活球回合，并让站位落定。
fn live_holder(world: &mut World) -> usize {
    for _ in 0..40_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::Live) && world.holder().is_some() {
            let possession = world.possession.id;
            for _ in 0..24 {
                world.step(TICK_SECONDS);
                if world.possession.id != possession {
                    break;
                }
            }
            if let Some(handle) = world.holder() {
                return handle;
            }
        }
    }
    panic!("no live possession with a holder");
}

/// 找出弱侧离篮最近的无球防守人（Low Man）与其对位对象。
///
/// 与 `solve_defense_targets` 同一口径：低位人是弱侧对位人离篮
/// 最近的那个；比 y 会在场地镜像后把低位人当成高位人。
fn low_man(world: &World, carrier: usize) -> Option<usize> {
    let offense = world.players[carrier].team;
    let defense = offense.other();
    let hoop = world.hoop_for(offense);
    let ball = world.players[carrier].pos;
    let defenders = world.on_court(defense);
    let mut best: Option<(f32, usize)> = None;
    for (slot, defender) in defenders.as_slice().iter().enumerate() {
        let man = world.possession.matchup[slot];
        if man == UNASSIGNED || man == carrier {
            continue;
        }
        let man_pos = world.players[man].pos;
        if (man_pos.y - ball.y).abs()
            <= world
                .rules
                .tactics
                .defense
                .potential_field
                .weak_side_lateral_ft
        {
            continue;
        }
        let distance = (hoop - man_pos).length();
        if best.map(|(current, _)| distance < current).unwrap_or(true) {
            best = Some((distance, *defender));
        }
    }
    best.map(|(_, defender)| defender)
}

/// 强侧底角防守人：对位对象在球同侧、远离球、且站在三分线外。
fn strong_side_corner(world: &World, carrier: usize) -> Option<usize> {
    let offense = world.players[carrier].team;
    let defense = offense.other();
    let ball = world.players[carrier].pos;
    let hoop = world.hoop_for(offense);
    let arc = world.rules.league.three_point_distance_ft;
    let defenders = world.on_court(defense);
    let mut best: Option<(f32, usize)> = None;
    for (slot, defender) in defenders.as_slice().iter().enumerate() {
        let man = world.possession.matchup[slot];
        if man == UNASSIGNED || man == carrier {
            continue;
        }
        let man_pos = world.players[man].pos;
        if (man_pos.y - ball.y).abs()
            <= world
                .rules
                .tactics
                .defense
                .potential_field
                .weak_side_lateral_ft
        {
            continue;
        }
        if (man_pos - ball).length() < arc {
            continue;
        }
        if (man_pos.y - ball.y).signum() != (ball.y - 25.0).signum() {
            continue;
        }
        let distance = (man_pos - hoop).length();
        if best.map(|(current, _)| distance > current).unwrap_or(true) {
            best = Some((distance, *defender));
        }
    }
    best.map(|(_, defender)| defender)
}

fn target_of(world: &World, defender: usize) -> Vec2 {
    let defense = world.possession.team.other();
    let defenders = world.on_court(defense);
    let targets = solve_defense_targets(world);
    let slot = defenders
        .as_slice()
        .iter()
        .position(|handle| *handle == defender)
        .expect("defender is on court");
    targets[slot]
}

#[test]
fn strong_side_corner_holds_position_when_carrier_enters_the_paint() {
    let mut world = world(13);
    let carrier = live_holder(&mut world);
    spread_five_out(&mut world, carrier);
    refresh_matchup(&mut world);
    let hoop = world.hoop_for(world.possession.team);
    let corner = strong_side_corner(&world, carrier).expect("a strong-side corner defender exists");
    let perimeter = world.players[carrier].pos;
    let outside = target_of(&world, corner);
    // 持球人突到篮下（保持在同侧油漆区）。
    let side_offset = if perimeter.y >= hoop.y { 1.5 } else { -1.5 };
    world.players[carrier].pos = hoop + Vec2::new(-4.0, side_offset);
    let inside = target_of(&world, corner);
    world.players[carrier].pos = perimeter;
    let shift = (inside - outside).length();
    assert!(
        shift < 2.5,
        "strong-side corner defender shifted {shift:.2} ft when the carrier entered the paint"
    );
}

#[test]
fn weak_side_low_man_sinks_to_the_rim_when_carrier_enters_the_paint() {
    let mut world = world(13);
    let carrier = live_holder(&mut world);
    spread_five_out(&mut world, carrier);
    refresh_matchup(&mut world);
    let hoop = world.hoop_for(world.possession.team);
    let forward = if world.attacking_right(world.possession.team) {
        1.0
    } else {
        -1.0
    };
    // 先把持球人放到顶弧：低位人相对球的位置才有意义，球已经贴到篮下时
    // 弱侧没有人比他更靠近底线。
    world.players[carrier].pos = hoop + Vec2::new(-25.75 * forward, 0.0);
    let defender = low_man(&world, carrier).expect("a weak-side low man exists");
    world.players[carrier].pos = hoop + Vec2::new(-3.5 * forward, 0.0);
    let target = target_of(&world, defender);
    let distance = (target - hoop).length();
    assert!(
        distance < 6.0,
        "weak-side low man target sits {distance:.2} ft from the rim"
    );
}

#[test]
fn drive_and_kick_receiver_is_more_open_than_a_perimeter_pass() {
    let mut world = world(13);
    let carrier = live_holder(&mut world);
    spread_five_out(&mut world, carrier);
    refresh_matchup(&mut world);
    let hoop = world.hoop_for(world.possession.team);
    let defense = world.possession.team.other();
    let perimeter = world.players[carrier].pos;
    let forward = if world.attacking_right(world.possession.team) {
        1.0
    } else {
        -1.0
    };
    // 接球人是弱侧低位人放弃的那个人：他收缩护筐后，自己的对位对象就空了。
    world.players[carrier].pos = hoop + Vec2::new(-25.75 * forward, 0.0);
    let helper = low_man(&world, carrier).expect("a weak-side low man exists");
    let slot = world
        .on_court(defense)
        .as_slice()
        .iter()
        .position(|handle| *handle == helper)
        .expect("the helper is on court");
    let receiver = world.possession.matchup[slot];
    let gap =
        |world: &World| (solve_defense_targets(world)[slot] - world.players[receiver].pos).length();
    let before = gap(&world);
    world.players[carrier].pos = hoop + Vec2::new(-3.0 * forward, 0.0);
    let after = gap(&world);
    world.players[carrier].pos = perimeter;
    let gain = after - before;
    assert!(
        gain >= 3.2,
        "drive-and-kick receiver gained only {gain:.2} ft of space (before {before:.2}, after {after:.2})"
    );
}

#[test]
fn matchup_holds_while_no_screen_or_beat_happens() {
    let mut world = world(13);
    let carrier = live_holder(&mut world);
    refresh_matchup(&mut world);
    let baseline = world.possession.matchup;
    let possession = world.possession.id;
    let mut checked = 0;
    for tick in 0..200 {
        world.step(TICK_SECONDS);
        if world.possession.id != possession {
            break;
        }
        let screen = world
            .possession
            .last_screen
            .map(|read| world.now() - read.at < 3.0)
            .unwrap_or(false);
        let beat = world
            .possession
            .last_drive_at
            .map(|at| world.now() - at < 3.0)
            .unwrap_or(false);
        if screen || beat {
            continue;
        }
        assert_eq!(
            world.possession.matchup, baseline,
            "matchup changed without a screen or a beat at tick {tick}; carrier={carrier} holder={:?} now={} baseline={baseline:?} matchup={:?} last_drive={:?} last_screen={:?} pairing_changed={} defender_distances={:?}",
            world.holder(),
            world.now(),
            world.possession.matchup,
            world.possession.last_drive_at,
            world.possession.last_screen,
            world.possession.pairing_changed_at,
            world.on_court(world.possession.team.other()).as_slice().iter().map(|defender| (defender, world.possession.matchup.iter().position(|man| man == defender), world.players[*defender].pos, world.holder().map(|holder| (world.players[*defender].pos - world.players[holder].pos).length()))).collect::<Vec<_>>()
        );
        checked += 1;
    }
    assert!(checked > 0, "no stable tick was observed");
    let _ = carrier;
    assert_eq!(world.possession.matchup.len(), MAX_ON_COURT);
}

#[test]
fn weak_side_high_man_rotates_x_out_to_cover_vacated_shooter_when_low_man_helps_rim() {
    let mut world = world(13);
    let carrier = live_holder(&mut world);
    spread_five_out(&mut world, carrier);
    refresh_matchup(&mut world);
    let hoop = world.hoop_for(world.possession.team);
    let forward = if world.attacking_right(world.possession.team) {
        1.0
    } else {
        -1.0
    };
    world.players[carrier].pos = hoop + Vec2::new(-25.75 * forward, 0.0);
    let helper = low_man(&world, carrier).expect("a weak-side low man exists");
    let defense = world.possession.team.other();
    let slot = world
        .on_court(defense)
        .as_slice()
        .iter()
        .position(|handle| *handle == helper)
        .expect("the helper is on court");
    let receiver = world.possession.matchup[slot];
    let receiver_pos = world.players[receiver].pos;

    // 当持球人深入篮下时，弱侧低位人收缩护筐，其他弱侧人员进行 X-Out 补位向 receiver_pos 靠拢
    world.players[carrier].pos = hoop + Vec2::new(-2.5 * forward, 0.0);
    let targets = solve_defense_targets(&world);
    let mut found_x_out = false;
    for (s, _defender) in world.on_court(defense).as_slice().iter().enumerate() {
        if s == slot || Some(world.possession.matchup[s]) == Some(carrier) {
            continue;
        }
        let man = world.possession.matchup[s];
        if man != UNASSIGNED && world.players[man].on_court {
            let same_side =
                (world.players[man].pos.y - 25.0) * (world.players[carrier].pos.y - 25.0) > 0.0;
            if !same_side {
                let dist_to_vacated = (targets[s] - receiver_pos).length();
                if dist_to_vacated < 25.0 {
                    found_x_out = true;
                    break;
                }
            }
        }
    }
    assert!(found_x_out, "弱侧应产生 X-Out 补位轮转");
}

#[test]
fn defender_in_bait_is_beaten_by_drive() {
    let mut world = world(13);
    let carrier = live_holder(&mut world);
    refresh_matchup(&mut world);
    let defense = world.possession.team.other();
    let carrier_pos = world.players[carrier].pos;
    let defender = world
        .on_court(defense)
        .as_slice()
        .iter()
        .copied()
        .min_by(|a, b| {
            (world.players[*a].pos - carrier_pos)
                .length_squared()
                .partial_cmp(&(world.players[*b].pos - carrier_pos).length_squared())
                .unwrap()
        })
        .expect("defender on court");
    world.players[defender].bait = Some(nba_sim_core::state::Bait {
        kind: nba_sim_core::action::FakeKind::Hesitation,
        until: world.now() + 2.0,
        displacement_ft: 2.0,
    });
    let intent = nba_sim_core::action::ActionIntent {
        actor: carrier,
        kind: nba_sim_core::action::ActionKind::Drive(
            nba_domain::action_window::DriveKind::Straight,
        ),
        target: None,
        from_ft: world.players[carrier].pos,
        to_ft: world.players[carrier].pos + Vec2::new(5.0, 0.0),
        start_time: world.now(),
        duration: 0.8,
    };
    let events_before = world.events.len();
    let outcome = nba_sim_core::resolve::drive::resolve(
        &mut world,
        &intent,
        nba_domain::action_window::DriveKind::Straight,
    );
    assert_eq!(outcome.completion, nba_sim_core::Completion::Completed);
    let beat_fact = world.events[events_before..].iter().any(|e| {
        matches!(
            &e.fact,
            nba_sim_core::action::Fact::Drive {
                defender_beaten: true,
                ..
            }
        )
    });
    assert!(beat_fact, "扑空失衡的防守人必须被突破过掉");
}
