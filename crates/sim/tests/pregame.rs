//! 赛前名单检查：首发五人、名单上限、号码唯一。

use nba_domain::rules::GameRules;
use nba_sim_core::MatchSetup;

#[test]
fn a_duplicated_starter_is_rejected() {
    let mut setup = MatchSetup::builtin(GameRules::default());
    setup.home_lineup = [0, 0, 1, 2, 3];
    let error = setup.validate().expect_err("重复首发必须被拒绝");
    assert!(error.contains("listed twice"), "实际错误：{error}");
}

#[test]
fn a_roster_over_the_league_limit_is_rejected() {
    let mut setup = MatchSetup::builtin(GameRules::default());
    let limit = setup.rules.league.roster_limit as usize;
    while setup.home.players.len() <= limit {
        let mut extra = setup.home.players[0].clone();
        extra.jersey = format!("{}", setup.home.players.len() + 20);
        setup.home.players.push(extra);
    }
    let error = setup.validate().expect_err("超编名单必须被拒绝");
    assert!(error.contains("limit is"), "实际错误：{error}");
}

#[test]
fn a_duplicated_jersey_is_rejected() {
    let mut setup = MatchSetup::builtin(GameRules::default());
    setup.home.players[1].jersey = setup.home.players[0].jersey.clone();
    let error = setup.validate().expect_err("重号必须被拒绝");
    assert!(error.contains("belongs to two players"), "实际错误：{error}");
}

#[test]
fn a_roster_shorter_than_a_lineup_is_rejected() {
    let mut setup = MatchSetup::builtin(GameRules::default());
    setup.away.players.truncate(4);
    let error = setup.validate().expect_err("不足五人的名单必须被拒绝");
    assert!(error.contains("at least 5 required"), "实际错误：{error}");
}
