//! 罚球行为：投篮犯规、bonus 门槛、罚球序列与球态表现。
//!
//! 守卫对象：犯规事实如何进入罚球程序、罚球次数来自联赛档案、罚球期间球态
//! 不得声称持球人、命中与不中的后续程序。
//!
//! 对应 `docs/architecture.md` §3.3 的 `Dead{Foul} → FreeThrow` 边、
//! `docs/quality.md` §2.1 的回合因果要求。

#![allow(clippy::field_reassign_with_default)]

mod support;

use nba_domain::GameFlowState;
use nba_engine::MatchEngine;

#[test]
fn test_shooting_foul_enters_free_throw_state_and_scores_free_throws() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    rules.free_throw_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(101, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.push_event_for_test(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_01".to_string(),
        fouler_id: "A_01".to_string(),
        is_shooting: true,
    });
    let first = engine.step();
    assert!(
        first.frame.events.iter().any(|event| event == "FOUL")
            || first.frame.event_type.as_deref() == Some("FOUL")
    );
    assert_eq!(engine.game_flow(), GameFlowState::FreeThrow);
    assert_eq!(engine.free_throws_remaining(), 2);
    let score_before = engine.engine_snapshot().game.home_score;
    for _ in 0..20 {
        engine.step();
        if engine.free_throws_remaining() == 0 {
            break;
        }
    }
    assert_eq!(engine.free_throws_remaining(), 0);
    assert!(engine.engine_snapshot().game.home_score >= score_before);
}

#[test]
fn test_free_throw_phase_is_visible_in_stream_contract() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.decision_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(404, rules);
    engine.push_event_for_test(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_01".to_string(),
        fouler_id: "A_01".to_string(),
        is_shooting: true,
    });
    let tick = engine.step();
    assert_eq!(tick.frame.game_flow, "FreeThrow");
    assert_eq!(tick.frame.free_throws_remaining, 2);
    assert_eq!(tick.frame.team_fouls_away, 1);
    assert_eq!(
        engine
            .physics()
            .get_player("A_01")
            .expect("fouler")
            .foul_count,
        1
    );
}

#[test]
fn test_bonus_foul_uses_configured_threshold() {
    let mut rules = nba_domain::GameRules::default();
    rules.league.bonus_fouls_per_period = 1;
    let mut engine = MatchEngine::with_rules(707, rules);
    engine.push_event_for_test(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_01".to_string(),
        fouler_id: "A_01".to_string(),
        is_shooting: false,
    });
    engine.step();
    assert_eq!(engine.team_fouls_away(), 1);
    assert_eq!(engine.free_throws_remaining(), 2);
    assert_eq!(engine.game_flow(), GameFlowState::FreeThrow);
    assert_eq!(
        engine
            .physics()
            .get_player("A_01")
            .expect("fouler")
            .foul_count,
        1
    );
}

#[test]
fn test_shooting_foul_awards_free_throws_to_fouled_team_not_possession_flip() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.decision_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(901, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    let possession_before = engine.possession();
    engine.push_event_for_test(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_03".to_string(),
        fouler_id: "A_02".to_string(),
        is_shooting: true,
    });
    engine.step();
    assert_eq!(engine.free_throws_remaining(), 2);
    assert_eq!(engine.possession(), possession_before);
    assert_eq!(engine.free_throw_shooter(), Some("H_03"));
}

#[test]
fn test_made_final_free_throw_gives_ball_to_opponent_inbound() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.decision_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(902, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.push_event_for_test(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_01".to_string(),
        fouler_id: "A_01".to_string(),
        is_shooting: true,
    });
    engine.step();
    let mut saw_inbound_setup = false;
    for _ in 0..30 {
        engine.resolve_forced_free_throw(true);
        if engine.game_flow() == GameFlowState::DeadBall
            && engine.free_throws_remaining() == 0
            && engine.possession() == nba_domain::Possession::Away
        {
            saw_inbound_setup = true;
            break;
        }
    }
    assert!(saw_inbound_setup);
}

#[test]
fn test_missed_final_free_throw_starts_rebound_from_last_ball_position() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.decision_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(904, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.push_event_for_test(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_01".to_string(),
        fouler_id: "A_01".to_string(),
        is_shooting: true,
    });
    engine.step();
    engine.resolve_forced_free_throw(false);
    engine.resolve_forced_free_throw(false);
    assert!(matches!(
        engine.ball_state(),
        nba_physics::BallTrajectoryKind::RimRebound { from_pos, from_z, .. }
            if *from_pos == engine.ball_pos_3d().0 && *from_z == engine.ball_pos_3d().1
    ));
    assert_eq!(engine.game_flow(), GameFlowState::LiveBall);
    assert_eq!(engine.sub_phase(), nba_domain::SubPhase::FlightAndRebound);
}

/// F1.1 红测试：罚球全程不得输出 holderId（伪持球）。
///
/// 根因：罚球期间权威球态仍是 `Held{carrier_id}`，而 `ball_pos_3d` 被放到
/// 篮筐/罚球点，导致 `BALL_WITH_HOLDER` Hard（seed=1 full tick=58954，26.51ft）。
#[test]
fn test_free_throw_never_reports_ball_holder() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    rules.free_throw_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(101, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.push_event_for_test(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_01".to_string(),
        fouler_id: "A_01".to_string(),
        is_shooting: true,
    });

    // 第一 tick 触发犯规，随后每个 tick 检查罚球期间的球权投影。
    let mut saw_free_throw = false;
    for _ in 0..40 {
        let tick = engine.step();
        if tick.frame.game_flow == "FreeThrow" || engine.game_flow() == GameFlowState::FreeThrow {
            saw_free_throw = true;
            assert!(
                tick.frame.ball.holder_id.is_none(),
                "during FreeThrow the ball must not advertise a holder (tick {}, holder={:?}, status={})",
                engine.tick_index(),
                tick.frame.ball.holder_id,
                tick.frame.ball.status
            );
        }
        if engine.free_throws_remaining() == 0 {
            break;
        }
    }
    assert!(saw_free_throw, "free-throw flow must be observed");
}

/// F1.1 红测试：罚球期间球与任何球员的距离必须在 leash 内，否则球态必须是 Dead。
///
/// 这是 `BALL_WITH_HOLDER` 不变量语义的正面表述。
#[test]
fn test_free_throw_ball_is_dead_or_attached() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    rules.free_throw_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(102, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.push_event_for_test(nba_domain::GameEvent::Foul {
        fouled_player_id: "H_01".to_string(),
        fouler_id: "A_01".to_string(),
        is_shooting: true,
    });
    for _ in 0..40 {
        let tick = engine.step();
        if engine.game_flow() != GameFlowState::FreeThrow {
            if engine.free_throws_remaining() == 0 {
                break;
            }
            continue;
        }
        if let Some(holder) = tick.frame.ball.holder_id.as_deref() {
            let rules = &tick.frame.rules;
            let bft = (
                tick.frame.ball.x * rules.court_width_ft,
                tick.frame.ball.y * rules.court_height_ft,
            );
            if let Some(p) = engine.physics().get_player(holder) {
                let d = ((bft.0 - p.pos_ft.x).powi(2) + (bft.1 - p.pos_ft.y).powi(2)).sqrt();
                assert!(
                    d <= rules.holder_leash_ft,
                    "advertised holder {} is {:.2} ft from ball during free throw (tick {})",
                    holder,
                    d,
                    engine.tick_index()
                );
            }
        }
        if engine.free_throws_remaining() == 0 {
            break;
        }
    }
}
