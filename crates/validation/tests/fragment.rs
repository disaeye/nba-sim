use nba_domain::league::LeagueId;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{RuntimeParams, World};
use nba_validation::fragment::{capture, check_frames, run, FragmentCheckpoint, FragmentInput, Scenario, DEFAULT_FRAGMENT_TICKS};

#[test]
fn registered_fragments_end_within_their_budgets() {
    let params = RuntimeParams::embedded();
    for league in [LeagueId::Nba, LeagueId::Fiba] {
        for scenario in Scenario::ALL {
            let input = FragmentInput::builtin(league, 1, scenario, &params);
            let result = run(input, &params).expect("scenario must reach its endpoint");
            assert!(result.report.passed(), "{league:?} {scenario:?}: {:?}", result.report.findings);
            assert!(result.report.end_tick > result.report.start_tick);
            assert!(result.report.advanced_ticks <= u64::from(DEFAULT_FRAGMENT_TICKS));
            assert_eq!(result.frames.len() as u64, 1 + result.report.end_tick - result.report.start_tick);
        }
    }
}

#[test]
fn a_captured_live_world_resumes_deterministically_from_its_current_tick() {
    let params = RuntimeParams::embedded();
    let input = FragmentInput::builtin(LeagueId::Nba, 1, Scenario::FirstPossession, &params);
    let checkpoint = capture(input.clone(), &params).expect("first live possession");
    assert!(checkpoint.tick() > 0);
    let tick = checkpoint.tick();
    let directory = nba_sim_core::params::workspace_root().join(".work/fragment-tests").join(std::process::id().to_string());
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("world.cbor");
    checkpoint.save(&path).expect("persist checkpoint");
    drop(checkpoint);
    let restarted = FragmentCheckpoint::load(input.clone(), &path, &params)
        .expect("load checkpoint")
        .resume().expect("restored fragment");
    std::fs::remove_dir_all(&directory).unwrap();
    let mut continuous = World::new(input.manifest.setup.clone(), params.clone(), input.manifest.seed);
    let mut expected = Vec::new();
    while continuous.clock.tick() <= restarted.report.end_tick {
        if continuous.clock.tick() >= tick {
            expected.push(continuous.stream_tick());
        }
        continuous.step(TICK_SECONDS);
    }
    assert_eq!(serde_json::to_vec(&expected).unwrap(), serde_json::to_vec(&restarted.frames).unwrap());
    assert_eq!(restarted.report.start_tick, tick);
    assert_eq!(serde_json::to_vec(&restarted).unwrap(), serde_json::to_vec(&run(input.clone(), &params).unwrap()).unwrap());
    let replay = serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    assert_eq!(serde_json::to_vec(&restarted).unwrap(), serde_json::to_vec(&run(replay, &params).unwrap()).unwrap());
}

#[test]
fn two_nonadjacent_production_frames_localize_the_missing_interval() {
    let params = RuntimeParams::embedded();
    let input = FragmentInput::builtin(LeagueId::Nba, 1, Scenario::OpeningControl, &params);
    let result = run(input, &params).expect("opening control");
    let first = result.frames.first().unwrap();
    let last = result.frames.last().unwrap();
    assert!(last.frame.tick_index > first.frame.tick_index + 1);
    let findings = check_frames(&[first.clone(), last.clone()]);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].check, "frame_sequence");
    assert_eq!(findings[0].tick, last.frame.tick_index);
}

#[test]
fn failure_to_reach_the_target_is_an_execution_error() {
    let params = RuntimeParams::embedded();
    for (scenario, boundary) in [(Scenario::OpeningControl, "end"), (Scenario::FirstPossession, "start")] {
        let mut input = FragmentInput::builtin(LeagueId::Nba, 1, scenario, &params);
        input.max_ticks = 1;
        let message = run(input, &params).expect_err("one tick is too short to reach the target");
        assert!(message.contains(&format!("did not reach {boundary} at tick 1")), "{message}");
    }
}
