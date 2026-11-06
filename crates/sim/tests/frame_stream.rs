use std::collections::BTreeMap;

use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{Params, World};

mod common;

#[test]
fn frame_stream_satisfies_motion_contact_rules_and_causal_ids() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(common::short_setup(), params, 1);
    let width = world.rules.court.width_ft;
    let height = world.rules.court.height_ft;
    let tick_seconds = world.rules.tick_seconds;
    let mut last_t = 0.0_f32;
    let mut last_tick = 0_u64;
    let mut last_score = (0_u32, 0_u32);
    let mut last_possession = 0_u32;
    let mut events: BTreeMap<u64, (String, serde_json::Value)> = BTreeMap::new();
    let mut frames = 0_u32;
    let mut observed_active_action_families: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    while !world.finished && frames < 30_000 {
        let tick = world.stream_tick();
        let frame = &tick.frame;
        frames += 1;
        assert!(frame.t_game >= last_t, "frame time went backwards");
        assert!(frame.tick_index >= last_tick, "tick index went backwards");
        assert!(frame.shot_clock >= 0.0);
        assert!(frame.shot_clock <= frame.rules.shot_clock_seconds + 0.01);
        assert!(frame.score.home >= last_score.0 && frame.score.away >= last_score.1);
        assert!(
            frame.possession_id == 0 || frame.possession_id >= last_possession,
            "possession id went backwards: {} then {} at tick {}",
            last_possession,
            frame.possession_id,
            frame.tick_index
        );
        if frame.possession_id != 0 {
            last_possession = frame.possession_id;
        }
        assert_eq!(frame.rules.tick_seconds, tick_seconds);
        assert_eq!(frame.stream_projection, "facts");
        assert_eq!(frame.players.len(), 10, "a frame must carry ten players");
        assert!(frame.ball.z.is_finite() && frame.ball.z >= 0.0);
        assert!(frame.ball.z <= frame.rules.ball_z_max_ft + 1.0);
        for player in &frame.players {
            let out_of_bounds_holder = player.has_ball
                && (!(0.0..=1.0).contains(&player.x) || !(0.0..=1.0).contains(&player.y));
            if !out_of_bounds_holder {
                assert!(
                    (0.0..=1.0).contains(&player.x),
                    "player {} x {} at tick {}",
                    player.id,
                    player.x,
                    frame.tick_index
                );
                assert!(
                    (0.0..=1.0).contains(&player.y),
                    "player {} y {} at tick {}",
                    player.id,
                    player.y,
                    frame.tick_index
                );
            }
            assert!(player.target_x.is_some() && player.target_y.is_some());
            for action in &player.active_actions {
                assert!(action.action_id > 0, "active action has zero id");
                assert!(!action.family.is_empty(), "action family is empty");
                assert!(!action.kind.is_empty(), "action kind is empty");
                assert!(!action.variant.is_empty(), "action variant is empty");
                assert!(!action.phase.is_empty(), "action phase is empty");
                assert!(!action.source.is_empty(), "action source is empty");
                observed_active_action_families.insert(action.family.clone());
            }
        }
        for event in &frame.event_log {
            assert!(event.event_id > 0, "frame event has no stable id");
            assert!(event.contact_time.is_none_or(|time| time.is_finite()));
            if event.kind == "BALL_CONTACT" {
                let data = event.data.as_ref().expect("contact has data");
                let x = data["x"].as_f64().expect("contact x");
                let y = data["y"].as_f64().expect("contact y");
                let z = data["z"].as_f64().expect("contact z");
                assert!(
                    (-20.0..=width as f64 + 20.0).contains(&x),
                    "contact x {x} outside court width {width}"
                );
                assert!((-20.0..=height as f64 + 20.0).contains(&y));
                assert!(z >= 0.0 && z <= frame.rules.ball_z_max_ft as f64 + 1.0);
                let time = data["contact_time"].as_f64().expect("contact time");
                assert!(time.is_finite());
                assert!((event.time as f64 - tick_seconds as f64 - 0.01
                    ..=event.time as f64 + 0.01)
                    .contains(&time));
            }
            if let Some((old_kind, old_data)) = events.insert(
                event.event_id,
                (event.kind.clone(), event.data.clone().unwrap_or_default()),
            ) {
                assert_eq!(old_kind, event.kind, "event id changed kind");
                assert_eq!(old_data, event.data.clone().unwrap_or_default());
            }
        }
        last_t = frame.t_game;
        last_tick = frame.tick_index;
        last_score = (frame.score.home, frame.score.away);
        world.step(TICK_SECONDS);
    }
    assert!(frames > 1_000);
    assert!(!events.is_empty());
    assert!(
        events.values().any(|(kind, _)| kind == "SHOT"),
        "no shot event reached the frame stream"
    );
    assert!(
        events.values().any(|(kind, _)| kind == "BALL_CONTACT"),
        "no contact event reached the frame stream"
    );
    assert!(
        !observed_active_action_families.is_empty(),
        "frame stream must project real active_actions"
    );
    assert!(
        observed_active_action_families.contains("ACTION-DEFENSE-ON-BALL"),
        "frame stream must project defensive posture or action"
    );
}
