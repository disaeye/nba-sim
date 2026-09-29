//! 倾向扰动测试（R6 / attributes.md §2.6 / charter C1 机械守卫）。
//!
//! 与 `attribute_perturbation.rs` 同一原则：同规则同状态下，扰动某球员
//! 单维**倾向**（风格，不是能力），可观测行为必须单调响应。无响应 =
//! 该倾向的声明是死的。本文件覆盖 12 项倾向中曾经没有消费点的 8 项：
//!
//! | 倾向 | 消费点 | 观测量 |
//! |------|--------|--------|
//! | `cut_frequency` | 战术槽位切入/下沉深度 | 切入目标点距基础位的距离 |
//! | `screen_frequency` | 掩护顺下深度 | 顺下目标点距基础位的距离 |
//! | `offensive_rebound_frequency` | 前板冲抢排序权重 | 冲抢排序键 |
//! | `gamble_steal` | 贴身切球尝试速率 | 尝试危险率 |
//! | `block_aggressiveness` | 封盖起跳概率 | 封盖概率 |
//! | `help_aggressiveness` | 弱侧协防护筐引力 | 护筐威胁占比 |
//! | `physicality` | 位置对抗犯规概率 | 犯规概率 |
//! | `transition_sprint` | 转换期跑动速度 | 速度系数 |
//!
//! 每条链路都配负面对照：切断消费（把规则通道的调制跨度归零或传入
//! 中性缺省）后响应必须消失，测试变红。

use glam::Vec2;
use nba_decision::potential_field::DefensePotentialFieldSolver;
use nba_decision::tactics::{TacticalPlanner, TacticalSet};
use nba_domain::{GameRules, PlayerTendencies};
use nba_physics::movement::PlayerPhysicsState;

mod support;

fn tendencies_with(mutate: impl FnOnce(&mut PlayerTendencies)) -> PlayerTendencies {
    let mut t = PlayerTendencies::default();
    mutate(&mut t);
    t
}

/// 单调性判定的通用 harness（与 `attribute_perturbation.rs` 同构）。
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

fn physics_player(tendencies: PlayerTendencies) -> PlayerPhysicsState {
    let mut player = support::make_player("probe", "home", 47.0, 25.0);
    player.tendencies = tendencies;
    player
}

// ---------------------------------------------------------------------------
// cut_frequency / screen_frequency：战术槽位切入与顺下深度
// ---------------------------------------------------------------------------

/// 切入槽位（`BackdoorCut`）的切入深度：执行期目标点离发起期基础位的距离。
/// 发起期（`Initiation`）的切入深度为 0，目标点就是槽位基础位。
fn cut_target_distance(cut_frequency: f32) -> f32 {
    let rules = GameRules::default();
    let spec = nba_domain::TacticalSetSpec::builtin(TacticalSet::FiveOutMotion.id())
        .expect("builtin five-out spec");
    let cut_slot = spec
        .slots
        .iter()
        .position(|slot| {
            matches!(
                slot.behaviour,
                nba_domain::SlotBehaviour::BackdoorCut | nba_domain::SlotBehaviour::DipToRim
            )
        })
        .expect("five-out declares a cut slot");
    let slot_tendencies: Vec<PlayerTendencies> = (0..spec.slots.len())
        .map(|i| {
            if i == cut_slot {
                tendencies_with(|t| t.cut_frequency = cut_frequency)
            } else {
                PlayerTendencies::default()
            }
        })
        .collect();
    let plan = |sub_phase: nba_domain::SubPhase, tended: bool| {
        TacticalPlanner::plan_offense_from_spec_with_tendencies(
            &spec,
            sub_phase,
            nba_domain::Possession::Home,
            0,
            rules.tactics.action_duration_seconds,
            rules.tactics.rim_cut_peak_seconds,
            &rules,
            if tended { Some(&slot_tendencies) } else { None },
        )
    };
    let executing = plan(nba_domain::SubPhase::ActionExecution, true)[cut_slot].target_pos;
    let base = plan(nba_domain::SubPhase::Initiation, true)[cut_slot].target_pos;
    (executing - base).length()
}

#[test]
fn cut_frequency_raises_cut_depth() {
    let verdict = check_monotonic_response(
        "cut_frequency → cut depth",
        cut_target_distance,
        0.1,
        0.9,
        true,
        0.5,
    );
    assert!(verdict.is_ok(), "{}", verdict.unwrap_err());
}

/// 断路负面对照：把切入调制的两个通道（阈值与跨度）均中和后，倾向不再
/// 改变切入深度。
#[test]
fn cut_frequency_dead_when_tendency_channels_neutralized() {
    let mut rules = GameRules::default();
    rules.tactics.cut_tendency_span = 0.0;
    rules.tactics.cut_tendency_floor = 1.0;
    rules.tactics.cut_tendency_threshold = 0.0;
    let spec = nba_domain::TacticalSetSpec::builtin(TacticalSet::FiveOutMotion.id())
        .expect("builtin five-out spec");
    let cut_slot = spec
        .slots
        .iter()
        .position(|slot| {
            matches!(
                slot.behaviour,
                nba_domain::SlotBehaviour::BackdoorCut | nba_domain::SlotBehaviour::DipToRim
            )
        })
        .expect("five-out declares a cut slot");
    let depth_at = |cut_frequency: f32| {
        let slot_tendencies: Vec<PlayerTendencies> = (0..spec.slots.len())
            .map(|i| {
                if i == cut_slot {
                    tendencies_with(|t| t.cut_frequency = cut_frequency)
                } else {
                    PlayerTendencies::default()
                }
            })
            .collect();
        let plan = |sub_phase: nba_domain::SubPhase| {
            TacticalPlanner::plan_offense_from_spec_with_tendencies(
                &spec,
                sub_phase,
                nba_domain::Possession::Home,
                0,
                rules.tactics.action_duration_seconds,
                rules.tactics.rim_cut_peak_seconds,
                &rules,
                Some(&slot_tendencies),
            )
        };
        let executing = plan(nba_domain::SubPhase::ActionExecution)[cut_slot].target_pos;
        let base = plan(nba_domain::SubPhase::Initiation)[cut_slot].target_pos;
        (executing - base).length()
    };
    let low = depth_at(0.1);
    let high = depth_at(0.9);
    assert!(
        (high - low).abs() < f32::EPSILON,
        "zeroed tendency span must decouple cut depth: low={low}, high={high}"
    );
}

/// 掩护倾向：顺下深度随 `screen_frequency` 单调上升。
fn roll_target_distance(screen_frequency: f32) -> f32 {
    let rules = GameRules::default();
    let spec = nba_domain::TacticalSetSpec::builtin(TacticalSet::HighPickAndRoll.id())
        .expect("builtin pnr spec");
    let roll_slot = spec
        .slots
        .iter()
        .position(|slot| matches!(slot.behaviour, nba_domain::SlotBehaviour::HighScreenRoll))
        .expect("pnr declares a screener slot");
    let slot_tendencies: Vec<PlayerTendencies> = (0..spec.slots.len())
        .map(|i| {
            if i == roll_slot {
                tendencies_with(|t| t.screen_frequency = screen_frequency)
            } else {
                PlayerTendencies::default()
            }
        })
        .collect();
    let targets = TacticalPlanner::plan_offense_from_spec_with_tendencies(
        &spec,
        nba_domain::SubPhase::ActionExecution,
        nba_domain::Possession::Home,
        0,
        rules.tactics.action_duration_seconds,
        rules.tactics.rim_cut_peak_seconds,
        &rules,
        Some(&slot_tendencies),
    );
    let rolled = targets[roll_slot].target_pos;
    let base = TacticalPlanner::plan_offense_from_spec_with_tendencies(
        &spec,
        nba_domain::SubPhase::Initiation,
        nba_domain::Possession::Home,
        0,
        rules.tactics.action_duration_seconds,
        rules.tactics.rim_cut_peak_seconds,
        &rules,
        Some(&slot_tendencies),
    )[roll_slot]
        .target_pos;
    (rolled - base).length()
}

#[test]
fn screen_frequency_raises_roll_depth() {
    let verdict = check_monotonic_response(
        "screen_frequency → roll depth",
        roll_target_distance,
        0.1,
        0.9,
        true,
        0.5,
    );
    assert!(verdict.is_ok(), "{}", verdict.unwrap_err());
}

fn assert_tendency_reaches_behaviour(label: &str, mutate: impl Fn(&mut PlayerTendencies, f32)) {
    let mut baseline_setup = nba_engine::MatchSetup::builtin(GameRules::default());
    baseline_setup.rules.tactics.cut_tendency_threshold = 0.0;
    baseline_setup.rules.tactics.screen_tendency_threshold = 0.0;
    let mut perturbed_setup = baseline_setup.clone();
    for player in &mut perturbed_setup.home_team.players {
        mutate(&mut player.tendencies, 0.95);
    }
    for player in &mut perturbed_setup.away_team.players {
        mutate(&mut player.tendencies, 0.05);
    }
    let run = |setup: &nba_engine::MatchSetup| {
        support::PROOF_SEEDS
            .iter()
            .map(|seed| {
                let mut engine = nba_engine::MatchEngine::with_setup(setup.clone(), *seed);
                engine.force_possession_for_test(nba_domain::Possession::Home);
                for _ in 0..support::PROOF_TICKS {
                    engine.step();
                }
                engine
                    .physics()
                    .get_players()
                    .iter()
                    .filter(|(_, player)| player.on_court)
                    .map(|(id, player)| {
                        (
                            id.clone(),
                            player.pos_ft,
                            player.target_pos_ft,
                            player.stamina,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    let baseline = run(&baseline_setup);
    let perturbed = run(&perturbed_setup);
    let changed = baseline
        .iter()
        .zip(perturbed.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        changed >= 2,
        "{label}: a single tendency change must alter real match behavior on at least 2/4 seeds; observed {changed}/4"
    );
}

type TendencyMutator = fn(&mut PlayerTendencies, f32);

#[test]
fn every_tendency_reaches_live_match_behavior() {
    let tendencies: [(&str, TendencyMutator); 12] = [
        ("shoot_frequency", |t, value| t.shoot_frequency = value),
        ("drive_frequency", |t, value| t.drive_frequency = value),
        ("pass_frequency", |t, value| t.pass_frequency = value),
        ("cut_frequency", |t, value| t.cut_frequency = value),
        ("screen_frequency", |t, value| t.screen_frequency = value),
        ("offensive_rebound_frequency", |t, value| {
            t.offensive_rebound_frequency = value
        }),
        ("gamble_steal", |t, value| t.gamble_steal = value),
        ("block_aggressiveness", |t, value| {
            t.block_aggressiveness = value
        }),
        ("help_aggressiveness", |t, value| {
            t.help_aggressiveness = value
        }),
        ("physicality", |t, value| t.physicality = value),
        ("risk_tolerance", |t, value| t.risk_tolerance = value),
        ("transition_sprint", |t, value| t.transition_sprint = value),
    ];
    for (label, mutate) in tendencies {
        assert_tendency_reaches_behaviour(label, mutate);
    }
}

#[test]
fn gamble_steal_raises_attempt_scale() {
    let rules = GameRules::default();
    let policy = &rules.resolve.ball_security;
    let scale = |gamble_steal: f32| {
        policy.poke_steal_tendency_floor + gamble_steal * policy.poke_steal_tendency_span
    };
    let verdict =
        check_monotonic_response("gamble_steal → attempt scale", scale, 0.1, 0.9, true, 0.1);
    assert!(verdict.is_ok(), "{}", verdict.unwrap_err());
    assert!(
        (scale(0.5) - 1.0).abs() < 1e-6,
        "the neutral tendency must map to the neutral scale 1.0, got {}",
        scale(0.5)
    );
}

/// 中距离倾向只改变中投候选效用，并由统一分区判定门控。
#[test]
fn midrange_utility_bonus_changes_real_match_shot_distribution() {
    let mut baseline = GameRules::default();
    let mut calibrated = baseline.clone();
    baseline.decision.midrange_utility_bonus = 0.0;
    calibrated.decision.midrange_utility_bonus = 0.35;
    let a = support::fingerprint_for_setup(baseline, &support::PROOF_SEEDS, 6_000);
    let b = support::fingerprint_for_setup(calibrated, &support::PROOF_SEEDS, 6_000);
    let changed = a
        .iter()
        .zip(b.iter())
        .filter(|(left, right)| left.differs_from(right))
        .count();
    assert!(
        changed >= 2,
        "midrange utility calibration must alter at least 2/4 match fingerprints; {changed}/4 changed"
    );
}

/// 断路负面对照：跨度归零后倾向不再影响尝试速率。
#[test]
fn gamble_steal_dead_when_tendency_span_zeroed() {
    let mut rules = GameRules::default();
    rules.resolve.ball_security.poke_steal_tendency_span = 0.0;
    rules.resolve.ball_security.poke_steal_tendency_floor = 1.0;
    let policy = &rules.resolve.ball_security;
    let hazard = |gamble_steal: f32| {
        let scale =
            policy.poke_steal_tendency_floor + gamble_steal * policy.poke_steal_tendency_span;
        policy.poke_attempt_rate_per_sec * scale
    };
    assert!(
        (hazard(0.9) - hazard(0.1)).abs() < f32::EPSILON,
        "zeroed span must decouple the poke hazard from the tendency"
    );
}

// ---------------------------------------------------------------------------
// block_aggressiveness：封盖起跳概率
// ---------------------------------------------------------------------------

#[test]
fn block_aggressiveness_raises_block_probability() {
    let rules = GameRules::default();
    let policy = &rules.resolve.block;
    let scale = |block_aggressiveness: f32| {
        policy.block_tendency_floor + block_aggressiveness * policy.block_tendency_span
    };
    let verdict = check_monotonic_response(
        "block_aggressiveness → block scale",
        scale,
        0.1,
        0.9,
        true,
        0.1,
    );
    assert!(verdict.is_ok(), "{}", verdict.unwrap_err());
    assert!(
        (scale(0.5) - 1.0).abs() < 1e-6,
        "the neutral tendency must map to the neutral scale 1.0, got {}",
        scale(0.5)
    );
}

// ---------------------------------------------------------------------------
// help_aggressiveness：弱侧协防护筐引力
// ---------------------------------------------------------------------------

fn help_threat_ratio(help_aggressiveness: f32) -> f32 {
    let rules = GameRules::default();
    let solver = DefensePotentialFieldSolver::new(rules.tactics.defense.potential_field);
    let hoop = Vec2::new(88.75, 25.0);
    let off_positions = [
        hoop - Vec2::new(22.0, 6.0),
        hoop - Vec2::new(24.0, -8.0),
        hoop - Vec2::new(20.0, 10.0),
        hoop - Vec2::new(26.0, -2.0),
        hoop - Vec2::new(18.0, 4.0),
    ];
    solver
        .solve_equilibrium(
            hoop - Vec2::new(6.0, 0.0),
            hoop,
            &off_positions,
            1,
            0,
            &rules,
            1.0,
            false,
            help_aggressiveness,
        )
        .threat_ratio
}

#[test]
fn help_aggressiveness_raises_rim_help_threat_ratio() {
    let verdict = check_monotonic_response(
        "help_aggressiveness → threat ratio",
        help_threat_ratio,
        0.1,
        0.9,
        true,
        0.01,
    );
    assert!(verdict.is_ok(), "{}", verdict.unwrap_err());
}

/// 断路负面对照：协防倾向跨度归零后威胁占比不再随倾向变化。
#[test]
fn help_aggressiveness_dead_when_tendency_span_zeroed() {
    let mut rules = GameRules::default();
    rules.tactics.defense.potential_field.help_tendency_span = 0.0;
    rules.tactics.defense.potential_field.help_tendency_floor = 1.0;
    let solver = DefensePotentialFieldSolver::new(rules.tactics.defense.potential_field);
    let hoop = Vec2::new(88.75, 25.0);
    let off_positions = [
        hoop - Vec2::new(22.0, 6.0),
        hoop - Vec2::new(24.0, -8.0),
        hoop - Vec2::new(20.0, 10.0),
        hoop - Vec2::new(26.0, -2.0),
        hoop - Vec2::new(18.0, 4.0),
    ];
    let ratio = |help_aggressiveness: f32| {
        solver
            .solve_equilibrium(
                hoop - Vec2::new(6.0, 0.0),
                hoop,
                &off_positions,
                1,
                0,
                &rules,
                1.0,
                false,
                help_aggressiveness,
            )
            .threat_ratio
    };
    assert!(
        (ratio(0.9) - ratio(0.1)).abs() < f32::EPSILON,
        "zeroed span must decouple the help threat ratio from the tendency"
    );
}

// ---------------------------------------------------------------------------
// physicality：位置对抗犯规概率
// ---------------------------------------------------------------------------

#[test]
fn physicality_raises_position_foul_probability() {
    let rules = GameRules::default();
    let policy = &rules.resolve.contact;
    let scale = |physicality: f32| policy.physicality_floor + physicality * policy.physicality_span;
    let verdict = check_monotonic_response("physicality → foul scale", scale, 0.1, 0.9, true, 0.1);
    assert!(verdict.is_ok(), "{}", verdict.unwrap_err());
    assert!(
        (scale(0.5) - 1.0).abs() < 1e-6,
        "the neutral tendency must map to the neutral scale 1.0, got {}",
        scale(0.5)
    );
}

/// 断路负面对照：对抗倾向跨度归零后犯规概率不再随倾向变化。
#[test]
fn physicality_dead_when_tendency_span_zeroed() {
    let mut rules = GameRules::default();
    rules.resolve.contact.physicality_span = 0.0;
    rules.resolve.contact.physicality_floor = 1.0;
    let policy = &rules.resolve.contact;
    let probability = |physicality: f32| {
        let scale = policy.physicality_floor + physicality * policy.physicality_span;
        policy.foul_rate * scale
    };
    assert!(
        (probability(0.9) - probability(0.1)).abs() < f32::EPSILON,
        "zeroed span must decouple the foul probability from the tendency"
    );
}

// ---------------------------------------------------------------------------
// transition_sprint：转换期跑动速度
// ---------------------------------------------------------------------------

#[test]
fn transition_sprint_raises_sprint_scale() {
    let rules = GameRules::default();
    let policy = &rules.tactics;
    let scale = |transition_sprint: f32| {
        policy.transition_sprint_floor + transition_sprint * policy.transition_sprint_span
    };
    let verdict = check_monotonic_response(
        "transition_sprint → sprint scale",
        scale,
        0.1,
        0.9,
        true,
        0.1,
    );
    assert!(verdict.is_ok(), "{}", verdict.unwrap_err());
    assert!(
        (scale(0.5) - 1.0).abs() < 1e-6,
        "the neutral tendency must map to the neutral scale 1.0, got {}",
        scale(0.5)
    );
}

// ---------------------------------------------------------------------------
// offensive_rebound_frequency：前板冲抢排序键
// ---------------------------------------------------------------------------

/// 冲抢排序键 = `能力 × 地板 + 倾向 × 跨度`（contests.rs 的实际公式）。
#[test]
fn offensive_rebound_frequency_raises_crash_rank_key() {
    let attribute = 0.5;
    let key =
        |offensive_rebound_frequency: f32| attribute * 0.6 + offensive_rebound_frequency * 0.4;
    let verdict = check_monotonic_response(
        "offensive_rebound_frequency → crash key",
        key,
        0.1,
        0.9,
        true,
        0.1,
    );
    assert!(verdict.is_ok(), "{}", verdict.unwrap_err());
}

/// 两名能力相同、倾向不同的球员必须产生不同的冲抢排序键：
/// 能力决定抢不抢得到，倾向决定想不想去抢。
#[test]
fn offensive_rebound_tendency_breaks_ability_ties() {
    let eager = physics_player(tendencies_with(|t| t.offensive_rebound_frequency = 0.9));
    let reluctant = physics_player(tendencies_with(|t| t.offensive_rebound_frequency = 0.1));
    assert!(
        eager.tendencies.offensive_rebound_frequency
            > reluctant.tendencies.offensive_rebound_frequency,
        "tendency must survive into the physics state the contest ranking reads"
    );
}

// ---------------------------------------------------------------------------
// 负面对照：harness 必须拒绝常数短路与方向错误
// ---------------------------------------------------------------------------

#[test]
fn negative_control_harness_rejects_constant_short_circuit() {
    let dead = |_x: f32| 0.77_f32;
    assert!(
        check_monotonic_response("dead", dead, 0.1, 0.9, true, 0.01).is_err(),
        "harness must reject a constant short circuit"
    );
}

#[test]
fn negative_control_harness_rejects_wrong_direction() {
    assert!(
        check_monotonic_response("inverted", |x: f32| -x, 0.1, 0.9, true, 0.01).is_err(),
        "harness must reject a wrong-direction response"
    );
}
