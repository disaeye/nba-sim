use glam::Vec2;
use nba_sim_core::params::RuntimeParams;
use nba_sim_core::state::Phase;
use nba_sim_core::value;
use nba_sim_core::world::{MatchSetup, World};
use nba_sim_core::TeamSide;

fn value_world() -> World {
    let mut world = World::new(
        MatchSetup::builtin(nba_domain::GameRules::default()),
        RuntimeParams::embedded(),
        41,
    );
    for player in world.players.iter_mut() {
        player.on_court = false;
    }
    let defender = world
        .players
        .iter()
        .position(|player| player.team != TeamSide::Home)
        .expect("away defender");
    world.players[0].on_court = true;
    world.players[1].on_court = true;
    world.players[defender].on_court = true;
    world.players[0].pos = Vec2::new(60.0, 25.0);
    world.players[1].pos = Vec2::new(75.0, 25.0);
    world.players[defender].pos = Vec2::new(66.0, 25.0);
    world.possession.team = TeamSide::Home;
    world.phase = Phase::Live;
    world.ball.set_held(0);
    world
}

#[test]
fn retained_value_is_non_negative_and_holding_is_worth_the_clock_it_keeps() {
    let world = value_world();
    let retained = value::retained_value(&world);
    assert!(retained >= 0.0);
    // 持球的价值不低于保留价值，并且随进攻时钟单调增加：手里的时间本身
    // 就是一份期权。
    let short = value::dribble_value(&world, 2.0);
    let long = value::dribble_value(&world, 20.0);
    assert!(short >= retained);
    assert!(long > short);
    assert_eq!(long, value::dribble_value(&world, 20.0));
    assert!(long < world.params.policy.w("possession_value") + 2.0);
}

#[test]
fn failed_drive_cannot_retain_the_success_value() {
    let world = value_world();
    let state = world.decision_state();
    let retained = value::retained_value(&world);
    let failure = value::drive_value(&world, &state, 0.0);
    let success = value::drive_value(&world, &state, 1.0);
    assert!(failure <= retained + 1e-5);
    assert!(success > failure + 0.1);
}

#[test]
fn three_and_two_are_compared_on_the_same_horizon() {
    let world = value_world();
    let pos = Vec2::new(75.0, 25.0);
    let two = value::shot_value_for(&world, 0, pos, 0.5, 2.0);
    let three = value::shot_value_for(&world, 0, pos, 0.5, 3.0);
    assert!(three > two);
    assert!((three - two) > 0.4);
}

#[test]
fn pass_probability_responds_to_real_geometry() {
    let mut near = value_world();
    let near_probability = value::pass_completion_probability(&near, 0, 1, nba_sim_core::action::PassKind::Chest);
    let defender = near
        .players
        .iter()
        .position(|player| player.on_court && player.team != TeamSide::Home)
        .expect("defender");
    near.players[defender].pos = Vec2::new(60.0, 40.0);
    let far_probability = value::pass_completion_probability(&near, 0, 1, nba_sim_core::action::PassKind::Chest);
    assert!(far_probability > near_probability, "{near_probability} -> {far_probability}");
}

#[test]
fn pass_value_contains_one_continuation_and_not_a_second_full_possession() {
    let world = value_world();
    let probability = value::pass_completion_probability(&world, 0, 1, nba_sim_core::action::PassKind::Chest);
    let receiver = &world.players[1];
    let catch = value::catch_value(
        &world,
        receiver.pos,
        1,
        nba_sim_core::resolve::contest_for(&world, 1, receiver.pos),
    );
    let pass = value::pass_value(&world, 0, 1);
    assert!(pass <= probability * catch + 1e-5);
    assert!(catch < world.params.policy.w("possession_value") + 2.0);
}

#[test]
fn drive_value_is_between_failure_and_success_on_the_same_horizon() {
    let world = value_world();
    let state = world.decision_state();
    let failure = value::drive_value(&world, &state, 0.0);
    let mixed = value::drive_value(&world, &state, 0.5);
    let success = value::drive_value(&world, &state, 1.0);
    assert!(mixed > failure);
    assert!(mixed < success);
}
