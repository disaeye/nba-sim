//! 联赛档案的 11 个策略族：结构完整性与缺字段失败。
//!
//! 档案是规则的唯一来源：某个策略族的字段缺失时档案读不进来，就地失败，
//! 不让一条缺了规则的档案进入比赛。

use nba_domain::league::{LeagueId, LeagueProfile};
use serde_json::Value;

/// 11 个策略族各挑一个代表字段。
///
/// 代表字段是这个族在档案里最核心的那一项：缺了它，这个族就没有内容。
const POLICY_FAMILIES: &[(&str, &str)] = &[
    ("节次", "period_duration_seconds"),
    ("时钟", "closely_guarded_seconds"),
    ("进攻钟", "shot_clock_seconds"),
    ("犯规", "max_personal_fouls"),
    ("罚球", "shooting_foul_free_throws"),
    ("发球", "inbound_seconds"),
    ("跳球", "center_circle_radius_ft"),
    ("暂停", "timeouts_per_game"),
    ("换人", "roster_limit"),
];

/// 三个独立成对象的策略族：它们在档案里是嵌套对象，缺成员或缺对象都要失败。
const NESTED_FAMILIES: &[(&str, &str, &str)] = &[
    ("违例", "violations", "recognized"),
    ("复核", "review", "challenges_per_game"),
];

fn league_json(league: LeagueId) -> Value {
    let profile = match league {
        LeagueId::Nba => LeagueProfile::nba(),
        LeagueId::Fiba => LeagueProfile::fiba(),
    };
    serde_json::to_value(&profile).expect("league profile serializes")
}

fn league_name(league: LeagueId) -> &'static str {
    match league {
        LeagueId::Nba => "nba",
        LeagueId::Fiba => "fiba",
    }
}

#[test]
fn every_policy_family_is_populated_in_both_leagues() {
    for league in [LeagueId::Nba, LeagueId::Fiba] {
        let profile = match league {
            LeagueId::Nba => LeagueProfile::nba(),
            LeagueId::Fiba => LeagueProfile::fiba(),
        };
        assert!(
            profile.periods.regulation_periods > 0,
            "{}: 节次策略族没有常规节数",
            league_name(league)
        );
        assert!(
            profile.clocks.closely_guarded_seconds > 0.0,
            "{}: 时钟策略族没有紧逼盯防时限",
            league_name(league)
        );
        assert!(
            profile.shot_clock.shot_clock_seconds > 0.0,
            "{}: 进攻钟策略族没有进攻钟时长",
            league_name(league)
        );
        assert!(
            profile.fouls.max_personal_fouls > 0,
            "{}: 犯规策略族没有犯满离场阈值",
            league_name(league)
        );
        assert!(
            profile.free_throws.shooting_foul_free_throws > 0,
            "{}: 罚球策略族没有投篮犯规罚球数",
            league_name(league)
        );
        assert!(
            profile.inbound.inbound_seconds > 0.0,
            "{}: 发球策略族没有发球时限",
            league_name(league)
        );
        assert!(
            profile.jump_ball.center_circle_radius_ft > 0.0,
            "{}: 跳球策略族没有中圈半径",
            league_name(league)
        );
        assert!(
            profile.timeouts.timeouts_per_game > 0,
            "{}: 暂停策略族没有暂停额度",
            league_name(league)
        );
        assert!(
            profile.substitutions.roster_limit > 0,
            "{}: 换人策略族没有名单上限",
            league_name(league)
        );
        assert!(
            !profile.violations.recognized.is_empty(),
            "{}: 违例策略族没有登记违例种类",
            league_name(league)
        );
        assert!(
            !profile.review.review_scope.is_empty(),
            "{}: 复核策略族没有复核范围",
            league_name(league)
        );
    }
}

#[test]
fn a_missing_field_in_any_flat_policy_family_is_rejected() {
    for league in [LeagueId::Nba, LeagueId::Fiba] {
        for (family, field) in POLICY_FAMILIES {
            let mut document = league_json(league);
            let object = document
                .as_object_mut()
                .expect("league profile serializes as an object");
            let removed = object.remove(*field);
            assert!(
                removed.is_some(),
                "{}: 档案里没有 {family} 策略族的字段 {field}",
                league_name(league)
            );
            let error = serde_json::from_value::<LeagueProfile>(document)
                .expect_err("缺字段的档案必须读不进来");
            let message = error.to_string();
            assert!(
                message.contains(*field),
                "{}: 错误必须指出缺失的字段 {field}，实际是：{message}",
                league_name(league)
            );
        }
    }
}

#[test]
fn a_missing_member_in_any_nested_policy_family_is_rejected() {
    for league in [LeagueId::Nba, LeagueId::Fiba] {
        for (family, object_key, member) in NESTED_FAMILIES {
            let mut document = league_json(league);
            let object = document
                .as_object_mut()
                .expect("league profile serializes as an object");
            let family_object = object
                .get_mut(*object_key)
                .and_then(Value::as_object_mut)
                .unwrap_or_else(|| panic!("{}: 档案里没有 {family} 策略族", league_name(league)));
            let removed = family_object.remove(*member);
            assert!(
                removed.is_some(),
                "{}: {family} 策略族里没有字段 {member}",
                league_name(league)
            );
            let error = serde_json::from_value::<LeagueProfile>(document)
                .expect_err("缺字段的档案必须读不进来");
            let message = error.to_string();
            assert!(
                message.contains(*member),
                "{}: 错误必须指出缺失的字段 {member}，实际是：{message}",
                league_name(league)
            );
        }
    }
}

#[test]
fn a_missing_policy_family_object_is_rejected() {
    for league in [LeagueId::Nba, LeagueId::Fiba] {
        for (family, object_key, _) in NESTED_FAMILIES {
            let mut document = league_json(league);
            let object = document
                .as_object_mut()
                .expect("league profile serializes as an object");
            assert!(
                object.remove(*object_key).is_some(),
                "{}: 档案里没有 {family} 策略族",
                league_name(league)
            );
            serde_json::from_value::<LeagueProfile>(document)
                .expect_err("缺策略族的档案必须读不进来");
        }
    }
}

#[test]
fn an_unknown_field_is_rejected() {
    for league in [LeagueId::Nba, LeagueId::Fiba] {
        let mut document = league_json(league);
        document
            .as_object_mut()
            .expect("league profile serializes as an object")
            .insert("not_a_rule_field".to_string(), Value::from(1));
        serde_json::from_value::<LeagueProfile>(document)
            .expect_err("档案里的未知字段必须读不进来");
    }
}
