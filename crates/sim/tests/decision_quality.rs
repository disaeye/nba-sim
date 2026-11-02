use nba_domain::rules::GameRules;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{MatchSetup, Params, World};

mod common;

fn world(seed: u64) -> World {
    common::short_world(seed)
}

#[test]
fn observation_digest_is_reconstructable_and_respects_visibility() {
    let mut world = world(7);
    for _ in 0..2_000 {
        world.step(nba_sim_core::world::TICK_SECONDS);
        if let Some(actor) = world.holder() {
            let before = nba_sim_core::views::observation_digest(&world, actor);
            let observation = nba_sim_core::views::observe(&world, actor);
            assert_eq!(before, nba_sim_core::views::observation_digest(&world, actor));
            assert_eq!(
                before,
                nba_sim_core::views::observation_digest(&world, actor)
            );
            assert!(observation.players.len() <= 9);
            let hidden = (0..world.players.len()).find(|handle| {
                *handle != actor
                    && world.players[*handle].on_court
                    && !observation
                        .players
                        .iter()
                        .any(|observed| observed.index == *handle)
            });
            if let Some(hidden) = hidden {
                let pos = world.players[hidden].pos;
                world.players[hidden].pos = world.players[actor].pos
                    - world.players[actor].facing.normalize_or_zero() * 20.0;
                let after = nba_sim_core::views::observation_digest(&world, actor);
                assert_eq!(before, after, "hidden player changed the visible digest");
                world.players[hidden].pos = pos;
            }
            return;
        }
    }
    panic!("no live holder reached");
}

#[test]
fn g4a_decisions_carry_reconstructable_audit_and_bounded_regret() {
    let mut decisions = 0_usize;
    let mut regret_sum = 0.0_f32;
    let mut regret_max = 0.0_f32;
    for seed in 1..=4u64 {
        let mut world = world(seed);
        let report = world.run();
        for (index, trace) in report.decision_traces.iter().enumerate() {
            assert!(!trace.candidates.is_empty());
            assert!(trace.chosen < trace.candidates.len());
            let chosen = trace.candidates[trace.chosen].total;
            let best = trace
                .candidates
                .iter()
                .map(|candidate| candidate.total)
                .fold(f32::MIN, f32::max);
            let regret = (best - chosen).max(0.0);
            assert!(regret.is_finite());
            regret_sum += regret;
            regret_max = regret_max.max(regret);
            decisions += 1;
            let record = report
                .decision_records
                .get(index)
                .expect("decision trace has a record");
            assert_eq!(record.decision_id, index as u64);
            assert!(record.observation_digest > 0);
            assert!(matches!(
                record.revalidation,
                nba_sim_core::state::Revalidation::Accepted
                    | nba_sim_core::state::Revalidation::Cancelled(_)
            ));
        }
    }
    let mean = regret_sum / decisions.max(1) as f32;
    println!("G4a decisions={decisions} mean_regret={mean:.4} max_regret={regret_max:.4}");
    assert!(decisions > 100, "not enough decisions: {decisions}");
    assert!(mean <= 0.25, "mean regret {mean:.4} above 0.25");
    assert!(regret_max <= 2.0, "max regret {regret_max:.4} above 2.0");
}
