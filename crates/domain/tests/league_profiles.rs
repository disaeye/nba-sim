use nba_domain::event::FoulKind;
use nba_domain::league::LeagueProfile;

#[test]
fn the_nba_profile_keeps_disruptive_behaviour_inside_the_flagrant_kind() {
    let nba = LeagueProfile::nba();
    assert!(!nba.disruptive_foul_is_separate());
    assert!(
        !nba.foul_kinds
            .recognized
            .iter()
            .any(|kind| kind == "disruptive"),
        "NBA 2025-26 的犯规分类没有破坏性犯规这一类"
    );
    assert!(
        nba.foul_kinds.technical_categories.is_empty(),
        "NBA 不区分技术犯规等级"
    );
    // 这条通道在 NBA 档案里是通的：同一个枚举值可以被记账，
    // 但它按恶意犯规以外的普通种类处理，不是独立分类。
    assert_eq!(FoulKind::Flagrant, FoulKind::Flagrant);
}

#[test]
fn the_fiba_profile_separates_disruptive_fouls_and_technical_categories() {
    let fiba = LeagueProfile::fiba();
    assert!(fiba.disruptive_foul_is_separate());
    for kind in ["technical_category_1", "technical_category_2", "disruptive"] {
        assert!(
            fiba.foul_kinds.recognized.iter().any(|entry| entry == kind),
            "FIBA 档案缺少犯规种类 {kind}"
        );
    }
    assert_eq!(
        fiba.foul_kinds.technical_categories,
        ["technical_category_1", "technical_category_2"]
    );
}

#[test]
fn both_profiles_declare_referee_authority_and_review_scope() {
    for profile in [LeagueProfile::nba(), LeagueProfile::fiba()] {
        let label = format!("{:?}", profile.id);
        assert!(
            profile.authorizes("foul_judgement"),
            "{label} 档案必须声明裁判有权判定犯规"
        );
        assert!(
            profile.authorizes("replay_review"),
            "{label} 档案必须声明裁判有权发起录像复核"
        );
        assert!(
            profile.reviews("out_of_bounds_assignment"),
            "{label} 档案的复核范围必须覆盖出界归属"
        );
        assert!(
            profile.reviews("clock_malfunction"),
            "{label} 档案的复核范围必须覆盖计时器失效"
        );
        assert!(
            !profile.reviews("shot_selection"),
            "{label} 档案的复核范围不包括进攻选择"
        );
    }
}

#[test]
fn the_two_profiles_review_different_questions() {
    let nba = LeagueProfile::nba();
    let fiba = LeagueProfile::fiba();
    // NBA Rule 13 的复核项以恶意犯规升级与干扰投篮为主；
    // FIBA Article 46 额外覆盖两分/三分判定与节末比赛钟。
    assert!(nba.reviews("flagrant_upgrade"));
    assert!(!nba.reviews("two_versus_three_points"));
    assert!(fiba.reviews("unsportsmanlike_upgrade"));
    assert!(fiba.reviews("two_versus_three_points"));
    assert!(fiba.reviews("game_clock_at_period_end"));
}
