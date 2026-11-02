use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{Params, SessionService};

fn running_session(seed: u64) -> SessionService {
    let mut session = SessionService::new(seed);
    session
        .setup(
            Params::load(&workspace_calibration_dir()).expect("calibration tables load"),
            seed,
        )
        .expect("session setup");
    session.start().expect("session start");
    session
}

#[test]
fn checkpoint_restore_matches_uninterrupted_session() {
    let mut uninterrupted = running_session(29);
    uninterrupted
        .fast_forward(8.025)
        .expect("partial fixed-step interval");
    let checkpoint = uninterrupted.checkpoint().expect("checkpoint creation");
    uninterrupted
        .fast_forward(1.975)
        .expect("uninterrupted continuation");
    let expected_snapshot =
        serde_json::to_string(&uninterrupted.snapshot().unwrap()).expect("snapshot serialization");
    let expected_events =
        serde_json::to_string(&uninterrupted.events(usize::MAX)).expect("event serialization");

    let mut restored = running_session(97);
    restored.restore(checkpoint).expect("checkpoint restore");
    restored.fast_forward(1.975).expect("restored continuation");
    let actual_snapshot =
        serde_json::to_string(&restored.snapshot().unwrap()).expect("snapshot serialization");
    let actual_events =
        serde_json::to_string(&restored.events(usize::MAX)).expect("event serialization");
    assert_eq!(actual_snapshot, expected_snapshot);
    assert_eq!(actual_events, expected_events);
}

#[test]
fn event_cursor_returns_incremental_stable_ids_in_order() {
    let mut session = running_session(31);
    session.fast_forward(10.0).expect("session advances");
    let all = session.events(usize::MAX);
    assert!(!all.is_empty(), "simulation should emit events");
    assert!(all
        .windows(2)
        .all(|pair| pair[0].event_id < pair[1].event_id));

    let first = session
        .events_since(0, 3)
        .expect("initial cursor is retained");
    assert_eq!(first.events.len(), 3);
    assert_eq!(first.events[0].event_id, 1);
    assert_eq!(first.next_after_event_id, 3);
    assert!(first.truncated);

    let second = session
        .events_since(first.next_after_event_id, 3)
        .expect("next cursor is retained");
    assert_eq!(second.events[0].event_id, 4);
    assert_eq!(second.latest_event_id, all.last().unwrap().event_id);
}
