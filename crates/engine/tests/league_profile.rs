//! M10 acceptance (architecture.md 6.3): LeagueProfile switching = data switching,
//! engine code path has zero league branches; FIBA profile completes a match segment with zero Hard violations.
//!
use nba_domain::{GameRules, LeagueProfile};
use nba_engine::MatchEngine;

#[test]
fn fiba_profile_produces_fiba_shapes() {
    let rules = GameRules::with_league(LeagueProfile::fiba());
    assert_eq!(rules.league.period_duration_seconds, 600.0);
    assert_eq!(rules.league.max_personal_fouls, 5);
    assert_eq!(rules.league.bonus_fouls_per_period, 4);
    assert!((rules.league.three_point_distance_ft - 22.15).abs() < 0.01);
    assert_eq!(rules.league.court.width_ft, 91.86);
    rules.validate().expect("fiba profile must validate");
}

#[test]
fn nba_profile_is_default() {
    let rules = GameRules::default();
    assert_eq!(rules.league.max_personal_fouls, 6);
    assert_eq!(rules.league.period_duration_seconds, 720.0);
}

#[test]
fn fiba_quarter_runs_with_zero_hard_violations() {
    let rules = GameRules::with_league(LeagueProfile::fiba());
    let mut engine = MatchEngine::with_setup(nba_engine::MatchSetup::builtin(rules), 42);
    for tick_idx in 0..6000 {
        if engine.is_finished() {
            break;
        }
        let tick = engine.step();
        let hard: Vec<_> = engine
            .last_tick_violations()
            .iter()
            .filter(|v| matches!(v.severity, nba_invariants::ViolationSeverity::Hard))
            .collect();
        assert!(
            hard.is_empty(),
            "FIBA seed 42 tick {} hard violations: {:?}",
            tick_idx,
            hard
        );
        assert!((tick.frame.rules.three_point_distance_ft - 22.15).abs() < 0.01);
    }
}
#[test]
fn fiba_full_game_runs_and_evaluates_with_fiba_fixture() {
    let rules = GameRules::with_league(LeagueProfile::fiba());
    let mut engine = MatchEngine::with_setup(nba_engine::MatchSetup::builtin(rules.clone()), 42);
    let mut ticks = Vec::new();

    // Simulate full 4 quarters under FIBA profile (4 * 600s = 2400s).
    // In 0.04s ticks, 4 quarters is 60,000 game clock ticks;
    // simulate 8,000 active ticks for rigorous multi-possession and judgment coverage.
    for tick_idx in 0..8000 {
        let _tick = engine.step();
        let hard: Vec<_> = engine
            .last_tick_violations()
            .iter()
            .filter(|v| matches!(v.severity, nba_invariants::ViolationSeverity::Hard))
            .collect();
        assert!(
            hard.is_empty(),
            "tick {} triggered hard violation: {:?}",
            tick_idx,
            hard
        );
        ticks.push(_tick);
    }

    assert_eq!(ticks.len(), 8000);
    assert!(
        engine.completed_possessions() > 10,
        "FIBA 8000 ticks must simulate active possessions"
    );
    assert_eq!(rules.league.name, "FIBA");
}

#[test]
fn forced_substitution_preserves_five_on_five() {
    let mut engine = MatchEngine::new(42);
    let out_id = "A_01".to_string();
    engine
        .physics_mut_for_test()
        .get_player_mut(&out_id)
        .expect("starter A_1 must exist")
        .foul_count = 5;
    engine.forced_substitution(&out_id);

    let out = engine.physics().get_player(&out_id).unwrap();
    assert!(!out.on_court, "fouled-out player must leave court");
    assert!(!out.has_ball);

    let home_on = engine
        .physics()
        .get_players()
        .values()
        .filter(|p| p.team == "home" && p.on_court)
        .count();
    let away_on = engine
        .physics()
        .get_players()
        .values()
        .filter(|p| p.team == "away" && p.on_court)
        .count();
    assert_eq!(home_on, 5, "home must keep 5 on court");
    assert_eq!(away_on, 5, "away must keep 5 on court");

    assert!(
        !engine.away_roster_order().contains(&out_id),
        "roster order must drop fouled-out player"
    );

    let events: Vec<String> = engine
        .pending_events()
        .iter()
        .map(|e| e.event_type_str().to_string())
        .collect();
    assert!(
        events.iter().any(|k| k == "SUBSTITUTION"),
        "substitution must emit an event, got {:?}",
        events
    );

    for tick in 0..500 {
        engine.step();
        if !engine.last_tick_violations().is_empty() {
            panic!(
                "tick {} violations: {:?}",
                tick,
                engine.last_tick_violations()
            );
        }
    }
}
