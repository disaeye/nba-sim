use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{run_game, Params, SessionService, SessionState};

fn configured(seed: u64) -> SessionService {
    let mut service = SessionService::new(seed);
    service
        .setup(
            Params::load(&workspace_calibration_dir()).expect("calibration tables load"),
            seed,
        )
        .expect("match setup");
    service.start().expect("match starts");
    service
}

#[test]
fn full_game_runner_matches_session_service_results() {
    let seed = 107;
    let calibration = workspace_calibration_dir();
    // 直接跑完整比赛与逐段推进的会话并发执行，两者互不依赖。
    let (direct, streamed) = std::thread::scope(|scope| {
        let direct = scope.spawn(|| run_game(seed, &calibration));
        let mut session = configured(seed);
        while session.state() != SessionState::Finished {
            session
                .fast_forward(120.0)
                .expect("session advances to completion");
        }
        let streamed = session.report().expect("completed session report");
        (
            direct.join().expect("runner thread must not panic"),
            streamed,
        )
    });
    assert_eq!(
        (streamed.home_score, streamed.away_score),
        (direct.home_score, direct.away_score)
    );
    assert_eq!(
        serde_json::to_string(&streamed.box_score).unwrap(),
        serde_json::to_string(&direct.box_score).unwrap()
    );
    assert_eq!(streamed.possessions.len(), direct.possessions.len());
    assert_eq!(streamed.events.len() as u64, direct.events.len() as u64);
}

#[test]
fn repeated_session_runs_produce_identical_frames_and_event_batches() {
    let mut left = configured(101);
    let mut right = configured(101);
    left.fast_forward(12.0).expect("left run advances");
    right.fast_forward(12.0).expect("right run advances");

    let left_snapshot = serde_json::to_string(&left.snapshot().unwrap()).expect("snapshot JSON");
    let right_snapshot = serde_json::to_string(&right.snapshot().unwrap()).expect("snapshot JSON");
    assert_eq!(left_snapshot, right_snapshot);
    let left_events =
        serde_json::to_string(&left.events_since(0, 5_000).unwrap()).expect("event batch JSON");
    let right_events =
        serde_json::to_string(&right.events_since(0, 5_000).unwrap()).expect("event batch JSON");
    assert_eq!(left_events, right_events);
}
