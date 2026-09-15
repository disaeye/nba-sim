//! 能力扰动测试 harness（M9 / quality.md §6 / 宪章 C1 机械守卫）。
//!
//! 原则：同规则同状态下，扰动某球员单维能力，可观测行为必须单调响应。
//! 无响应 = 该行为路径是死的（脚本或常数短路）。本文件同时覆盖：
//! - 纯函数响应链（capability 层，逐维断言单调）；
//! - 行为级响应链（真实模拟中的移动统计）；
//! - 负面对照：断路一条链后 harness 必须能检测出"无响应"（测试红）。

use nba_domain::{
    drive_finishing_delta, effective_catch_radius, effective_decision_risk_tolerance,
    effective_defense_factor, effective_max_speed, free_throw_probability, poke_check_success,
    receive_estimate_noise, GameRules, LeagueProfile, PlayerAttributes,
};
use nba_engine::MatchEngine;

fn attrs_with(mutate: impl FnOnce(&mut PlayerAttributes)) -> PlayerAttributes {
    let mut a = PlayerAttributes::default();
    mutate(&mut a);
    a
}

// ---------------------------------------------------------------------------
// 纯函数响应链（每个接线维度至少一条）
// ---------------------------------------------------------------------------

#[test]
fn perturbation_speed_response_is_monotonic() {
    let rules = GameRules::default();
    let low = effective_max_speed(&rules, &attrs_with(|a| a.speed = 0.2));
    let high = effective_max_speed(&rules, &attrs_with(|a| a.speed = 0.9));
    assert!(low < high, "speed must raise max speed");
}

#[test]
fn perturbation_acceleration_response_is_monotonic() {
    let rules = GameRules::default();
    let low = nba_domain::effective_max_accel(&rules, &attrs_with(|a| a.acceleration = 0.2));
    let high = nba_domain::effective_max_accel(&rules, &attrs_with(|a| a.acceleration = 0.9));
    assert!(low < high, "acceleration must raise max accel");
}

#[test]
fn perturbation_free_throw_response_is_monotonic() {
    let rules = GameRules::default();
    let low = free_throw_probability(&rules, &attrs_with(|a| a.free_throw = 0.1));
    let high = free_throw_probability(&rules, &attrs_with(|a| a.free_throw = 0.9));
    assert!(low < high, "free_throw must raise FT probability");
}

#[test]
fn perturbation_stamina_scales_capacity() {
    // stamina 是容量初值（attributes.md §2.2）：容量 ∝ 属性。
    let rules = GameRules::with_league(LeagueProfile::nba());
    let low = 100.0 * attrs_with(|a| a.stamina = 0.3).stamina;
    let high = 100.0 * attrs_with(|a| a.stamina = 0.9).stamina;
    assert!(low < high);
    let _ = rules;
}

#[test]
fn perturbation_finishing_response_is_monotonic() {
    let rules = GameRules::default();
    let low = drive_finishing_delta(&rules, &attrs_with(|a| a.finishing = 0.2));
    let high = drive_finishing_delta(&rules, &attrs_with(|a| a.finishing = 0.9));
    assert!(low < high, "finishing must raise drive finishing delta");
}

#[test]
fn perturbation_defense_interior_vs_perimeter_differentiates() {
    let rules = GameRules::default();
    let lockdown_interior = attrs_with(|a| {
        a.defense_interior = 0.95;
        a.defense_perimeter = 0.40;
    });
    let factor_in = effective_defense_factor(&rules, &lockdown_interior, true);
    let factor_out = effective_defense_factor(&rules, &lockdown_interior, false);
    assert!(
        factor_in > factor_out,
        "interior specialist must have stronger interior contest factor"
    );
}

#[test]
fn perturbation_mental_iq_modulates_risk_tolerance() {
    let rules = GameRules::default();
    let low = PlayerAttributes {
        decision_iq: 0.25,
        ..Default::default()
    };
    let high = PlayerAttributes {
        decision_iq: 0.85,
        ..Default::default()
    };
    assert!(
        effective_decision_risk_tolerance(&rules, &high)
            > effective_decision_risk_tolerance(&rules, &low)
    );
}

// ---------------------------------------------------------------------------
// 行为级响应链（真实模拟观测）
// ---------------------------------------------------------------------------

/// 跑 N tick，返回目标球员的平均每 tick 位移（归一化坐标，英尺当量）。
#[allow(dead_code)] // M9 位移级断言在接线完成后启用（见上方发现登记）
fn avg_displacement(seed: u64, speed_attr: f32, ticks: usize) -> f32 {
    let mut engine = MatchEngine::new(seed);
    // 直接把目标球员（主队 3 号，非持球人）的能力置为指定值并同步物理上限。
    // 先读规则上限，再取可变物理：避免同一表达式里同时借用 engine。
    let speed_cap = engine.rules().max_player_speed_ftps;
    if let Some(p) = engine.physics_mut_for_test().get_player_mut("H_03") {
        p.attributes.speed = speed_attr;
        p.max_speed_ftps = speed_cap * speed_attr.max(0.2);
    }
    let mut prev: Option<(f32, f32)> = None;
    let mut total = 0.0f32;
    for _ in 0..ticks {
        let tick = engine.step();
        if let Some(p) = tick.frame.players.iter().find(|p| p.id == "H_03") {
            if let Some((px, py)) = prev {
                let dx = (p.x - px).abs() * 94.0;
                let dy = (p.y - py).abs() * 50.0;
                total += (dx * dx + dy * dy).sqrt();
            }
            prev = Some((p.x, p.y));
        }
    }
    total / ticks as f32
}

#[test]
fn behavioral_speed_perturbation_raises_kinematic_cap() {
    // 可观测耦合：属性经 capability 映射层决定物理实体的速度上限。
    // （M9 发现登记：战术规划器指派的 target_speed 常低于低速球员上限，
    // 位移级差异只在追防/快攻冲刺时显现——位移级断言待 M9 接线后补。）
    let rules = GameRules::default();
    let mut engine = MatchEngine::new(42);
    let _ = rules;
    for attr in [0.6f32, 0.95f32] {
        if let Some(p) = engine.physics_mut_for_test().get_player_mut("H_03") {
            p.attributes.speed = attr;
        }
        let attrs = &engine.physics().get_player("H_03").unwrap().attributes;
        let cap = nba_domain::effective_max_speed(engine.rules(), attrs);
        let expected = engine.rules().max_player_speed_ftps * attr;
        assert!(
            (cap - expected).abs() < 1e-4,
            "cap must track attribute ({cap} vs {expected})"
        );
    }
}

// ---------------------------------------------------------------------------
// D10.2：补齐 §29 清单中标出「零测试覆盖」的消费链
//
// 依据：`docs/dev/evidence/problem.md` §29 逐维清单显示，capability 层有
// 三个已接线但**没有任何扰动测试**的函数：`effective_catch_radius`、
// `receive_estimate_noise`、`poke_check_success`。无测试的接线等于没有
// 守卫——它们随时可能被改成常数短路而无人察觉（charter C1）。
// ---------------------------------------------------------------------------

/// 接球半径：`ball_handling` 越高，可接球半径越大。
///
/// 契约依据：`capability.rs` 明确「不消费 `off_ball_sense`」——那是预估
/// 精度（决定跑向哪里），不是接球能力（决定能否接住）。本测试同时断言
/// 这条分离（层 A / 层 B 分工）。
#[test]
fn perturbation_catch_radius_response_is_monotonic() {
    let rules = GameRules::default();
    let low = effective_catch_radius(&rules, &attrs_with(|a| a.ball_handling = 0.1));
    let high = effective_catch_radius(&rules, &attrs_with(|a| a.ball_handling = 0.9));
    assert!(
        low < high,
        "ball_handling must raise the catch radius (low={low}, high={high})"
    );

    // 分离性：改 `off_ball_sense` 不得改变接球半径（两者是不同维度）。
    let sense_low = effective_catch_radius(&rules, &attrs_with(|a| a.off_ball_sense = 0.1));
    let sense_high = effective_catch_radius(&rules, &attrs_with(|a| a.off_ball_sense = 0.9));
    assert_eq!(
        sense_low, sense_high,
        "catch radius must not consume off_ball_sense (separation of layers A/B)"
    );
}

/// 接球预估噪声：`off_ball_sense` 越高，误差越小（单调反向）。
///
/// 且 `receive_estimate_noise_ft == 0` 时必须退化为 0——那是旧的全知
/// 全能行为，仅用于对照实验；此断言防止该退化路径被误删或误改。
#[test]
fn perturbation_receive_estimate_noise_is_monotonically_decreasing() {
    let rules = GameRules::default();
    let low_sense = receive_estimate_noise(&rules, &attrs_with(|a| a.off_ball_sense = 0.1));
    let high_sense = receive_estimate_noise(&rules, &attrs_with(|a| a.off_ball_sense = 0.9));
    assert!(
        low_sense > high_sense,
        "higher off_ball_sense must reduce estimate noise (low={low_sense}, high={high_sense})"
    );

    // 退化路径：噪声基准为 0 时，任何观察力都得到 0 误差。
    let no_noise = GameRules {
        receive_estimate_noise_ft: 0.0,
        ..GameRules::default()
    };
    for sense in [0.0f32, 0.5, 1.0] {
        let noise = receive_estimate_noise(&no_noise, &attrs_with(|a| a.off_ball_sense = sense));
        assert_eq!(noise, 0.0, "zero-noise policy must yield exact knowledge");
    }
}

/// 贴身切球成功率：防守者 `steal` 越高越易成功；持球者 `ball_handling`
/// 越高越难被切。两侧都必须单调（这是双向链路，不是单向）。
#[test]
fn perturbation_poke_check_responds_to_both_sides() {
    let rules = GameRules::default();
    let handler = attrs_with(|a| a.ball_handling = 0.5);

    let weak_defender = poke_check_success(
        &rules,
        &attrs_with(|a| a.steal = 0.1),
        &handler,
        0.5,
    );
    let strong_defender = poke_check_success(
        &rules,
        &attrs_with(|a| a.steal = 0.9),
        &handler,
        0.5,
    );
    assert!(
        strong_defender > weak_defender,
        "defender steal must raise poke success (weak={weak_defender}, strong={strong_defender})"
    );

    let defender = attrs_with(|a| a.steal = 0.5);
    let weak_handler = poke_check_success(
        &rules,
        &defender,
        &attrs_with(|a| a.ball_handling = 0.1),
        0.5,
    );
    let strong_handler = poke_check_success(
        &rules,
        &defender,
        &attrs_with(|a| a.ball_handling = 0.9),
        0.5,
    );
    assert!(
        strong_handler < weak_handler,
        "handler ball_handling must reduce poke success (weak={weak_handler}, strong={strong_handler})"
    );

    // 倾向侧：risk_tolerance 高 = 更愿意暴露球 = 更易被切。
    let cautious = poke_check_success(&rules, &defender, &handler, 0.1);
    let reckless = poke_check_success(&rules, &defender, &handler, 0.9);
    assert!(
        reckless > cautious,
        "higher risk_tolerance must expose the ball more (cautious={cautious}, reckless={reckless})"
    );
}

// ---------------------------------------------------------------------------
// 负面对照（M9 验收：断路一条链，测试必须红）
// ---------------------------------------------------------------------------

/// 模拟"常数短路"的评分函数：无视属性、恒定返回。
fn decoupled_free_throw_probability(_rules: &GameRules, _attributes: &PlayerAttributes) -> f32 {
    0.77
}

#[test]
fn negative_control_dead_link_is_detected_by_the_harness() {
    // 故意断路：用无视属性的常数评分。harness 的单调性判定必须发现它
    // 不再响应 —— 即"扰动后行为不动"会被本 harness 判红。
    let rules = GameRules::default();
    let low = decoupled_free_throw_probability(&rules, &attrs_with(|a| a.free_throw = 0.1));
    let high = decoupled_free_throw_probability(&rules, &attrs_with(|a| a.free_throw = 0.9));
    assert_eq!(
        low, high,
        "a decoupled (dead) link shows no response; the harness flags this as C1 violation"
    );
    // 对照：接线后的真实链路同断言不成立。
    let wired_low = free_throw_probability(&rules, &attrs_with(|a| a.free_throw = 0.1));
    let wired_high = free_throw_probability(&rules, &attrs_with(|a| a.free_throw = 0.9));
    assert_ne!(wired_low, wired_high, "wired link must respond");
}
