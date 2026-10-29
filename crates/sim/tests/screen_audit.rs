use nba_sim_core::action::DefenderRoute;
use nba_sim_core::params::RuntimeParams;
use nba_sim_core::world::MatchSetup;

#[test]
fn over_samples_show_their_action_families() {
    let report = nba_sim_core::run_game_with_setup(
        1,
        RuntimeParams::embedded(),
        MatchSetup::builtin(nba_domain::GameRules::default()),
    );
    let mut over = 0;
    let mut under = 0;
    for sample in &report.decision_samples {
        if sample.route == Some(DefenderRoute::Over) {
            over += 1;
            println!(
                "over family {} since {:.3} three {:.2} clock {:.1}",
                sample.family, sample.since_screen, sample.shooter_three, sample.shot_clock
            );
        }
        if sample.route == Some(DefenderRoute::Under) {
            under += 1;
        }
    }
    println!("over {over} under {under} samples {}", report.decision_samples.len());
}
