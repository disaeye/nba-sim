//! D27 规则系数因果闭环验收（`plan.md` §8.3）。
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
//! 3000 tick，要求至少 3/4 的模拟指纹发生变化。指纹取会被下游消费的事实
//! （比分、出手数、失误、球位置、事件种类序列），与 `wiring_proof.rs` 同一口径。
//!
//! ## 纪律
//!
//! 断言一旦变红，不得通过放宽阈值或改写测试消除：先判断是「系数没进计算」
//! 还是「契约需要重新裁定」，前者修代码，后者走 `docs/decisions.md`。

use nba_domain::GameRules;
use nba_engine::{MatchEngine, MatchSetup};

const SEEDS: [u64; 4] = [42, 1, 7, 100];
const TICKS: usize = 15000;

#[derive(Debug)]
struct Fingerprint {
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

impl Fingerprint {
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

fn run(rules: GameRules) -> Vec<Fingerprint> {
    SEEDS
        .iter()
        .map(|&seed| {
            let mut engine = MatchEngine::with_setup(MatchSetup::builtin(rules.clone()), seed);
            let mut events: Vec<String> = Vec::new();
            for _ in 0..TICKS {
                let tick = engine.step();
                for e in &tick.frame.event_log {
                    events.push(e.kind.clone());
                }
            }
            let snap = engine.engine_snapshot();
            Fingerprint {
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
        })
        .collect()
}

/// 扰动一个规则系数，要求至少 3/4 的 seed 行为改变。
fn assert_rule_coefficient_reaches_behaviour(label: &str, mutate: impl Fn(&mut GameRules)) {
    let baseline = run(GameRules::default());
    let mut perturbed_rules = GameRules::default();
    mutate(&mut perturbed_rules);
    // 系数必须真的被改动，否则断言退化为恒真（自检）。
    assert_ne!(
        format!("{perturbed_rules:?}"),
        format!("{:?}", GameRules::default()),
        "{label}: 扰动函数未改动任何规则字段"
    );
    let perturbed = run(perturbed_rules);

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

#[test]
fn capability_transition_leakout_gain_reaches_behaviour() {
    // 基线阈值为 0.36，而内置阵容的最大快下机会约 0.46，因此基线**会**走快下。
    // 把阈值抬到 1.0 则禁用该分支，选择回到「交给控卫组织」：
    // 这才真正改变受传球人，而把阈值调到 0.0 不会（0.46 已超过两者）。
    assert_rule_coefficient_reaches_behaviour("capability.transition_leakout_threshold", |r| {
        r.capability.transition_leakout_threshold = 1.0;
    });
}
