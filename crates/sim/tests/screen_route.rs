//! 掩护路线回写：结算出的路线必须出现在后续决策的视图里。

use nba_sim_core::action::Fact;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::Phase;
use nba_sim_core::{MatchSetup, Params, World};

#[test]
fn screen_route_reaches_the_next_decision() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(
        MatchSetup::builtin(nba_domain::GameRules::default()),
        params,
        5,
    );

    let mut cursor = 0usize;
    let mut resolved = None;
    for _ in 0..40_000 {
        world.step(0.05);
        while cursor < world.events.len() {
            if let Fact::Screen { route, .. } = &world.events[cursor].fact {
                resolved = Some(*route);
            }
            cursor += 1;
        }
        let Some(route) = resolved else {
            continue;
        };
        if !matches!(world.phase, Phase::Live) || world.holder().is_none() {
            continue;
        }
        let state = world.decision_state();
        if let Some(defender) = state.primary_defender {
            if let Some(view_route) = defender.route {
                assert_eq!(
                    view_route, route,
                    "决策视图的路线必须等于最近一次掩护结算出的路线"
                );
                return;
            }
        }
    }
    panic!("整场没有任何一次决策读到掩护路线");
}
