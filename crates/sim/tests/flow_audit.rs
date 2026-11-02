use nba_sim_core::params::RuntimeParams;
use nba_sim_core::state::Phase;
use nba_sim_core::world::{MatchSetup, World, TICK_SECONDS};

#[test]
fn first_decisions_show_where_actions_start() {
    let mut world = World::new(
        MatchSetup::builtin(nba_domain::GameRules::default()),
        RuntimeParams::embedded(),
        3,
    );
    let mut seen = 0usize;
    let mut previous = 0usize;
    while !world.finished && seen < 40 {
        world.step(TICK_SECONDS);
        if world.decision_traces.len() > previous {
            previous = world.decision_traces.len();
            let trace = world.decision_traces.last().expect("trace");
            let chosen = &trace.candidates[trace.chosen];
            let actor = world.decision_records.last().expect("record").actor;
            let pos = world.players[actor].pos;
            let hoop = world.hoop_for(world.players[actor].team);
            println!(
                "decision {seen} actor {actor} pos ({:.1},{:.1}) hoop ({:.1},{:.1}) dist {:.1} chosen {:?} target {:?} phase {:?}",
                pos.x,
                pos.y,
                hoop.x,
                hoop.y,
                (pos - hoop).length(),
                chosen.kind,
                chosen.to_ft,
                world.phase()
            );
            seen += 1;
        }
        if matches!(world.phase(), Phase::Live) && world.holder().is_none() {
            // no-op
        }
    }
}
