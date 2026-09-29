//! 罚球行为：投篮犯规、bonus 门槛、罚球序列与球态表现。
//!
//! 守卫对象：犯规事实如何进入罚球程序、罚球次数来自联赛档案、罚球期间球态
//! 不得声称持球人、命中与不中的后续程序。
//!
//! 对应 `docs/architecture.md` §3.3 的 `Dead{Foul} → FreeThrow` 边、
//! `docs/quality.md` §2.1 的回合因果要求。

#![allow(clippy::field_reassign_with_default)]

mod support;

use nba_domain::court::Court;
use nba_domain::GameFlowState;
use nba_engine::MatchEngine;

#[test]
fn test_shooting_foul_enters_free_throw_state_and_scores_free_throws() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    rules.free_throw_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(101, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.push_event_for_test(nba_domain::GameEvent::new_foul("H_01", "A_01", true));
    let first = engine.step();
    assert!(
        first.frame.events.iter().any(|event| event == "FOUL")
            || first.frame.event_type.as_deref() == Some("FOUL")
    );
    assert_eq!(engine.game_flow(), GameFlowState::FreeThrow);
    assert_eq!(engine.free_throws_remaining(), 2);
    let score_before = engine.engine_snapshot().game.home_score;
    let mut attempts = Vec::new();
    for _ in 0..200 {
        let tick = engine.step();
        attempts.extend(
            tick.frame
                .event_log
                .iter()
                .filter(|event| event.kind == "FREE_THROW")
                .map(|event| {
                    let attempt = event
                        .data
                        .as_ref()
                        .and_then(|data| data.get("FreeThrowAttempt"))
                        .expect("FREE_THROW event payload");
                    (
                        attempt
                            .get("attempt")
                            .and_then(serde_json::Value::as_u64)
                            .expect("free-throw attempt number") as u8,
                        attempt
                            .get("made")
                            .and_then(serde_json::Value::as_bool)
                            .expect("free-throw result"),
                    )
                }),
        );
        if engine.free_throws_remaining() == 0 {
            break;
        }
    }
    assert_eq!(
        engine.free_throws_remaining(),
        0,
        "free-throw sequence stalled after {attempts:?}: flow={:?}, phase={:?}, ball={:?}",
        engine.game_flow(),
        engine.sub_phase(),
        engine.ball_state(),
    );
    assert_eq!(
        attempts.len(),
        2,
        "each awarded free throw must resolve once"
    );
    assert_eq!(
        attempts.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(engine.box_score().ft_attempts, attempts.len() as u32);
    assert_eq!(
        engine.box_score().ft_made,
        attempts.iter().filter(|(_, made)| *made).count() as u32
    );
    assert_eq!(
        engine.engine_snapshot().game.home_score - score_before,
        engine.box_score().ft_made
    );
}

#[test]
fn test_free_throw_phase_is_visible_in_stream_contract() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.decision_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(404, rules);
    engine.push_event_for_test(nba_domain::GameEvent::new_foul("H_01", "A_01", true));
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
    engine.push_event_for_test(nba_domain::GameEvent::new_foul("H_01", "A_01", false));
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
    engine.push_event_for_test(nba_domain::GameEvent::new_foul("H_03", "A_02", true));
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
    engine.push_event_for_test(nba_domain::GameEvent::new_foul("H_01", "A_01", true));
    engine.step();
    let score_before = engine.engine_snapshot().game.home_score;
    let mut resolved_attempts = Vec::new();
    let mut saw_inbound_setup = false;
    for _ in 0..100 {
        let tick = match engine.ball_state() {
            nba_physics::BallTrajectoryKind::Dead { .. }
            | nba_physics::BallTrajectoryKind::FreeThrowSetup { .. } => {
                engine.resolve_forced_free_throw(resolved_attempts.len() == 1);
                engine.step()
            }
            _ => engine.step(),
        };
        resolved_attempts.extend(
            tick.frame
                .event_log
                .iter()
                .filter(|event| event.kind == "FREE_THROW")
                .map(|event| {
                    let attempt = event
                        .data
                        .as_ref()
                        .and_then(|data| data.get("FreeThrowAttempt"))
                        .expect("FREE_THROW event payload");
                    (
                        attempt
                            .get("attempt")
                            .and_then(serde_json::Value::as_u64)
                            .expect("free-throw attempt number") as u8,
                        attempt
                            .get("made")
                            .and_then(serde_json::Value::as_bool)
                            .expect("free-throw result"),
                    )
                }),
        );
        if engine.game_flow() == GameFlowState::DeadBall
            && engine.free_throws_remaining() == 0
            && engine.possession() == nba_domain::Possession::Away
        {
            saw_inbound_setup = true;
            break;
        }
    }
    assert!(saw_inbound_setup);
    assert_eq!(resolved_attempts, [(1, false), (2, true)]);
    assert_eq!(engine.engine_snapshot().game.home_score, score_before + 1);
    assert_eq!(engine.box_score().ft_attempts, 2);
    assert_eq!(engine.box_score().ft_made, 1);
}

#[test]
fn test_missed_final_free_throw_starts_rebound_from_last_ball_position() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.decision_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(904, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.push_event_for_test(nba_domain::GameEvent::new_foul("H_01", "A_01", true));
    engine.step();
    let score_before = engine.engine_snapshot().game.home_score;
    let mut resolved_attempts = Vec::new();
    engine.resolve_forced_free_throw(false);
    for _ in 0..100 {
        let tick = engine.step();
        resolved_attempts.extend(
            tick.frame
                .event_log
                .iter()
                .filter(|event| event.kind == "FREE_THROW")
                .map(|event| {
                    let attempt = event
                        .data
                        .as_ref()
                        .and_then(|data| data.get("FreeThrowAttempt"))
                        .expect("FREE_THROW event payload");
                    (
                        attempt
                            .get("attempt")
                            .and_then(serde_json::Value::as_u64)
                            .expect("free-throw attempt number") as u8,
                        attempt
                            .get("made")
                            .and_then(serde_json::Value::as_bool)
                            .expect("free-throw result"),
                    )
                }),
        );
        if engine.box_score().ft_attempts == 1 && engine.free_throws_remaining() == 1 {
            break;
        }
    }
    assert_eq!(resolved_attempts, [(1, false)]);
    engine.resolve_forced_free_throw(false);
    for _ in 0..100 {
        let tick = engine.step();
        resolved_attempts.extend(
            tick.frame
                .event_log
                .iter()
                .filter(|event| event.kind == "FREE_THROW")
                .map(|event| {
                    let attempt = event
                        .data
                        .as_ref()
                        .and_then(|data| data.get("FreeThrowAttempt"))
                        .expect("FREE_THROW event payload");
                    (
                        attempt
                            .get("attempt")
                            .and_then(serde_json::Value::as_u64)
                            .expect("free-throw attempt number") as u8,
                        attempt
                            .get("made")
                            .and_then(serde_json::Value::as_bool)
                            .expect("free-throw result"),
                    )
                }),
        );
        if matches!(
            engine.ball_state(),
            nba_physics::BallTrajectoryKind::RimRebound { .. }
                | nba_physics::BallTrajectoryKind::LooseBall { .. }
        ) {
            break;
        }
    }
    assert_eq!(resolved_attempts, [(1, false), (2, false)]);
    assert_eq!(engine.box_score().ft_attempts, 2);
    assert_eq!(engine.box_score().ft_made, 0);
    assert_eq!(engine.engine_snapshot().game.home_score, score_before);
    assert_eq!(engine.free_throws_remaining(), 0);
    assert!(matches!(
        engine.ball_state(),
        nba_physics::BallTrajectoryKind::RimRebound { from_pos, from_z, .. }
            if *from_pos == engine.ball_pos_3d().0 && *from_z == engine.ball_pos_3d().1
    ));
    assert_eq!(engine.game_flow(), GameFlowState::LiveBall);
    assert_eq!(engine.sub_phase(), nba_domain::SubPhase::FlightAndRebound);
}

#[test]
fn free_throw_setup_reaches_the_stripe_before_flight_starts() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.02;
    rules.free_throw_interval_seconds = 0.02;
    let mut engine = MatchEngine::with_rules(906, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.push_event_for_test(nba_domain::GameEvent::new_foul("H_01", "A_01", true));
    engine.step();
    let mut previous = engine.ball_pos_3d();
    let speed_limit =
        engine.rules().ball_max_speed_ftps + engine.rules().invariant_speed_tolerance_ftps;
    let mut saw_setup = false;
    let mut saw_flight = false;
    for _ in 0..100 {
        let tick = engine.step();
        let current = engine.ball_pos_3d();
        let elapsed = tick.frame.rules.tick_seconds;
        let speed = ((current.0 - previous.0).length_squared() + (current.1 - previous.1).powi(2))
            .sqrt()
            / elapsed;
        assert!(
            speed <= speed_limit + 1e-3,
            "free-throw setup/flight moved at {speed:.2} ft/s"
        );
        previous = current;
        match engine.ball_state() {
            nba_physics::BallTrajectoryKind::FreeThrowSetup { .. } => saw_setup = true,
            nba_physics::BallTrajectoryKind::FreeThrow { .. } => {
                assert!(saw_setup, "setup trajectory must precede free-throw flight");
                assert_eq!(
                    current,
                    (
                        Court::free_throw_pos(true, engine.rules()),
                        engine.rules().ball_holder_height_ft
                    )
                );
                saw_flight = true;
                break;
            }
            _ => {}
        }
    }
    assert!(saw_setup, "free-throw setup trajectory must be observable");
    assert!(
        saw_flight,
        "free-throw flight must start after setup reaches the stripe"
    );
}

#[test]
fn consecutive_free_throws_publish_placement_and_settle_each_attempt_once() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.02;
    rules.free_throw_interval_seconds = 0.02;
    let mut engine = MatchEngine::with_rules(907, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.push_event_for_test(nba_domain::GameEvent::new_foul("H_01", "A_01", true));
    engine.step();
    let mut previous_position = engine.ball_pos_3d();
    let mut attempts = Vec::new();
    let mut saw_initial_placement = false;
    let mut saw_next_placement = false;
    let mut violations = Vec::new();
    let speed_limit =
        engine.rules().ball_max_speed_ftps + engine.rules().invariant_speed_tolerance_ftps;
    for _ in 0..500 {
        let tick = engine.step();
        let current_position = engine.ball_pos_3d();
        let displacement = ((current_position.0 - previous_position.0).length_squared()
            + (current_position.1 - previous_position.1).powi(2))
        .sqrt();
        let speed = displacement / tick.frame.rules.tick_seconds;
        assert!(
            speed <= speed_limit + 1e-3,
            "free-throw sequence changed ball position at {speed:.2} ft/s"
        );
        previous_position = current_position;
        attempts.extend(
            tick.frame
                .event_log
                .iter()
                .filter(|event| event.kind == "FREE_THROW")
                .map(|event| {
                    let attempt = event
                        .data
                        .as_ref()
                        .and_then(|data| data.get("FreeThrowAttempt"))
                        .expect("FREE_THROW event payload");
                    (
                        attempt
                            .get("attempt")
                            .and_then(serde_json::Value::as_u64)
                            .expect("free-throw attempt number") as u8,
                        attempt
                            .get("made")
                            .and_then(serde_json::Value::as_bool)
                            .expect("free-throw result"),
                    )
                }),
        );
        saw_initial_placement |= tick.frame.event_log.iter().any(|event| {
            event.kind == "BALL_PLACEMENT_APPLIED"
                && event
                    .data
                    .as_ref()
                    .and_then(|data| data.get("BallPlacementApplied"))
                    .and_then(|placement| placement.get("reason"))
                    .and_then(|reason| reason.as_str())
                    == Some("FREE_THROW_SETUP")
        });
        let has_setup_placement = tick.frame.event_log.iter().any(|event| {
            event.kind == "BALL_PLACEMENT_APPLIED"
                && event
                    .data
                    .as_ref()
                    .and_then(|data| data.get("BallPlacementApplied"))
                    .and_then(|placement| placement.get("reason"))
                    .and_then(|reason| reason.as_str())
                    == Some("FREE_THROW_SETUP")
        });
        saw_next_placement |= has_setup_placement && attempts.len() == 1;
        violations.extend(engine.last_tick_violations().iter().map(|item| item.rule));
        if engine.free_throws_remaining() == 0 {
            break;
        }
    }
    assert_eq!(attempts.len(), 2);
    assert_eq!(
        attempts
            .iter()
            .map(|(number, _)| *number)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(engine.box_score().ft_attempts, attempts.len() as u32);
    assert_eq!(
        engine.box_score().ft_made,
        attempts.iter().filter(|(_, made)| *made).count() as u32
    );
    assert!(saw_initial_placement);
    assert!(saw_next_placement);
    assert!(
        violations.is_empty(),
        "free-throw violations: {violations:?}"
    );
}

#[test]
fn free_throw_flight_samples_between_the_stripe_and_hoop() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.02;
    let mut engine = MatchEngine::with_rules(905, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.push_event_for_test(nba_domain::GameEvent::new_foul("H_01", "A_01", true));
    engine.step();
    let mut sampled_flight = false;
    for _ in 0..100 {
        let tick = engine.step();
        if let nba_physics::BallTrajectoryKind::FreeThrowSetup { .. } = engine.ball_state() {
            engine.resolve_forced_free_throw(false);
        } else if let nba_physics::BallTrajectoryKind::Dead { .. } = engine.ball_state() {
            engine.resolve_forced_free_throw(false);
        }
        if matches!(
            engine.ball_state(),
            nba_physics::BallTrajectoryKind::FreeThrow { .. }
        ) {
            let current = engine.ball_pos_3d();
            if current
                .0
                .distance(Court::free_throw_pos(true, engine.rules()))
                > 0.1
            {
                sampled_flight = true;
                break;
            }
            assert!(tick.frame.ball.holder_id.is_none());
        }
    }
    assert!(
        sampled_flight,
        "free-throw ball position must advance during flight"
    );
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
    engine.push_event_for_test(nba_domain::GameEvent::new_foul("H_01", "A_01", true));

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
    engine.push_event_for_test(nba_domain::GameEvent::new_foul("H_01", "A_01", true));
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
