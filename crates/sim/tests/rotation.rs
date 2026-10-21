use nba_domain::rules::GameRules;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::rotation::{record_foul, substitute_if_needed, update_stamina};
use nba_sim_core::state::Phase;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

#[test]
fn a_player_who_sits_four_minutes_regains_stamina() {
    let mut world = world(13);
    world.phase = Phase::Live;
    let bench = (0..world.players.len())
        .find(|handle| !world.players[*handle].on_court)
        .expect("a bench player exists");
    world.players[bench].stamina = 0.60;
    let before = world.players[bench].stamina;
    // 4 分钟比赛时间，每 tick 0.05 秒。
    let ticks = (240.0 / TICK_SECONDS) as usize;
    for _ in 0..ticks {
        update_stamina(&mut world, TICK_SECONDS);
    }
    let after = world.players[bench].stamina;
    assert!(
        after > before,
        "bench stamina did not rise: {before:.4} -> {after:.4}"
    );
}

#[test]
fn a_six_foul_player_never_returns_to_the_court() {
    let mut world = world(13);
    for _ in 0..40_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase, Phase::Live) && world.holder().is_some() {
            break;
        }
    }
    let rules = world.rules.clone();
    let limit = rules.league.max_personal_fouls;
    let victim = world
        .on_court(nba_sim_core::TeamSide::Home)
        .as_slice()[0];
    world.players[victim].fouls = limit - 1;
    record_foul(&mut world, victim);
    assert!(world.players[victim].disqualified, "the sixth foul did not disqualify");
    substitute_if_needed(&mut world, &rules);
    assert!(
        !world.players[victim].on_court,
        "the disqualified player stayed on the court"
    );
    // 后续任何时刻都不再上场。
    for _ in 0..4000 {
        world.step(TICK_SECONDS);
        let rules = world.rules.clone();
        substitute_if_needed(&mut world, &rules);
        assert!(
            !world.players[victim].on_court,
            "the disqualified player returned to the court"
        );
    }
}
