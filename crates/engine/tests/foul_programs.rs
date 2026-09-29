//! R5 罚则与犯满程序回归（charter §5.2-§5.3）。
//!
//! 逐次罚球：死球期间的新犯规不覆写进行中的罚球程序，队列按序执行；
//! 犯满离场：被持球状态阻塞的强制换人在阻塞解除后被重试执行。

use nba_engine::MatchEngine;

/// 收集到目前为止的全部 FREE_THROW 事实（shooter, attempt）。
fn collect_free_throws(engine: &MatchEngine, sink: &mut Vec<(String, u8)>) {
    for event in engine.current_event_log_for_test() {
        if event.kind == "FREE_THROW" {
            if let Some(attempt) = event
                .data
                .as_ref()
                .and_then(|d| d.get("FreeThrowAttempt"))
                .and_then(|ft| {
                    let shooter = ft.get("shooter_id")?.as_str()?.to_string();
                    let attempt = ft.get("attempt")?.as_u64()? as u8;
                    Some((shooter, attempt))
                })
            {
                sink.push(attempt);
            }
        }
    }
}

#[test]
fn queued_free_throw_programs_execute_in_order() {
    let rules = nba_domain::GameRules {
        tip_off_duration_seconds: 0.0,
        free_throw_interval_seconds: 0.1,
        ..Default::default()
    };
    let mut engine = MatchEngine::with_rules(720, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);

    // 第一次犯规：H_01 获得两次罚球，程序立即开始。
    engine.push_event_for_test(nba_domain::GameEvent::new_foul("H_01", "A_01", true));
    engine.step();
    assert_eq!(
        engine.free_throw_shooter_for_test().as_deref(),
        Some("H_01"),
        "first program must start immediately"
    );

    // 程序进行中的第二次犯规：进入队列，不得覆写当前程序。
    engine.push_event_for_test(nba_domain::GameEvent::new_foul("H_02", "A_02", true));
    engine.step();
    assert_eq!(
        engine.free_throw_shooter_for_test().as_deref(),
        Some("H_01"),
        "queued foul must not overwrite the active program"
    );
    assert_eq!(
        engine.free_throw_queue_len_for_test(),
        1,
        "second foul's free throws must be queued"
    );

    // 步进直到两套程序全部完成：H_01 两次 + H_02 两次，按序执行。
    let mut attempts: Vec<(String, u8)> = Vec::new();
    for _ in 0..400 {
        let tick = engine.step();
        collect_free_throws(&engine, &mut attempts);
        if attempts.len() >= 4 {
            break;
        }
        let _ = tick;
    }
    assert_eq!(
        attempts,
        vec![
            ("H_01".to_string(), 1),
            ("H_01".to_string(), 2),
            ("H_02".to_string(), 1),
            ("H_02".to_string(), 2),
        ],
        "both programs must execute in award order"
    );
}

#[test]
fn fouled_out_player_is_substituted_after_block_clears() {
    let rules = nba_domain::GameRules {
        tip_off_duration_seconds: 0.0,
        free_throw_interval_seconds: 0.1,
        ..Default::default()
    };
    let mut engine = MatchEngine::with_rules(721, rules.clone());
    engine.force_possession_for_test(nba_domain::Possession::Home);

    // H_01 持球状态下累计 6 次个人犯规：强制换人被持球阻塞。
    engine.set_ball_state_for_test(nba_physics::ballistics::BallTrajectoryKind::Held {
        carrier_id: "H_01".to_string(),
    });
    let mut substituted = false;
    for _ in 0..6 {
        engine.push_event_for_test(nba_domain::GameEvent::new_foul("A_01", "H_01", false));
        let tick = engine.step();
        if tick.frame.events.iter().any(|e| e == "SUBSTITUTION") {
            substituted = true;
        }
    }
    let fouled_out = engine
        .physics_mut_for_test()
        .get_player("H_01")
        .map(|p| p.foul_count >= rules.league.max_personal_fouls)
        .unwrap_or(false);
    assert!(fouled_out, "six personal fouls must foul the player out");

    // 阻塞解除后重试机制必须把他换下（有限步内完成）；哨响时持球
    // 归因载荷不再阻塞，换人可能在犯满当 tick 就已执行。
    for _ in 0..600 {
        let tick = engine.step();
        if tick.frame.events.iter().any(|e| e == "SUBSTITUTION") {
            substituted = true;
        }
        let off_court = engine
            .physics_mut_for_test()
            .get_player("H_01")
            .map(|p| !p.on_court)
            .unwrap_or(true);
        if off_court {
            break;
        }
    }
    assert!(
        substituted,
        "foul-out substitution must be retried and executed"
    );
    assert!(
        engine
            .physics_mut_for_test()
            .get_player("H_01")
            .map(|p| !p.on_court)
            .unwrap_or(true),
        "fouled-out player must leave the court"
    );
}
