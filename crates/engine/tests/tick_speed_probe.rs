use nba_domain::GameRules;
use nba_engine::MatchEngine;

#[test]
fn probe_full_game_ticks() {
    let mut engine = MatchEngine::with_rules(42, GameRules::default());
    engine.set_scope("full").expect("full scope is valid");
    let mut ticks = 0u64;
    let mut last_summary_tick = 0u64;
    let mut longest_run = 0u64;
    let mut summaries = 0u64;
    while !engine.is_finished() {
        let tick = engine.step();
        ticks += 1;
        if tick
            .frame
            .event_log
            .iter()
            .any(|e| e.kind == "POSSESSION_SUMMARY")
        {
            summaries += 1;
            let run = ticks - last_summary_tick;
            if run > longest_run {
                longest_run = run;
            }
            last_summary_tick = ticks;
        }
        if ticks > 60000 {
            println!("ABORT at {ticks}: summaries {summaries}, longest run {longest_run}");
            return;
        }
    }
    println!("done: {ticks} ticks, {summaries} summaries, longest run {longest_run}");
}
