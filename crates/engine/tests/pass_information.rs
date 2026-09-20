//! 传球有限信息原则（P-1）的机械守卫。
//!
//! ## 原则
//!
//! > 传球人预估传球路线并传出；接球人**同样只能预估**该路线。
//! > 预估可能错 ⇒ **接球人可能接不到**。
//!
//! 全知全能（接球人被告知精确落点、因而必然到位）**违反真实性**：
//! 真实比赛里，大个弧顶策应给小个传突破提前量，小个完全可能接不到。
//!
//! ## 当前违反
//!
//! `match_engine` 把 `BallState::Pass.to_pos`（传球人的冻结意图）直接注入
//! 接球人的运动目标：
//!
//! ```ignore
//! BallTrajectoryKind::Pass { target_id, to_pos, .. } => Some((target_id.clone(), *to_pos)),
//! // ... 随后 → receive_approach(*rx_pos, &player_id) → 接球人向冻结落点收敛
//! ```
//!
//! 接球人因此获得了传球人的私有意图，**必然到位**。
//!
//! ## 本守卫的判定
//!
//! 判定采用**行为断言**：让接球人与传球人的估计不一致，观察是否产生"接不到"。
//! （静态 grep 只能证明"没有直读"，不能证明"确有预估"。）
//!
//! 用规则通道制造分歧：把接球人的 `off_ball_sense` 压到极低（预估能力差），
//! 同时让传球人按满速传提前量。若实现是"接球人直读冻结落点"，则无论
//! `off_ball_sense` 如何都不会出现空间分离 ⇒ 测试红。

use nba_domain::GameRules;
use nba_engine::MatchEngine;
use nba_engine::MatchSetup;

fn mean(v: &[f32]) -> f32 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f32>() / v.len() as f32
    }
}

#[test]
fn receiver_must_estimate_not_know_the_frozen_landing() {
    // 判定必须是**确定性**的：用同一批种子、只改接球人的预估能力。
    // 第一次尝试用「catches 计数差异」判据，实测 304 vs 300 —— 这是
    // 跨种子 RNG 噪声，不是效应（`off_ball_sense` 在接球路径上消费点为 0）。
    // 因此改为**同种子逐样本配对比较**：同一 seed 下，仅 `off_ball_sense`
    // 不同，若逐样本结果完全一致，则接球过程与该能力无关 ⇒ 全知全能。
    let seeds: [u64; 4] = [42, 1, 7, 100];
    let mut identical_traces = 0;
    for seed in seeds {
        let a = trace(seed, 0.95);
        let b = trace(seed, 0.05);
        if a == b {
            identical_traces += 1;
        }
    }
    assert_eq!(
        identical_traces,
        0,
        "receiver's estimate ability must affect the pass outcome: \
         {identical_traces}/{} seeds produced byte-identical reception traces \
         under off_ball_sense=0.95 vs 0.05. Identical traces prove the receiver \
         reads the passer's frozen landing point instead of estimating it — \
         omniscience, violating P-1 (a receiver may legitimately fail to catch).",
        seeds.len()
    );
}

/// 逐样本记录：每次接球时「球位置 − 接球人位置」的量化距离序列。
fn trace(seed: u64, receiver_sense: f32) -> Vec<i64> {
    let rules = GameRules::default();
    let mut setup = MatchSetup::builtin(rules.clone());
    for p in setup.away_team.players.iter_mut() {
        p.attributes.off_ball_sense = receiver_sense;
    }
    let mut e = MatchEngine::with_setup(setup, seed);
    e.set_scope("1q").unwrap();
    let mut out = Vec::new();
    let mut ticks = 0usize;
    while !e.is_finished() && ticks < 60_000 {
        let t = e.step();
        let frame = &t.frame;
        if frame.event_log.iter().any(|ev| ev.kind == "PASS_RECEIVED") {
            if let Some(holder) = &frame.ball.holder_id {
                if let Some(p) = frame.players.iter().find(|p| &p.id == holder) {
                    let bx = frame.ball.x * rules.court.width_ft;
                    let by = frame.ball.y * rules.court.height_ft;
                    let px = p.x * rules.court.width_ft;
                    let py = p.y * rules.court.height_ft;
                    let gap = ((bx - px).powi(2) + (by - py).powi(2)).sqrt();
                    out.push((gap * 1000.0).round() as i64);
                }
            }
        }
        ticks += 1;
    }
    out
}

/// 第二道门：接球人的**落点估计**必须与传球人的意图不同（层 A 生效的证据）。
///
/// ## 与第一版的口径更正
///
/// 第一版用「球位置 vs 接球人位置」的到达差判定，期望它非恒定。但在
/// round-10 的实现里，接球成功时球会被**收到接球人身上**（持球锚点，这是
/// 物理事实，也满足 `BALL_WITH_HOLDER ≤ leash`），因此该差值恒为 0 ——
/// 门的判据本身失效了。
///
/// 正确的判据是引擎发布的 `PASS_LANDING_CORRECTED` 事实：它记录
/// 「传球人冻结意图 vs 接球人实际到达」的差异。该差异 > 0 即证明
/// 接球人**没有**直读传球人的意图，按自己的估计跑位。
#[test]
fn receiver_landing_estimate_diverges_from_passer_intent() {
    let seeds: [u64; 4] = [42, 1, 7, 100];
    let mut divergences: Vec<f32> = Vec::new();
    for seed in seeds {
        divergences.extend(landing_divergences(seed));
    }
    eprintln!(
        "PASS_LANDING_CORRECTED: n={} mean={:.3} ft max={:.3} ft",
        divergences.len(),
        mean(&divergences),
        divergences.iter().cloned().fold(0.0f32, f32::max)
    );
    assert!(
        !divergences.is_empty(),
        "the engine must publish PASS_LANDING_CORRECTED when the receiver's \
         arrival differs from the passer's frozen intent"
    );
    assert!(
        divergences.iter().any(|d| *d > 0.01),
        "at least some receptions must show a non-zero landing divergence; \
         an all-zero divergence means the receiver reads the passer's intent \
         directly (omniscience), violating P-1"
    );
}

/// 收集一场比赛中所有 `PASS_LANDING_CORRECTED` 的偏差量。
fn landing_divergences(seed: u64) -> Vec<f32> {
    let rules = GameRules::default();
    let setup = MatchSetup::builtin(rules.clone());
    let mut e = MatchEngine::with_setup(setup, seed);
    e.set_scope("1q").unwrap();
    let mut out = Vec::new();
    let mut ticks = 0usize;
    while !e.is_finished() && ticks < 60_000 {
        let t = e.step();
        for ev in &t.frame.event_log {
            if ev.kind == "PASS_LANDING_CORRECTED" {
                if let Some(d) = ev
                    .data
                    .as_ref()
                    .and_then(|v| v.get("PassLandingCorrected"))
                    .and_then(|v| v.get("divergence_ft"))
                    .and_then(|v| v.as_f64())
                {
                    out.push(d as f32);
                }
            }
        }
        ticks += 1;
    }
    out
}
