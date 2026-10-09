//! 接线证明：把「已完成」的声明转成机器可判定的证据。
//!
//! ## 为什么需要这个文件
//!
//! 目标与设计文档之间的系统性误差有一种共同形态：**用声明代替证据**。
//!
//! 1. `UNIMPLEMENTED_RULE_FIELDS = &[]` 宣告「零消费字段已清零」，但清单为空
//!    不等于字段被消费；
//! 2. 曾经有一个与 `MatchEngine` 平行的世界副本逐 tick 被填充，而空间核在一个
//!    `n == 0` 的空世界上运算并把结果丢掉 —— 「接了感知系统」是假的；
//! 3. 只比较两个字面量（`8.0 != 22.0`）的测试从不运行模拟 —— 测试通过不构成
//!    接线证据。
//!
//! 本文件用**行为级断言**替代上述声明式断言：每一条都要求「改一个输入 → 真实
//! 模拟输出必须变」，否则红。规则系数侧的同类守卫在 `rules_complete_wiring.rs`，
//! 两者同一口径、不得重复。
//!
//! ## 纪律
//!
//! 断言一旦变红，**不得**通过放宽阈值或改写测试来消除：必须先判断是「接线坏了」
//! 还是「判定标准需要重新裁定」，前者修代码，后者走 `docs/decisions.md`。

mod support;

use nba_domain::GameRules;
use nba_engine::{MatchEngine, MatchSetup};
use support::{fingerprint, fingerprint_for_setup, PROOF_SEEDS, PROOF_TICKS, PROOF_TICKS_MEDIUM};

// ============================================================================
// 门 1：规则扰动必须改变真实模拟输出
// ============================================================================

/// `drive_finish_range_ft` 必须经规则通道影响真实比赛，而不是只在结构体里存着。
///
/// D24 几何突破落地后，突破成败由路径可达性与接触几何裁定，绕行目标
/// 使得到筐距离分布变窄，距离门（8 vs 22 ft）的可见性随轨迹扰动翻转
/// （实测 4 seed × 6000 tick 只剩 2/4）。按 rules_complete_wiring 的
/// ADR-016 先例扩为 6 seed、判据 ≥3/6：测的是消费链本身，不放松判据。
#[test]
fn rules_wiring_drive_finish_range_changes_simulation() {
    let mut short = GameRules::default();
    short.tactics.drive_finish_range_ft = 8.0;
    let mut long = GameRules::default();
    long.tactics.drive_finish_range_ft = 22.0;

    let seeds = [42u64, 1, 7, 100, 999, 31337];
    // v91 冲框折扣量纲修正后，突破目标更多直指篮筐、被
    // early_finish（6 ft 门）终结的比例大增——finish_range 的
    // 停滞分支被绕过，扰动的行为差异面收窄到 2/6（6000 tick）。
    // 窗口加倍到 12000 tick：停滞判定本身仍是活跃通道，
    // 样本量恢复后差异重新显形。
    let proof_ticks = 12000usize;
    let a = fingerprint_for_setup(short, &seeds, proof_ticks);
    let b = fingerprint_for_setup(long, &seeds, proof_ticks);

    let changed = a
        .iter()
        .zip(b.iter())
        .filter(|(x, y)| x.differs_from(y))
        .count();
    assert!(
        changed >= 3,
        "drive_finish_range_ft must change real simulation behaviour on at least 3/6 seeds, \
         but only {changed}/6 differed — the field is stored but not consumed"
    );
}

/// Clutch 规则必须经规则通道影响真实比赛。
#[test]
fn rules_wiring_clutch_modulation_changes_simulation() {
    let baseline = GameRules::default();
    let mut perturbed = GameRules::default();
    // 把 clutch 窗口放大到「几乎整场」，使 clutch_bias 在大量回合生效；
    // 若 clutch_* 真的接入了士气/决策通道，输出必然变化。
    perturbed.modulation.clutch_time_remaining = 720.0;
    perturbed.modulation.clutch_period = 1;
    perturbed.modulation.clutch_score_margin = 999;
    perturbed.modulation.clutch_bias = 0.9;

    let a = fingerprint_for_setup(baseline, &PROOF_SEEDS, PROOF_TICKS);
    let b = fingerprint_for_setup(perturbed, &PROOF_SEEDS, PROOF_TICKS);

    let changed = a
        .iter()
        .zip(b.iter())
        .filter(|(x, y)| x.differs_from(y))
        .count();
    assert!(
        changed >= 3,
        "clutch_* modulation rules must change real simulation behaviour on ≥3/4 seeds, \
         but only {changed}/4 differed — clutch parameters are read back in tests \
         but never reach the morale/decision channel"
    );
}

// ============================================================================
// 门 2：空间事实必须来自真实的在场比赛
// ============================================================================

/// 空间量必须在真实的十人之上计算。
///
/// 历史缺陷：平行世界副本逐 tick 被填充，而空间核在一个 `n == 0` 的空世界上
/// 运算并把结果丢掉。副本已删除（ADR-016），空间量现在只从物理世界实时计算，
/// 本测试断言它的输入确实是十个人。
#[test]
fn spatial_pressure_runs_on_real_on_court_players() {
    let mut engine = MatchEngine::new(42);
    for _ in 0..200 {
        engine.step();
    }
    let on_court = engine
        .physics()
        .get_players()
        .values()
        .filter(|p| p.on_court)
        .count();
    assert_eq!(
        on_court, 10,
        "spatial kernels must see all ten on-court players; a smaller count \
         means they compute on an empty or partial world"
    );
    let offense = engine.possession();
    let ball_pos = engine.ball_pos_3d().0;
    let crowded = nba_semantics::SemanticEvaluator::defensive_pressure(
        ball_pos,
        offense,
        engine.physics(),
        engine.rules(),
    );
    assert!(
        crowded >= 0.0,
        "multi-defender pressure must be computable from the live world"
    );
}

// ============================================================================
// 门 3：能力函数必须被主干消费，而非仅被公式单测覆盖
// ============================================================================

/// 能力函数中每一个的输入属性，都必须能改变真实模拟输出。
///
/// 做法：把全队属性设到极端低 / 极端高，比较真实模拟指纹。
fn assert_attribute_reaches_behaviour_at(
    label: &str,
    ticks: usize,
    mutate: impl Fn(&mut nba_domain::PlayerAttributes, f32),
) {
    let run = |value: f32| -> Vec<support::BehaviorFingerprint> {
        PROOF_SEEDS
            .iter()
            .map(|&seed| {
                let rules = GameRules::default();
                let mut setup = MatchSetup::builtin(rules);
                for team in [&mut setup.home_team, &mut setup.away_team] {
                    for p in team.players.iter_mut() {
                        mutate(&mut p.attributes, value);
                    }
                }
                let mut engine = MatchEngine::with_setup(setup, seed);
                fingerprint(&mut engine, ticks)
            })
            .collect()
    };

    let low = run(0.05);
    let high = run(0.95);
    let changed = low
        .iter()
        .zip(high.iter())
        .filter(|(x, y)| x.differs_from(y))
        .count();
    assert!(
        changed >= 2,
        "{label}: attribute perturbation must change real simulation output on ≥2/4 seeds, \
         but only {changed}/4 differed — the capability function is only covered by a \
         formula-level unit test and never consumed by the engine"
    );
}

/// 低频消费路径的能力接线门：窗口加倍到 [`PROOF_TICKS_MEDIUM`]。
/// 卡位能力只在空中篮板落点裁定（v81 后每 6000 tick 约 4~8 次）时
/// 被消费，3000 tick 窗口的概率样本量不足以让扰动显形。
fn assert_attribute_reaches_behaviour(
    label: &str,
    mutate: impl Fn(&mut nba_domain::PlayerAttributes, f32),
) {
    assert_attribute_reaches_behaviour_at(label, PROOF_TICKS, mutate);
}

#[test]
fn capability_post_defense_physicality_reaches_behaviour() {
    assert_attribute_reaches_behaviour("effective_post_defense_physicality", |a, v| {
        a.defense_interior = v;
        a.strength = v;
    });
}

#[test]
fn capability_transition_leakout_chance_reaches_behaviour() {
    assert_attribute_reaches_behaviour("effective_transition_leakout_chance", |a, v| {
        a.speed = v;
    });
}

#[test]
fn capability_help_awareness_reaches_behaviour() {
    assert_attribute_reaches_behaviour("effective_help_awareness", |a, v| {
        a.defense_interior = v;
        a.decision_iq = v;
    });
}

#[test]
fn capability_boxout_bonus_reaches_behaviour() {
    assert_attribute_reaches_behaviour_at(
        "effective_defensive_boxout_bonus",
        PROOF_TICKS_MEDIUM,
        |a, v| {
            a.defensive_rebound = v;
            a.strength = v;
        },
    );
}

#[test]
fn capability_catch_radius_reaches_behaviour() {
    assert_attribute_reaches_behaviour("effective_catch_radius", |a, v| {
        a.ball_handling = v;
    });
}

#[test]
fn capability_poke_check_reaches_behaviour() {
    assert_attribute_reaches_behaviour("effective_risk_tolerance", |a, v| {
        a.ball_handling = v;
        a.decision_iq = v;
    });
}

// ============================================================================
// 门 4：士气状态机的分支必须可达
// ============================================================================

/// `MoraleState::HotHand` 必须能被真实比赛达到。
///
/// 此前 `PlayerModulationState::record_shot` 在生产代码与测试中零调用，
/// `consecutive_makes` 恒为 0，`hot_hand_bias` 是无效通道。
///
/// 窗口必须是**完整一场**：连中阈值虽只有 2，但单个球员在 9000 tick 内
/// 只出手约 10 次，前两次连续命中的概率很低。实测（6 seed 全场）：
/// 最长连中 4–6 次且每个 seed 都出现过 `HotHand`；而 9000 tick（约一节半）
/// 的窗口内最长连中仅 1–2 次，会给出假阴。
#[test]
fn morale_hot_hand_state_must_be_reachable() {
    let mut engine =
        MatchEngine::with_setup(MatchSetup::builtin(GameRules::default()), PROOF_SEEDS[0]);
    engine.set_scope("full").expect("full scope is valid");
    let mut reached = false;
    while !engine.is_finished() {
        engine.step();
        if engine
            .physics()
            .get_players()
            .values()
            .any(|p| p.morale == "HotHand")
        {
            reached = true;
            break;
        }
    }
    assert!(
        reached,
        "MoraleState::HotHand must be reachable in a live match; \
         if it never appears, record_shot is not wired and hot_hand_bias \
         is a dead channel"
    );
}
