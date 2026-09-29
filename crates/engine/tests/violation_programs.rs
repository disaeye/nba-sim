//! R5 违例程序回归（charter §6.2）：攻方三秒与回场。
//!
//! 判据链：时钟层按控球谓词累加（三秒：限制区停留；回场：前场建立），
//! 约束层验证判据并给出责任人，违例经 `start_violation_turnover` 终结
//! 回合并把责任人写进回合总结（TURNOVER_ACTOR_CONSISTENCY 硬门的输入）。

use glam::Vec2;
use nba_domain::Possession;
use nba_engine::MatchEngine;
use nba_physics::ballistics::BallTrajectoryKind;

/// 把一名球员固定在指定位置（位置与目标一致，消除战术移动噪声）。
fn pin_player(engine: &mut MatchEngine, id: &str, pos: Vec2) {
    if let Some(p) = engine.physics_mut_for_test().get_player_mut(id) {
        p.pos_ft = pos;
        p.target_pos_ft = pos;
    }
}

#[test]
fn offensive_three_second_lane_violation_fires() {
    let rules = nba_domain::GameRules {
        tip_off_duration_seconds: 0.0,
        tick_seconds: 0.1,
        three_second_lane_seconds: 0.5,
        decision_interval_seconds: 100.0,
        ..Default::default()
    };
    let mut engine = MatchEngine::with_rules(710, rules);
    engine.force_possession_for_test(Possession::Home);
    engine.set_game_flow(nba_domain::GameFlowState::LiveBall);
    engine.set_ball_state_for_test(BallTrajectoryKind::Held {
        carrier_id: "H_01".to_string(),
    });
    // 持球人在前场弧顶（Home 攻右篮，x >= 47 为前场）。
    let top = Vec2::new(75.0, 25.0);
    engine.set_ball_pos_for_test(top, 4.0);
    pin_player(&mut engine, "H_01", top);
    // H_04 长时间驻留限制区（距右底线 6 ft、居中）。
    let lane_post = Vec2::new(88.0, 25.0);
    pin_player(&mut engine, "H_04", lane_post);

    let mut fired = false;
    for i in 0..20 {
        // 每步把 H_04 钉回限制区（战术目标会试图把他移出去）。
        pin_player(&mut engine, "H_04", lane_post);
        let tick = engine.step();
        if tick.frame.events.iter().any(|e| e == "VIOLATION") {
            fired = true;
            assert!(
                tick.frame
                    .callout
                    .as_deref()
                    .is_some_and(|c| c.contains("three_second_lane")),
                "violation callout must name the three_second_lane program: {:?}",
                tick.frame.callout
            );
            // 违例责任人必须是限制区停留者，写入回合总结（硬门输入）。
            let summary = tick.frame.event_log.iter().find_map(|e| {
                if e.kind == "POSSESSION_SUMMARY" {
                    e.data
                        .as_ref()
                        .and_then(|d| d.get("PossessionSummary"))
                        .and_then(|s| s.get("turnover_player_id"))
                        .cloned()
                } else {
                    None
                }
            });
            assert_eq!(
                summary,
                Some(serde_json::json!("H_04")),
                "three-second violation must attribute the lane dweller, tick {i}"
            );
            break;
        }
    }
    assert!(fired, "lane camper must be whistled within 20 ticks");
}

#[test]
fn three_second_lane_satisfied_by_leaving_the_lane() {
    let rules = nba_domain::GameRules {
        tip_off_duration_seconds: 0.0,
        tick_seconds: 0.1,
        three_second_lane_seconds: 0.5,
        decision_interval_seconds: 100.0,
        ..Default::default()
    };
    let mut engine = MatchEngine::with_rules(711, rules);
    engine.force_possession_for_test(Possession::Home);
    engine.set_game_flow(nba_domain::GameFlowState::LiveBall);
    engine.set_ball_state_for_test(BallTrajectoryKind::Held {
        carrier_id: "H_01".to_string(),
    });
    let top = Vec2::new(75.0, 25.0);
    engine.set_ball_pos_for_test(top, 4.0);
    pin_player(&mut engine, "H_01", top);
    // H_04 在限制区与弧顶之间往返：每次停留都低于时限，不得判违例。
    let lane_post = Vec2::new(88.0, 25.0);
    let wing = Vec2::new(84.0, 12.0);
    let mut violated = false;
    for step in 0..16 {
        let pos = if step % 4 < 2 { lane_post } else { wing };
        pin_player(&mut engine, "H_04", pos);
        let tick = engine.step();
        if tick.frame.events.iter().any(|e| e == "VIOLATION") {
            violated = true;
            break;
        }
    }
    assert!(
        !violated,
        "intermittent lane presence below the threshold must not be whistled"
    );
}

#[test]
fn over_and_back_violation_fires_after_frontcourt_establishment() {
    let rules = nba_domain::GameRules {
        tip_off_duration_seconds: 0.0,
        tick_seconds: 0.1,
        backcourt_seconds: 100.0,
        decision_interval_seconds: 100.0,
        ..Default::default()
    };
    let mut engine = MatchEngine::with_rules(712, rules);
    engine.force_possession_for_test(Possession::Home);
    engine.set_game_flow(nba_domain::GameFlowState::LiveBall);
    engine.set_ball_state_for_test(BallTrajectoryKind::Held {
        carrier_id: "H_01".to_string(),
    });
    // 先在前场建立控制。
    let frontcourt = Vec2::new(70.0, 25.0);
    engine.set_ball_pos_for_test(frontcourt, 4.0);
    pin_player(&mut engine, "H_01", frontcourt);
    engine.step();
    // 持球人带球回后场。
    let backcourt = Vec2::new(20.0, 25.0);
    engine.set_ball_pos_for_test(backcourt, 4.0);
    pin_player(&mut engine, "H_01", backcourt);
    let tick = engine.step();
    assert!(
        tick.frame.events.iter().any(|e| e == "VIOLATION"),
        "returning the ball to the backcourt must be whistled, events={:?}",
        tick.frame.events
    );
    assert!(
        tick.frame
            .callout
            .as_deref()
            .is_some_and(|c| c.contains("over_and_back")),
        "violation callout must name the over_and_back program: {:?}",
        tick.frame.callout
    );
}

#[test]
fn over_and_back_does_not_fire_before_frontcourt_establishment() {
    let rules = nba_domain::GameRules {
        tip_off_duration_seconds: 0.0,
        tick_seconds: 0.1,
        decision_interval_seconds: 100.0,
        ..Default::default()
    };
    let mut engine = MatchEngine::with_rules(713, rules);
    engine.force_possession_for_test(Possession::Home);
    engine.set_game_flow(nba_domain::GameFlowState::LiveBall);
    engine.set_ball_state_for_test(BallTrajectoryKind::Held {
        carrier_id: "H_01".to_string(),
    });
    // 整回合从未进入前场：后场持球推进不构成回场。
    let backcourt = Vec2::new(20.0, 25.0);
    engine.set_ball_pos_for_test(backcourt, 4.0);
    pin_player(&mut engine, "H_01", backcourt);
    let tick = engine.step();
    assert!(
        !tick.frame.events.iter().any(|e| e == "VIOLATION"),
        "backcourt possession without frontcourt establishment must not be over-and-back, events={:?}",
        tick.frame.events
    );
}

/// 前场已建立后，**向前场推进的传球**在飞行前半段球仍采样在中线后，
/// 这不是回场（真实规则里只有把球**带回**后场才吹）。
///
/// 回归起因：该场景曾让 `decision_wiring` 的过场传球在到达前被吹回场，
/// 球权直接易手，接球人 30 tick 内接不到球。
#[test]
fn over_and_back_ignores_forward_pass_crossing_midcourt() {
    let rules = nba_domain::GameRules {
        tip_off_duration_seconds: 0.0,
        tick_seconds: 0.1,
        backcourt_seconds: 100.0,
        decision_interval_seconds: 100.0,
        ..Default::default()
    };
    let mut engine = MatchEngine::with_rules(714, rules);
    engine.force_possession_for_test(Possession::Home);
    engine.set_game_flow(nba_domain::GameFlowState::LiveBall);
    // 前场建立控制。
    engine.set_ball_state_for_test(BallTrajectoryKind::Held {
        carrier_id: "H_01".to_string(),
    });
    let frontcourt = Vec2::new(70.0, 25.0);
    engine.set_ball_pos_for_test(frontcourt, 4.0);
    pin_player(&mut engine, "H_01", frontcourt);
    engine.step();
    // 从后场向**前场**目标点（x=60 > 中线 47）的传球，飞行中球位仍在后场。
    let mid_flight = Vec2::new(40.0, 25.0);
    engine.set_ball_pos_for_test(mid_flight, 4.0);
    engine.set_ball_state_for_test(BallTrajectoryKind::Pass {
        from_pos: Vec2::new(30.0, 25.0),
        from_z: 4.0,
        to_pos: Vec2::new(60.0, 25.0),
        target_id: "H_02".to_string(),
        start_time: engine.current_time(),
        duration: 1.0,
        peak_z: 4.0,
        inbound: false,
    });
    let tick = engine.step();
    assert!(
        !tick.frame.events.iter().any(|e| e == "VIOLATION"
            && tick
                .frame
                .callout
                .as_deref()
                .is_some_and(|c| c.contains("over_and_back"))),
        "a forward pass crossing midcourt must not be whistled as over-and-back, events={:?}",
        tick.frame.events
    );
}
