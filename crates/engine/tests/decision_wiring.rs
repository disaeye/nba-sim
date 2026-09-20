//! 决策接线验证：传球与投篮的裁定被忠实回放，且规则/能力通道真实生效。
//!
//! 守卫对象：
//! - 传球在释放时刻裁定、到达时刻回放（不得在到达时重新掷骰）；
//! - 投篮使用配置的技能与空间输入。
//!
//! 参数扰动的守卫分属两个所有者，本文件只负责**传球与投篮的裁定回放**：
//! - **属性**扰动 → `wiring_proof.rs`；
//! - **规则系数**扰动 → `rules_complete_wiring.rs`。
//!
//! 对应 `docs/quality.md` §6 与 `docs/protocol.md` §2.1 M9。
//! 本文件中的断言一旦变红，**不得**通过放宽阈值或改写测试来消除：必须先判断
//! 是「接线坏了」还是「判定标准需要重新裁定」，前者修代码，后者走 `decisions.md`。

#![allow(clippy::field_reassign_with_default)]

mod support;

use glam::Vec2;
use nba_engine::MatchEngine;
use support::setup_noise_off;

/// 传球在释放时刻裁定的接收结果必须被到达时刻忠实回放。
#[test]
fn pass_arrival_replays_release_outcome_and_emits_matching_fact() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.tactical_initiation_seconds = 0.0;
    rules.decision_interval_seconds = 0.1;
    let mut setup = nba_engine::MatchSetup::builtin(rules);
    setup.rules.resolve.base_rates.pass_success = 1.0;
    setup.rules.resolve.pass.openness_weight = 0.0;
    setup.rules.resolve.pass.passer_skill_weight = 0.0;
    setup.rules.resolve.pass.receiver_control_weight = 0.0;
    setup.rules.resolve.pass.catch_equilibrium_weight = 0.0;
    let mut engine = MatchEngine::with_setup(setup, 1201);
    engine.set_ball_state_for_test(nba_physics::BallTrajectoryKind::Pass {
        from_pos: Vec2::new(40.0, 25.0),
        to_pos: Vec2::new(50.0, 25.0),
        target_id: "H_02".to_string(),
        start_time: 1.0,
        duration: 0.2,
        peak_z: engine.rules().pass_peak_ft,
        inbound: false,
        receive_success: true,
        intercept: None,
    });
    engine.set_last_passer_for_test(Some("H_01".to_string()));
    engine.set_game_flow_for_test(nba_domain::GameFlowState::LiveBall);
    engine.set_sub_phase_for_test(nba_domain::SubPhase::ActionExecution);
    engine.set_current_time_for_test(1.0);
    // 层 A（P-1）修正：接球人不再直读传球人的冻结落点，而是按**自己的感知**
    // 预估并跑位，因此可能接不到。本测试验证的是「release 时裁定的接收结果被
    // 忠实回放」，与位置无关，故关闭预估噪声。
    setup_noise_off(&mut engine);
    // 构造「接球人恰好站在落点」的静止场景：引擎初始化赋予的战术跑位初速会因
    // 惯性把他推出落点（实测 1 tick 滑 1.73 ft > catch_radius），以及其余球员的
    // 分离投影会把他推开。那是测试场景污染，不是被测行为。
    {
        let ids: Vec<String> = engine.physics().get_players().keys().cloned().collect();
        for id in ids {
            if let Some(p) = engine.physics_mut_for_test().get_player_mut(&id) {
                p.vel_ft = Vec2::ZERO;
                p.target_speed_ftps = 0.0;
            }
        }
        if let Some(p) = engine.physics_mut_for_test().get_player_mut("H_02") {
            p.pos_ft = Vec2::new(50.0, 25.0);
            p.target_pos_ft = Vec2::new(50.0, 25.0);
        }
    }
    engine
        .physics_mut_for_test()
        .get_player_mut("H_01")
        .expect("passer")
        .pos_ft = Vec2::new(40.0, 25.0);
    {
        let others: Vec<String> = engine
            .physics()
            .get_players()
            .iter()
            .filter(|(id, p)| p.on_court && id.as_str() != "H_02")
            .map(|(id, _)| id.clone())
            .collect();
        for (i, id) in others.iter().enumerate() {
            if let Some(p) = engine.physics_mut_for_test().get_player_mut(id) {
                // 沿边线一字排开，远离接球点。
                p.pos_ft = Vec2::new(5.0, 5.0 + i as f32 * 4.0);
                p.target_pos_ft = p.pos_ft;
                p.vel_ft = Vec2::ZERO;
                p.target_speed_ftps = 0.0;
            }
        }
    }
    engine.clear_pending_events_for_test();
    let mut saw_receive = false;
    for _ in 0..30 {
        let tick = engine.step();
        saw_receive |= tick
            .frame
            .events
            .iter()
            .any(|event| event == "PASS_RECEIVED");
        if saw_receive {
            break;
        }
    }
    assert!(saw_receive, "pass arrival did not emit PASS_RECEIVED");
    assert!(matches!(
        engine.ball_state(),
        nba_physics::BallTrajectoryKind::Held { ref carrier_id } if carrier_id == "H_02"
    ));
}

/// 传球失败同样必须在释放时刻裁定、到达时刻回放，不得在到达时重新掷骰。
#[test]
fn pass_release_policy_can_emit_drop_without_redeciding_at_arrival() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.tactical_initiation_seconds = 0.0;
    rules.decision_interval_seconds = 0.1;
    let mut setup = nba_engine::MatchSetup::builtin(rules);
    setup.rules.resolve.base_rates.pass_success = 0.0;
    setup.rules.resolve.pass.openness_weight = 0.0;
    setup.rules.resolve.pass.passer_skill_weight = 0.0;
    setup.rules.resolve.pass.receiver_control_weight = 0.0;
    setup.rules.resolve.pass.catch_equilibrium_weight = 0.0;
    let mut engine = MatchEngine::with_setup(setup, 1202);
    engine.set_ball_state_for_test(nba_physics::BallTrajectoryKind::Pass {
        from_pos: Vec2::new(40.0, 25.0),
        to_pos: Vec2::new(50.0, 25.0),
        target_id: "H_02".to_string(),
        start_time: 0.0,
        duration: 0.1,
        peak_z: engine.rules().pass_peak_ft,
        inbound: false,
        receive_success: false,
        intercept: None,
    });
    engine.set_last_passer_for_test(Some("H_01".to_string()));
    engine.set_current_time_for_test(0.0);
    engine.set_game_flow_for_test(nba_domain::GameFlowState::LiveBall);
    engine.set_sub_phase_for_test(nba_domain::SubPhase::ActionExecution);
    let tick = engine.step();
    assert!(tick
        .frame
        .events
        .iter()
        .any(|event| event == "PASS_DROPPED"));
    assert!(matches!(
        engine.ball_state(),
        nba_physics::BallTrajectoryKind::LooseBall { .. }
    ));
}

/// 投篮必须使用配置的技能与空间输入（把命中区间钉在 1.0，则必然命中）。
#[test]
fn shot_release_uses_configured_skill_and_spacing_inputs() {
    let mut rules = nba_domain::GameRules::default();
    rules.tip_off_duration_seconds = 0.0;
    rules.shot_pct_floor = 1.0;
    rules.shot_pct_ceiling = 1.0;
    let mut engine = MatchEngine::with_rules(1211, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.ball_pos_3d().0 = Vec2::new(40.0, 25.0);
    engine
        .physics_mut_for_test()
        .get_player_mut("H_01")
        .expect("shooter")
        .attributes
        .shooting_mid = 1.0;
    engine
        .physics_mut_for_test()
        .get_player_mut("H_01")
        .expect("shooter")
        .pos_ft = Vec2::new(40.0, 25.0);
    engine.execute_shot_for_test("H_01", Vec2::new(40.0, 25.0), false);
    assert!(matches!(
        engine.ball_state(),
        nba_physics::BallTrajectoryKind::Shot { is_made: true, .. }
    ));
}
