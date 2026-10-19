use nba_sim_core::action::{Fact, TurnoverKind};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::Phase;
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

/// 封盖与抢断是动作，不是聚合项：同一种子下只改防守人的能力，
/// 封盖/抢断次数必须跟着变。
#[test]
fn defensive_skill_changes_blocks_and_steals() {
    let base = params();
    let rules = nba_domain::GameRules::default();
    let mut weak = MatchSetup::builtin(rules.clone());
    for player in weak.home.players.iter_mut() {
        player.attributes.block = 0.1;
        player.attributes.steal = 0.1;
    }
    let mut strong = MatchSetup::builtin(rules.clone());
    for player in strong.home.players.iter_mut() {
        player.attributes.block = 0.95;
        player.attributes.steal = 0.95;
    }
    let mut weak_blocks = 0u32;
    let mut strong_blocks = 0u32;
    let mut weak_steals = 0u32;
    let mut strong_steals = 0u32;
    for seed in 31u64..39 {
        let weak_report = World::new(weak.clone(), clone_params(&base), seed).run();
        let weak_line = weak_report.box_score.team_totals(TeamSide::Home);
        weak_blocks += weak_line.blocks;
        weak_steals += weak_line.steals;

        let strong_report = World::new(strong.clone(), clone_params(&base), seed).run();
        let strong_line = strong_report.box_score.team_totals(TeamSide::Home);
        strong_blocks += strong_line.blocks;
        strong_steals += strong_line.steals;
    }
    assert!(
        strong_blocks > weak_blocks,
        "raising block skill must raise blocks ({weak_blocks} -> {strong_blocks})"
    );
    assert!(
        strong_steals > weak_steals,
        "raising steal skill must raise steals ({weak_steals} -> {strong_steals})"
    );
}

/// 投篮犯规里的 and-1 是少数：命中不再走普通命中率，而是独立的
/// and-1 概率；否则会在犯规分支里重复一次投进。
#[test]
fn and_one_is_a_minority_of_shooting_fouls() {
    let mut fouls = 0u32;
    let mut and_one = 0u32;
    for seed in 51u64..61 {
        let report = play(seed);
        for team in [TeamSide::Home, TeamSide::Away] {
            let totals = report.box_score.team_totals(team);
            fouls += totals.shooting_fouls;
            and_one += totals.and_one;
        }
    }
    assert!(fouls > 100, "sample too small: {fouls} shooting fouls");
    let share = and_one as f64 / fouls as f64;
    // 这里只查“少数”这个性质；具体份额由批次判据 and_one_per_game 盯。
    assert!(
        (0.01..0.25).contains(&share),
        "and-1 share of shooting fouls must stay a minority: {share:.3} ({and_one}/{fouls})"
    );
}

/// 非投篮犯规（手部犯规、移动掩护）必须真的发生：总犯规数大于
/// 投篮犯规数，否则 bonus 永远不触发。
#[test]
fn non_shooting_fouls_occur() {
    let mut total = 0u32;
    let mut shooting = 0u32;
    for seed in 71u64..81 {
        let report = play(seed);
        for team in [TeamSide::Home, TeamSide::Away] {
            let totals = report.box_score.team_totals(team);
            total += totals.fouls;
            shooting += totals.shooting_fouls;
        }
    }
    assert!(
        total > shooting,
        "non-shooting fouls must occur: {total} vs {shooting}"
    );
}

/// 24 秒：把回合时钟拨到零，下一个刻必须吹哨并转换球权。
#[test]
fn shot_clock_violation_ends_the_possession() {
    let rules = nba_domain::GameRules::default();
    let mut world = World::new(MatchSetup::builtin(rules), params(), 5);
    let mut guard = 0;
    while !matches!(world.phase, Phase::Live) {
        world.step(0.05);
        guard += 1;
        assert!(guard < 4000, "world never reached live play");
    }
    let before = world.report().possessions.len();
    world.clock.shot_clock = 0.0;
    world.step(0.05);
    let report = world.report();
    let turnover = report
        .events
        .iter()
        .rev()
        .find(|event| matches!(event.fact, Fact::Turnover { .. }))
        .expect("shot clock must produce a turnover fact");
    match turnover.fact {
        Fact::Turnover { kind, .. } => assert_eq!(kind, TurnoverKind::ShotClock),
        _ => unreachable!(),
    }
    assert!(
        report.possessions.len() > before,
        "shot clock violation must close the possession"
    );
    assert_eq!(
        report.possessions.last().map(|record| record.ended_by),
        Some("shot_clock")
    );
}

/// 罚球程序：每一次投篮犯规之后，罚球事实的次数与顺序等于判罚的
/// 罚球数；命中数与箱体上的罚球命中一致。
#[test]
fn free_throw_facts_match_the_called_foul() {
    let report = play(17);
    let mut sequences = 0;
    for (index, event) in report.events.iter().enumerate() {
        let Fact::Foul {
            shooting: true,
            free_throws,
            ..
        } = event.fact
        else {
            continue;
        };
        let mut seen = 0u8;
        for next in &report.events[index + 1..] {
            if let Fact::FreeThrow {
                index: attempt,
                total,
                ..
            } = next.fact
            {
                seen += 1;
                assert_eq!(total, free_throws, "attempt total must match the call");
                assert_eq!(attempt, seen, "attempt index must be sequential");
            }
            if seen == free_throws {
                break;
            }
        }
        assert_eq!(
            seen, free_throws,
            "the called foul must be followed by exactly {free_throws} free throws"
        );
        sequences += 1;
    }
    assert!(
        sequences > 5,
        "expected several shooting fouls, got {sequences}"
    );

    let made: u32 = report
        .events
        .iter()
        .filter(|event| matches!(event.fact, Fact::FreeThrow { made: true, .. }))
        .count() as u32;
    let totals = report.box_score.team_totals(TeamSide::Home);
    let away = report.box_score.team_totals(TeamSide::Away);
    assert_eq!(
        made,
        totals.ftm + away.ftm,
        "made free throws must match the box score"
    );
}

/// 二次运球：收球之后再发起运球，判 `DoubleDribble` 并转换球权。
#[test]
fn double_dribble_after_gather_is_a_turnover() {
    use nba_sim_core::action::{ActionIntent, ActionKind, TurnoverKind};
    use nba_sim_core::state::DribbleState;

    let rules = nba_domain::GameRules::default();
    let mut world = World::new(MatchSetup::builtin(rules), params(), 5);
    let mut guard = 0;
    while !matches!(world.phase, Phase::Live) {
        world.step(0.05);
        guard += 1;
        assert!(guard < 4000, "world never reached live play");
    }
    let actor = world.holder().expect("live play has a holder");
    world.possession.dribble_state = DribbleState::Gathered;
    world.possession.gather_pos = Some(world.players[actor].pos);
    world.active = Some(ActionIntent {
        actor,
        kind: ActionKind::Dribble,
        target: None,
        from_ft: world.players[actor].pos,
        to_ft: world.players[actor].pos,
        start_time: world.now(),
        duration: 1.4,
    });
    let outcome = nba_sim_core::resolve::resolve(&mut world);
    assert_eq!(outcome.completion, nba_sim_core::action::Completion::Failed);
    assert_eq!(outcome.delta.turnover, Some(TurnoverKind::DoubleDribble));
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
