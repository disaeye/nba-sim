use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{MatchSetup, Params, World};
use nba_validation::core_validation::rule_profile;

#[test]
fn engine_and_validator_share_the_same_rule_profile() {
    let profile = rule_profile();
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let world = World::new(MatchSetup::builtin(profile.clone()), params, 1);
    assert_eq!(
        world.rules.league.bonus_fouls_per_period,
        profile.league.bonus_fouls_per_period
    );
    assert_eq!(
        world.rules.league.bonus_fouls_final,
        profile.league.bonus_fouls_final
    );
    assert_eq!(
        world.rules.league.max_personal_fouls,
        profile.league.max_personal_fouls
    );
    assert_eq!(
        world.rules.league.offensive_rebound_shot_clock_seconds,
        profile.league.offensive_rebound_shot_clock_seconds
    );
}
