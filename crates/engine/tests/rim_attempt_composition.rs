use nba_engine::{MatchEngine, StreamMode};
use nba_test_support::TempArtifact;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Default)]
struct RimAttemptCounts {
    total_fga: usize,
    rim_fga: usize,
    near_fga: usize,
    mid_fga: usize,
    three_fga: usize,
    drive_rim_fga: usize,
    drive_pull_up_rim_fga: usize,
    drive_pull_up_fga: usize,
    near_rim_reception_fga: usize,
    offensive_rebound_fga: usize,
    transition_rim_fga: usize,
    set_play_rim_fga: usize,
    unclassified_rim_fga: usize,
    reached_drives: usize,
    drive_rim_targets: usize,
}

fn position_distance(position: &[Value], player_id: &str) -> f32 {
    let x = position
        .first()
        .and_then(Value::as_f64)
        .expect("event position must include x") as f32;
    let y = position
        .get(1)
        .and_then(Value::as_f64)
        .expect("event position must include y") as f32;
    let hoop_x = if player_id.starts_with('H') {
        88.75
    } else {
        5.25
    };
    ((x - hoop_x).powi(2) + (y - 25.0).powi(2)).sqrt()
}

fn count_rim_attempts(stream: &str) -> RimAttemptCounts {
    let ticks = nba_evaluator::parse_stream(stream).expect("facts stream must parse strictly");
    let drive_initiations: HashMap<u64, (String, f32)> = ticks
        .iter()
        .flat_map(|tick| &tick.frame.event_log)
        .filter(|event| event.kind == "DRIVE_INITIATED")
        .map(|event| {
            let initiation = event
                .data
                .as_ref()
                .and_then(|data| data.get("DriveInitiated"))
                .expect("DRIVE_INITIATED must carry DriveInitiated data");
            let driver = initiation
                .get("driver_id")
                .and_then(Value::as_str)
                .expect("DriveInitiated must include driver_id")
                .to_owned();
            let target = initiation
                .get("target_pos")
                .and_then(Value::as_array)
                .expect("DriveInitiated must include target_pos");
            (
                event.event_id,
                (driver.clone(), position_distance(target, &driver)),
            )
        })
        .collect();

    let events_by_id: HashMap<u64, &nba_protocol::FrameEvent> = ticks
        .iter()
        .flat_map(|tick| &tick.frame.event_log)
        .map(|event| (event.event_id, event))
        .collect();
    let mut counts = RimAttemptCounts::default();
    for tick in &ticks {
        for event in &tick.frame.event_log {
            match event.kind.as_str() {
                "DRIVE_REACHED" => {
                    let outcome = event
                        .data
                        .as_ref()
                        .and_then(|data| data.get("DriveOutcome"))
                        .expect("DRIVE_REACHED must carry DriveOutcome data");
                    assert_eq!(
                        outcome.get("successful").and_then(Value::as_bool),
                        Some(true),
                        "DRIVE_REACHED must report successful=true"
                    );
                    let driver = outcome
                        .get("driver_id")
                        .and_then(Value::as_str)
                        .expect("DriveOutcome must include driver_id");
                    let parent_id = event
                        .parent_event_id
                        .expect("DRIVE_REACHED must reference DRIVE_INITIATED");
                    let (initiated_driver, target_distance) = drive_initiations
                        .get(&parent_id)
                        .expect("drive initiation parent must exist in the complete facts stream");
                    assert_eq!(
                        initiated_driver, driver,
                        "drive causal chain actor must match"
                    );
                    counts.reached_drives += 1;
                    counts.drive_rim_targets += usize::from(*target_distance <= 4.5);
                }
                "PASS_RECEIVED" => {
                    let received = event
                        .data
                        .as_ref()
                        .and_then(|data| data.get("PassReceived"))
                        .expect("PASS_RECEIVED must carry PassReceived data");
                    assert!(
                        received
                            .get("is_cut_reception")
                            .and_then(Value::as_bool)
                            .is_some(),
                        "PassReceived must include is_cut_reception"
                    );
                }
                "REBOUND" => {
                    let rebound = event
                        .data
                        .as_ref()
                        .and_then(|data| data.get("ReboundContest"))
                        .expect("REBOUND must carry ReboundContest data");
                    assert!(
                        rebound
                            .get("is_offensive")
                            .and_then(Value::as_bool)
                            .is_some(),
                        "ReboundContest must include is_offensive"
                    );
                }
                "SHOT_RELEASE" => {
                    let shot = event
                        .data
                        .as_ref()
                        .and_then(|data| data.get("ShotRelease"))
                        .expect("SHOT_RELEASE must carry ShotRelease data");
                    let shooter = shot
                        .get("shooter_id")
                        .and_then(Value::as_str)
                        .expect("ShotRelease must include shooter_id");
                    let position = shot
                        .get("pos")
                        .and_then(Value::as_array)
                        .expect("ShotRelease must include pos");
                    let is_three = shot
                        .get("is_three")
                        .and_then(Value::as_bool)
                        .expect("ShotRelease must include is_three");
                    let distance = position_distance(position, shooter);
                    counts.total_fga += 1;
                    let source = shot
                        .get("creation_source")
                        .and_then(Value::as_str)
                        .expect("ShotRelease must include creation_source");
                    let parent = event
                        .parent_event_id
                        .and_then(|parent_id| events_by_id.get(&parent_id).copied());
                    match source {
                        "drive_finish" => {
                            let parent = parent.expect("drive finish must have a causal parent");
                            assert_eq!(parent.kind, "DRIVE_REACHED");
                            let outcome = parent
                                .data
                                .as_ref()
                                .and_then(|data| data.get("DriveOutcome"))
                                .expect("drive parent must carry DriveOutcome data");
                            let driver = outcome
                                .get("driver_id")
                                .and_then(Value::as_str)
                                .expect("DriveOutcome must include driver_id");
                            assert_eq!(driver, shooter);
                            assert_eq!(
                                outcome.get("successful").and_then(Value::as_bool),
                                Some(true)
                            );
                            assert_eq!(
                                shot.get("source_event_id").and_then(Value::as_u64),
                                Some(parent.event_id),
                                "drive finish source ID must match its DRIVE_REACHED parent"
                            );
                            counts.drive_rim_fga += usize::from(
                                !is_three && distance < nba_domain::court::RIM_ZONE_MAX_DIST_FT,
                            );
                        }
                        "drive_pull_up" => {
                            counts.drive_pull_up_fga += 1;
                            let parent = parent.expect("drive pull-up must have a causal parent");
                            assert_eq!(
                                shot.get("source_event_id").and_then(Value::as_u64),
                                Some(parent.event_id),
                                "drive pull-up parent must match its typed source event ID"
                            );
                            let parent_data = parent
                                .data
                                .as_ref()
                                .expect("drive parent must carry event data");
                            let drive_player = if parent.kind == "DRIVE_INITIATED" {
                                parent_data
                                    .get("DriveInitiated")
                                    .and_then(|data| data.get("driver_id"))
                            } else {
                                parent_data
                                    .get("DriveOutcome")
                                    .and_then(|data| data.get("driver_id"))
                            }
                            .and_then(Value::as_str)
                            .expect("drive parent must identify its driver");
                            assert_eq!(drive_player, shooter);
                            counts.drive_pull_up_rim_fga += usize::from(
                                !is_three && distance < nba_domain::court::RIM_ZONE_MAX_DIST_FT,
                            );
                        }
                        "cut_reception" => {
                            let parent = parent.expect("cut finish must have a causal parent");
                            assert_eq!(parent.kind, "PASS_RECEIVED");
                            assert_eq!(
                                shot.get("source_event_id").and_then(Value::as_u64),
                                Some(parent.event_id),
                                "cut-reception parent must match its typed source event ID"
                            );
                            let received = parent
                                .data
                                .as_ref()
                                .and_then(|data| data.get("PassReceived"))
                                .expect("cut reception parent must carry PassReceived data");
                            assert_eq!(
                                received.get("is_cut_reception").and_then(Value::as_bool),
                                Some(true)
                            );
                            assert_eq!(
                                received.get("receiver_id").and_then(Value::as_str),
                                Some(shooter)
                            );
                            counts.near_rim_reception_fga += usize::from(
                                !is_three && distance < nba_domain::court::RIM_ZONE_MAX_DIST_FT,
                            );
                        }
                        "offensive_rebound_putback" => {
                            let parent = parent.expect("putback must have a causal parent");
                            assert_eq!(parent.kind, "REBOUND");
                            assert_eq!(
                                shot.get("source_event_id").and_then(Value::as_u64),
                                Some(parent.event_id),
                                "putback parent must match its typed source event ID"
                            );
                            let rebound = parent
                                .data
                                .as_ref()
                                .and_then(|data| data.get("ReboundContest"))
                                .expect("putback parent must carry ReboundContest data");
                            assert_eq!(
                                rebound.get("is_offensive").and_then(Value::as_bool),
                                Some(true)
                            );
                            assert_eq!(
                                rebound.get("rebounder_id").and_then(Value::as_str),
                                Some(shooter)
                            );
                            counts.offensive_rebound_fga += usize::from(
                                !is_three && distance < nba_domain::court::RIM_ZONE_MAX_DIST_FT,
                            );
                        }
                        "transition_finish" => {
                            let parent =
                                parent.expect("transition finish must have a causal parent");
                            assert_eq!(parent.kind, "TRANSITION_STARTED");
                            assert_eq!(
                                shot.get("transition_context").and_then(Value::as_bool),
                                Some(true),
                                "transition shot must preserve its explicit transition context"
                            );
                            assert_eq!(
                                shot.get("transition_event_id").and_then(Value::as_u64),
                                Some(parent.event_id),
                                "transition parent must match its typed transition event ID"
                            );
                            assert_eq!(
                                shot.get("source_event_id").and_then(Value::as_u64),
                                Some(parent.event_id),
                                "transition parent must match its typed source event ID"
                            );
                            counts.transition_rim_fga += usize::from(
                                !is_three && distance < nba_domain::court::RIM_ZONE_MAX_DIST_FT,
                            );
                        }
                        "set_play" => {
                            assert!(event.parent_event_id.is_none());
                            assert!(shot.get("source_event_id").is_some_and(Value::is_null));
                            counts.set_play_rim_fga += usize::from(
                                !is_three && distance < nba_domain::court::RIM_ZONE_MAX_DIST_FT,
                            );
                        }
                        other => panic!("unknown shot creation source: {other}"),
                    }
                    if is_three {
                        counts.three_fga += 1;
                    } else if distance < nba_domain::court::RIM_ZONE_MAX_DIST_FT {
                        counts.rim_fga += 1;
                        if !matches!(
                            source,
                            "drive_finish"
                                | "cut_reception"
                                | "offensive_rebound_putback"
                                | "transition_finish"
                                | "set_play"
                        ) {
                            counts.unclassified_rim_fga += 1;
                        }
                    } else if distance < nba_domain::court::NEAR_ZONE_MAX_DIST_FT {
                        counts.near_fga += 1;
                    } else {
                        counts.mid_fga += 1;
                    }
                }
                _ => {}
            }
        }
    }
    counts
}

#[test]
fn full_game_events_reconcile_rim_attempt_sources_with_evaluator() {
    let seed = 1;
    let artifact = TempArtifact::new("rim_attempt_composition");
    let summary = {
        let mut engine = MatchEngine::new(seed);
        engine
            .simulate_scope_and_export_with_mode("full", &artifact.path_str(), StreamMode::Facts)
            .expect("full-game facts export must succeed")
    };
    let artifact_bytes = artifact.assert_within_limit();
    let stream = std::fs::read_to_string(artifact.path()).expect("facts stream must be readable");
    let counts = count_rim_attempts(&stream);
    let box_fga = summary.box_score.fg2_attempts as usize + summary.box_score.fg3_attempts as usize;
    let rim_share = counts.rim_fga as f32 / counts.total_fga as f32;
    let reference = nba_evaluator::ReferenceDistributions::nba_v2();
    let judgments = nba_evaluator::evaluate_stream(
        &nba_evaluator::parse_stream(&stream).expect("facts stream must parse strictly"),
        &reference,
    );
    let zone_judgment = judgments
        .iter()
        .find(|judgment| judgment.criterion == "SHOT_PROFILE_ZONE_MIX")
        .expect("game evaluation must report zone mix");
    eprintln!(
        "seed={seed} artifact={artifact_bytes} FGA={} rim={} ({:.3}) near={} mid={} three={} drive_rim={} drive_pull_up_rim={} drive_pull_ups={} rim_catch={} offensive_rebound={} transition_rim={} set_play_rim={} unclassified_rim={} reached_drives={} rim_targets={} zone_verdict={:?} zone_detail={}",
        counts.total_fga,
        counts.rim_fga,
        rim_share,
        counts.near_fga,
        counts.mid_fga,
        counts.three_fga,
        counts.drive_rim_fga,
        counts.drive_pull_up_rim_fga,
        counts.drive_pull_up_fga,
        counts.near_rim_reception_fga,
        counts.offensive_rebound_fga,
        counts.transition_rim_fga,
        counts.set_play_rim_fga,
        counts.unclassified_rim_fga,
        counts.reached_drives,
        counts.drive_rim_targets,
        zone_judgment.verdict,
        zone_judgment.detail,
    );
    assert_eq!(counts.total_fga, box_fga, "event FGA must match box score");
    assert_eq!(
        counts.total_fga,
        counts.rim_fga + counts.near_fga + counts.mid_fga + counts.three_fga,
        "every field-goal attempt must belong to exactly one zone"
    );
    assert!(
        counts.reached_drives > 0,
        "seed must exercise drive finishes"
    );
    assert!(
        counts.drive_rim_targets > 0,
        "seed must target the rim on drives"
    );
    assert_eq!(
        counts.rim_fga,
        counts.drive_rim_fga
            + counts.drive_pull_up_rim_fga
            + counts.near_rim_reception_fga
            + counts.offensive_rebound_fga
            + counts.transition_rim_fga
            + counts.set_play_rim_fga
            + counts.unclassified_rim_fga,
        "every rim attempt must be accounted for by a structured route or set play"
    );
    assert_eq!(
        zone_judgment.verdict,
        nba_evaluator::Verdict::Pass,
        "the evaluator must accept the full-game shot-zone mix"
    );
}
