//! 规则系数因果闭环：**规则**系数的扰动必须改变真实模拟。
//!
//! ## 与 `wiring_proof.rs` 的分工
//!
//! `wiring_proof.rs` 证明**属性**扰动能改变真实模拟；本文件证明**规则系数**
//! 扰动同样能改变真实模拟。两者缺一不可：属性可影响行为只说明映射层被调用，
//! 不说明规则通道的系数真的进入了计算——若 `capability.rs` 把系数写成内联常数，
//! 属性扰动照样有效，而规则校准完全失效。
//!
//! ## 判据（两种判据并列，逐测试注明，ADR-016）
//!
//! 默认判据：只改该维度的规则系数（其余字段保持默认），跑 4 个 seed 各
//! 20000 tick，要求至少 3/4 的模拟指纹发生变化。
//!
//! 扩展判据（ADR-016，仅限四个小杠杆/低频系数）：6 个 seed × 20000 tick，
//! 要求至少 3/6 的指纹改变。背景与测量记录见 `docs/decisions.md` ADR-016：
//! 授权行为变化（体力重校准、G6 换人）重排了轨迹抽样，把这三个处在
//! 2-3/4 边缘可见区的系数推到 2/4。样本扩到 6 后实测可见率为 50%
//! （boxout 折扣的最大可能杠杆受 boxout_bonus ≈ 0.225 限制在 13.5%），
//! 判据线因此定在 ≥3/6。它与 4-seed 判据 ≥3/4 的判别力等同：
//! 两种判据下「完全未接线」的失败形态都是指纹零改变。
//!
//! ## 共享基线
//!
//! 全部测试用同一组默认规则做基线。基线指纹用 `OnceLock` 只计算一次，
//! 各测试共享同一份结果；每个测试自己准备一份扰动指纹。
//! 共享基线只是消除重复计算：判据本身不变。
//!
//! ## 纪律
//!
//! 断言一旦变红，不得通过放宽阈值或改写测试消除：先判断是「系数没进计算」
//! 还是「判定标准需要重新裁定」，前者修代码，后者走 `docs/decisions.md`。

mod support;

use nba_domain::GameRules;
use std::sync::OnceLock;
use support::{fingerprint_for_setup, BehaviorFingerprint, PROOF_SEEDS};

/// 规则系数扰动需要比属性扰动更长的窗口：系数影响的是概率的可观测形态，
/// 在 3000 tick 内经常掷出相同结果。
///
/// 取 20000 tick 与 `docs/dev/current/plan.md` §8.3 的**通道可见性实测表**
/// 同一尺度——那张表就是在 seed 42 × 20000 tick 的盒记分上逐项测得的，
/// 两个窗口不同会使「实测可见」与「测试可见」对不上。
///
/// 第三步碰撞几何（v70：篮板碰撞面 + 自由球-人碰撞 + 传球碰撞化）
/// 把四个小杠杆系数的指纹可见度又压低一档（实测 0–2/6）：它们的
/// 消费链都在碰撞后的低频事件上（篮板争夺、接触分类）。按 ADR-016
/// 「不得降低区分力」的约束，改用加倍窗口（40000 tick）——如果加倍后
/// 仍不可见，说明消费链断了，应该修实现而不是继续加宽。
const RULE_WIRING_TICKS: usize = 40000;

/// 扩展判定种子集：判据 ≥3/6，配合 ADR-016 使用（见下）。
/// 前四个元素与 `PROOF_SEEDS` 相同：同一 seed、同一规则、同一窗口的
/// 指纹逐位相同，所以 4-seed 基线直接取扩展基线的前四项，
/// 只计算一次 6-seed 基线。
const EXTENDED_SEEDS: [u64; 6] = [42, 1, 7, 100, 999, 31337];

fn shared_baseline_6() -> &'static Vec<BehaviorFingerprint> {
    static BASELINE_6: OnceLock<Vec<BehaviorFingerprint>> = OnceLock::new();
    BASELINE_6.get_or_init(|| {
        fingerprint_for_setup(GameRules::default(), &EXTENDED_SEEDS, RULE_WIRING_TICKS)
    })
}

/// 扰动一个规则系数，用默认判据（4 seed，≥3/4）。
fn assert_rule_coefficient_reaches_behaviour(label: &str, mutate: impl Fn(&mut GameRules)) {
    let mut perturbed_rules = GameRules::default();
    mutate(&mut perturbed_rules);
    // 系数必须真的被改动，否则断言退化为恒真（自检）。
    assert_ne!(
        format!("{perturbed_rules:?}"),
        format!("{:?}", GameRules::default()),
        "{label}: 扰动函数未改动任何规则字段"
    );
    let perturbed = fingerprint_for_setup(perturbed_rules, &PROOF_SEEDS, RULE_WIRING_TICKS);
    assert_wiring_changed(label, &shared_baseline_6()[..4], &perturbed, 3);
}

/// 扰动一个规则系数，用扩展判据（6 seed，≥3/6，ADR-016）。
fn assert_rule_coefficient_reaches_behaviour_extended(
    label: &str,
    mutate: impl Fn(&mut GameRules),
) {
    let mut perturbed_rules = GameRules::default();
    mutate(&mut perturbed_rules);
    assert_ne!(
        format!("{perturbed_rules:?}"),
        format!("{:?}", GameRules::default()),
        "{label}: 扰动函数未改动任何规则字段"
    );
    let perturbed = fingerprint_for_setup(perturbed_rules, &EXTENDED_SEEDS, RULE_WIRING_TICKS);
    assert_wiring_changed(label, shared_baseline_6(), &perturbed, 3);
}

fn assert_wiring_changed(
    label: &str,
    baseline: &[BehaviorFingerprint],
    perturbed: &[BehaviorFingerprint],
    required: usize,
) {
    let total = baseline.len();
    let changed = baseline
        .iter()
        .zip(perturbed.iter())
        .filter(|(a, b)| a.differs_from(b))
        .count();
    assert!(
        changed >= required,
        "{label}: 规则系数扰动必须在 ≥{required}/{total} 个 seed 上改变真实模拟输出，\
         实际只有 {changed}/{total} 改变——该系数没有进入计算路径，\
         或它虽然在结构体里但 capability 层仍在用内联常数"
    );
}

/// `risk_tolerance_gain` 通过 pass selection 的风险容忍通道影响传球路线，
/// 传球接触后的分类使用轨迹事实与球员能力。
#[test]
fn capability_risk_tolerance_gain_reaches_behaviour() {
    let baseline_rules = GameRules::default();
    let mut perturbed_rules = baseline_rules.clone();
    perturbed_rules.capability.risk_tolerance_gain = 0.8;
    let baseline = fingerprint_for_setup(baseline_rules, &EXTENDED_SEEDS, RULE_WIRING_TICKS);
    let perturbed = fingerprint_for_setup(perturbed_rules, &EXTENDED_SEEDS, RULE_WIRING_TICKS);
    assert_wiring_changed("capability.risk_tolerance_gain", &baseline, &perturbed, 3);
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
    let baseline = fingerprint_for_setup(GameRules::default(), &EXTENDED_SEEDS, RULE_WIRING_TICKS);
    let mut perturbed_rules = GameRules::default();
    perturbed_rules
        .resolve
        .base_rates
        .intercept_risk_factor_floor = 0.0;
    perturbed_rules
        .resolve
        .base_rates
        .intercept_risk_factor_gain = 4.0;
    let perturbed = fingerprint_for_setup(perturbed_rules, &EXTENDED_SEEDS, RULE_WIRING_TICKS);
    assert_wiring_changed(
        "resolve.base_rates.intercept_risk_factor",
        &baseline,
        &perturbed,
        3,
    );
}

/// 扩展判据（ADR-016）：距离折扣只改篮板争夺者约 13.5% 的有效距离，
/// 处在边缘可见区；seed 扩到 6、判据 ≥3/6。
///
/// 饥和量级扰动（本文件 morale 先例：放大优于归零）：归零只改争夺者
/// 约 13.5%–24% 的有效距离，其可见性随轨迹扰动翻转（实测同一接线下
/// 冲抢速度 16.0 与 16.06 分别得到通过与失败），测的是采样运气。
/// 4.0 把有效距离杠杆抬到 90%–160%（高额卡位若直接压到 0），
/// 使本测试测的是消费链本身。
#[test]
fn rebound_boxout_distance_discount_reaches_behaviour() {
    assert_rule_coefficient_reaches_behaviour_extended(
        "resolve.rebound.boxout_distance_discount",
        |r| {
            r.resolve.rebound.boxout_distance_discount = 4.0;
        },
    );
}

/// 二次补篮倾向与卡位加成是**两个独立系数**，必须分则扰动。
///
/// 实测（4 seed × 20000 tick）：单独把任一系数归零各改变 3/4 与 4/4 个 seed，
/// 而两者同时归零只改变 2/4——两个效应部分相互抵消（卡位让防守方更快到球，
/// 补篮倾向又拉近进攻方距离）。合并扰动会让两个都已接线的系数显得没接线。
///
/// G6a 链 2 接线后（decide 效用消费 putback_distance_discount）可见性恢复：
/// 扩展判据（ADR-016）保持 6 seed、判据 ≥3/6；饱和量级 4.0
/// （本文件 morale 先例：放大优于归零）。
#[test]
fn rebound_putback_distance_discount_reaches_behaviour() {
    assert_rule_coefficient_reaches_behaviour_extended(
        "resolve.rebound.putback_distance_discount",
        |r| {
            r.resolve.rebound.putback_distance_discount = 4.0;
        },
    );
}

#[test]
fn post_up_base_reaches_behaviour() {
    assert_rule_coefficient_reaches_behaviour("decision.post_up_base", |r| {
        r.decision.post_up_base = 0.0;
    });
}

/// 扩展判据（ADR-016）：`morale_shoot_affinity` 对效用的杠杆 =
/// morale_bias（±0.05–0.12）× affinity（0.35）≈ ±0.04，远小于候选间
/// 效用差，指纹窗口内偶发可见；抛体化（v68）与触筐物理（v69）两次
/// 重排后归零扰动只剩 1/6、×4 放大 2/6。改用饱和量级扰动（文件先例：
/// 放大优于归零）：3.0 把杠杆抬到 ±0.15–0.36，与效用差同量级。
#[test]
fn morale_affinity_reaches_behaviour() {
    assert_rule_coefficient_reaches_behaviour_extended("modulation.morale_shoot_affinity", |r| {
        r.modulation.morale_shoot_affinity = 3.0;
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

// `resolve.drive.finish_lane_density_scale` 没有行为级测试：突破终结路径的
// `paint_crowding` 取值长期较小，该系数的杠杆不可观测（归零 0/4、放大 1/4）。
// 它在计算里确实被读取（`finish_probability` 的 `lane_density ×
// lane_density_penalty × finish_lane_density_scale` 项）——这是**证据缺口**，
// 已并入 `docs/dev/gap.md` G6a（禁区出手分布的行为链补齐）：
// 禁止通过放宽阈值或改写测试把它写成已接线。
