use nba_evaluator::{evaluate_stream, parse_stream, ReferenceDistributions, Verdict};

fn tick(period: u32, clock: f32, events: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "t": clock, "t_game": 720.0, "shotClock": 24.0, "period": period,
        "phase": "ShotAttempt", "possession_id": 1, "possession_team": "home",
        "score": {"home": 0, "away": 0}, "players": [],
        "ball": {"x": 0.5, "y": 0.5, "z": 4.0, "status": "HELD", "holderId": "H_01"},
        "event_log": events,
        "rules": {
            "tick_seconds": 0.04, "court_width_ft": 94.0, "court_height_ft": 50.0,
            "hoop_left_x_ft": 5.25, "hoop_right_x_ft": 88.75, "hoop_y_ft": 25.0,
            "player_radius_ft": 1.8, "min_player_separation_ft": 3.6,
            "max_player_speed_ftps": 22.0, "max_player_accel_ftps2": 35.0,
            "ball_max_speed_ftps": 85.0, "three_point_distance_ft": 23.75,
            "shot_clock_seconds": 24.0, "holder_leash_ft": 3.0,
            "speed_tolerance_ftps": 1.5, "ball_z_max_ft": 35.0,
            "max_personal_fouls": 6, "bonus_fouls_per_period": 5
        },
        "tactical_set": "test", "gameClock": clock, "keyframeIndex": null
    })
}

fn fixture() -> ReferenceDistributions {
    ReferenceDistributions::nba_v3()
}

fn generated_stream(
    zone_makes: [usize; 4],
    q4_delta_test: Option<bool>,
) -> Vec<nba_protocol::StreamTick> {
    let positions = [[88.75, 25.0], [80.0, 25.0], [72.0, 25.0], [65.0, 25.0]];
    let mut late_events = Vec::new();
    let mut early_events = Vec::new();
    let mut id = 1u64;
    for (zone, pos) in positions.iter().enumerate() {
        for shot in 0..20usize {
            let made = shot < zone_makes[zone];
            let late = match (q4_delta_test, zone, shot) {
                (Some(false), 3, shot) => shot < 16,
                (Some(false), 0..=2, shot) => shot >= 12,
                (Some(true), _, shot) => shot >= 10,
                (None, _, _) => true,
                _ => false,
            };
            let release_id = id;
            id += 1;
            let arrival_id = id;
            id += 1;
            let outcome_id = id;
            id += 1;
            let destination = if late {
                &mut late_events
            } else {
                &mut early_events
            };
            destination.push(serde_json::json!({
                "sequence": release_id, "time": 10.0, "kind": "SHOT_RELEASE",
                "event_id": release_id, "parent_event_id": null,
                "data": {"ShotRelease": {"shooter_id": "H_01", "pos": pos, "is_three": zone == 3,
                    "contest_level": 0.2, "make_probability": 0.5, "transition_context": false}}
            }));
            destination.push(serde_json::json!({
                "sequence": arrival_id, "time": 10.1, "kind": "SHOT_TRAJECTORY_ARRIVAL",
                "event_id": arrival_id, "parent_event_id": release_id,
                "data": {"ShotTrajectoryArrival": {"shooter_id": "H_01", "ball_position": [pos[0], pos[1], 0.0]}}
            }));
            destination.push(serde_json::json!({
                "sequence": outcome_id, "time": 10.2, "kind": if made { "SCORE" } else { "SHOT_MISS" },
                "event_id": outcome_id, "parent_event_id": arrival_id,
                "data": {"HoopArrival": {"is_made": made, "is_three": zone == 3, "shooter_id": "H_01"}}
            }));
        }
    }
    let mut stream = vec![tick(4, 270.0, late_events), tick(4, 500.0, early_events)];
    stream.extend((0..20).map(|_| tick(4, 500.0, vec![])));
    let stream = stream
        .into_iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    parse_stream(&stream.join("\n")).expect("synthetic event stream parses")
}

#[test]
fn v3_fixture_contains_source_metadata_and_is_not_implicitly_selected() {
    let v3 = fixture();
    assert_eq!(v3.version, "nba.v3");
    assert_eq!(
        v3.joint_situational_bands
            .as_ref()
            .unwrap()
            .provenance
            .games,
        1230
    );
    assert_eq!(
        v3.joint_situational_bands
            .as_ref()
            .unwrap()
            .provenance
            .season,
        "2023-24 NBA regular season"
    );
    let metadata = &v3.joint_situational_bands.as_ref().unwrap().provenance;
    let benchmark = metadata.external_check.as_ref().unwrap();
    assert_eq!(benchmark.metric, "league 3PA/FGA");
    assert_eq!(benchmark.value, 0.395);
    assert_eq!(ReferenceDistributions::for_league("NBA").version, "nba.v2");
}

#[test]
fn shot_zone_make_joint_detects_zone_rate_permutation_with_equal_marginals() {
    let baseline = generated_stream([12, 9, 8, 7], None);
    let swapped = generated_stream([9, 12, 7, 8], None);
    let baseline_judgments = evaluate_stream(&baseline, &fixture());
    let swapped_judgments = evaluate_stream(&swapped, &fixture());
    let get = |judgments: &[nba_evaluator::Judgment]| {
        judgments
            .iter()
            .find(|j| j.criterion == "SHOT_ZONE_MAKE_JOINT")
            .unwrap()
            .verdict
    };
    assert_eq!(get(&baseline_judgments), Verdict::Pass);
    assert_eq!(get(&swapped_judgments), Verdict::Defect);
}

#[test]
fn q4_three_rate_delta_is_judged_with_fixed_window_and_denominators() {
    let valid = generated_stream([12, 9, 8, 7], Some(true));
    let invalid = generated_stream([12, 9, 8, 7], Some(false));
    let get = |ticks: &[nba_protocol::StreamTick]| {
        evaluate_stream(ticks, &fixture())
            .into_iter()
            .find(|j| j.criterion == "LATE_Q4_SHOT_PROFILE")
            .unwrap()
            .verdict
    };
    assert_eq!(get(&valid), Verdict::Pass);
    assert_eq!(get(&invalid), Verdict::Defect);
}

#[test]
fn q4_judgment_requires_valid_clock_for_every_q4_attempt() {
    let mut stream = generated_stream([12, 9, 8, 7], Some(true));
    let mut late_tick = serde_json::to_value(&stream[0]).unwrap();
    late_tick["gameClock"] = serde_json::json!(721.0);
    stream[0] = serde_json::from_value(late_tick).unwrap();
    let judgment = evaluate_stream(&stream, &fixture())
        .into_iter()
        .find(|j| j.criterion == "LATE_Q4_SHOT_PROFILE")
        .unwrap();
    assert_eq!(judgment.verdict, Verdict::InsufficientEvidence);
}

#[test]
fn missing_shot_parent_or_position_yields_insufficient_evidence() {
    let stream = generated_stream([12, 9, 8, 7], None);
    let mut broken = stream.clone();
    let mut json: serde_json::Value = serde_json::to_value(&broken[0]).unwrap();
    let events = json["event_log"].as_array_mut().unwrap();
    events[1]["parent_event_id"] = serde_json::Value::Null;
    broken[0] = serde_json::from_value(json).unwrap();
    let judgments = evaluate_stream(&broken, &fixture());
    assert_eq!(
        judgments
            .iter()
            .find(|j| j.criterion == "SHOT_ZONE_MAKE_JOINT")
            .unwrap()
            .verdict,
        Verdict::InsufficientEvidence
    );
}
