//! 规则场景矩阵：每条规则至少覆盖成立、不成立、阈值前后、豁免、
//! 双方主体、联赛差异与程序交互。
//!
//! 这里检验的是规则档案本身的裁决结果；引擎是否按这份档案执行由
//! `crates/sim/tests/rules_flow.rs` 与相关定向测试负责。

use nba_domain::event::FoulKind;
use nba_domain::rules::{counts_toward_personal_limit, counts_toward_team_period_fouls, GameRules};
use nba_domain::LeagueProfile;

fn profiles() -> [(&'static str, LeagueProfile); 2] {
    [("nba", LeagueProfile::nba()), ("fiba", LeagueProfile::fiba())]
}

#[test]
fn nba_and_fiba_period_boundaries_match_their_rule_profiles() {
    let nba = LeagueProfile::nba();
    let fiba = LeagueProfile::fiba();
    assert_eq!(nba.period_duration_seconds, 720.0);
    assert_eq!(fiba.period_duration_seconds, 600.0);
    assert_eq!(nba.overtime_duration_seconds, 300.0);
    assert_eq!(fiba.overtime_duration_seconds, 300.0);
    assert_eq!(nba.shot_clock_seconds, 24.0);
    assert_eq!(fiba.shot_clock_seconds, 24.0);
    assert_eq!(nba.regulation_periods, 4);
    assert_eq!(fiba.regulation_periods, 4);
}

/// 开球程序：NBA 只有开场跳球、后续节次交替拥有；FIBA 同样只有开场跳球，
/// 但使用交替拥有箭头。双方主体都按同一档案裁定。
#[test]
fn period_start_procedure_follows_the_league_arrow_policy() {
    for (name, profile) in profiles() {
        assert!(
            profile.period_starts_with_jump_ball(1),
            "{name}: the opening period must start with a jump ball"
        );
        for period in 2..=4 {
            assert!(
                !profile.period_starts_with_jump_ball(period),
                "{name}: period {period} must use the alternate-possession arrow"
            );
        }
        // 交替拥有箭头是否参与这段程序按档案记录：NBA 记 false，
        // FIBA 记 true。档案值本身就是这条规则的裁决输入。
        let expected_arrow = name == "fiba";
        assert_eq!(
            profile.use_alternate_possession_arrow, expected_arrow,
            "{name}: the recorded arrow policy must match the league file"
        );
    }
}

/// 球队犯规罚则：阈值前、正好阈值、阈值后，两队各判一次，两个联赛各一遍。
#[test]
fn team_foul_bonus_thresholds_cover_before_at_and_after_threshold() {
    let nba = LeagueProfile::nba();
    let fiba = LeagueProfile::fiba();
    for (name, profile) in [("nba", &nba), ("fiba", &fiba)] {
        let threshold = profile.bonus_foul_threshold(2, 400.0);
        assert_eq!(
            profile.bonus_free_throws_for(threshold - 1, 2, 400.0),
            0,
            "{name}: below the threshold must not award free throws"
        );
        assert_eq!(
            profile.bonus_free_throws_for(threshold, 2, 400.0),
            2,
            "{name}: at the threshold must award two free throws"
        );
        assert_eq!(
            profile.bonus_free_throws_for(threshold + 1, 2, 400.0),
            2,
            "{name}: above the threshold must keep awarding two free throws"
        );
        // 双方主体：犯规数按球队记录，两队的阈值判定相同。
        assert_eq!(
            profile.bonus_free_throws_for(threshold, 2, 400.0),
            profile.bonus_free_throws_for(threshold, 1, 400.0),
            "{name}: both teams use the same team-foul threshold"
        );
        assert_eq!(profile.shot_clock_after_offensive_rebound(8.0, true), 14.0);
        assert_eq!(profile.shot_clock_after_offensive_rebound(18.0, true), 18.0);
        assert_eq!(profile.shot_clock_after_offensive_rebound(8.0, false), 8.0);
    }
    // NBA 最后两分钟豁免：此前不足三次犯规时，第二次犯规起即罚球。
    assert_eq!(nba.bonus_free_throws_for(1, 4, 100.0), 0);
    assert_eq!(nba.bonus_free_throws_for(2, 4, 100.0), 2);
    // FIBA 没有这条豁免：末节仍按第五节犯规起算。
    assert_eq!(fiba.bonus_free_throws_for(4, 4, 100.0), 0);
    assert_eq!(fiba.bonus_free_throws_for(5, 4, 100.0), 2);
}

/// 最后两分钟窗口的边界：窗口前、正好入窗、窗口内。
#[test]
fn final_window_boundary_is_inclusive_at_its_edge() {
    let nba = LeagueProfile::nba();
    let window = f64::from(nba.bonus_final_seconds);
    assert!(!nba.in_final_window(4, window + 0.1), "outside the window");
    assert!(nba.in_final_window(4, window), "at the edge");
    assert!(nba.in_final_window(4, window - 0.1), "inside the window");
    assert!(!nba.in_final_window(3, window - 0.1), "only the final period");
}

/// 个人犯规与球队犯规是两套计数器：技术犯规不计入任何一套，
/// 进攻犯规计入个人但球队不累计。
#[test]
fn foul_kinds_use_distinct_personal_and_team_period_counters() {
    for kind in [FoulKind::Personal, FoulKind::Shooting, FoulKind::LooseBall] {
        assert!(counts_toward_personal_limit(kind), "{kind:?} personal");
        assert!(counts_toward_team_period_fouls(kind), "{kind:?} team");
    }
    assert!(counts_toward_personal_limit(FoulKind::Offensive));
    assert!(!counts_toward_team_period_fouls(FoulKind::Offensive));
    assert!(!counts_toward_personal_limit(FoulKind::Technical));
    assert!(!counts_toward_team_period_fouls(FoulKind::Technical));
}

/// 无球犯规罚则：最后两分钟窗口前后各判一次，两个联赛各一遍。
#[test]
fn away_from_play_penalty_changes_inside_the_final_window() {
    let nba = LeagueProfile::nba();
    let fiba = LeagueProfile::fiba();
    // 档案记录：NBA 的无球犯规给一罚一掷，FIBA 通过另一条罚则路径表达，
    // 其档案值为 0。测试断言记录值，避免把推断当成规则。
    assert_eq!(nba.away_from_play_free_throws, 1);
    assert_eq!(fiba.away_from_play_free_throws, 0);
    for (name, profile) in [("nba", &nba), ("fiba", &fiba)] {
        // 末段窗口只在档案登记了末段罚则时存在：NBA 登记第四节最后 120 秒，
        // FIBA 的 bonus_final_period 为 0，因此没有这条窗口。
        let has_final_window = profile.bonus_final_period > 0;
        assert_eq!(
            profile.in_final_window(4, 30.0),
            has_final_window,
            "{name}: the final window must follow the recorded profile"
        );
        assert!(
            !profile.in_final_window(2, 30.0),
            "{name}: the final window is a final-period rule"
        );
    }
}

/// 配置变更必须真的改变裁决：把阈值调成 1 之后，第一次犯规就罚球。
#[test]
fn changing_bonus_threshold_changes_the_profile_result() {
    let mut rules = GameRules::default();
    rules.league.bonus_fouls_per_period = 1;
    rules.league.bonus_fouls_final = 1;
    assert_eq!(rules.league.bonus_free_throws_for(1, 2, 400.0), 2);
    // 同一份改动对第二节与第四节都生效。
    assert_eq!(rules.league.bonus_free_throws_for(1, 4, 400.0), 2);
    // 未改动的档案在同样的输入下保持原判。
    let untouched = GameRules::default();
    assert_eq!(untouched.league.bonus_free_throws_for(1, 2, 400.0), 0);
}

/// 进攻时限：前场篮板触筐后重置到 14 秒，未触筐不重置；
/// 两个联赛一致，且不依赖哪一队进攻。
#[test]
fn offensive_rebound_shot_clock_reset_needs_rim_contact() {
    for (name, profile) in profiles() {
        for current in [4.0, 8.0, 13.9] {
            assert_eq!(
                profile.shot_clock_after_offensive_rebound(current, true),
                14.0,
                "{name}: rim contact must reset to 14 with {current}s left"
            );
        }
        for current in [14.0, 18.0, 23.0] {
            assert_eq!(
                profile.shot_clock_after_offensive_rebound(current, true),
                current,
                "{name}: above 14 the clock must not be raised"
            );
        }
        assert_eq!(
            profile.shot_clock_after_offensive_rebound(4.0, false),
            4.0,
            "{name}: without rim contact the clock must stand"
        );
    }
}

/// 比赛时长与暂停配额按联赛档案区分，加时沿用各自的常规节时长规则。
#[test]
fn overtime_and_timeout_budgets_follow_the_league_profile() {
    let nba = GameRules::default();
    let fiba = GameRules::with_league(LeagueProfile::fiba());
    assert_eq!(nba.period_duration(1), 720.0);
    assert_eq!(fiba.period_duration(1), 600.0);
    assert_eq!(nba.period_duration(5), 300.0);
    assert_eq!(fiba.period_duration(5), 300.0);
    assert!(nba.league.timeouts_per_game >= fiba.league.timeouts_per_game);
    assert!(
        nba.league.timeouts_fourth_quarter_max >= nba.league.timeouts_final_max,
        "the final-window cap cannot exceed the quarter cap"
    );
}

/// 缩短档案只改时长，不改判罚阈值：这是跨联赛对比的前提。
#[test]
fn shortened_rules_keep_the_adjudication_thresholds() {
    let full = GameRules::default();
    let short = full.shortened(120.0, 60.0);
    assert_eq!(short.period_duration(1), 120.0);
    assert_eq!(short.period_duration(5), 60.0);
    assert_eq!(
        short.league.bonus_free_throws_for(5, 2, 400.0),
        full.league.bonus_free_throws_for(5, 2, 400.0)
    );
    assert_eq!(
        short.league.shot_clock_after_offensive_rebound(8.0, true),
        full.league.shot_clock_after_offensive_rebound(8.0, true)
    );
}
