use nba_domain::rules::GameRules;
use nba_sim_core::action::{Fact, FoulKind, TeamSide};
use nba_sim_core::boxscore::{BoxScore, PlayerLine};
use nba_sim_core::params::{workspace_calibration_dir, Params};
use nba_sim_core::report::GameReport;
use nba_sim_core::world::MatchSetup;
use nba_sim_core::world::TimedFact;
use nba_validation::core_validation::{check_ledger, reconstruct};

fn player_line(id: &str, team: TeamSide) -> PlayerLine {
    PlayerLine::new(id, id, "0", team, 0.0)
}

fn report_with_events(events: Vec<Fact>) -> GameReport {
    let mut home = vec![player_line("home_shooter", TeamSide::Home); 5];
    let mut away = vec![player_line("away_blocker", TeamSide::Away); 5];
    home[0].points = 3;
    home[0].fgm = 1;
    home[0].fga = 2;
    home[0].tpa = 1;
    home[0].fta = 1;
    home[0].ftm = 1;
    home[0].and_one = 1;
    home[0].oreb = 1;
    home[0].turnovers = 1;
    home[1].assists = 1;
    away[0].blocks = 1;
    away[0].fouls = 1;
    away[0].shooting_fouls = 1;
    away[0].steals = 1;
    away[0].dreb = 1;
    let events = events
        .into_iter()
        .enumerate()
        .map(|(index, fact)| TimedFact {
            time: 0.0,
            period: 1,
            tick_index: 1,
            sequence: index as u64,
            event_id: index as u64 + 1,
            fact,
            trace: None,
        })
        .collect();
    GameReport {
        seed: 9,
        home_team: "Home".to_string(),
        away_team: "Away".to_string(),
        home_score: 3,
        away_score: 0,
        periods: 1,
        regulation_periods: 4,
        period_duration_seconds: 600.0,
        overtime_duration_seconds: 300.0,
        tick_seconds: 0.05,
        ticks: 1,
        redecide_interval_seconds: 0.0,
        open_business_instances: 0,
        rebound_attempts: 0,
        contested_rebounds: [0, 0],
        possessions: Vec::new(),
        box_score: BoxScore { home, away },
        events,
        shot_clock_at_release: Vec::new(),
        contest_samples: Vec::new(),
        shot_samples: Vec::new(),
        pass_samples: Vec::new(),
        decision_samples: Vec::new(),
        decision_traces: Vec::new(),
        decision_records: Vec::new(),
        shot_zone_counts: [[0; 4]; 2],
        shot_zone_makes: [[0; 4]; 2],
        creation_counts: [0; 7],
        violations: Vec::new(),
        games: 1,
    }
}

#[test]
fn reconstruct_attributes_blocked_attempts_and_shooting_fouls() {
    let report = report_with_events(vec![
        Fact::Shot {
            shot_id: 0,
            shooter: 0,
            kind: nba_sim_core::action::ShotKind::CatchShoot,
            zone: "rim",
            distance_ft: 3.0,
            contest: 0.8,
            is_three: false,
            blocked_by: Some(5),
            made: false,
            points: 0,
            assisted_by: None,
        },
        Fact::Shot {
            shot_id: 1,
            shooter: 0,
            kind: nba_sim_core::action::ShotKind::CatchShoot,
            zone: "rim",
            distance_ft: 3.0,
            contest: 0.0,
            is_three: false,
            blocked_by: None,
            made: true,
            points: 2,
            assisted_by: Some(1),
        },
        Fact::FreeThrow {
            shooter: 0,
            made: true,
            index: 1,
            total: 1,
            foul_id: 1,
        },
        Fact::Rebound {
            player: 0,
            offensive: true,
            distance_ft: 2.0,
        },
        Fact::Rebound {
            player: 5,
            offensive: false,
            distance_ft: 20.0,
        },
        Fact::Turnover {
            player: 0,
            kind: nba_sim_core::action::TurnoverKind::BadPass,
        },
        Fact::Steal {
            player: 5,
            victim: 0,
        },
        Fact::Foul {
            fouler: 5,
            fouled: 0,
            kind: FoulKind::Shooting,
            free_throws: 1,
            and_one: true,
            foul_id: 1,
        },
    ]);

    let ledger = reconstruct(&report);
    assert_eq!(ledger.points, 3);
    assert_eq!(ledger.fga, 2);
    assert_eq!(ledger.fgm, 1);
    assert_eq!(ledger.player_blocks.get(&5), Some(&1));
    assert_eq!(ledger.shooting_fouls.get(&5), Some(&1));
    assert_eq!(ledger.and_ones.get(&0), Some(&1));
    assert_eq!(ledger.player_assists.get(&1), Some(&1));
    assert_eq!(ledger.player_free_throw_attempts.get(&0), Some(&1));
    assert_eq!(ledger.player_free_throw_makes.get(&0), Some(&1));
    assert_eq!(ledger.player_rebounds.get(&0), Some(&(1, 0)));
    assert_eq!(ledger.player_rebounds.get(&5), Some(&(0, 1)));
    assert_eq!(ledger.player_turnovers.get(&0), Some(&1));
    assert_eq!(ledger.player_steals.get(&5), Some(&1));
}

#[test]
fn check_ledger_reconciles_each_player_stat_from_events() {
    let mut report = report_with_events(vec![
        Fact::Shot {
            shot_id: 0,
            shooter: 0,
            kind: nba_sim_core::action::ShotKind::CatchShoot,
            zone: "rim",
            distance_ft: 3.0,
            contest: 0.0,
            is_three: false,
            blocked_by: None,
            made: true,
            points: 2,
            assisted_by: Some(1),
        },
        Fact::Shot {
            shot_id: 1,
            shooter: 0,
            kind: nba_sim_core::action::ShotKind::CatchShoot,
            zone: "three",
            distance_ft: 24.0,
            contest: 0.8,
            is_three: true,
            blocked_by: Some(5),
            made: false,
            points: 0,
            assisted_by: None,
        },
        Fact::FreeThrow {
            shooter: 0,
            made: true,
            index: 1,
            total: 1,
            foul_id: 2,
        },
        Fact::Rebound {
            player: 0,
            offensive: true,
            distance_ft: 2.0,
        },
        Fact::Rebound {
            player: 5,
            offensive: false,
            distance_ft: 20.0,
        },
        Fact::Turnover {
            player: 0,
            kind: nba_sim_core::action::TurnoverKind::BadPass,
        },
        Fact::Steal {
            player: 5,
            victim: 0,
        },
        Fact::Foul {
            fouler: 5,
            fouled: 0,
            kind: FoulKind::Shooting,
            free_throws: 1,
            and_one: true,
            foul_id: 2,
        },
    ]);

    let ledger = check_ledger(9, &report);
    assert!(ledger.points_reconcile, "{}", ledger.detail);
    assert!(ledger.attempts_reconcile, "{}", ledger.detail);
    assert!(ledger.fouls_reconcile, "{}", ledger.detail);
    assert!(ledger.player_stats_reconcile, "{}", ledger.detail);
    assert!(ledger.minutes_reconcile, "{}", ledger.detail);
    assert!(ledger.rebounds_reconcile, "{}", ledger.detail);
    assert!(ledger.turnovers_reconcile, "{}", ledger.detail);

    report.box_score.home.iter_mut().for_each(|line| line.seconds = 40.1 * 60.0);
    report.box_score.away.iter_mut().for_each(|line| line.seconds = 40.1 * 60.0);
    report.period_duration_seconds = 10.0 * 60.0;
    report.periods = 4;
    let ledger = check_ledger(9, &report);
    assert!(!ledger.minutes_reconcile);
    assert!(ledger.detail.contains("player-time ticks"), "{}", ledger.detail);

    report.box_score.home.iter_mut().for_each(|line| line.seconds = 40.0 * 60.0);
    report.box_score.away.iter_mut().for_each(|line| line.seconds = 40.0 * 60.0);
    let ledger = check_ledger(9, &report);
    assert!(ledger.minutes_reconcile, "{}", ledger.detail);
}

#[test]
fn check_ledger_reconciles_a_production_game_report() {
    let report = nba_sim_core::run_game(1, &workspace_calibration_dir());
    let ledger = check_ledger(1, &report);
    assert!(ledger.points_reconcile, "{}", ledger.detail);
    assert!(ledger.attempts_reconcile, "{}", ledger.detail);
    assert!(ledger.rebounds_reconcile, "{}", ledger.detail);
    assert!(ledger.turnovers_reconcile, "{}", ledger.detail);
    assert!(ledger.minutes_reconcile, "{}", ledger.detail);
    assert!(ledger.fouls_reconcile, "{}", ledger.detail);
    assert!(ledger.player_stats_reconcile, "{}", ledger.detail);
    assert!(ledger.possessions_reconcile, "{}", ledger.detail);
    assert!(ledger.minutes_reconcile, "{}", ledger.detail);
}

#[test]
fn check_ledger_reconciles_a_fiba_game_report() {
    let mut rules = GameRules::default();
    rules.league = nba_domain::LeagueProfile::fiba();
    let report = nba_sim_core::run_game_with_setup(
        7,
        Params::load(&workspace_calibration_dir()).expect("calibration tables load"),
        MatchSetup::builtin(rules),
    );
    let ledger = check_ledger(7, &report);
    assert!(ledger.points_reconcile, "{}", ledger.detail);
    assert!(ledger.attempts_reconcile, "{}", ledger.detail);
    assert!(ledger.rebounds_reconcile, "{}", ledger.detail);
    assert!(ledger.turnovers_reconcile, "{}", ledger.detail);
    assert!(ledger.fouls_reconcile, "{}", ledger.detail);
    assert!(ledger.player_stats_reconcile, "{}", ledger.detail);
    assert!(ledger.possessions_reconcile, "{}", ledger.detail);
    assert!(ledger.minutes_reconcile, "{}", ledger.detail);
}

#[test]
fn check_ledger_reports_blocker_mismatch() {
    let mut report = report_with_events(vec![Fact::Shot {
        shot_id: 0,
        shooter: 0,
        kind: nba_sim_core::action::ShotKind::CatchShoot,
        zone: "rim",
        distance_ft: 3.0,
        contest: 0.8,
        is_three: false,
        blocked_by: Some(5),
        made: false,
        points: 0,
        assisted_by: None,
    }]);
    report.box_score.home[0] = player_line("home_shooter", TeamSide::Home);
    report.box_score.home[0].fga = 1;
    report.box_score.away[0] = player_line("away_blocker", TeamSide::Away);

    let ledger = check_ledger(9, &report);
    assert!(!ledger.player_stats_reconcile);
    assert!(ledger.detail.contains("blocks"), "{}", ledger.detail);
}
