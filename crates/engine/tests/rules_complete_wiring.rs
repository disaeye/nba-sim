//! 规则系数因果闭环：**规则**系数的扰动必须改变真实模拟。
//!
//! ## 与 `wiring_proof.rs` 的分工
//!
//! `wiring_proof.rs` 证明**属性**扰动能改变真实模拟；本文件证明**规则系数**
//! 扰动同样能改变真实模拟。两者缺一不可：属性可影响行为只说明映射层被调用，
//! 不说明规则通道的系数真的进入了计算——若 `capability.rs` 把系数写成内联常数，
//! 属性扰动照样有效，而规则校准完全失效。
//!
//! ## 判据
//!
//! 对每个维度：只改该维度的规则系数（其余字段保持默认），跑 4 个 seed 各
//! 15000 tick，要求至少 3/4 的模拟指纹发生变化。
//!
//! ## 纪律
//!
//! 断言一旦变红，不得通过放宽阈值或改写测试消除：先判断是「系数没进计算」
//! 还是「契约需要重新裁定」，前者修代码，后者走 `docs/decisions.md`。

mod support;

use nba_domain::GameRules;
use support::{fingerprint_for_setup, BehaviorFingerprint, PROOF_SEEDS};

/// 规则系数扰动需要比属性扰动更长的窗口：系数影响的是概率的可观测形态，
/// 在 3000 tick 内经常掷出相同结果。
///
/// 取 20000 tick 与 `docs/dev/current/plan.md` §8.3 的**通道可见性实测表**
/// 同一尺度——那张表就是在 seed 42 × 20000 tick 的盒记分上逐项测得的，
/// 两个窗口不同会使「实测可见」与「测试可见」对不上。
const RULE_WIRING_TICKS: usize = 20000;

/// 扰动一个规则系数，要求至少 3/4 的 seed 行为改变。
fn assert_rule_coefficient_reaches_behaviour(label: &str, mutate: impl Fn(&mut GameRules)) {
    let baseline = fingerprint_for_setup(GameRules::default(), &PROOF_SEEDS, RULE_WIRING_TICKS);
    let mut perturbed_rules = GameRules::default();
    mutate(&mut perturbed_rules);
    // 系数必须真的被改动，否则断言退化为恒真（自检）。
    assert_ne!(
        format!("{perturbed_rules:?}"),
        format!("{:?}", GameRules::default()),
        "{label}: 扰动函数未改动任何规则字段"
    );
    let perturbed = fingerprint_for_setup(perturbed_rules, &PROOF_SEEDS, RULE_WIRING_TICKS);
    assert_wiring_changed(label, &baseline, &perturbed);
}

fn assert_wiring_changed(
    label: &str,
    baseline: &[BehaviorFingerprint],
    perturbed: &[BehaviorFingerprint],
) {
    let changed = baseline
        .iter()
        .zip(perturbed.iter())
        .filter(|(a, b)| a.differs_from(b))
        .count();
    assert!(
        changed >= 3,
        "{label}: 规则系数扰动必须在 ≥3/4 个 seed 上改变真实模拟输出，\
         实际只有 {changed}/4 改变——该系数没有进入计算路径，\
         或它虽然在结构体里但 capability 层仍在用内联常数"
    );
}

#[test]
fn capability_risk_tolerance_gain_reaches_behaviour() {
    assert_rule_coefficient_reaches_behaviour("capability.risk_tolerance_gain", |r| {
        r.capability.risk_tolerance_base = 1.0;
        r.capability.risk_tolerance_gain = 1.0;
    });
}

#[test]
fn capability_boxout_bonus_gain_reaches_behaviour() {
    assert_rule_coefficient_reaches_behaviour("capability.boxout_bonus_primary_gain", |r| {
        r.capability.boxout_bonus_primary_gain = 1.0;
        r.capability.boxout_bonus_secondary_gain = 1.0;
    });
}

#[test]
fn capability_putback_bias_gain_reaches_behaviour() {
    assert_rule_coefficient_reaches_behaviour("capability.putback_bias_gain", |r| {
        r.capability.putback_bias_base = 0.5;
        r.capability.putback_bias_gain = 0.5;
    });
}

#[test]
fn capability_help_awareness_gain_reaches_behaviour() {
    assert_rule_coefficient_reaches_behaviour("capability.help_awareness_primary_gain", |r| {
        r.capability.help_awareness_primary_gain = 3.0;
        r.capability.help_awareness_secondary_gain = 3.0;
    });
}

#[test]
fn capability_post_defense_physicality_gain_reaches_behaviour() {
    assert_rule_coefficient_reaches_behaviour("capability.post_defense_primary_gain", |r| {
        r.capability.post_defense_primary_gain = 1.0;
        r.capability.post_defense_secondary_gain = 1.0;
    });
}

/// 快下阈值：基线阈值 0.36，而内置阵容的最大快下机会约 0.46，因此基线**会**
/// 走快下。把阈值抬到 1.0 则禁用该分支，选择回到「交给控卫组织」——这才真正
/// 改变受传球人；把阈值调到 0.0 不会（0.46 已超过两者）。
#[test]
fn capability_transition_leakout_gain_reaches_behaviour() {
    assert_rule_coefficient_reaches_behaviour("capability.transition_leakout_threshold", |r| {
        r.capability.transition_leakout_threshold = 1.0;
    });
}

#[test]
fn intercept_risk_factor_reaches_behaviour() {
    assert_rule_coefficient_reaches_behaviour("resolve.base_rates.intercept_risk_factor", |r| {
        r.resolve.base_rates.intercept_risk_factor_floor = 0.0;
        r.resolve.base_rates.intercept_risk_factor_gain = 0.0;
    });
}

#[test]
fn rebound_boxout_distance_discount_reaches_behaviour() {
    assert_rule_coefficient_reaches_behaviour("resolve.rebound.boxout_distance_discount", |r| {
        r.resolve.rebound.boxout_distance_discount = 0.0;
    });
}

/// 二次补篮倾向与卡位加成是**两个独立系数**，必须分则扰动。
///
/// 实测（4 seed × 20000 tick）：单独把任一系数归零各改变 3/4 与 4/4 个 seed，
/// 而两者同时归零只改变 2/4——两个效应部分相互抵消（卡位让防守方更快到球，
/// 补篮倾向又拉近进攻方距离）。合并扰动会让两个都已接线的系数显得没接线。
#[test]
fn rebound_putback_distance_discount_reaches_behaviour() {
    assert_rule_coefficient_reaches_behaviour("resolve.rebound.putback_distance_discount", |r| {
        r.resolve.rebound.putback_distance_discount = 0.0;
    });
}

#[test]
fn post_up_base_reaches_behaviour() {
    assert_rule_coefficient_reaches_behaviour("decision.post_up_base", |r| {
        r.decision.post_up_base = 0.0;
    });
}

#[test]
fn morale_affinity_reaches_behaviour() {
    assert_rule_coefficient_reaches_behaviour("modulation.morale_shoot_affinity", |r| {
        r.modulation.morale_shoot_affinity = 0.0;
    });
}

/// 突破结算的系数（从内联常量收编进 `DrivePolicy`）。
///
/// 收编不改行为：默认值逐字等于原内联值，黄金哈希不变。
///
/// 扰动取向说明：`skill_delta_scale` 归零只改变 2/4 个 seed（技巧差异项被移除后，
/// 基础概率仍能掷出大量相同结果），放大到 10 倍则 4/4 可见。两者都是合法扰动，
/// 取后者是因为它把「该系数确实进入计算」变得可观测。
#[test]
fn drive_skill_delta_scale_reaches_behaviour() {
    assert_rule_coefficient_reaches_behaviour("resolve.drive.skill_delta_scale", |r| {
        r.resolve.drive.skill_delta_scale = 10.0;
    });
}

#[test]
fn drive_foul_base_share_reaches_behaviour() {
    assert_rule_coefficient_reaches_behaviour("resolve.drive.foul_base_share", |r| {
        r.resolve.drive.foul_base_share = 1.0;
    });
}

#[test]
fn drive_foul_contest_scale_reaches_behaviour() {
    assert_rule_coefficient_reaches_behaviour("resolve.drive.foul_contest_scale", |r| {
        r.resolve.drive.foul_contest_scale = 5.0;
    });
}

// `resolve.drive.finish_lane_density_scale` 没有行为级测试，因为实测它在此尺度上
// 不可观测：归零改变 0/4 个 seed，放大到 5 倍只改变 1/4。
//
// 该系数在计算里确实被读取（`finish_probability` 的 `lane_density ×
// lane_density_penalty × finish_lane_density_scale` 项），但突破终结路径的
// `paint_crowding` 取值长期较小，使该项对结果的杠杆过小——这是**证据缺口**，
// 不是接线缺口：不得通过放宽阈值或改写测试把它写成已接线。
// 要关闭该缺口，需要先让突破终结的密集度分布变得可区分（另立任务）。
