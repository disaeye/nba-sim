use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_sim_core::defense::{refresh_matchup, solve_defense_targets, UNASSIGNED};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::{Phase, MAX_ON_COURT};
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

/// 推进到有持球人的活球回合，并让站位落定。
fn live_holder(world: &mut World) -> usize {
    for _ in 0..40_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase, Phase::Live) && world.holder().is_some() {
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
        if (man_pos.y - ball.y).abs() <= world.rules.tactics.defense.potential_field.weak_side_lateral_ft
        {
            continue;
        }
        if man_pos.y >= ball.y {
            continue;
        }
        let distance = (world.players[*defender].pos - hoop).length();
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
        if (man_pos.y - ball.y).abs() <= world.rules.tactics.defense.potential_field.weak_side_lateral_ft
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
    refresh_matchup(&mut world);
    let hoop = world.hoop_for(world.possession.team);
    let corner = strong_side_corner(&world, carrier).expect("a strong-side corner defender exists");
    let perimeter = world.players[carrier].pos;
    let outside = target_of(&world, corner);
    // 持球人突到篮下。
    world.players[carrier].pos = hoop + Vec2::new(-4.0, 1.5);
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
    refresh_matchup(&mut world);
    let hoop = world.hoop_for(world.possession.team);
    world.players[carrier].pos = hoop + Vec2::new(-3.5, 1.0);
    let defender = low_man(&world, carrier).expect("a weak-side low man exists");
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
    refresh_matchup(&mut world);
    let hoop = world.hoop_for(world.possession.team);
    let defense = world.possession.team.other();
    let perimeter = world.players[carrier].pos;
    // 接球人是弱侧低位人放弃的那个人：他收缩护筐后，自己的对位对象就空了。
    let helper = low_man(&world, carrier).expect("a weak-side low man exists");
    let slot = world
        .on_court(defense)
        .as_slice()
        .iter()
        .position(|handle| *handle == helper)
        .expect("the helper is on court");
    let receiver = world.possession.matchup[slot];
    let gap = |world: &World| {
        (solve_defense_targets(world)[slot] - world.players[receiver].pos).length()
    };
    let before = gap(&world);
    world.players[carrier].pos = hoop + Vec2::new(-3.0, 1.0);
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
    for _ in 0..200 {
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
            "matchup changed without a screen or a beat"
        );
        checked += 1;
    }
    assert!(checked > 0, "no stable tick was observed");
    let _ = carrier;
    assert_eq!(world.possession.matchup.len(), MAX_ON_COURT);
}

