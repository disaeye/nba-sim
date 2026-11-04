use nba_sim_core::action::{Fact, TurnoverKind};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::Phase;
use nba_sim_core::{MatchSetup, Params, TeamSide, World};

mod common;

fn short_play(seed: u64) -> nba_sim_core::GameReport {
    common::short_world(seed).run()
}

fn params() -> Params {
    Params::load(&workspace_calibration_dir()).expect("calibration tables load")
}

#[test]
fn same_seed_replays_identically() {
    let first = short_play(7);
    let second = short_play(7);
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
    let report = short_play(11);
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
    let report = short_play(13);
    let limit = nba_domain::GameRules::default()
        .league
        .fouls
        .max_personal_fouls as u32;
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
    let report = short_play(17);
    let period = common::nba_short().league.periods.period_duration_seconds as f64;
    let overtime = common::nba_short().league.periods.overtime_duration_seconds as f64;
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

/// 24 秒：把回合时钟拨到零，下一个刻必须吹哨并转换球权。
#[test]
fn shot_clock_violation_ends_the_possession() {
    let rules = nba_domain::GameRules::default();
    let mut world = World::new(MatchSetup::builtin(rules), params(), 5);
    let mut guard = 0;
    while !matches!(world.phase(), Phase::Live) {
        world.step(0.05);
        guard += 1;
        assert!(guard < 4000, "world never reached live play");
    }
    let before = world.report().possessions.len();
    world.clock.set_shot_clock(0.0);
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
    // 罚球序列需要完整比赛的样本量（短场只有一两个投篮犯规）。
    let report = nba_sim_core::run_game(17, &workspace_calibration_dir());
    let mut sequences = 0;
    for event in &report.events {
        let Fact::Foul {
            kind: nba_domain::event::FoulKind::Shooting,
            free_throws,
            foul_id,
            ..
        } = event.fact
        else {
            continue;
        };
        let mut seen = 0u8;
        for next in report
            .events
            .iter()
            .filter(|next| next.event_id > event.event_id)
        {
            if let Fact::FreeThrow {
                index: attempt,
                total,
                foul_id: free_throw_foul_id,
                ..
            } = next.fact
            {
                if free_throw_foul_id != foul_id {
                    continue;
                }
                seen += 1;
                assert_eq!(total, free_throws, "attempt total must match the call");
                assert_eq!(attempt, seen, "attempt index must be sequential");
                if seen == free_throws {
                    break;
                }
            }
        }
        assert_eq!(
            seen, free_throws,
            "called foul {foul_id} has incomplete free throws"
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
    while !(matches!(world.phase(), Phase::Live) && world.holder().is_some()) {
        world.step(0.05);
        guard += 1;
        assert!(guard < 4000, "world never reached live play");
    }
    let actor = world.holder().expect("live play has a holder");
    world.possession.dribble_state = DribbleState::Gathered;
    world.possession.gather_pos = Some(world.players[actor].pos);
    // 收球确立轴心脚：二次运球按「收球 → 轴心脚 → 释放」判定。
    let actor_pos = world.players[actor].pos;
    nba_sim_core::footwork::establish_pivot(&mut world, actor, actor_pos);
    world.interactions.clear();
    let intent = ActionIntent {
        actor,
        kind: ActionKind::Dribble(nba_domain::action_window::DribbleMoveKind::Crossover),
        target: None,
        from_ft: world.players[actor].pos,
        to_ft: world.players[actor].pos,
        start_time: world.now(),
        duration: 1.4,
    };
    let outcome = nba_sim_core::resolve::resolve(&mut world, intent);
    assert_eq!(outcome.completion, nba_sim_core::action::Completion::Failed);
    assert_eq!(outcome.delta.turnover, Some(TurnoverKind::DoubleDribble));
}

/// 走步判定：收球确立轴心脚，移动量程由身高给出。收球后的第三步是
/// 走步，两步之内合法。
#[test]
fn the_footwork_model_counts_steps_instead_of_displacement() {
    use nba_domain::footwork::PivotState;
    let mut pivot = PivotState::new(nba_domain::action_window::Foot::Right, glam::Vec2::ZERO, 2);
    assert!(pivot.has_step_left());
    // 两步之内：还有额度。
    pivot.record_contact(glam::Vec2::new(2.0, 0.0));
    assert!(pivot.has_step_left());
    pivot.record_contact(glam::Vec2::new(4.0, 0.0));
    assert!(!pivot.has_step_left());
    // 轴心脚被移动：抬起的那只脚就是轴心脚，落点与原位不同即移动。
    assert!(!pivot.pivot_moved(nba_domain::action_window::Foot::Right, glam::Vec2::new(0.2, 0.0)));
    assert!(pivot.pivot_moved(nba_domain::action_window::Foot::Right, glam::Vec2::new(1.0, 0.0)));
    // 抬起的是另一只脚：不算移动轴心脚。
    assert!(!pivot.pivot_moved(nba_domain::action_window::Foot::Left, glam::Vec2::new(1.0, 0.0)));
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
    let first_event_id = events.first().expect("retained event").event_id;
    let incremental = service
        .events_since(first_event_id - 1, 10)
        .expect("event cursor is within retained history");
    assert_eq!(incremental.events[0].event_id, first_event_id);
    assert_eq!(
        incremental.next_after_event_id,
        incremental.events.last().unwrap().event_id
    );
    assert!(!incremental.events.is_empty());
    assert!(service.events_since(u64::MAX, 10).is_err());
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
        if !world.clock.tick().is_multiple_of(250) {
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
