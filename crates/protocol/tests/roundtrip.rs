use nba_protocol::{DecisionDebug, FrameEvent, RenderBall, RenderPlayer};
use serde_json::json;

#[test]
fn player_action_phase_and_job_roundtrip() {
    let value = json!({
        "id": "p1",
        "jersey": "7",
        "team": "home",
        "position": "Wing",
        "offensive_role": "SpotUp",
        "defensive_role": "Wing",
        "x": 0.4,
        "y": 0.5,
        "zone": "Mid",
        "hasBall": true,
        "onCourt": true,
        "action": "SpotUp",
        "action_phase": "execute",
        "job": "SpotUp(corner)",
        "responsibility": "OnBall",
        "slot": "Wing",
        "morale": "steady",
        "stm": 9.5,
        "stmMax": 10.0,
        "foul_count": 2
    });
    let player: RenderPlayer = serde_json::from_value(value).expect("player decodes");
    assert_eq!(player.action_phase.as_deref(), Some("execute"));
    assert_eq!(player.job.as_deref(), Some("SpotUp(corner)"));
    assert_eq!(player.responsibility.as_deref(), Some("OnBall"));
    let text = serde_json::to_string(&player).expect("player encodes");
    let back: RenderPlayer = serde_json::from_str(&text).expect("player roundtrips");
    assert_eq!(back.action_phase, player.action_phase);
    assert_eq!(back.job, player.job);
    assert_eq!(back.responsibility, player.responsibility);
}

#[test]
fn ball_z_and_velocity_roundtrip() {
    let value = json!({
        "x": 0.5,
        "y": 0.5,
        "z": 12.5,
        "vx": 3.0,
        "vy": -4.0,
        "vz": 8.0,
        "status": "shot",
        "holderId": null
    });
    let ball: RenderBall = serde_json::from_value(value).expect("ball decodes");
    assert_eq!(ball.z, 12.5);
    assert_eq!((ball.vx, ball.vy, ball.vz), (3.0, -4.0, 8.0));
    let text = serde_json::to_string(&ball).expect("ball encodes");
    let back: RenderBall = serde_json::from_str(&text).expect("ball roundtrips");
    assert_eq!((back.z, back.vx, back.vy, back.vz), (12.5, 3.0, -4.0, 8.0));
}

#[test]
fn event_geometry_and_contact_time_roundtrip() {
    let value = json!({
        "sequence": 3,
        "time": 1.25,
        "kind": "CONTACT",
        "x": 41.0,
        "y": 25.0,
        "z": 4.5,
        "sub_tick": 2,
        "contact_time": 1.233,
        "event_id": 9,
        "tick_index": 30
    });
    let event: FrameEvent = serde_json::from_value(value).expect("event decodes");
    assert_eq!(event.contact_time, Some(1.233));
    assert_eq!(event.sub_tick, 2);
    assert_eq!((event.x, event.y, event.z), (Some(41.0), Some(25.0), Some(4.5)));
    let text = serde_json::to_string(&event).expect("event encodes");
    let back: FrameEvent = serde_json::from_str(&text).expect("event roundtrips");
    assert_eq!(back.contact_time, event.contact_time);
    assert_eq!(back.sub_tick, event.sub_tick);
    assert_eq!((back.x, back.y, back.z), (event.x, event.y, event.z));
}

#[test]
fn decision_record_projection_roundtrip() {
    let value = json!({
        "player": "p1",
        "chosen": "Shoot",
        "utilities": [],
        "probabilities": [],
        "flags": [],
        "blocked": [],
        "trace_id": 4,
        "decision_id": 11,
        "observation_digest": 123456789,
        "revalidation": "cancelled:recheck",
        "beta": 2.5,
        "gamma": -0.1,
        "active_constraints": ["shot_clock"],
        "enforcement": [],
        "active_play_id": null,
        "play_adjustments": []
    });
    let debug: DecisionDebug = serde_json::from_value(value).expect("decision decodes");
    assert_eq!(debug.decision_id, 11);
    assert_eq!(debug.observation_digest, 123456789);
    assert_eq!(debug.revalidation.as_deref(), Some("cancelled:recheck"));
    let text = serde_json::to_string(&debug).expect("decision encodes");
    let back: DecisionDebug = serde_json::from_str(&text).expect("decision roundtrips");
    assert_eq!(back.decision_id, debug.decision_id);
    assert_eq!(back.observation_digest, debug.observation_digest);
    assert_eq!(back.revalidation, debug.revalidation);
}
