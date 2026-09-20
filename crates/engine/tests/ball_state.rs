//! 球态行为：球权归属、发球程序、松球与违例的球态表现。
//!
//! 守卫对象：权威球态如何表达球权（含前场篮板不换球权）、违例如何进入发球
//! 程序、发球员的界外豁免与改派、交接与投篮弧线的物理边界。
//!
//! 对应 `docs/architecture.md` §3（球权状态机）与 ADR-016（球权是球态的派生量）。

#![allow(clippy::field_reassign_with_default)]

mod support;

use glam::Vec2;
use nba_domain::Possession;
use nba_engine::MatchEngine;

#[test]
fn test_offensive_rebound_does_not_switch_possession() {
    let mut engine = MatchEngine::new(19);
    engine.force_possession_for_test(Possession::Home);
    let before = engine.possession();
    let rebound = nba_physics::BallTrajectoryKind::RimRebound {
        from_pos: Vec2::new(88.75, 25.0),
        from_z: engine.rules().rim_height_ft,
        last_touch_team: engine.possession(),
        hoop_pos: engine.rules().court.hoop_pos(true),
        target_landing: Vec2::new(70.0, 25.0),
        start_time: engine.current_time(),
        duration: 10.0,
        peak_z: engine.rules().rebound_peak_ft,
    };
    engine.set_ball_state_for_test(rebound);
    assert_eq!(engine.possession(), before);
    assert_eq!(engine.possession(), Possession::Home);
}

#[test]
fn test_shot_clock_violation_starts_continuous_inbound_transfer() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.decision_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(903, rules);
    engine.force_possession_for_test(Possession::Home);
    engine.set_game_flow_for_test(nba_domain::GameFlowState::LiveBall);
    engine.set_shot_clock_for_test(0.0);
    engine.set_ball_state_for_test(nba_physics::BallTrajectoryKind::Held {
        carrier_id: "H_01".to_string(),
    });
    let source = engine.ball_pos_3d().0;
    let _tick = engine.step();
    assert_eq!(engine.possession(), Possession::Away);
    assert!(matches!(
        engine.ball_state(),
        nba_physics::BallTrajectoryKind::InboundTransfer { .. }
    ));
    assert_eq!(engine.ball_pos_3d().0, source);
    let mut saw_ready = false;
    for _ in 0..50 {
        let tick = engine.step();
        if tick.frame.ball.status == "INBOUND_READY" {
            saw_ready = true;
            assert!((engine.ball_pos_3d().0 - engine.inbound_baseline()).length() <= 4.5);
            break;
        }
    }
    assert!(saw_ready);
}

#[test]
fn test_inbound_pass_arrival_releases_dead_ball() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.decision_interval_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(808, rules);
    engine.push_event_for_test(nba_domain::GameEvent::RuleViolation {
        constraint_id: "test".to_string(),
        reason: "test".to_string(),
    });
    let mut saw_live = false;
    for _ in 0..30 {
        let tick = engine.step();
        saw_live |= tick.frame.game_flow == "LiveBall";
        if saw_live {
            break;
        }
    }
    assert!(saw_live);
}

/// F1.3c 红测试：被固定在边线的防守者不得阻塞发球程序。
///
/// 根因：发球员必须步行到界外发球点，而防守者被场地 clamp 固定在同一路径上，
/// 形成几何僵局（seed 6/9/11 实测约 19 万 tick 的 `OUT_OF_BOUNDS` 活锁）。
///
/// 口径：计数从「任意球员越界的连续 tick」改为「**同一球员**连续越界的 tick 数」。
/// 前者把多名球员接力越界累加成一个长 streak，与 `gap.md` §4.3 的僵局定义
/// （单球员被永久固定在边界）不符，会产生假阳性。
#[test]
fn test_wall_pinned_defender_does_not_block_inbound() {
    for seed in [6u64, 9, 11, 16, 21] {
        let mut engine = MatchEngine::new(seed);
        engine.set_scope("full").unwrap();
        let mut ticks = 0usize;
        let mut streaks: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut max_oob_streak = 0usize;
        while !engine.is_finished() && ticks < 250_000 {
            let tick = engine.step();
            let mut crossed: Vec<String> = Vec::new();
            for e in &tick.frame.event_log {
                if e.kind != "OUT_OF_BOUNDS" {
                    continue;
                }
                if let Some(pid) = e
                    .data
                    .as_ref()
                    .and_then(|d| d.get("BoundaryCross"))
                    .and_then(|b| b.get("player_id"))
                    .and_then(|v| v.as_str())
                {
                    crossed.push(pid.to_string());
                }
            }
            for pid in &crossed {
                let s = streaks.entry(pid.clone()).or_insert(0);
                *s += 1;
                max_oob_streak = max_oob_streak.max(*s);
            }
            for (pid, s) in streaks.iter_mut() {
                if !crossed.contains(pid) {
                    *s = 0;
                }
            }
            ticks += 1;
        }
        assert!(engine.is_finished(), "seed {} livelocked", seed);
        assert!(
            max_oob_streak < 200,
            "seed {} had a player pinned out of bounds for {} consecutive ticks",
            seed,
            max_oob_streak
        );
        assert!(
            engine.last_tick_violations().is_empty(),
            "seed {} ended with violations: {:?}",
            seed,
            engine.last_tick_violations()
        );
    }
}

/// F1.3：发球员被换下/犯满离场后，发球程序必须自动改派在场球员并最终完成。
///
/// 根因：`new_possession_pg` 曾按 roster 顺序取发球员，可能返回替补；
/// 替补不参与物理步进，`inbounder_arrived` 永不成立。
#[test]
fn test_inbounder_reassigned_when_leaving_court() {
    for seed in [3u64, 15, 26] {
        let mut engine = MatchEngine::new(seed);
        engine.set_scope("full").unwrap();
        let mut ticks = 0usize;
        let mut stalled_ticks = 0usize;
        let mut max_stalled = 0usize;
        while !engine.is_finished() && ticks < 250_000 {
            engine.step();
            let inbounder_off_court = match &engine.ball_state() {
                nba_physics::BallTrajectoryKind::InboundTransfer { inbounder_id, .. }
                | nba_physics::BallTrajectoryKind::InboundReady { inbounder_id, .. } => engine
                    .physics()
                    .get_player(inbounder_id)
                    .map(|p| !p.on_court)
                    .unwrap_or(true),
                _ => false,
            };
            if inbounder_off_court {
                stalled_ticks += 1;
                max_stalled = max_stalled.max(stalled_ticks);
            } else {
                stalled_ticks = 0;
            }
            ticks += 1;
        }
        assert!(engine.is_finished(), "seed {} livelocked", seed);
        // 恢复应在极短窗口内完成，而不是无限期停留。
        assert!(
            max_stalled < 60,
            "seed {} stayed with an off-court inbounder for {} ticks",
            seed,
            max_stalled
        );
    }
}

/// F1.3：发球员站界外不得被判违例，且发球程序必须真实完成推进到 `InboundReady`。
///
/// 非持球人的 `BOUNDARY_CROSSING` 是咨询性信号（flagged），不属于本节范围。
#[test]
fn test_inbounder_out_of_bounds_does_not_emit_boundary_turnover() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    rules.tick_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(703, rules);
    engine.force_possession_for_test(Possession::Home);
    engine.start_inbound_transition_for_test();
    let mut inbounder_violation = false;
    let mut reached_ready = false;
    for _ in 0..200 {
        let tick = engine.step();
        if matches!(
            engine.ball_state(),
            nba_physics::BallTrajectoryKind::InboundReady { .. }
        ) {
            reached_ready = true;
        }
        for e in &tick.frame.event_log {
            if e.kind == "VIOLATION" {
                let d = e.data.as_ref().and_then(|v| v.as_object());
                let mentions_inbounder = d
                    .map(|o| {
                        let text = serde_json::to_string(o).unwrap_or_default();
                        text.contains("INBOUND_SETUP") || text.contains("INBOUND")
                    })
                    .unwrap_or(false);
                if mentions_inbounder {
                    inbounder_violation = true;
                }
            }
        }
        if reached_ready {
            break;
        }
    }
    assert!(
        !inbounder_violation,
        "inbounder standing out of bounds must not be judged as a violation"
    );
    assert!(
        reached_ready,
        "inbound program must reach InboundReady instead of livelocking"
    );
}

/// D5.1 红测试：交接（`ControlTransfer`）不得永久悬置。
///
/// 根因：接球人被动作窗口锁定（`lock_kinematics`）时无法走到冻结点，
/// `receiver_ready` 永不成立 —— seed 6 full 实测 69,466 帧（约 2,780 秒）
/// 活锁，全场仅 11 个回合、10 分。
#[test]
fn test_control_transfer_never_hangs_forever() {
    for seed in [6u64, 21, 42] {
        let mut engine = MatchEngine::with_rules(seed, nba_domain::GameRules::default());
        engine.set_scope("full").unwrap();
        let mut max_ct_streak = 0usize;
        let mut streak = 0usize;
        let mut ticks = 0usize;
        while !engine.is_finished() && ticks < 250_000 {
            engine.step();
            ticks += 1;
            if matches!(
                engine.ball_state(),
                nba_physics::BallTrajectoryKind::ControlTransfer { .. }
            ) {
                streak += 1;
                max_ct_streak = max_ct_streak.max(streak);
            } else {
                streak = 0;
            }
        }
        assert!(engine.is_finished(), "seed {seed} did not reach GameEnd");
        // 交接是一次短飞行（约 0.5s = 13 tick）；超过数秒即为悬置。
        assert!(
            max_ct_streak < 500,
            "seed {seed}: ControlTransfer hung for {max_ct_streak} consecutive ticks"
        );
        assert!(
            engine.completed_possessions() > 150,
            "seed {seed}: only {} possessions completed (expected a full game)",
            engine.completed_possessions()
        );
    }
}

/// D5.1：投篮弧线峰值必须服从规则高度上限（`BALL_HEIGHT_BOUNDS`）。
///
/// 根因：`arc = A·p·(1-p)` 的真实极值点在 p>0.5，线性缩放的 A 使实际峰值
/// 高于请求值（请求 35.0 ft → 采样 35.08 ft）。
#[test]
fn test_shot_arc_respects_height_ceiling() {
    let rules = nba_domain::GameRules::default();
    let ceiling = rules.ball_z_max_ft;
    for seed in [6u64, 21, 42] {
        let mut engine = MatchEngine::with_rules(seed, rules.clone());
        engine.set_scope("full").unwrap();
        let mut worst = 0.0f32;
        let mut ticks = 0usize;
        while !engine.is_finished() && ticks < 250_000 {
            let tick = engine.step();
            ticks += 1;
            worst = worst.max(tick.frame.ball.z);
        }
        assert!(
            worst <= ceiling,
            "seed {seed}: ball reached {worst:.4} ft, above the {ceiling} ft ceiling"
        );
    }
}

/// F1.2 红测试：进攻方越过中线进入前场后，后场计时必须重置。
///
/// 根因：`backcourt_elapsed` 仅在进入 `Initiation` 阶段清零，跨场后继续累加，
/// 8 秒后无条件判 `EIGHT_SECOND_BACKCOURT`（seed 0 单节 26 次违例主因）。
#[test]
fn test_backcourt_clock_resets_on_halfcourt_cross() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    rules.tick_seconds = 0.1;
    rules.backcourt_seconds = 3.0;
    let mut engine = MatchEngine::with_rules(701, rules);
    engine.force_possession_for_test(Possession::Home);
    engine.set_game_flow(nba_domain::GameFlowState::LiveBall);
    engine.set_ball_state_for_test(nba_physics::BallTrajectoryKind::Held {
        carrier_id: "H_01".to_string(),
    });
    // 把球与持球人放到前场（Home 攻击右侧，x >= 47 为前场）。
    let frontcourt = Vec2::new(70.0, 25.0);
    engine.set_ball_pos_for_test(frontcourt, 4.0);
    if let Some(p) = engine.physics_mut_for_test().get_player_mut("H_01") {
        p.pos_ft = frontcourt;
        p.target_pos_ft = frontcourt;
    }
    // 人为把后场计时推过阈值，若引擎不按位置重置就会立刻误判违例。
    engine.set_backcourt_elapsed_for_test(99.0);
    let tick = engine.step();
    assert!(
        !tick.frame.events.iter().any(|e| e == "RULE_VIOLATION"),
        "frontcourt possession must not trigger an 8-second violation, events={:?}",
        tick.frame.events
    );
    assert!(
        engine.backcourt_elapsed() < 1.0,
        "backcourt clock must reset after crossing halfcourt, got {}",
        engine.backcourt_elapsed()
    );
}

/// F1.2 负面对照：若球队仍滞留后场，计时必须继续累加并最终判违例。
#[test]
fn test_eight_second_violation_requires_continuous_backcourt() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    rules.tick_seconds = 0.1;
    rules.backcourt_seconds = 0.5;
    rules.decision_interval_seconds = 100.0;
    let mut engine = MatchEngine::with_rules(702, rules);
    engine.force_possession_for_test(Possession::Home);
    engine.set_game_flow(nba_domain::GameFlowState::LiveBall);
    engine.set_ball_state_for_test(nba_physics::BallTrajectoryKind::Held {
        carrier_id: "H_01".to_string(),
    });
    let backcourt = Vec2::new(20.0, 25.0);
    engine.set_ball_pos_for_test(backcourt, 4.0);
    if let Some(p) = engine.physics_mut_for_test().get_player_mut("H_01") {
        p.pos_ft = backcourt;
        p.target_pos_ft = backcourt;
        p.target_speed_ftps = 0.0;
        p.vel_ft = Vec2::ZERO;
    }
    let mut saw_violation = false;
    for _ in 0..40 {
        let tick = engine.step();
        if tick
            .frame
            .events
            .iter()
            .any(|e| e == "VIOLATION" || e == "RULE_VIOLATION")
        {
            saw_violation = true;
            break;
        }
        // 保持球员停留在后场，排除运动学把它们带过中线。
        if let Some(p) = engine.physics_mut_for_test().get_player_mut("H_01") {
            p.pos_ft = backcourt;
            p.target_pos_ft = backcourt;
            p.vel_ft = Vec2::ZERO;
        }
        engine.set_ball_pos_for_test(backcourt, 4.0);
    }
    assert!(
        saw_violation,
        "continuous backcourt possession must eventually trigger the 8-second violation"
    );
}
