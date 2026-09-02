//! M8 评判器测试：准则行为 + 端到端工件完整性。

use nba_evaluator::{
    attribution_report, evaluate_stream, observed_possession_indices, parse_stream,
    ReferenceDistributions, Verdict,
};

#[test]
fn embedded_fixture_parses_and_is_versioned() {
    let f = ReferenceDistributions::nba_v1();
    assert!(f.version.starts_with("nba."));
    assert!(f.pass_corridor_radius_ft > 0.0);
    assert!(!f.phase_transitions.is_empty());
}

#[test]
fn fiba_embedded_fixture_parses_and_matches_shapes() {
    let f = ReferenceDistributions::fiba_v1();
    assert_eq!(f.league, "FIBA");
    assert_eq!(f.version, "fiba.v1");
    assert_eq!(f.court_width_ft, 91.86);
    assert_eq!(f.court_height_ft, 49.21);
    assert!(!f.phase_transitions.is_empty());

    let for_fiba = ReferenceDistributions::for_league("fiba");
    assert_eq!(for_fiba.league, "FIBA");
    let for_nba = ReferenceDistributions::for_league("nba");
    assert_eq!(for_nba.league, "NBA");
}

#[test]
fn empty_stream_yields_no_judgments() {
    let f = ReferenceDistributions::nba_v1();
    assert!(evaluate_stream(&[], &f).is_empty());
}

#[test]
fn possession_coverage_flags_missing_indices() {
    let f = ReferenceDistributions::nba_v1();
    let tick_json = |idx: u64| {
        serde_json::json!({
            "t": 0.0, "t_game": 720.0, "shotClock": 24.0, "period": 1,
            "phase": "Initiation", "possession_id": 1, "possession_team": "home",
            "score": {"home": 0, "away": 0}, "players": [],
            "ball": {"x": 0.5, "y": 0.5, "z": 4.0, "status": "HELD", "holderId": "H_1"},
            "event_log": [{
                "sequence": 1, "time": 0.0, "kind": "POSSESSION_SUMMARY",
                "data": {"PossessionSummary": {
                    "possession_index": idx, "offense_team": "home",
                    "start_clock": 720.0, "end_clock": 700.0,
                    "duration_seconds": 20.0, "passes_count": 2,
                    "terminal_event": "SCORE", "shooter_id": "H_1",
                    "shot_contest_intensity": 0.3
                }}
            }],
            "rules": {
                "tick_seconds": 0.04, "court_width_ft": 94.0, "court_height_ft": 50.0,
                "hoop_left_x_ft": 5.25, "hoop_right_x_ft": 88.75, "hoop_y_ft": 25.0,
                "player_radius_ft": 1.0, "min_player_separation_ft": 3.6,
                "max_player_speed_ftps": 22.0, "max_player_accel_ftps2": 35.0,
                "ball_max_speed_ftps": 85.0, "three_point_distance_ft": 23.75,
                "shot_clock_seconds": 24.0, "holder_leash_ft": 3.0,
                "speed_tolerance_ftps": 1.5, "ball_z_max_ft": 35.0
            },
            "tactical_set": "T", "gameClock": 720.0, "keyframeIndex": null
        })
        .to_string()
    };
    let stream = format!("{}\n{}", tick_json(0), tick_json(2));
    let ticks = parse_stream(&stream);
    let seen = observed_possession_indices(&ticks);
    assert_eq!(seen.len(), 2, "both synthetic summaries must parse");
    let judgments = evaluate_stream(&ticks, &f);
    assert!(judgments.iter().any(|j| {
        j.criterion == "POSSESSION_COVERAGE"
            && j.verdict == Verdict::Defect
            && j.detail.contains("1 possession summary missing")
    }));
}

#[test]
fn end_to_end_real_stream_is_fully_judged() {
    // 用引擎真实跑一节，评判每个回合 + 阶段转换；工件完整性断言：
    // 结构因果准则对每个得分/失误回合都必须有裁决（quality.md §2.1）。
    let mut engine = nba_engine::MatchEngine::new(7);
    let mut ndjson = String::new();
    for _ in 0..6000 {
        if engine.is_finished() {
            break;
        }
        let tick = engine.step();
        ndjson.push_str(&serde_json::to_string(&tick).unwrap());
        ndjson.push('\n');
    }
    let ticks = parse_stream(&ndjson);
    assert!(!ticks.is_empty());
    let fixture = ReferenceDistributions::nba_v1();
    let judgments = evaluate_stream(&ticks, &fixture);

    // 每个回合时长准则必须覆盖（回合零遗漏的代理断言）。
    let seen = observed_possession_indices(&ticks);
    let duration_judged: std::collections::HashSet<u64> = judgments
        .iter()
        .filter(|j| j.criterion == "POSSESSION_DURATION_BOUNDS")
        .filter_map(|j| j.possession)
        .collect();
    assert_eq!(duration_judged, seen, "every possession must be duration-judged");

    // 得分回合必须有得分来源裁决（非得分回合不适用该准则）。
    let scoring: std::collections::HashSet<u64> = ticks
        .iter()
        .flat_map(|t| {
            t.frame
                .event_log
                .iter()
                .filter(|e| e.kind == "POSSESSION_SUMMARY")
                .filter_map(|e| e.data.as_ref())
                .filter_map(|d| d.as_object().and_then(|o| o.values().next()).cloned())
                .filter_map(|v| {
                    v.get("terminal_event")
                        .and_then(|t| t.as_str().map(String::from))
                        .and_then(|te| {
                            v.get("possession_index").and_then(|i| i.as_u64()).map(|i| (i, te))
                        })
                })
                .filter(|(_, te)| te == "SCORE")
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
        })
        .collect();
    for &idx in &scoring {
        assert!(
            judgments
                .iter()
                .any(|j| j.criterion == "SCORE_SOURCE_CAUSALITY" && j.possession == Some(idx)),
            "possession {} missing SCORE_SOURCE_CAUSALITY judgment",
            idx
        );
    }

    // 归因账本可聚合且指数在 [0,1]。
    let report = attribution_report(&judgments, &fixture.version);
    assert!((0.0..=1.0).contains(&report.realism_index));
    assert_eq!(report.total_judgments, judgments.len());
}

#[test]
fn shot_quality_contest_pass_judgment_is_emitted() {
    let fixture = ReferenceDistributions::nba_v1();
    let tick_json = serde_json::json!({
        "t": 0.0, "t_game": 720.0, "shotClock": 24.0, "period": 1,
        "phase": "Initiation", "possession_id": 1, "possession_team": "home",
        "score": {"home": 0, "away": 0}, "players": [],
        "ball": {"x": 0.5, "y": 0.5, "z": 4.0, "status": "HELD", "holderId": "H_1"},
        "event_log": [
            {
                "sequence": 1, "time": 0.0, "kind": "SHOT_RELEASE",
                "data": {"ShotRelease": {
                    "shooter_id": "H_1", "is_three": false, "contest_level": 0.2
                }}
            },
            {
                "sequence": 2, "time": 0.0, "kind": "POSSESSION_SUMMARY",
                "data": {"PossessionSummary": {
                    "possession_index": 1, "offense_team": "home",
                    "start_clock": 720.0, "end_clock": 705.0,
                    "duration_seconds": 15.0, "passes_count": 2,
                    "terminal_event": "SCORE", "shooter_id": "H_1",
                    "shot_contest_intensity": 0.2
                }}
            }
        ],
        "rules": {
            "tick_seconds": 0.04, "court_width_ft": 94.0, "court_height_ft": 50.0,
            "hoop_left_x_ft": 5.25, "hoop_right_x_ft": 88.75, "hoop_y_ft": 25.0,
            "player_radius_ft": 1.0, "min_player_separation_ft": 3.6,
            "max_player_speed_ftps": 22.0, "max_player_accel_ftps2": 35.0,
            "ball_max_speed_ftps": 85.0, "three_point_distance_ft": 23.75,
            "shot_clock_seconds": 24.0, "holder_leash_ft": 3.0,
            "speed_tolerance_ftps": 1.5, "ball_z_max_ft": 35.0
        },
        "tactical_set": "T", "gameClock": 720.0, "keyframeIndex": null
    }).to_string();
    let ticks = parse_stream(&tick_json);
    let judgments = evaluate_stream(&ticks, &fixture);
    assert!(
        judgments
            .iter()
            .any(|j| j.criterion == "SHOT_QUALITY_CONTEST" && j.verdict == Verdict::Pass),
        "SHOT_QUALITY_CONTEST pass judgment should be emitted when contest <= threshold"
    );
}

#[test]
fn pbp_converter_generates_valid_distributions() {
    let sample_events = vec![
        nba_evaluator::PbpEvent {
            period: 1,
            game_clock: 710.0,
            event_type: "SCORE".to_string(),
            possession_id: 1,
            team: "HOME".to_string(),
            is_three: Some(true),
            contest_level: Some(0.2),
            duration_seconds: Some(14.5),
            pass_count: Some(3),
        },
        nba_evaluator::PbpEvent {
            period: 1,
            game_clock: 695.0,
            event_type: "TURNOVER".to_string(),
            possession_id: 2,
            team: "AWAY".to_string(),
            is_three: None,
            contest_level: None,
            duration_seconds: Some(12.0),
            pass_count: Some(1),
        },
        nba_evaluator::PbpEvent {
            period: 1,
            game_clock: 680.0,
            event_type: "DEFENSIVE_REBOUND".to_string(),
            possession_id: 3,
            team: "HOME".to_string(),
            is_three: Some(false),
            contest_level: Some(0.4),
            duration_seconds: Some(18.0),
            pass_count: Some(4),
        },
    ];

    let fixture = nba_evaluator::convert_pbp_events_to_fixture(&sample_events, "pbp.test.v1", "NBA");
    assert_eq!(fixture.version, "pbp.test.v1");
    assert_eq!(fixture.league, "NBA");
    assert!(fixture.duration_bands.score.min <= 14.5);
    assert!(fixture.duration_bands.score.max >= 14.5);
    assert!(fixture.passes_bands.score.min <= 3.0);
    assert!(fixture.passes_bands.score.max >= 3.0);
}
