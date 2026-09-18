//! 接线证明（Wiring Proof）：把「已落地」的声明转成机器可判定的证据。
//!
//! ## 为什么需要这个文件
//!
//! 本轮审计发现的目标与设计文档之间存在系统性偏差，其共同形态是
//! **用声明代替证据**：
//!
//! 1. `UNIMPLEMENTED_RULE_FIELDS = &[]` 宣告「零消费字段已清零」，
//!    但 `effective_post_defense_physicality` / `effective_transition_leakout_chance`
//!    在主干中零调用 —— 清单为空不等于字段被消费；
//! 2. `let _ = PerceptionSystem::evaluate(&self.world)` 看起来「接了感知系统」，
//!    但 `world.transforms` 从未被真实引擎填充，`n == 0`，返回值被丢弃；
//! 3. `test_drive_finish_range_rules_perturbation` 只比较两个字面量 8.0 != 22.0，
//!    从不运行模拟 —— 测试通过不构成接线证据。
//!
//! 本文件用**行为级断言**替代上述三种声明式断言：每一条都要求
//! 「改一个输入 → 真实模拟输出必须变」，否则红。
//!
//! ## 纪律
//!
//! 本文件中的断言一旦变红，**不得**通过放宽阈值或改写测试来消除；
//! 必须先判断是「接线真的坏了」还是「契约本身需要重新裁定」，
//! 前者修代码，后者走 `docs/decisions.md`。

use nba_domain::GameRules;
use nba_engine::MatchEngine;

/// 跑一段固定 tick 数的模拟并返回可分发的行为指纹。
///
/// 指纹刻意取「会被下游消费的事实」而非内部字段：比分、投篮出手数、
/// 球位置、以及事件种类序列。若某条接线真实生效，扰动该输入必然
/// 改变其中至少一项。
fn behavior_fingerprint(engine: &mut MatchEngine, ticks: usize) -> BehaviorFingerprint {
    let mut events: Vec<String> = Vec::new();
    for _ in 0..ticks {
        let tick = engine.step();
        for e in &tick.frame.event_log {
            events.push(e.kind.clone());
        }
    }
    let snap = engine.engine_snapshot();
    BehaviorFingerprint {
        home_score: snap.game.home_score,
        away_score: snap.game.away_score,
        fg2_attempts: snap.box_score.fg2_attempts,
        fg3_attempts: snap.box_score.fg3_attempts,
        ft_attempts: snap.box_score.ft_attempts,
        turnovers: snap.box_score.turnovers,
        ball_x: snap.ball.pos_3d.0.x,
        ball_y: snap.ball.pos_3d.0.y,
        events,
    }
}

#[derive(Debug)]
struct BehaviorFingerprint {
    home_score: u32,
    away_score: u32,
    fg2_attempts: u32,
    fg3_attempts: u32,
    ft_attempts: u32,
    turnovers: u32,
    ball_x: f32,
    ball_y: f32,
    events: Vec<String>,
}

impl BehaviorFingerprint {
    /// 两个指纹是否可观察到差异（用于「接线必须改变行为」的判据）。
    fn differs_from(&self, other: &Self) -> bool {
        self.home_score != other.home_score
            || self.away_score != other.away_score
            || self.fg2_attempts != other.fg2_attempts
            || self.fg3_attempts != other.fg3_attempts
            || self.ft_attempts != other.ft_attempts
            || self.turnovers != other.turnovers
            || (self.ball_x - other.ball_x).abs() > 1e-4
            || (self.ball_y - other.ball_y).abs() > 1e-4
            || self.events != other.events
    }
}

/// 多 seed 上跑同一组规则，返回指纹集合（降低单 seed 采样偶然性）。
fn fingerprints_with_rules(
    rules: GameRules,
    seeds: &[u64],
    ticks: usize,
) -> Vec<BehaviorFingerprint> {
    seeds
        .iter()
        .map(|&seed| {
            let mut engine = MatchEngine::with_rules(seed, rules.clone());
            behavior_fingerprint(&mut engine, ticks)
        })
        .collect()
}

const PROOF_SEEDS: [u64; 4] = [42, 1, 7, 100];
const PROOF_TICKS: usize = 3000;

// ============================================================================
// 门 1：规则扰动必须改变真实模拟输出
// ============================================================================

/// `drive_finish_range_ft` 必须经规则通道影响真实比赛，而不是只在结构体里存着。
///
/// 取代原先的同义反复断言 `assert_ne!(8.0, 22.0)`。
#[test]
fn rules_wiring_drive_finish_range_changes_simulation() {
    let mut short = GameRules::default();
    short.tactics.drive_finish_range_ft = 8.0;
    let mut long = GameRules::default();
    long.tactics.drive_finish_range_ft = 22.0;

    let a = fingerprints_with_rules(short, &PROOF_SEEDS, PROOF_TICKS);
    let b = fingerprints_with_rules(long, &PROOF_SEEDS, PROOF_TICKS);

    let changed = a
        .iter()
        .zip(b.iter())
        .filter(|(x, y)| x.differs_from(y))
        .count();
    assert!(
        changed >= 3,
        "drive_finish_range_ft must change real simulation behaviour on at least 3/4 seeds, \
         but only {changed}/4 differed — the field is stored but not consumed \
         (structural assertion assert_ne!(8.0, 22.0) cannot detect this)"
    );
}

/// Clutch 规则必须经规则通道影响真实比赛。
///
/// 取代原先只读回默认值的 `test_clutch_rules_wired_to_engine`。
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

    let a = fingerprints_with_rules(baseline, &PROOF_SEEDS, PROOF_TICKS);
    let b = fingerprints_with_rules(perturbed, &PROOF_SEEDS, PROOF_TICKS);

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
// 门 2：感知系统不得在空世界上运行
// ============================================================================

/// `MatchEngine` 驱动 `MatchWorld` 时，球员组件必须被真实填充。
///
/// 当前实现只同步 clock/ledger/ball 等标量，`world.transforms` 恒为空，
/// 导致 `PerceptionSystem::evaluate` 的循环体一次都不执行（n == 0），
/// 返回的全空结果被 `let _ =` 丢弃 —— 「接了感知系统」是假的。
#[test]
fn perception_system_must_run_on_populated_world() {
    let mut engine = MatchEngine::new(42);
    for _ in 0..200 {
        engine.step();
    }
    let world = engine.world();
    assert_eq!(
        world.player_count(),
        10,
        "MatchWorld must hold all 10 on-court players so that PerceptionSystem \
         iterates real entities; player_count()==0 means every spatial kernel \
         (Voronoi openness, contest density, weak-side detection) computes on an \
         empty world and its output is discarded via `let _ =`"
    );
}

/// 感知系统的输出必须被真实消费：不同防守空间形态必须产生不同的感知结果，
/// 且该结果必须出现在对比赛有影响的下游。
#[test]
fn spatial_perception_output_must_reach_behaviour() {
    use nba_engine::world::PerceptionSystem;

    let mut engine = MatchEngine::new(42);
    for _ in 0..200 {
        engine.step();
    }
    let p = PerceptionSystem::evaluate(engine.world());
    assert_eq!(
        p.defensive_contest_density.len(),
        10,
        "perception must produce one density sample per on-court player; \
         an empty vector means the system observed zero entities"
    );
    assert!(
        p.voronoi_openness.iter().any(|&o| o > 0.0),
        "at least one player must have non-zero Voronoi openness in a live \
         possession; all-zero indicates the kernel never ran on real positions"
    );
}

// ============================================================================
// 门 3：能力函数必须被主干消费，而非仅被公式单测覆盖
// ============================================================================

/// 六个 D27 能力函数中每一个的输入属性，都必须能改变真实模拟输出。
///
/// 取代「清单为空即视为清零」的声明式断言（`UNIMPLEMENTED_RULE_FIELDS`）。
/// 做法：把全队属性设到极端低 / 极端高，比较真实模拟指纹。
fn assert_attribute_reaches_behaviour(
    label: &str,
    mutate: impl Fn(&mut nba_domain::PlayerAttributes, f32),
) {
    // 通过 setup 直接改写球员属性，跑真实模拟。
    let run = |value: f32| -> Vec<BehaviorFingerprint> {
        PROOF_SEEDS
            .iter()
            .map(|&seed| {
                let rules = GameRules::default();
                let mut setup = nba_engine::MatchSetup::builtin(rules);
                for team in [&mut setup.home_team, &mut setup.away_team] {
                    for p in team.players.iter_mut() {
                        mutate(&mut p.attributes, value);
                    }
                }
                let mut engine = MatchEngine::with_setup(setup, seed);
                behavior_fingerprint(&mut engine, PROOF_TICKS)
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

// ============================================================================
// 门 4：士气状态机的分支必须可达
// ============================================================================

/// `MoraleState::HotHand` 必须能被真实比赛达到。
///
/// 取代「变体存在即视为接线」的声明式判断。此前
/// `PlayerModulationState::record_shot` 在生产代码与测试中零调用，
/// `consecutive_makes` 恒为 0，`hot_hand_bias` 是死通道；
/// 而 `MoraleState::Clutch` 没有任何赋值点（已删除该变体）。
#[test]
fn morale_hot_hand_state_must_be_reachable() {
    let mut engine = MatchEngine::with_setup(
        nba_engine::MatchSetup::builtin(GameRules::default()),
        PROOF_SEEDS[0],
    );
    let mut reached = false;
    // 连中阈值默认 2，因此在一场比赛的前半段就应出现。
    for _ in 0..PROOF_TICKS * 3 {
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
