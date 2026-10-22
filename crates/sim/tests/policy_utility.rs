//! 决策效用的可解释性与玻尔兹曼选择守卫。
//!
//! 这些测试盯的是「每个动作的效用能不能被逐项读出来」以及「改变一个原因
//! 是否只在对应那一项上生效」。它们不检查数值是否好看——那是校准表的事。

use nba_domain::rules::GameRules;
use nba_sim_core::action::ActionKind;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::policy::{
    candidate_utilities, decision_temperature, variance_appetite,
};
use nba_sim_core::state::Phase;
use nba_sim_core::utility::{softmax_pick, EpvTerms, Utility};
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, TeamSide, World};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

fn live_world(seed: u64) -> World {
    let mut world = world(seed);
    for _ in 0..40_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase, Phase::Live) && world.holder().is_some() {
            return world;
        }
    }
    panic!("the engine never reached a live ball state");
}

fn shot_epv(world: &World) -> Option<f32> {
    let state = world.decision_state();
    candidate_utilities(world, &state)
        .into_iter()
        .find(|(kind, _, _, _)| matches!(kind, ActionKind::Shoot(_)))
        .map(|(_, _, _, utility)| utility.epv_base())
}

fn pass_epv(world: &World) -> Option<f32> {
    let state = world.decision_state();
    candidate_utilities(world, &state)
        .into_iter()
        .find(|(kind, _, _, _)| matches!(kind, ActionKind::Pass(_)))
        .map(|(_, _, _, utility)| utility.epv_base())
}

#[test]
fn the_utility_total_is_the_documented_formula() {
    let utility = Utility {
        epv: EpvTerms {
            ability: 1.4,
            contest: -0.2,
            lane: 0.1,
            help: -0.05,
            tendency: 0.02,
            context: 0.08,
            turnover: -0.03,
        },
        variance: 0.5,
        clock_cost: 0.25,
        foul_cost: 0.06,
        tactical_bonus: 0.12,
    };
    let epv = 1.4 - 0.2 + 0.1 - 0.05 + 0.02 + 0.08 - 0.03;
    assert!((utility.epv_base() - epv).abs() < 1e-6);
    let gamma = -0.4;
    let expected = epv + gamma * 0.5 - 0.25 - 0.06 + 0.12;
    assert!((utility.total(gamma) - expected).abs() < 1e-6);
}

#[test]
fn every_decision_records_a_finite_decomposition() {
    let mut world = world(7);
    while !world.finished && world.clock.tick < 1_000_000 {
        world.step(TICK_SECONDS);
    }
    assert!(world.finished, "the game did not finish");
    assert!(
        world.decision_samples.len() > 100,
        "too few decisions were recorded: {}",
        world.decision_samples.len()
    );
    for sample in &world.decision_samples {
        let utility = sample.utility;
        for term in [
            utility.epv.ability,
            utility.epv.contest,
            utility.epv.lane,
            utility.epv.help,
            utility.epv.tendency,
            utility.epv.context,
            utility.epv.turnover,
            utility.variance,
            utility.clock_cost,
            utility.foul_cost,
            utility.tactical_bonus,
        ] {
            assert!(term.is_finite(), "a utility term is not finite: {utility:?}");
        }
        assert!(
            (utility.epv.sum() - utility.epv_base()).abs() < 1e-5,
            "the epv decomposition does not sum to its own total"
        );
        assert!(sample.beta > 0.0, "the inverse temperature must be positive");
        assert!(sample.candidates > 0, "a decision had no candidates");
        assert!(sample.gamma.is_finite(), "the variance appetite is not finite");
    }
}

#[test]
fn a_better_shooter_earns_a_higher_shot_epv() {
    let mut world = live_world(13);
    let state = world.decision_state();
    let actor = state.actor;
    let team = world.players[actor].team;
    // 把持球人放到弧顶，三分才是可选项。
    let hoop = world.hoop_for(team);
    let spot = hoop + (hoop - world.players[actor].pos).normalize_or_zero() * 25.0;
    world.players[actor].pos = spot;
    world.players[actor].target = spot;
    world.players[actor].data.attributes.shooting_three = 0.15;
    let low = shot_epv(&world).expect("an open three must be a candidate");
    world.players[actor].data.attributes.shooting_three = 0.95;
    let high = shot_epv(&world).expect("an open three must be a candidate");
    assert!(
        high > low,
        "a better shooter must have a higher shot epv: {low:.3} -> {high:.3}"
    );
}

#[test]
fn a_closer_defender_lowers_the_shot_epv() {
    let mut world = live_world(13);
    let actor = world.decision_state().actor;
    let team = world.players[actor].team;
    let hoop = world.hoop_for(team);
    let spot = hoop + (hoop - world.players[actor].pos).normalize_or_zero() * 25.0;
    world.players[actor].pos = spot;
    world.players[actor].target = spot;
    let defender = world.on_court(team.other()).as_slice()[0];
    // 先把防守人放到远处，再压到脸上：只有干扰变了。
    world.players[defender].pos = spot + glam::Vec2::new(12.0, 0.0);
    world.players[defender].target = world.players[defender].pos;
    let open = shot_epv(&world).expect("an open three must be a candidate");
    world.players[defender].pos = spot + glam::Vec2::new(0.6, 0.0);
    world.players[defender].target = world.players[defender].pos;
    let contested = shot_epv(&world).expect("a contested three is still a candidate");
    assert!(
        contested < open,
        "a closer defender must lower the shot epv: {open:.3} -> {contested:.3}"
    );
}

#[test]
fn a_clearer_pass_lane_raises_the_pass_epv() {
    let mut world = live_world(13);
    let state = world.decision_state();
    let holder = state.actor;
    let team = world.players[holder].team;
    let clear = world.hoop_for(team);
    // 先把所有防守人挪到远离传球线路的位置。
    for defender in world.on_court(team.other()).as_slice().to_vec() {
        world.players[defender].pos = clear;
        world.players[defender].target = clear;
    }
    let open = pass_epv(&world).expect("a pass must be a candidate");

    // 在每一条传球线路的中点都放一个防守人：所有目标的可接球质量同时下降。
    let targets: Vec<glam::Vec2> = world
        .decision_state()
        .teammates
        .iter()
        .map(|view| view.pos)
        .collect();
    for (defender, target) in world
        .on_court(team.other())
        .as_slice()
        .to_vec()
        .into_iter()
        .zip(targets)
    {
        world.players[defender].pos = (world.players[holder].pos + target) / 2.0;
        world.players[defender].target = world.players[defender].pos;
    }
    let blocked = pass_epv(&world).expect("a pass must be a candidate");
    assert!(
        blocked < open,
        "defenders on every lane must lower the best pass epv: {open:.3} -> {blocked:.3}"
    );
}

#[test]
fn decision_temperature_rises_with_basketball_iq_and_falls_with_fatigue() {
    let mut world = live_world(13);
    let actor = world.decision_state().actor;
    world.players[actor].stamina = 1.0;
    world.players[actor].data.attributes.decision_iq = 0.1;
    let low_iq = decision_temperature(&world, actor);
    world.players[actor].data.attributes.decision_iq = 0.95;
    let high_iq = decision_temperature(&world, actor);
    assert!(
        high_iq > low_iq,
        "a higher iq must sharpen the choice: {low_iq:.3} -> {high_iq:.3}"
    );
    world.players[actor].stamina = 0.05;
    let tired = decision_temperature(&world, actor);
    assert!(
        tired < high_iq,
        "fatigue must blunt the choice: {high_iq:.3} -> {tired:.3}"
    );
}

#[test]
fn the_variance_appetite_flips_sign_between_leading_and_trailing() {
    let mut world = live_world(13);
    world.clock.period = 4;
    world.clock.game_clock = 180.0;
    let state = world.decision_state();
    let actor = state.actor;
    let team = world.players[actor].team;
    let index = World::team_index(team);
    world.score[index] = 100;
    world.score[World::team_index(team.other())] = 90;
    let leading = variance_appetite(&world.decision_state());
    world.score[index] = 90;
    world.score[World::team_index(team.other())] = 100;
    let trailing = variance_appetite(&world.decision_state());
    assert!(
        leading < 0.0,
        "the leading team must prefer low variance, got {leading:.3}"
    );
    assert!(
        trailing > 0.0,
        "the trailing team must prefer high variance, got {trailing:.3}"
    );
    // 无分差时没有方差偏好。
    world.score[index] = 100;
    world.score[World::team_index(team.other())] = 100;
    let tied = variance_appetite(&world.decision_state());
    assert!(tied.abs() < 1e-6, "a tie must be variance neutral, got {tied:.3}");
    let _ = TeamSide::Home;
}

#[test]
fn the_boltzmann_pick_favours_the_higher_utility() {
    let mut world = live_world(13);
    let high = 1.4;
    let low = 0.2;
    let mut picked_high = 0;
    for _ in 0..2000 {
        if softmax_pick(&mut world.rng, &[low, high], 6.0) == 1 {
            picked_high += 1;
        }
    }
    assert!(
        picked_high > 1900,
        "the higher utility must dominate the draw, got {picked_high}/2000"
    );
}

#[test]
fn a_stationary_arc_holder_has_a_shot_candidate() {
    let mut world = live_world(13);
    let actor = world.decision_state().actor;
    let team = world.players[actor].team;
    let hoop = world.hoop_for(team);
    let spot = hoop + (hoop - world.players[actor].pos).normalize_or_zero() * 25.0;
    world.players[actor].pos = spot;
    world.players[actor].target = spot;
    world.possession.last_screen = None;
    world.clock.shot_clock = 6.0;
    let state = world.decision_state();
    let kinds: Vec<String> = candidate_utilities(&world, &state)
        .into_iter()
        .map(|(kind, _, _, utility)| format!("{kind:?}@{:.2}", utility.total(0.0)))
        .collect();
    assert!(
        kinds.iter().any(|label| label.starts_with("Shoot")),
        "a stationary holder at the arc must have a shot candidate: {kinds:?}"
    );
}
