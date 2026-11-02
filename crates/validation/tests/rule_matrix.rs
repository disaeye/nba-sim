use nba_domain::event::FoulKind;
use nba_domain::rules::{counts_toward_personal_limit, counts_toward_team_period_fouls, GameRules};

#[test]
fn nba_and_fiba_period_boundaries_match_their_rule_profiles() {
    let nba = nba_domain::LeagueProfile::nba();
    let fiba = nba_domain::LeagueProfile::fiba();
    assert_eq!(nba.period_duration_seconds, 720.0);
    assert_eq!(fiba.period_duration_seconds, 600.0);
    assert_eq!(nba.overtime_duration_seconds, 300.0);
    assert_eq!(fiba.overtime_duration_seconds, 300.0);
    assert_eq!(nba.shot_clock_seconds, 24.0);
    assert_eq!(fiba.shot_clock_seconds, 24.0);
}

#[test]
fn team_foul_bonus_thresholds_cover_before_at_and_after_threshold() {
    let nba = nba_domain::LeagueProfile::nba();
    let fiba = nba_domain::LeagueProfile::fiba();
    for profile in [&nba, &fiba] {
        assert_eq!(profile.bonus_free_throws_for(3, 2, 400.0), 0);
        assert_eq!(profile.bonus_free_throws_for(4, 2, 400.0), 0);
        assert_eq!(profile.bonus_free_throws_for(5, 2, 400.0), 2);
        assert_eq!(profile.shot_clock_after_offensive_rebound(8.0, true), 14.0);
        assert_eq!(profile.shot_clock_after_offensive_rebound(18.0, true), 18.0);
        assert_eq!(profile.shot_clock_after_offensive_rebound(8.0, false), 8.0);
    }
    assert_eq!(nba.bonus_free_throws_for(1, 4, 100.0), 0);
    assert_eq!(nba.bonus_free_throws_for(2, 4, 100.0), 2);
    assert_eq!(fiba.bonus_free_throws_for(4, 4, 100.0), 0);
    assert_eq!(fiba.bonus_free_throws_for(5, 4, 100.0), 2);
    assert_eq!(nba.shot_clock_after_offensive_rebound(8.0, true), 14.0);
    assert_eq!(fiba.shot_clock_after_offensive_rebound(8.0, true), 14.0);
}

#[test]
fn foul_kinds_use_distinct_personal_and_team_period_counters() {
    for kind in [FoulKind::Personal, FoulKind::Shooting, FoulKind::LooseBall] {
        assert!(counts_toward_personal_limit(kind));
        assert!(counts_toward_team_period_fouls(kind));
    }
    assert!(counts_toward_personal_limit(FoulKind::Offensive));
    assert!(!counts_toward_team_period_fouls(FoulKind::Offensive));
    assert!(!counts_toward_personal_limit(FoulKind::Technical));
    assert!(!counts_toward_team_period_fouls(FoulKind::Technical));
}

#[test]
fn changing_bonus_threshold_changes_the_profile_result() {
    let mut rules = GameRules::default();
    rules.league.bonus_fouls_per_period = 1;
    rules.league.bonus_fouls_final = 1;
    assert_eq!(rules.league.bonus_free_throws_for(1, 2, 400.0), 2);
}
