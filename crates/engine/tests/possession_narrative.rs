use nba_engine::MatchEngine;

#[test]
fn test_possession_summary_events_are_emitted_and_causal() {
    let mut engine = MatchEngine::new(42);
    let mut summaries_count = 0;

    for _ in 0..2500 {
        let tick = engine.step();
        for event in &tick.frame.events {
            if event == "POSSESSION_SUMMARY" {
                summaries_count += 1;
            }
        }
        assert!(
            engine.last_tick_violations().is_empty(),
            "tick violations: {:?}",
            engine.last_tick_violations()
        );
    }

    assert!(
        summaries_count > 0,
        "must emit possession summary events at possession boundaries, got 0"
    );
}
