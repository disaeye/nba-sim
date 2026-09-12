//! M8 评判器测试：准则行为 + 端到端工件完整性。

use nba_evaluator::{
    attribution_report, evaluate_stream, observed_possession_indices, parse_stream,
    Judgment, ReferenceDistributions, Verdict,
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
fn empty_stream_yields_insufficient_not_empty() {
    // D1 证据模型：空流 = 零证据，必须显式产出 InsufficientEvidence，
    // 不得静默产空裁决（那会被读成"无缺陷"的假安全感）。
    let f = ReferenceDistributions::nba_v1();
    let judgments = evaluate_stream(&[], &f);
    assert!(
        judgments.iter().all(|j| j.verdict == Verdict::InsufficientEvidence),
        "empty stream must yield only InsufficientEvidence, got {:?}",
        judgments
    );
    // 且空流报告不得满分（D1.1）。
    let report = attribution_report(&judgments, "nba.v1");
    assert_eq!(report.passes, 0, "empty stream must have zero passes");
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
    let ticks = parse_stream(&stream).expect("parse");
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
    let ticks = parse_stream(&ndjson).expect("parse");
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
    assert_eq!(
        duration_judged, seen,
        "every possession must be duration-judged"
    );

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
                            v.get("possession_index")
                                .and_then(|i| i.as_u64())
                                .map(|i| (i, te))
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
    })
    .to_string();
    let ticks = parse_stream(&tick_json).expect("parse");
    let judgments = evaluate_stream(&ticks, &fixture);
    assert!(
        judgments
            .iter()
            .any(|j| j.criterion == "SHOT_QUALITY_CONTEST" && j.verdict == Verdict::Pass),
        "SHOT_QUALITY_CONTEST pass judgment should be emitted when contest <= threshold"
    );
}

#[test]
fn tipped_pass_is_a_valid_turnover_attribution() {
    let fixture = ReferenceDistributions::nba_v1();
    let tick = serde_json::json!({
        "t": 1.0, "t_game": 719.0, "shotClock": 23.0, "period": 1,
        "phase": "Initiation", "possession_id": 1, "possession_team": "home",
        "score": {"home": 0, "away": 0}, "players": [],
        "ball": {"x": 0.5, "y": 0.5, "z": 2.0, "status": "LOOSE_BALL", "holderId": null},
        "events": ["PASS_TIPPED"],
        "event_log": [
            {"sequence": 1, "time": 1.0, "kind": "PASS_TIPPED", "data": {"PassTipped": {
                "passer_id": "H_1", "receiver_id": "H_2", "defender_id": "A_1", "position": [47.0, 25.0]
            }}},
            {"sequence": 2, "time": 1.0, "kind": "POSSESSION_SUMMARY", "data": {"PossessionSummary": {
                "possession_index": 0, "offense_team": "home", "start_clock": 720.0,
                "end_clock": 719.0, "duration_seconds": 1.0, "passes_count": 0,
                "terminal_event": "TURNOVER_PASS_TIPPED"
            }}}
        ],
        "rules": {"tick_seconds": 0.04, "court_width_ft": 94.0, "court_height_ft": 50.0,
            "hoop_left_x_ft": 5.25, "hoop_right_x_ft": 88.75, "hoop_y_ft": 25.0,
            "player_radius_ft": 1.0, "min_player_separation_ft": 3.6,
            "max_player_speed_ftps": 22.0, "max_player_accel_ftps2": 35.0,
            "ball_max_speed_ftps": 85.0, "three_point_distance_ft": 23.75,
            "shot_clock_seconds": 24.0, "holder_leash_ft": 3.0,
            "speed_tolerance_ftps": 1.5, "ball_z_max_ft": 35.0},
        "tactical_set": "T", "gameClock": 719.0, "keyframeIndex": null
    }).to_string();
    let judgments = evaluate_stream(&parse_stream(&tick).expect("parse"), &fixture);
    assert!(judgments.iter().any(|judgment| {
        judgment.criterion == "TURNOVER_ATTRIBUTION"
            && judgment.verdict == Verdict::Pass
            && judgment.possession == Some(0)
    }));
}

#[test]
fn frame_event_fallback_does_not_leak_across_possession_boundary() {
    let fixture = ReferenceDistributions::nba_v1();
    let rules = serde_json::json!({
        "tick_seconds": 0.04, "court_width_ft": 94.0, "court_height_ft": 50.0,
        "hoop_left_x_ft": 5.25, "hoop_right_x_ft": 88.75, "hoop_y_ft": 25.0,
        "player_radius_ft": 1.0, "min_player_separation_ft": 3.6,
        "max_player_speed_ftps": 22.0, "max_player_accel_ftps2": 35.0,
        "ball_max_speed_ftps": 85.0, "three_point_distance_ft": 23.75,
        "shot_clock_seconds": 24.0, "holder_leash_ft": 3.0,
        "speed_tolerance_ftps": 1.5, "ball_z_max_ft": 35.0
    });
    let summary = |index: u64, terminal: &str| {
        serde_json::json!({
            "possession_index": index, "offense_team": "home",
            "start_clock": 720.0 - index as f32, "end_clock": 719.0 - index as f32,
            "duration_seconds": 1.0, "passes_count": 0,
            "terminal_event": terminal, "turnover_player_id": "H_1"
        })
    };
    let first = serde_json::json!({
        "t": 1.0, "t_game": 719.0, "shotClock": 23.0, "period": 1,
        "phase": "ActionExecution", "possession_id": 1, "possession_team": "home",
        "score": {"home": 0, "away": 0}, "players": [],
        "ball": {"x": 0.5, "y": 0.5, "z": 2.0, "status": "LOOSE_BALL", "holderId": null},
        "events": ["PASS_DROPPED"],
        "event_log": [
            {"sequence": 1, "time": 1.0, "kind": "PASS_DROPPED", "data": {"PassDropped": {
                "passer_id": "H_1", "receiver_id": "H_2", "position": [47.0, 25.0]
            }}},
            {"sequence": 2, "time": 1.0, "kind": "POSSESSION_SUMMARY", "data": {"PossessionSummary": summary(0, "TURNOVER_PASS_DROPPED")}}
        ],
        "rules": rules, "tactical_set": "T", "gameClock": 719.0, "keyframeIndex": null
    });
    let second = serde_json::json!({
        "t": 2.0, "t_game": 718.0, "shotClock": 22.0, "period": 1,
        "phase": "ActionExecution", "possession_id": 2, "possession_team": "home",
        "score": {"home": 0, "away": 0}, "players": [],
        "ball": {"x": 0.5, "y": 0.5, "z": 2.0, "status": "HELD", "holderId": "H_1"},
        "events": [],
        "event_log": [{"sequence": 3, "time": 2.0, "kind": "POSSESSION_SUMMARY", "data": {"PossessionSummary": summary(1, "TURNOVER_PASS_DROPPED")}}],
        "rules": rules, "tactical_set": "T", "gameClock": 718.0, "keyframeIndex": null
    });
    let stream = format!("{}\n{}", first, second);
    let judgments = evaluate_stream(&parse_stream(&stream).expect("parse"), &fixture);
    assert!(judgments.iter().any(|judgment| {
        judgment.criterion == "TURNOVER_ATTRIBUTION"
            && judgment.possession == Some(1)
            && judgment.verdict == Verdict::Defect
    }));
}

#[test]
fn turnover_actor_consistency_is_judged_from_summary() {
    let fixture = ReferenceDistributions::nba_v1();
    let tick = serde_json::json!({
        "t": 1.0, "t_game": 719.0, "shotClock": 23.0, "period": 1,
        "phase": "ActionExecution", "possession_id": 1, "possession_team": "home",
        "score": {"home": 0, "away": 0}, "players": [],
        "ball": {"x": 0.5, "y": 0.5, "z": 2.0, "status": "LOOSE_BALL", "holderId": null},
        "event_log": [{
            "sequence": 1, "time": 1.0, "kind": "POSSESSION_SUMMARY",
            "data": {"PossessionSummary": {
                "possession_index": 0, "offense_team": "home", "start_clock": 720.0,
                "end_clock": 719.0, "duration_seconds": 1.0, "passes_count": 0,
                "terminal_event": "TURNOVER_PASS_DROPPED", "turnover_player_id": "H_1"
            }}
        }],
        "rules": {"tick_seconds": 0.04, "court_width_ft": 94.0, "court_height_ft": 50.0,
            "hoop_left_x_ft": 5.25, "hoop_right_x_ft": 88.75, "hoop_y_ft": 25.0,
            "player_radius_ft": 1.0, "min_player_separation_ft": 3.6,
            "max_player_speed_ftps": 22.0, "max_player_accel_ftps2": 35.0,
            "ball_max_speed_ftps": 85.0, "three_point_distance_ft": 23.75,
            "shot_clock_seconds": 24.0, "holder_leash_ft": 3.0,
            "speed_tolerance_ftps": 1.5, "ball_z_max_ft": 35.0},
        "tactical_set": "T", "gameClock": 719.0, "keyframeIndex": null
    })
    .to_string();
    let judgments = evaluate_stream(&parse_stream(&tick).expect("parse"), &fixture);
    assert!(judgments.iter().any(|judgment| {
        judgment.criterion == "TURNOVER_ACTOR_CONSISTENCY" && judgment.verdict == Verdict::Pass
    }));
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

    let fixture =
        nba_evaluator::convert_pbp_events_to_fixture(&sample_events, "pbp.test.v1", "NBA");
    assert_eq!(fixture.version, "pbp.test.v1");
    assert_eq!(fixture.league, "NBA");
    assert!(fixture.duration_bands.score.min <= 14.5);
    assert!(fixture.duration_bands.score.max >= 14.5);
    assert!(fixture.passes_bands.score.min <= 3.0);
    assert!(fixture.passes_bands.score.max >= 3.0);
}

#[test]
fn try_parse_stream_rejects_corrupt_line() {
    let mut engine = nba_engine::MatchEngine::new(42);
    let valid_tick = engine.step();
    let valid_line = serde_json::to_string(&valid_tick).unwrap();
    let bad_data = format!("{}\n{{not_valid_json}}\n", valid_line);
    let res = nba_evaluator::try_parse_stream(&bad_data);
    assert!(res.is_err());
    let err_msg = res.unwrap_err();
    assert!(err_msg.contains("line 2"));
}

#[test]
fn empty_attribution_report_does_not_give_free_perfect_score() {
    let report = nba_evaluator::attribution_report(&[], "test.v1");
    assert_eq!(report.realism_index, 0.0);
    assert_eq!(report.total_judgments, 0);
}

#[test]
fn soft_criterion_pass_gets_soft_severity() {
    let j_pass = nba_evaluator::Judgment::pass("RHYTHM_DURATION", "decision", 1);
    assert_eq!(j_pass.severity, "soft");

    let j_pass_hard = nba_evaluator::Judgment::pass("POSSESSION_DURATION_BOUNDS", "engine", 1);
    assert_eq!(j_pass_hard.severity, "hard");
}

/// F2.1 红测试：窗口内存在 SCORE 事实时，SCORE_SOURCE_CAUSALITY 必须 Pass。
/// 修复前 made_arrivals 恒 0，该准则对每个得分回合都误报 Hard defect。
#[test]
fn score_fact_in_window_makes_source_causality_pass() {
    let fixture = ReferenceDistributions::nba_v1();
    let make_tick = |kind: &str, seq: u64, data: serde_json::Value| {
        serde_json::json!({
            "t": 0.0, "t_game": 720.0, "shotClock": 24.0, "period": 1,
            "phase": "Initiation", "possession_id": 1, "possession_team": "home",
            "score": {"home": 2, "away": 0}, "players": [],
            "ball": {"x": 0.5, "y": 0.5, "z": 4.0, "status": "HELD", "holderId": "H_1"},
            "event_log": [
                {"sequence": seq, "time": 0.0, "kind": kind, "data": data},
                {"sequence": seq + 1, "time": 0.0, "kind": "POSSESSION_SUMMARY",
                 "data": {"PossessionSummary": {
                    "possession_index": 1, "offense_team": "home",
                    "start_clock": 720.0, "end_clock": 705.0,
                    "duration_seconds": 15.0, "passes_count": 2,
                    "terminal_event": "SCORE", "shooter_id": "H_1",
                    "shot_contest_intensity": 0.2
                 }}}
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
        })
        .to_string()
    };

    // 正例：窗口内有 SCORE 到达事实。
    let with_source = make_tick(
        "SCORE",
        1,
        serde_json::json!({"HoopArrival": {
            "shooter_id": "H_1", "shot_origin": [10.0, 25.0],
            "is_made": true, "is_three": false, "contest_intensity": 0.2
        }}),
    );
    let judgments = evaluate_stream(&parse_stream(&with_source).expect("parse"), &fixture);
    assert!(
        judgments
            .iter()
            .any(|j| j.criterion == "SCORE_SOURCE_CAUSALITY" && j.verdict == Verdict::Pass),
        "SCORE fact must satisfy SCORE_SOURCE_CAUSALITY, got: {:?}",
        judgments
            .iter()
            .filter(|j| j.criterion == "SCORE_SOURCE_CAUSALITY")
            .collect::<Vec<_>>()
    );

    // 负面对照：窗口内没有得分来源事实，必须报 defect。
    let no_source = make_tick("SHOT_MISS", 1, serde_json::json!({"HoopArrival": {
        "shooter_id": "H_1", "shot_origin": [10.0, 25.0],
        "is_made": false, "is_three": false, "contest_intensity": 0.2
    }}));
    let judgments = evaluate_stream(&parse_stream(&no_source).expect("parse"), &fixture);
    assert!(
        judgments
            .iter()
            .any(|j| j.criterion == "SCORE_SOURCE_CAUSALITY" && j.verdict == Verdict::Defect),
        "missing source fact must fail SCORE_SOURCE_CAUSALITY"
    );
}

/// F2.1b 红测试：罚球得分回合（无出手到达事实、有 FT 命中）不适用
/// CONTEST_CONSISTENCY，不得报 Hard defect（gap.md §15.1 NotApplicable）。
#[test]
fn free_throw_score_is_not_applicable_for_contest_consistency() {
    let fixture = ReferenceDistributions::nba_v1();
    let tick = serde_json::json!({
        "t": 0.0, "t_game": 720.0, "shotClock": 24.0, "period": 1,
        "phase": "FreeThrow", "possession_id": 1, "possession_team": "home",
        "score": {"home": 1, "away": 0}, "players": [],
        "ball": {"x": 0.5, "y": 0.5, "z": 4.0, "status": "DEAD", "holderId": null},
        "event_log": [
            {"sequence": 1, "time": 0.0, "kind": "FREE_THROW",
             "data": {"FreeThrowAttempt": {"shooter_id": "H_1", "attempt": 1, "made": true}}},
            {"sequence": 2, "time": 0.0, "kind": "POSSESSION_SUMMARY",
             "data": {"PossessionSummary": {
                "possession_index": 1, "offense_team": "home",
                "start_clock": 720.0, "end_clock": 718.0,
                "duration_seconds": 2.0, "passes_count": 0,
                "terminal_event": "SCORE", "shooter_id": "H_1"
             }}}
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
    })
    .to_string();
    let judgments = evaluate_stream(&parse_stream(&tick).expect("parse"), &fixture);
    let contest: Vec<_> = judgments
        .iter()
        .filter(|j| j.criterion == "CONTEST_CONSISTENCY")
        .collect();
    assert!(
        contest.iter().all(|j| j.verdict == Verdict::NotApplicable),
        "free-throw score must be NotApplicable (no defect), got {:?}",
        contest
    );
    // 得分来源仍必须被满足（FT 命中即来源）。
    assert!(
        judgments
            .iter()
            .any(|j| j.criterion == "SCORE_SOURCE_CAUSALITY" && j.verdict == Verdict::Pass),
        "free-throw score must satisfy SCORE_SOURCE_CAUSALITY"
    );
}

/// F6.2：facts 紧凑流必须仍然能被完整评判（因果与回合准则不缺证据）。
#[test]
fn compact_facts_stream_is_fully_judged() {
    use nba_engine::StreamMode;
    let mut engine = nba_engine::MatchEngine::new(42);
    // RAII：panic 展开也会删除临时流（test-support）。
    let artifact = nba_test_support::TempArtifact::new("facts_judge");
    engine
        .simulate_scope_and_export_with_mode("1q", &artifact.path_str(), StreamMode::Facts)
        .expect("facts export");
    let content = std::fs::read_to_string(artifact.path()).expect("read facts stream");

    let ticks = nba_evaluator::try_parse_stream(&content).expect("facts stream must parse");
    assert!(!ticks.is_empty());
    let fixture = ReferenceDistributions::nba_v1();
    let judgments = evaluate_stream(&ticks, &fixture);
    assert!(!judgments.is_empty(), "facts stream must yield judgments");

    // 回合覆盖与得分因果必须在 facts 流上完整成立。
    let seen = observed_possession_indices(&ticks);
    assert!(!seen.is_empty(), "facts stream must carry possession summaries");
    let scoring: std::collections::HashSet<u64> = judgments
        .iter()
        .filter(|j| j.criterion == "SCORE_SOURCE_CAUSALITY")
        .filter_map(|j| j.possession)
        .collect();
    assert!(
        !scoring.is_empty(),
        "facts stream must produce SCORE_SOURCE_CAUSALITY judgments"
    );
    assert!(
        !judgments.iter().any(|j| {
            j.criterion == "SCORE_SOURCE_CAUSALITY"
                && j.verdict == Verdict::Defect
                && j.detail.contains("without shot/free-throw source")
        }),
        "facts stream must not introduce false score-causality defects"
    );
}

// ==================== D1 证据模型（dev 方案 §4）====================

/// D1.1/G-D1a：空流不得满分——零裁决的报告 realism_index 必须为 0，
/// 且五元组分母全部为 0、覆盖率为 0（空证据按 pass 计是 0.994 假安全感的来源）。
#[test]
fn empty_stream_scores_zero_not_full() {
    let report = attribution_report(&[], "nba.v1");
    assert_eq!(report.realism_index, 0.0, "empty stream must not score full marks");
    assert_eq!(report.opportunities, 0);
    assert_eq!(report.passes, 0);
    assert_eq!(report.defects, 0);
    assert_eq!(report.evidence_coverage, 0.0);
    assert!(!report.hard_gate_failed);
}

/// D1.2：任一 Hard defect 存在时 realism_index 无效（置 0 + hard_gate_failed），
/// 而不是一个仍接近 1 的数。
#[test]
fn hard_defect_invalidates_realism_index() {
    let mut judgments = vec![Judgment::pass("TURNOVER_RATE", "decision", 0); 9];
    judgments.push(Judgment::defect(
        "SCORE_SOURCE_CAUSALITY",
        "hard",
        "score without source".to_string(),
        "engine",
        0,
    ));
    let report = attribution_report(&judgments, "nba.v1");
    assert!(report.hard_gate_failed, "hard defect must trip the gate");
    assert_eq!(
        report.realism_index, 0.0,
        "hard defect must invalidate the index, got {}",
        report.realism_index
    );
    // 分母五元组：9 pass + 1 defect。
    assert_eq!(report.passes, 9);
    assert_eq!(report.defects, 1);
    assert_eq!(report.opportunities, 10);
    assert!((report.evidence_coverage - 1.0).abs() < f32::EPSILON);
}

/// D1.2 对照：全部 Soft defect 不触发 hard 门，指数仍反映缺陷率。
#[test]
fn soft_defects_do_not_trip_hard_gate() {
    let judgments = vec![
        Judgment::pass("TURNOVER_RATE", "decision", 0),
        Judgment::defect(
            "RHYTHM_DURATION",
            "soft",
            "duration off".to_string(),
            "decision",
            1,
        ),
    ];
    let report = attribution_report(&judgments, "nba.v1");
    assert!(!report.hard_gate_failed);
    assert!(
        report.realism_index > 0.0,
        "soft defects must not zero the index"
    );
}

/// D1.1：NotApplicable / InsufficientEvidence 不计入分母、不产生真实性贡献。
#[test]
fn not_applicable_and_insufficient_do_not_count() {
    let judgments = vec![
        Judgment::pass("TURNOVER_RATE", "decision", 0),
        Judgment::not_applicable("CONTEST_CONSISTENCY", "physics+semantics", 1),
        Judgment::insufficient("ASSIST_PROFILE", "decision", 2),
    ];
    let report = attribution_report(&judgments, "nba.v1");
    assert_eq!(report.opportunities, 3);
    assert_eq!(report.passes, 1);
    assert_eq!(report.not_applicable, 1);
    assert_eq!(report.insufficient_evidence, 1);
    // 覆盖率 = (passes+defects)/opportunities = 1/3。
    assert!(
        (report.evidence_coverage - 1.0 / 3.0).abs() < 1e-4,
        "coverage must exclude NA/insufficient, got {}",
        report.evidence_coverage
    );
    // 唯一证据是 pass，指数为 1.0（无 hard、无 defect）。
    assert!((report.realism_index - 1.0).abs() < f32::EPSILON);
}

/// D1.3：严格解析默认化——坏行必须整体报错，不得静默保留可解析部分。
#[test]
fn strict_parse_rejects_corrupt_stream_wholesale() {
    let good = r#"{"t":0.0,"t_game":720.0,"period":1,"phase":"LiveBall","possession_id":0,"score":{"home":0,"away":0}}"#;
    let corrupt = format!("{good}\n{{broken json\n{good}");
    let res = parse_stream(&corrupt);
    assert!(
        res.is_err(),
        "strict parse must reject the whole stream on a bad line"
    );
}

/// D1.3：罚球得分回合对 CONTEST_CONSISTENCY 产出显式 NotApplicable 裁决。
#[test]
fn free_throw_score_marks_contest_not_applicable() {
    let tick = r#"{"t":10.0,"t_game":700.0,"period":1,"phase":"FreeThrow","possession_id":0,"score":{"home":1,"away":0},"event_log":[{"sequence":1,"time":10.0,"kind":"FREE_THROW","data":{"FreeThrowAttempt":{"shooter_id":"H_1","attempt":1,"made":true}}},{"sequence":2,"time":10.0,"kind":"POSSESSION_SUMMARY","data":{"PossessionSummary":{"possession_index":0,"offense_team":"home","start_clock":720.0,"end_clock":700.0,"duration_seconds":20.0,"passes_count":0,"terminal_event":"SCORE","shooter_id":"H_1"}}}]}"#;
    let ticks = parse_stream(tick).expect("parse");
    let fixture = ReferenceDistributions::nba_v1();
    let judgments = evaluate_stream(&ticks, &fixture);
    let contest: Vec<_> = judgments
        .iter()
        .filter(|j| j.criterion == "CONTEST_CONSISTENCY")
        .collect();
    assert_eq!(contest.len(), 1, "FT score must yield exactly one contest judgment");
    assert_eq!(
        contest[0].verdict,
        Verdict::NotApplicable,
        "FT score contest judgment must be NotApplicable"
    );
}

// ==================== D2 构成准则簇（dev 方案 §5）====================

/// 构造比赛级合成流：给定出手/命中/回合数，生成对应事件序列。
/// 每个回合 = 一条 SHOT_RELEASE（或失误）+ 一条 POSSESSION_SUMMARY。
fn composition_synthetic_stream(
    three_attempts: usize,
    two_attempts: usize,
    three_makes: usize,
    two_makes: usize,
    turnovers: usize,
    possessions: usize,
    game_seconds: f32,
) -> String {
    let mut ticks = String::new();
    let mut seq = 0u64;
    let mut made3 = three_makes;
    let mut made2 = two_makes;
    // tick 间隔（秒）直接决定墙钟总长与 pace；回合时长取同一间隔。
    let dur = game_seconds.max(0.1);
    let mut emit = |kind: &str, data: String, summary_terminal: &str, idx: usize| {
        let mut events = String::new();
        if !kind.is_empty() {
            events.push_str(&format!(
                r#"{{"sequence":{},"time":10.0,"kind":"{}","data":{}}},"#,
                seq, kind, data
            ));
            seq += 1;
        }
        if kind == "SHOT_RELEASE" {
            // 命中事件
        }
        events.push_str(&format!(
            r#"{{"sequence":{},"time":10.0,"kind":"POSSESSION_SUMMARY","data":{{"PossessionSummary":{{"possession_index":{},"offense_team":"home","start_clock":720.0,"end_clock":700.0,"duration_seconds":{:.1},"passes_count":1,"terminal_event":"{}","shooter_id":"H_1","turnover_player_id":{}}}}}}}"#,
            seq,
            idx,
            dur,
            summary_terminal,
            if summary_terminal.starts_with("TURNOVER") { r#""H_1""#.to_string() } else { "null".to_string() },
        ));
        seq += 1;
        ticks.push_str(&format!(
            r#"{{"t":{:.1},"t_game":700.0,"period":1,"phase":"LiveBall","possession_id":{},"possession_team":"home","score":{{"home":0,"away":0}},"event_log":[{}]}}"#,
            dur * (idx + 1) as f32,
            idx,
            events.trim_end_matches(','),
        ));
        ticks.push('\n');
    };

    let mut idx = 0usize;
    for _ in 0..three_attempts {
        let made = made3 > 0;
        if made { made3 -= 1; }
        emit(
            "SHOT_RELEASE",
            r#"{"ShotRelease":{"shooter_id":"H_1","pos":[70.0,25.0],"is_three":true,"contest_level":0.0,"make_probability":0.37}}"#.to_string(),
            if made { "SCORE" } else { "DEFENSIVE_REBOUND" },
            idx,
        );
        if made {
            // 补 SCORE 事实
        }
        idx += 1;
    }
    for _ in 0..two_attempts {
        let made = made2 > 0;
        if made { made2 -= 1; }
        emit(
            "SHOT_RELEASE",
            r#"{"ShotRelease":{"shooter_id":"H_1","pos":[80.0,25.0],"is_three":false,"contest_level":0.0,"make_probability":0.5}}"#.to_string(),
            if made { "SCORE" } else { "DEFENSIVE_REBOUND" },
            idx,
        );
        idx += 1;
    }
    for _ in 0..turnovers {
        emit("", String::new(), "TURNOVER_STEAL", idx);
        idx += 1;
    }
    // 补齐回合数
    while idx < possessions {
        emit("", String::new(), "DEFENSIVE_REBOUND", idx);
        idx += 1;
    }
    ticks
}

fn composition_judgments(stream: &str) -> Vec<Judgment> {
    let ticks = parse_stream(stream).expect("parse synthetic");
    let fixture = ReferenceDistributions::nba_v2();
    evaluate_stream(&ticks, &fixture)
}

/// D2/G-D2a 合成流 A：3PA 率 0.8、中距离≈0 → SHOT_PROFILE_* 必须报 defect。
#[test]
fn composition_synthetic_a_three_heavy_is_flagged() {
    // 80 次三分 + 20 次两分（全在篮下），0 失误。
    let stream = composition_synthetic_stream(80, 20, 30, 10, 0, 100, 1200.0);
    let judgments = composition_judgments(&stream);
    let rate = judgments.iter().find(|j| j.criterion == "SHOT_PROFILE_3PA_RATE");
    assert!(
        matches!(rate, Some(j) if j.verdict == Verdict::Defect),
        "3PA rate 0.8 must be flagged: {:?}",
        rate
    );
}

/// D2/G-D2a 合成流 B：回合数 300（pace ~720）→ PACE_POSSESSIONS 必须报 defect。
#[test]
fn composition_synthetic_b_excess_pace_is_flagged() {
    // 300 事件 × 10s/tick ≈ 2990s → pace = 300*2880/2990 ≈ 289，远超带 [185,220]。
    let stream = composition_synthetic_stream(30, 70, 12, 30, 40, 300, 10.0);
    let judgments = composition_judgments(&stream);
    let pace = judgments.iter().find(|j| j.criterion == "PACE_POSSESSIONS");
    assert!(
        matches!(pace, Some(j) if j.verdict == Verdict::Defect),
        "pace 720 must be flagged: {:?}",
        pace
    );
}

/// D2/G-D2a 合成流 C：构成全部在带内 → 构成准则必须 pass（不误报）。
#[test]
fn composition_synthetic_c_in_band_passes() {
    // 3PA 率 ~0.35（35 三 / 100 FGA），2P 率 ~0.65，15 失误（rate 0.13 入带）。
    // pace 由墙钟决定：115 事件 × 10s/tick ≈ 1140s → pace = 115*2880/1140 ≈ 290，
    // 需把 tick 间隔拉大到 ~15s 使 pace ≈ 197 入带 [185,220]。
    let stream = composition_synthetic_stream(35, 65, 13, 32, 15, 100, 14.5);
    let judgments = composition_judgments(&stream);
    for criterion in ["SHOT_PROFILE_3PA_RATE", "PACE_POSSESSIONS", "TEAM_TURNOVER_RATE"] {
        let j = judgments.iter().find(|j| j.criterion == criterion);
        assert!(
            matches!(j, Some(j) if j.verdict == Verdict::Pass),
            "{} must pass for in-band stream: {:?}",
            criterion,
            j
        );
    }
}

/// D2 证据模型：v1 fixture 无构成带 → 构成准则 NotApplicable，不得默认通过。
#[test]
fn composition_criteria_not_applicable_on_v1_fixture() {
    let stream = composition_synthetic_stream(35, 65, 13, 32, 15, 100, 1500.0);
    let ticks = parse_stream(&stream).expect("parse");
    let fixture = ReferenceDistributions::nba_v1();
    let judgments = evaluate_stream(&ticks, &fixture);
    let rate = judgments.iter().find(|j| j.criterion == "SHOT_PROFILE_3PA_RATE");
    assert!(
        matches!(rate, Some(j) if j.verdict == Verdict::NotApplicable),
        "v1 fixture must mark composition criteria NotApplicable: {:?}",
        rate
    );
}
