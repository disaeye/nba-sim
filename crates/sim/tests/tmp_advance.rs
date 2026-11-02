use nba_sim_core::action::{Fact, TurnoverKind};
use nba_sim_core::world::TICK_SECONDS;

mod common;

#[test]
fn probe_eight_second() {
    let mut world = common::short_world(1);
    let mid = world.rules.court.width_ft * 0.5;
    let mut printed = 0;
    let mut guard = 0;
    while !world.finished && guard < 200_000 && printed < 5 {
        let before = world.events.len();
        world.step(TICK_SECONDS);
        guard += 1;
        for event in &world.events[before..] {
            if matches!(
                &event.fact,
                Fact::Turnover { kind: TurnoverKind::EightSecond, .. }
            ) && printed < 5
            {
                printed += 1;
                let holder = world.holder();
                eprintln!(
                    "VIOLATION t={:.2} mid={:.1} holder={:?} pos={:?} possession_age={:.2} shot_clock={:.1}",
                    world.clock.game_clock(),
                    mid,
                    holder,
                    holder.map(|h| world.players[h].pos),
                    world.now() - world.possession.started_at,
                    world.clock.shot_clock(),
                );
            }
        }
    }
}
