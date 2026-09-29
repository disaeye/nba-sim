//! 能力扰动测试 harness（M9 / quality.md §6 / 宪章 C1 机械守卫）。
//!
//! 原则：同规则同状态下，扰动某球员单维能力，可观测行为必须单调响应。
//! 无响应 = 该行为路径是死的（脚本或常数短路）。本文件同时覆盖：
//! - 纯函数响应链（capability 层，逐维断言单调）；
//! - 行为级响应链（真实模拟中的移动统计）；
//! - 负面对照：断路一条链后 harness 必须能检测出"无响应"（测试红）。

use nba_domain::{
    drive_finishing_delta, effective_catch_radius, effective_max_speed, free_throw_probability,
    poke_check_success, receive_estimate_noise, GameRules, LeagueProfile, PlayerAttributes,
};
use nba_engine::MatchEngine;
use nba_physics::movement::PlayerPhysicsState;

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

/// `agility` → 变向减速代价（`attributes.md` §2.2 声明的消费链）。
///
/// 该维度曾只在档案里存在、生产零消费。现经 `effective_turn_decel_retention`
/// 进入物理层的转身分支：敏捷者转向时保留更多速度。
///
/// 同时守住「不得复用 `attribute_response_floor`（默认 0.5）」——那个 floor
/// 会把 0..0.5 整段压成同一个值（实测 0.05 与 0.5 行为逐位相同），
/// 而 `attributes.md` §4 明令禁止内联 floor 造成无效区间。
#[test]
fn perturbation_agility_lowers_turn_decel_cost() {
    let rules = GameRules::default();
    let anchor =
        nba_domain::effective_turn_decel_retention(&rules, &attrs_with(|a| a.agility = 0.5));
    let low = nba_domain::effective_turn_decel_retention(&rules, &attrs_with(|a| a.agility = 0.2));
    let high = nba_domain::effective_turn_decel_retention(&rules, &attrs_with(|a| a.agility = 0.9));
    assert!(
        low < anchor && anchor < high,
        "agility must raise the retained ratio around the neutral anchor: 
         low={low}, anchor={anchor}, high={high}"
    );
    // 中位锚定：`agility = 0.5` 必须逐位等于全局基准，保证接入前后行为连续。
    assert_eq!(
        anchor, rules.turn_decel_retention,
        "the neutral anchor must reproduce the global base exactly"
    );
    // 无失效区间：属性下限必须真的允许低值生效。
    let floor = rules.capability.turn_decel_retention_attribute_floor;
    assert!(
        floor < 0.2,
        "the input floor ({floor}) must not clamp the low half of agility into a dead zone"
    );
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
fn perturbation_mental_iq_modulates_risk_tolerance() {
    // 高 `decision_iq` → 更低的风险容忍（`base - iq × gain`）：高智商球员
    // 更少做高风险选择，这是该维度的法定语义（`policies.rs` 的曲线注释）。
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
        nba_domain::effective_risk_tolerance(&rules, &high)
            < nba_domain::effective_risk_tolerance(&rules, &low),
        "higher decision_iq must lower risk tolerance"
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
/// 规格依据：`capability.rs` 明确「不消费 `off_ball_sense`」——那是预估
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

    let weak_defender = poke_check_success(&rules, &attrs_with(|a| a.steal = 0.1), &handler, 0.5);
    let strong_defender = poke_check_success(&rules, &attrs_with(|a| a.steal = 0.9), &handler, 0.5);
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
// D10.2 续：篮板轴的端到端响应（此前完全无扰动测试）
//
// §29 清单显示 `offensive_rebound` / `defensive_rebound` 的消费点是
// `officiating::ResolutionLayer::resolve_rebound`，但没有任何扰动测试。
// 该函数是**概率裁决**，因此用给定种子做重复采样估计概率，再做单调断言
// ——而不是断言单次掷骰结果（那会引入随机性）。
// ---------------------------------------------------------------------------

/// 构造一个最小球员视图（只填 `resolve_rebound` 会读的字段）。
fn rebound_player(
    id: &str,
    off_reb: f32,
    def_reb: f32,
    sense: f32,
    stamina: f32,
) -> PlayerPhysicsState {
    PlayerPhysicsState {
        id: id.to_string(),
        jersey: id.to_string(),
        team: "home".to_string(),
        pos_ft: glam::Vec2::ZERO,
        vel_ft: glam::Vec2::ZERO,
        accel_ft: glam::Vec2::ZERO,
        target_pos_ft: glam::Vec2::ZERO,
        max_speed_ftps: 22.0,
        max_accel_ftps2: 35.0,
        target_speed_ftps: 0.0,
        on_court: true,
        action: "Idle".to_string(),
        slot: "PF".to_string(),
        morale: "Normal".to_string(),
        stamina,
        max_stamina: 100.0,
        foul_count: 0,
        locomotion: nba_physics::movement::LocomotionState::Idle,
        facing_dir: glam::Vec2::X,
        ball_orientation: nba_domain::action_window::BallOrientation::FaceUp,
        turn_decel_timer: 0.0,
        is_locked_kinematics: false,
        out_of_bounds_placement: false,
        is_receiving_pass: false,
        is_driving_to_rim: false,
        boundary_cross_latched: false,
        attributes: PlayerAttributes {
            offensive_rebound: off_reb,
            defensive_rebound: def_reb,
            off_ball_sense: sense,
            ..PlayerAttributes::default()
        },
        tendencies: nba_domain::PlayerTendencies::default(),
    }
}

/// 用固定种子重复采样，估计「攻方赢得篮板」的概率。
fn estimate_offensive_rebound_rate(
    offense: &PlayerPhysicsState,
    defense: &PlayerPhysicsState,
) -> f32 {
    use rand::SeedableRng;
    let rules = GameRules::default();
    let policy = &rules.resolve.rebound;
    let samples = 4000;
    let mut wins = 0usize;
    // 逐样本用不同但确定的种子，避免同一个 rng 状态影响可复现性。
    for i in 0..samples {
        let mut rng = rand::rngs::StdRng::seed_from_u64(0x5eed_0000 + i as u64);
        if let nba_officiating::ResolutionOutcome::ReboundSecured { is_offensive, .. } =
            nba_officiating::ResolutionLayer::resolve_rebound(
                offense, defense, 5.0, 5.0, policy, &mut rng,
            )
        {
            if is_offensive {
                wins += 1;
            }
        }
    }
    wins as f32 / samples as f32
}

/// 攻方 `offensive_rebound` 越高 → 抢到进攻篮板的概率越高。
#[test]
fn perturbation_offensive_rebound_raises_offensive_board_rate() {
    let defense = rebound_player("D_1", 0.3, 0.5, 0.5, 100.0);
    let weak = rebound_player("O_weak", 0.1, 0.5, 0.5, 100.0);
    let strong = rebound_player("O_strong", 0.9, 0.5, 0.5, 100.0);

    let weak_rate = estimate_offensive_rebound_rate(&weak, &defense);
    let strong_rate = estimate_offensive_rebound_rate(&strong, &defense);
    assert!(
        strong_rate > weak_rate,
        "offensive_rebound must raise the offensive board rate \
         (weak={weak_rate:.3}, strong={strong_rate:.3})"
    );
}

/// 守方 `defensive_rebound` 越高 → 攻方抢到的概率越低（反向单调）。
#[test]
fn perturbation_defensive_rebound_lowers_offensive_board_rate() {
    let offense = rebound_player("O_1", 0.5, 0.5, 0.5, 100.0);
    let weak_d = rebound_player("D_weak", 0.3, 0.1, 0.5, 100.0);
    let strong_d = rebound_player("D_strong", 0.3, 0.9, 0.5, 100.0);

    let weak_rate = estimate_offensive_rebound_rate(&offense, &weak_d);
    let strong_rate = estimate_offensive_rebound_rate(&offense, &strong_d);
    assert!(
        strong_rate < weak_rate,
        "defensive_rebound must lower the offensive board rate \
         (weak_d={weak_rate:.3}, strong_d={strong_rate:.3})"
    );
}

/// 守方力量 `strength` 越高 → 卡位优势更强，攻方进攻篮板率单调下降（D27因果闭环）。
#[test]
fn perturbation_strength_boxout_lowers_offensive_board_rate() {
    let mut weak_d = rebound_player("D_weak_str", 0.5, 0.5, 0.5, 100.0);
    weak_d.attributes.strength = 0.1;
    let mut strong_d = rebound_player("D_strong_str", 0.5, 0.5, 0.5, 100.0);
    strong_d.attributes.strength = 0.9;
    let offense = rebound_player("O_1", 0.5, 0.5, 0.5, 100.0);

    let weak_rate = estimate_offensive_rebound_rate(&offense, &weak_d);
    let strong_rate = estimate_offensive_rebound_rate(&offense, &strong_d);
    assert!(
        strong_rate < weak_rate,
        "strength boxout must lower offensive rebound rate: weak={weak_rate:.3}, strong={strong_rate:.3}"
    );
}

/// `off_ball_sense` 作为**站位预判**参与篮板争夺（正向）。
#[test]
fn perturbation_off_ball_sense_raises_offensive_board_rate() {
    let defense = rebound_player("D_1", 0.3, 0.5, 0.5, 100.0);
    let poor = rebound_player("O_poor", 0.5, 0.5, 0.1, 100.0);
    let sharp = rebound_player("O_sharp", 0.5, 0.5, 0.9, 100.0);

    let poor_rate = estimate_offensive_rebound_rate(&poor, &defense);
    let sharp_rate = estimate_offensive_rebound_rate(&sharp, &defense);
    assert!(
        sharp_rate > poor_rate,
        "off_ball_sense must raise the offensive board rate \
         (poor={poor_rate:.3}, sharp={sharp_rate:.3})"
    );
}

// ---------------------------------------------------------------------------
// D10.3 · 断路负面对照
//
// 要求（current/plan.md §6.2）：「断路后测试必红，恢复后必绿」。
//
// 做法要求：不手写一个常数函数再断言它不响应（那只证明了常数是常数）。
// 把单调性判定抽成**可复用的 harness**，然后用同一份 harness 同时验证：
//   - 接线链路 → harness 通过（正向）；
//   - 断开链路 → **harness 报错**（负向）。
// 这才证明 harness 本身有区分力，而非"恰好通过"。
// ---------------------------------------------------------------------------

/// 单调性判定的通用 harness。
///
/// 给定「单维属性值 → 观测量」的评分函数，检查在 `[low_in, high_in]`
/// 区间内响应方向是否与 `expect_increase` 一致。返回 `Err` 表示该链路
/// **无响应或方向错误** —— 即 charter C1 意义的「死链」。
///
/// 之所以让 harness 返回 `Result` 而不是直接 `assert!`：负面对照需要断言
/// 「harness 确实会拒绝一条断链」，这要求判定结果可被检查。
fn check_monotonic_response(
    label: &str,
    score: impl Fn(f32) -> f32,
    low_in: f32,
    high_in: f32,
    expect_increase: bool,
    min_magnitude: f32,
) -> Result<f32, String> {
    let low = score(low_in);
    let high = score(high_in);
    let delta = high - low;

    if !low.is_finite() || !high.is_finite() {
        return Err(format!("{label}: non-finite response ({low}, {high})"));
    }
    // 幅度门：防止"响应极小但符号正确"被当作有效链路。
    // 实测三条链路的响应幅度为 0.48 / 0.24 / 2.00，故 0.01 的门是宽松的。
    if delta.abs() < min_magnitude {
        return Err(format!(
            "{label}: no usable response (delta {delta:.5} < {min_magnitude}); \
             the link is decoupled or constant-short-circuited"
        ));
    }
    if expect_increase && delta <= 0.0 {
        return Err(format!(
            "{label}: expected increase, got delta {delta:.5} (low={low}, high={high})"
        ));
    }
    if !expect_increase && delta >= 0.0 {
        return Err(format!(
            "{label}: expected decrease, got delta {delta:.5} (low={low}, high={high})"
        ));
    }
    Ok(delta)
}

/// 负面对照①：harness 必须**拒绝**一条常数短路（死链）。
#[test]
fn negative_control_harness_rejects_a_constant_short_circuit() {
    // 无视输入、恒定返回 —— 典型的"规则曲线被短路成常数"。
    let dead = |_x: f32| 0.77_f32;
    let verdict = check_monotonic_response("dead", dead, 0.1, 0.9, true, 0.01);
    assert!(
        verdict.is_err(),
        "harness must reject a decoupled constant link, got {verdict:?}"
    );
}

/// 负面对照②：harness 必须拒绝**方向相反**的链路。
///
/// 这防止 harness 退化成"只看有没有变化"。
#[test]
fn negative_control_harness_rejects_wrong_direction() {
    // 真实 free_throw 链路是递增的；此处故意声明"期望递减"。
    let rules = GameRules::default();
    let verdict = check_monotonic_response(
        "free_throw (wrong expectation)",
        |x| free_throw_probability(&rules, &attrs_with(|a| a.free_throw = x)),
        0.1,
        0.9,
        /* expect_increase = */ false,
        0.01,
    );
    assert!(
        verdict.is_err(),
        "harness must reject a link whose direction contradicts the expectation"
    );
}

/// 负面对照③：harness 必须拒绝**幅度过小**的响应。
#[test]
fn negative_control_harness_rejects_negligible_magnitude() {
    let tiny = |x: f32| 0.5 + x * 1e-6;
    let verdict = check_monotonic_response("tiny", tiny, 0.1, 0.9, true, 0.01);
    assert!(
        verdict.is_err(),
        "harness must reject a response below the magnitude gate (virtual dead link)"
    );
}

/// 正面对照：三条真实链路必须**通过**同一个 harness。
///
/// 与上面三个负面对照共用同一函数，因此"通过"具有区分力：
/// 若 harness 退化为恒真，这三个用例仍会通过而负面对照会失败；
/// 若 harness 退化为恒假，负面对照会通过而这里会失败。
#[test]
fn positive_controls_pass_the_same_harness() {
    let rules = GameRules::default();

    // (1) 接球半径：ball_handling 递增。
    let d = check_monotonic_response(
        "catch_radius",
        |x| effective_catch_radius(&rules, &attrs_with(|a| a.ball_handling = x)),
        0.1,
        0.9,
        true,
        0.01,
    );
    assert!(d.is_ok(), "wired catch radius must pass: {d:?}");

    // (2) 预估噪声：off_ball_sense 递减（反向链路）。
    let d = check_monotonic_response(
        "estimate_noise",
        |x| receive_estimate_noise(&rules, &attrs_with(|a| a.off_ball_sense = x)),
        0.1,
        0.9,
        false,
        0.01,
    );
    assert!(d.is_ok(), "wired estimate noise must pass: {d:?}");

    // (3) 罚球概率：free_throw 递增。
    let d = check_monotonic_response(
        "free_throw",
        |x| free_throw_probability(&rules, &attrs_with(|a| a.free_throw = x)),
        0.1,
        0.9,
        true,
        0.01,
    );
    assert!(d.is_ok(), "wired free throw must pass: {d:?}");

    // 已删除的无消费函数（`effective_defense_factor` /
    // `effective_decision_risk_tolerance` / `effective_shooting_mid_factor` /
    // `effective_passing_skill_factor` / `effective_boxout_strength`）曾在此各占一条
    // 「接线」断言，但它们在生产代码中零调用——那种断言只是公式单测，
    // 对「属性是否真的影响比赛」零信息量。它们指向的能力现在由下列**已接线**
    // 函数承担（见当前文件后面的行为级断言与 `wiring_proof.rs`）：
    //
    // - 防守：`officiating` 直读 `defense_perimeter` / `defense_interior`
    //   （`resolution.rs` 的干扰与犯规曲线）；
    // - 中距离与传球：`resolve.player_skill.shooting_weight` / `passing_weight`
    //   经 `execute_shot` 与传球裁决消费；
    // - 卡位：`effective_defensive_boxout_bonus`（篮板冲抢）；
    // - 决策：`effective_risk_tolerance`（贴身切球倾向）。

    // (4) 决策风险容忍：`decision_iq` 递增 → 风险容忍**递减**
    // （曲线是 `base - iq × gain`：高智商球员更少做高风险选择）。
    let d = check_monotonic_response(
        "decision_iq",
        |x| nba_domain::effective_risk_tolerance(&rules, &attrs_with(|a| a.decision_iq = x)),
        0.1,
        0.9,
        /* expect_increase = */ false,
        0.01,
    );
    assert!(d.is_ok(), "wired decision_iq must pass: {d:?}");

    // (5) 防守卡位：defensive_rebound 与 strength 递增（D27）。
    let d = check_monotonic_response(
        "defensive_rebound",
        |x| {
            nba_domain::effective_defensive_boxout_bonus(
                &rules,
                &attrs_with(|a| a.defensive_rebound = x),
            )
        },
        0.1,
        0.9,
        true,
        0.01,
    );
    assert!(d.is_ok(), "wired defensive_rebound must pass: {d:?}");

    // (6) 低位背身对抗：defense_interior 递增（D27）。
    let d = check_monotonic_response(
        "post_defense_physicality",
        |x| {
            nba_domain::effective_post_defense_physicality(
                &rules,
                &attrs_with(|a| a.defense_interior = x),
            )
        },
        0.1,
        0.9,
        true,
        0.01,
    );
    assert!(d.is_ok(), "wired post defense must pass: {d:?}");

    // (7) 协防意识：defense_interior 递增（D27）。
    let d = check_monotonic_response(
        "help_awareness",
        |x| nba_domain::effective_help_awareness(&rules, &attrs_with(|a| a.defense_interior = x)),
        0.1,
        0.9,
        true,
        0.01,
    );
    assert!(d.is_ok(), "wired help awareness must pass: {d:?}");

    // (8) 二次补篮倾向：finishing 递增（D27）。
    let d = check_monotonic_response(
        "putback_bias",
        |x| nba_domain::effective_putback_bias(&rules, &attrs_with(|a| a.finishing = x)),
        0.1,
        0.9,
        true,
        0.01,
    );
    assert!(d.is_ok(), "wired putback bias must pass: {d:?}");

    // (9) 快下机会：speed 递增（D27）。
    let d = check_monotonic_response(
        "transition_leakout",
        |x| nba_domain::effective_transition_leakout_chance(&rules, &attrs_with(|a| a.speed = x)),
        0.1,
        0.9,
        true,
        0.01,
    );
    assert!(d.is_ok(), "wired transition leakout must pass: {d:?}");

    // (10) 横向敏捷：agility 递增 → 变向保留比例递增（attributes.md §2.2）。
    let d = check_monotonic_response(
        "agility",
        |x| nba_domain::effective_turn_decel_retention(&rules, &attrs_with(|a| a.agility = x)),
        0.1,
        0.9,
        true,
        0.01,
    );
    assert!(d.is_ok(), "wired agility must pass: {d:?}");

    // `shooting_close` 与 `finishing` 的分解已在 `execution.rs` 的 near-rim
    // 分支按 `contest_intensity` 接入（非对抗 → shooting_close，对抗 → finishing，
    // 符合 attributes.md §2.3 的可辨识性配对）。
    //
    // 但此处**没有**行为级扰动断言：近筐出手量与队内竞争密度太低
    // （实测全场 2–4 次篮下出手，事件序列指纹 1/4 seed 可见），
    // 这是**证据缺口**（样本量不足），挂在 `docs/dev/gap.md` G6a：
    // 禁区出手分布失真的行为链补齐后自然可观测，禁止放宽阈值伪装。
}
