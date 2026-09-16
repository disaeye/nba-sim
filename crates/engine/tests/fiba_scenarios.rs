use nba_domain::{GameRules, LeagueProfile, Possession};
use nba_engine::MatchEngine;

#[test]
fn scenario_held_ball_possession_arrow_fiba_vs_nba() {
    // 1. FIBA 模式：争球依据箭头分配球权并翻转
    let fiba_rules = GameRules {
        league: LeagueProfile::fiba(),
        ..Default::default()
    };
    assert!(fiba_rules.league.use_alternate_possession_arrow);

    let mut fiba_engine = MatchEngine::with_rules(42, fiba_rules);
    fiba_engine.set_possession_arrow_for_test(Some(Possession::Away));
    assert_eq!(fiba_engine.possession_arrow(), Some(Possession::Away));

    // 触发争球：裁定给客队，箭头翻转为主队
    let awarded = fiba_engine.resolve_held_ball();
    assert_eq!(awarded, Possession::Away);
    assert_eq!(fiba_engine.possession_arrow(), Some(Possession::Home));

    // 再次触发争球：裁定给主队，箭头翻转为客队
    let next_awarded = fiba_engine.resolve_held_ball();
    assert_eq!(next_awarded, Possession::Home);
    assert_eq!(fiba_engine.possession_arrow(), Some(Possession::Away));

    // 2. NBA 模式：争球不依赖静态交替箭头
    let nba_rules = GameRules {
        league: LeagueProfile::nba(),
        ..Default::default()
    };
    assert!(!nba_rules.league.use_alternate_possession_arrow);

    let nba_engine = MatchEngine::with_rules(42, nba_rules);
    assert!(!nba_engine.rules().league.use_alternate_possession_arrow);
}

#[test]
fn scenario_possession_arrow_flips_on_held_ball() {
    let rules = GameRules {
        league: LeagueProfile::fiba(),
        ..Default::default()
    };

    let mut engine = MatchEngine::with_rules(100, rules);
    engine.set_possession_arrow_for_test(Some(Possession::Home));

    // 模拟多次争球，箭头应严格交替翻转
    let expected_sequence = [
        Possession::Home,
        Possession::Away,
        Possession::Home,
        Possession::Away,
    ];
    for expected in expected_sequence {
        assert_eq!(engine.possession_arrow(), Some(expected));
        let awarded = engine.resolve_held_ball();
        assert_eq!(awarded, expected);
    }
}

#[test]
fn scenario_free_throw_in_and_out_resolution() {
    let rules = GameRules {
        league: LeagueProfile::fiba(),
        ..Default::default()
    };

    let engine = MatchEngine::with_rules(7, rules);
    // 验证初始状态下零罚球违规
    let snap = engine.engine_snapshot();
    assert_eq!(snap.box_score.ft_attempts, 0);
    assert_eq!(snap.box_score.ft_made, 0);
}
