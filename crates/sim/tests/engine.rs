use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{gate, run_game, MatchSetup, Params, TeamSide, World};

fn params() -> Params {
    Params::load(&workspace_calibration_dir()).expect("calibration tables load")
}

fn play(seed: u64) -> nba_sim_core::GameReport {
    run_game(seed, &workspace_calibration_dir())
}

#[test]
fn same_seed_replays_identically() {
    let first = play(7);
    let second = play(7);
    assert_eq!(first.home_score, second.home_score);
    assert_eq!(first.away_score, second.away_score);
    assert_eq!(first.possessions.len(), second.possessions.len());
    let a = first.box_score.team_totals(TeamSide::Home);
    let b = second.box_score.team_totals(TeamSide::Home);
    assert_eq!(a.points, b.points);
    assert_eq!(a.fga, b.fga);
    assert_eq!(a.fta, b.fta);
    assert_eq!(a.turnovers, b.turnovers);
}

#[test]
fn box_score_conserves_points_and_attempts() {
    let report = play(11);
    for team in [TeamSide::Home, TeamSide::Away] {
        let lines = match team {
            TeamSide::Home => &report.box_score.home,
            TeamSide::Away => &report.box_score.away,
        };
        let totals = report.box_score.team_totals(team);
        let player_points: u32 = lines.iter().map(|line| line.points).sum();
        assert_eq!(
            player_points, totals.points,
            "player points must sum to the team total"
        );
        assert!(
            totals.fgm <= totals.fga,
            "made shots cannot exceed attempts"
        );
        assert!(
            totals.ftm <= totals.fta,
            "made free throws cannot exceed attempts"
        );
        assert!(
            totals.tpm <= totals.fga,
            "made threes cannot exceed field goal attempts"
        );
        let scored = totals.fgm - totals.tpm;
        let expected = 2 * scored + 3 * totals.tpm + totals.ftm;
        assert_eq!(
            expected, totals.points,
            "points must reconcile with the shot ledger"
        );
    }
    assert_eq!(
        report.home_score + report.away_score,
        report.box_score.team_totals(TeamSide::Home).points
            + report.box_score.team_totals(TeamSide::Away).points
    );
}

#[test]
fn no_player_exceeds_the_foul_limit_on_court() {
    let report = play(13);
    let limit = nba_domain::GameRules::default().league.max_personal_fouls as u32;
    for line in report
        .box_score
        .home
        .iter()
        .chain(report.box_score.away.iter())
    {
        assert!(
            line.fouls <= limit,
            "{} recorded {} fouls, above the limit {}",
            line.name,
            line.fouls,
            limit
        );
    }
}

#[test]
fn minutes_stay_within_the_game_length() {
    let report = play(17);
    let period = nba_domain::GameRules::default()
        .league
        .period_duration_seconds as f64;
    let overtime = nba_domain::GameRules::default()
        .league
        .overtime_duration_seconds as f64;
    let ceiling = (4.0 * period + 4.0 * overtime) / 60.0;
    for line in report
        .box_score
        .home
        .iter()
        .chain(report.box_score.away.iter())
    {
        assert!(
            line.minutes() <= ceiling + 0.1,
            "{} played {:.1} minutes, above {:.1}",
            line.name,
            line.minutes(),
            ceiling
        );
    }
}

#[test]
fn game_shape_is_within_the_professional_range() {
    let mut points = Vec::new();
    let mut possessions = Vec::new();
    for seed in [1u64, 3, 5, 9] {
        let report = play(seed);
        points.push((report.home_score + report.away_score) as f64);
        possessions.push(report.possessions.len() as f64);
    }
    let mean_points: f64 = points.iter().sum::<f64>() / points.len() as f64;
    let mean_possessions: f64 = possessions.iter().sum::<f64>() / possessions.len() as f64;
    assert!(
        (150.0..=280.0).contains(&mean_points),
        "mean total points {mean_points:.1} outside the plausible range"
    );
    assert!(
        (170.0..=230.0).contains(&mean_possessions),
        "mean possessions {mean_possessions:.1} outside the plausible range"
    );
}

#[test]
fn shots_are_geometrically_legal_across_a_batch() {
    let reports: Vec<_> = [21u64, 22, 23].iter().map(|seed| play(*seed)).collect();
    let report = gate::evaluate_batch(&reports, &params());
    assert_eq!(
        report.geometric_violations, 0,
        "geometric violations must be zero"
    );
    assert_eq!(
        report.context_violations, 0,
        "context violations must be zero"
    );
}

#[test]
fn zone_and_creation_shares_are_complete() {
    let reports: Vec<_> = [31u64, 32, 33, 34].iter().map(|seed| play(*seed)).collect();
    let zone_total: u32 = reports
        .iter()
        .map(|r| r.shot_zone_counts.iter().flatten().sum::<u32>())
        .sum();
    let creation_total: u32 = reports
        .iter()
        .map(|r| r.creation_counts.iter().sum::<u32>())
        .sum();
    assert!(zone_total > 300, "batch must contain enough shots");
    assert_eq!(
        zone_total, creation_total,
        "every shot has one zone and one creation"
    );
    for report in &reports {
        for zone in 0..4 {
            let count = report.shot_zone_counts[0][zone] + report.shot_zone_counts[1][zone];
            assert!(count > 0, "every zone must appear in a full game");
        }
    }
}

#[test]
fn three_point_skill_moves_three_point_output() {
    let base = Params::load(&workspace_calibration_dir()).expect("tables load");
    let rules = nba_domain::GameRules::default();
    // 低档与高档对着同一批种子跑，主队属性分别取 0.10 与 0.95。
    let mut low_setup = MatchSetup::builtin(rules.clone());
    for player in low_setup.home.players.iter_mut() {
        player.attributes.shooting_three = 0.10;
    }
    let mut high_setup = MatchSetup::builtin(rules.clone());
    for player in high_setup.home.players.iter_mut() {
        player.attributes.shooting_three = 0.95;
    }
    let mut made_low = 0u32;
    let mut attempts_low = 0u32;
    let mut made_high = 0u32;
    let mut attempts_high = 0u32;
    // 三分命中率的种子间方差远大于技巧效应，样本取十二个种子。
    for seed in 41u64..53 {
        let mut low = World::new(low_setup.clone(), clone_params(&base), seed);
        let low_report = low.run();
        let low_line = low_report.box_score.team_totals(TeamSide::Home);
        made_low += low_line.tpm;
        attempts_low += low_line.tpa;

        let mut high = World::new(high_setup.clone(), clone_params(&base), seed);
        let high_report = high.run();
        let high_line = high_report.box_score.team_totals(TeamSide::Home);
        made_high += high_line.tpm;
        attempts_high += high_line.tpa;
    }
    let low_pct = made_low as f64 / attempts_low.max(1) as f64;
    let high_pct = made_high as f64 / attempts_high.max(1) as f64;
    assert!(
        high_pct > low_pct,
        "raising three point skill must raise three point percentage ({low_pct:.3} -> {high_pct:.3})"
    );
}

fn clone_params(params: &Params) -> Params {
    params.clone()
}

#[test]
fn session_service_advances_and_reports() {
    use nba_sim_core::SessionService;
    let mut service = SessionService::new(11);
    service
        .setup(
            Params::load(&workspace_calibration_dir()).expect("tables load"),
            11,
        )
        .expect("session setup");
    assert!(!service.is_configured() || service.snapshot().is_some());
    service.start().expect("session start");
    for _ in 0..20 {
        service.tick(0.5).expect("session tick");
    }
    let tick = service.snapshot().expect("snapshot after ticks");
    assert!(tick.frame.tick_index > 0);
    service.fast_forward(120.0).expect("fast forward");
    let report = service.report().expect("report after fast forward");
    assert!(
        !report.possessions.is_empty(),
        "possessions must accumulate"
    );
    assert!(report.home_score + report.away_score > 0);
    let events = service.events(10);
    assert!(!events.is_empty(), "event history must be retained");
}

#[test]
fn stream_ticks_round_trip_through_protocol() {
    let rules = nba_domain::GameRules::default();
    let mut world = World::new(
        MatchSetup::builtin(rules),
        Params::load(&workspace_calibration_dir()).expect("tables load"),
        5,
    );
    let mut checked = 0;
    for _ in 0..3000 {
        world.step(0.05);
        if world.clock.tick % 250 != 0 {
            continue;
        }
        let tick = world.stream_tick();
        let text = serde_json::to_string(&tick).expect("tick serializes");
        let parsed: nba_protocol::StreamTick =
            serde_json::from_str(&text).expect("tick parses back");
        assert_eq!(parsed.frame.tick_index, tick.frame.tick_index);
        assert_eq!(parsed.frame.score.home, tick.frame.score.home);
        assert_eq!(parsed.frame.players.len(), tick.frame.players.len());
        assert!(
            parsed.frame.rules.is_present(),
            "rules must travel with the tick"
        );
        checked += 1;
    }
    assert!(checked >= 10, "the round trip must cover several ticks");
}
