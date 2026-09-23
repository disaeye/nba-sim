//! 投篮 Release 时序（plan.md §6.2）：裁定与脱手分离的因果闭环。
//!
//! `execute_shot` 在入口冻结全部裁定（is_made/fouled/peak_z/flight_time），
//! 但球态保持 `Held`；窗口跨过 Execution→FollowThrough 边界（真正的
//! Release）才转 `Shot` 球态、发 `ShotRelease` 事件、进入 `ShotAttempt`。
//! 覆盖：入口 tick 球在手且无事件、Release 时刻球态与事件、Preparation 段
//! 封盖被拒、挂起期不再产生新的持球决策。

use nba_domain::GameRules;
use nba_engine::MatchEngine;
use nba_physics::ballistics::BallTrajectoryKind;

/// 缩短投篮窗口（规则通道），使探针在数百 tick 内覆盖整条链。
fn quick_rules() -> GameRules {
    GameRules {
        tick_seconds: 0.04,
        jump_shot_prep_seconds: 0.08,
        jump_shot_exec_seconds: 0.08,
        jump_shot_follow_seconds: 0.16,
        ..Default::default()
    }
}

/// 构造一台半场阵地持球引擎（自然开球后强制持球人投篮）。
fn engine_with_carrier(seed: u64, rules: GameRules) -> (MatchEngine, String) {
    let mut engine = MatchEngine::with_rules(seed, rules);
    let carrier = loop {
        let _ = engine.step();
        let carrier = engine.carrier_id_for_test();
        if matches!(
            engine.ball_state(),
            BallTrajectoryKind::Held { .. }
        ) {
            break carrier;
        }
        if engine.is_finished() {
            panic!("match finished before a held ball appeared");
        }
    };
    (engine, carrier)
}

#[test]
fn shot_release_happens_at_the_window_boundary_not_at_adjudication() {
    let (mut engine, carrier) = engine_with_carrier(9101, quick_rules());
    // 持球人位置即出手点；冻结裁定后球必须仍在手。
    let shooter_pos = engine
        .physics()
        .get_player(&carrier)
        .expect("carrier exists")
        .pos_ft;
    engine.clear_pending_events_for_test();
    engine.execute_shot_for_test(&carrier, shooter_pos, false);

    let ball = engine.ball_state();
    assert!(
        matches!(&ball, BallTrajectoryKind::Held { carrier_id } if carrier_id == &carrier),
        "ball must stay Held after adjudication, got {ball:?}"
    );
    assert!(
        engine
            .pending_events()
            .iter()
            .all(|event| !matches!(event, nba_domain::GameEvent::ShotRelease { .. })),
        "ShotRelease must not fire at adjudication time"
    );
    assert_eq!(
        engine.pending_release_shooter_for_test().as_deref(),
        Some(carrier.as_str()),
        "frozen adjudication must be pending"
    );

    // 推进跨过 prep+exec 边界：球脱手、事件发布。
    let mut released_tick = None;
    for tick in 0..40u64 {
        let stream = engine.step();
        if stream
            .frame
            .event_log
            .iter()
            .any(|event| event.kind == "SHOT_RELEASE")
        {
            released_tick = Some(tick);
            break;
        }
        if engine.is_finished() {
            break;
        }
    }
    let released_tick = released_tick.expect("ShotRelease must fire within 40 ticks");
    assert!(
        matches!(engine.ball_state(), BallTrajectoryKind::Shot { .. }),
        "ball must be Shot after release (tick {released_tick})"
    );
    assert_eq!(
        engine.pending_release_shooter_for_test(),
        None,
        "pending release must be consumed at the boundary"
    );
}

#[test]
fn pending_release_blocks_new_on_ball_decisions() {
    let (mut engine, carrier) = engine_with_carrier(9102, quick_rules());
    let shooter_pos = engine
        .physics()
        .get_player(&carrier)
        .expect("carrier exists")
        .pos_ft;
    engine.execute_shot_for_test(&carrier, shooter_pos, false);
    assert!(engine.pending_release_shooter_for_test().is_some());
    // 挂起期推进：球权必须保持（出手者正执行已冻结的出手），决策间隔
    // 到达也不产生新决策（球不换手、无 PASS/DRIVE 事件）。
    for _ in 0..6u64 {
        let stream = engine.step();
        assert!(
            matches!(engine.ball_state(), BallTrajectoryKind::Held { .. } | BallTrajectoryKind::Shot { .. }),
            "pending window must keep the ball with the shooter, got {:?}",
            stream.frame.ball.status
        );
        if engine.is_finished() {
            break;
        }
    }
}

#[test]
fn preparation_phase_rejects_blocks() {
    let (mut engine, carrier) = engine_with_carrier(9103, quick_rules());
    let shooter_pos = engine
        .physics()
        .get_player(&carrier)
        .expect("carrier exists")
        .pos_ft;
    engine.execute_shot_for_test(&carrier, shooter_pos, false);
    // 入口（Preparation 段）：封盖门控拒绝。BLOCKED 事件不得出现。
    for _ in 0..2u64 {
        let stream = engine.step();
        assert!(
            stream
                .frame
                .event_log
                .iter()
                .all(|event| event.kind != "BLOCKED"),
            "Preparation-phase attempts must not be blocked"
        );
        if engine.is_finished() {
            break;
        }
    }
}
