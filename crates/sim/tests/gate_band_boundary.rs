//! 分布门禁带宽的数值边界回归。

use nba_sim_core::gate::evaluate_merged;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{BoxScore, GameReport, Params, PlayerLine, TeamSide};

/// 观测值恰好落在闭区间下界时，浮点抵消不得把它判成越界。
///
/// `and_one_per_game` 的目标 2.5、观测 2.125，相对误差数学上等于 -0.15，
/// 正好落在 `default` 带宽的下界。`observed / target - 1.0` 会得到
/// -0.15000000000000002；门禁因此必须先减基准再除基准。
#[test]
fn a_value_exactly_on_the_lower_band_is_inside_the_band() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let band = params.policy.band("and_one_per_game");
    assert_eq!(band.lower, -0.15);
    assert_eq!(band.upper, 0.15);

    let mut report = GameReport::empty();
    report.games = 16;
    report.box_score = BoxScore {
        home: vec![PlayerLine::new(
            "home-0",
            "Home Zero",
            "0",
            TeamSide::Home,
            0.5,
        )],
        away: vec![PlayerLine::new(
            "away-0",
            "Away Zero",
            "0",
            TeamSide::Away,
            0.5,
        )],
    };
    // 16 场比赛的 2.125 加罚/场就是 34 次；闭区间下界正好是 -0.15。
    report.box_score.home[0].and_one = 17;
    report.box_score.away[0].and_one = 17;

    let gate = evaluate_merged(&report, &params);
    let layer = gate
        .layers
        .iter()
        .find(|layer| layer.criterion == "and_one_per_game")
        .expect("and_one_per_game must be evaluated");
    assert_eq!(layer.observed, 2.125);
    assert_eq!(layer.target, 2.5);
    assert_eq!(layer.relative_error, -0.15);
    assert!(
        layer.passed,
        "闭区间下界必须判通过：{}",
        layer.detail
    );
}

/// 门禁关闭期间，越界值仍被测量与报告，但不得阻断判定。
#[test]
fn an_out_of_band_value_is_reported_but_not_blocking_while_gates_are_closed() {
    assert!(!nba_sim_core::gate::DISTRIBUTION_GATES_ENFORCED);
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut report = GameReport::empty();
    report.games = 16;
    report.box_score = BoxScore {
        home: vec![PlayerLine::new(
            "home-0",
            "Home Zero",
            "0",
            TeamSide::Home,
            0.5,
        )],
        away: vec![PlayerLine::new(
            "away-0",
            "Away Zero",
            "0",
            TeamSide::Away,
            0.5,
        )],
    };
    // 16 场 0 次加罚：相对误差 -1.0，远超带宽。
    report.box_score.home[0].and_one = 0;
    report.box_score.away[0].and_one = 0;

    let gate = evaluate_merged(&report, &params);
    let layer = gate
        .layers
        .iter()
        .find(|layer| layer.criterion == "and_one_per_game")
        .expect("and_one_per_game must be evaluated");
    assert_eq!(layer.relative_error, -1.0);
    assert!(layer.passed, "门禁关闭期间越界值不阻断判定");
    assert!(gate.passed);
}
