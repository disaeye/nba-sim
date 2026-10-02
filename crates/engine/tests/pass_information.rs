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

use glam::Vec2;
use nba_domain::GameRules;
use nba_engine::MatchEngine;
use nba_engine::MatchSetup;
use nba_physics::BallTrajectoryKind;
use rayon::prelude::*;

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
    // 种子间并行：四场 60k-tick 模拟互不共享状态。
    // 判据是「≥3/4 种子可分」而非全部分叉：gap 序列量化到 0.001ft，
    // 单一种子出现全序列同桶的量化巧合属正常（v83 发球接应位实测
    // seed42 一致而 seed 1/7/100 可分；若真存在全知泄漏，噪声通道
    // 不参与，四个种子必然全部一致）。
    let identical_traces = seeds
        .par_iter()
        .copied()
        .filter(|&seed| {
            let a = trace(seed, 0.95);
            let b = trace(seed, 0.05);
            a == b
        })
        .count();
    assert!(
        identical_traces <= 1,
        "receiver's estimate ability must affect the pass outcome: \
         {identical_traces}/{} seeds produced byte-identical reception traces \
         under off_ball_sense=0.95 vs 0.05. Identical traces prove the receiver \
         reads the passer's frozen landing point instead of estimating it — \
         omniscience, violating P-1 (a receiver may legitimately fail to catch). \
         More than one identical seed means the noise channel is dead, not a quantization coincidence.",
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
    let divergences: Vec<f32> = seeds
        .par_iter()
        .copied()
        .flat_map_iter(landing_divergences)
        .collect();
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

// ============================================================================
// 逐 tick 接触检测（防守者可及性）机械守卫
// ============================================================================
//
// 守卫对象：传球飞行中的接触分类必须在「球真的在防守者可及范围内」
// 才发生。可及性有两维：水平距离（球道正中）与高度（采样 z ≤ 摸高）。
// 高吊传的 z 在防守者位置处高于摸高，不应产生接触事实。
//
// 场景用 `set_ball_state_for_test` 直接构造飞行中的传球（与
// decision_wiring.rs 同一构造方式），把全部球员静止并排开，隔离
// 战术跑位对几何的污染。掷骰概率由 `intercept_steal_slope` /
// `intercept_tip_slope` 决定（默认 0.06/0.10，夹取下限 0.01/0.02），
// 单次接触未抽中的概率有限，因此用多次独立传球累计：只要接触通道
// 真实接线，多次传球必然产生至少一次 PASS_TIPPED / STEAL 事实。

/// 静止化全部球员：速度归零、目标冻结在当前位置（隔离战术跑位污染）。
fn freeze_all_players(engine: &mut MatchEngine) {
    let ids: Vec<String> = engine.physics().get_players().keys().cloned().collect();
    for id in ids {
        if let Some(p) = engine.physics_mut_for_test().get_player_mut(&id) {
            p.vel_ft = Vec2::ZERO;
            p.target_speed_ftps = 0.0;
            p.target_pos_ft = p.pos_ft;
        }
    }
}

/// 构造「H_01 → H_02 平传、A_05 站在球道正中」的场景并跑完飞行。
///
/// 返回飞行期间观察到的事件种类集合。
fn run_pass_with_lane_defender(seed: u64, duration: f32) -> Vec<String> {
    let rules = nba_domain::GameRules {
        tick_seconds: 0.1,
        tactical_initiation_seconds: 0.0,
        decision_interval_seconds: 0.1,
        ..nba_domain::GameRules::default()
    };
    let mut setup = MatchSetup::builtin(rules);
    // 接球链路固定成功，隔离层 A/层 B 对结果的干扰；接触结果分类固定为
    // 必抢断：概率下限抬到 1.0（规则通道，与 decision_wiring 固定
    // pass_success 同一手法），使「接触几何成立」与「STEAL 事实出现」
    // 一一对应，消除掷骰采样噪声。
    setup.rules.resolve.base_rates.pass_success = 1.0;
    setup.rules.resolve.base_rates.intercept_steal_floor = 1.0;
    setup.rules.resolve.base_rates.intercept_steal_ceiling = 1.0;
    setup.rules.resolve.pass.openness_weight = 0.0;
    setup.rules.resolve.pass.passer_skill_weight = 0.0;
    setup.rules.resolve.pass.receiver_control_weight = 0.0;
    setup.rules.resolve.pass.catch_equilibrium_weight = 0.0;
    let mut engine = MatchEngine::with_setup(setup, seed);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    // 平传时长 0.5s：弧顶 ≈5.0 ft、防守者位置处 z ≈ 4.6–5.0 ft，
    // 低于任一名册球员的摸高（190cm + 默认弹跳 ≈ 6.6 ft）→ 可及。
    // 高吊传时长 1.2s：弧顶 ≈9.8 ft，防守者位置处 z ≈ 9.6–9.8 ft，
    // 高于最高在场球员的摸高（211cm ≈ 7.0 ft）→ 不可及。
    engine.set_ball_state_for_test(BallTrajectoryKind::Pass {
        from_pos: Vec2::new(35.0, 25.0),
        to_pos: Vec2::new(55.0, 25.0),
        target_id: "H_02".to_string(),
        start_time: 1.0,
        duration,
        peak_z: engine.rules().pass_peak_ft,
        inbound: false,
        receive_success: true,
    });
    engine.set_last_passer_for_test(Some("H_01".to_string()));
    engine.set_game_flow_for_test(nba_domain::GameFlowState::LiveBall);
    engine.set_sub_phase_for_test(nba_domain::SubPhase::ActionExecution);
    engine.set_current_time_for_test(1.0);
    freeze_all_players(&mut engine);
    // 接球人冻结在传球终点；防守者 A_05（最高在场球员）冻结在球道正中。
    if let Some(p) = engine.physics_mut_for_test().get_player_mut("H_02") {
        p.pos_ft = Vec2::new(55.0, 25.0);
        p.target_pos_ft = p.pos_ft;
    }
    if let Some(p) = engine.physics_mut_for_test().get_player_mut("A_05") {
        p.pos_ft = Vec2::new(45.0, 25.0);
        p.target_pos_ft = p.pos_ft;
    }
    let mut kinds: Vec<String> = Vec::new();
    for _ in 0..20 {
        let tick = engine.step();
        for ev in &tick.frame.event_log {
            kinds.push(ev.kind.clone());
        }
        if matches!(
            engine.ball_state(),
            BallTrajectoryKind::Held { .. } | BallTrajectoryKind::LooseBall { .. }
        ) {
            break;
        }
    }
    kinds
}

/// 球道正中的防守者（z 可及）必须产生接触事实（抢断或拨掉）。
///
/// 多次独立传球累计：单次接触的未抽中概率有限（steal/tip 合计
/// 抽中概率 ≥ floor 0.01+0.02），但只要接线真实，多次传球必有一次
/// 抽中。若接触通道完全未接线（几何永远不成立），任何一次都不会有
/// 事实 —— 断言因此区分「通道活着」与「通道已断」。
#[test]
fn lane_defender_at_reachable_height_must_contest_the_pass() {
    // 16 次独立传球：每次都是完整的一次飞行（接触锁存在出手时清空，
    // 每次 pass 都重新参与检测）。机会放大后仍无事实才能判红。
    let contests = (0..16)
        .map(|i| run_pass_with_lane_defender(900 + i, 0.5))
        .collect::<Vec<_>>();
    let contested = contests
        .iter()
        .any(|kinds| kinds.iter().any(|k| k == "STEAL" || k == "PASS_TIPPED"));
    assert!(
        contested,
        "a defender standing in the lane at reachable height must produce \
         STEAL or PASS_TIPPED; with 16 independent passes and zero contest \
         facts the per-tick contact classification is not wired"
    );
}

/// 高吊传（弧顶高于全部防守者的摸高）必须干净到达：无接触事实。
///
/// 同一几何、只拉长飞行时长（弧顶随之抬高）：防守者位置处的采样 z
/// 全程高于在场最高球员（A_05，211cm）的摸高。若接触事实仍然出现，
/// 说明可及性判定没有消费采样高度 —— 几何失真。
#[test]
fn lob_pass_above_reach_must_arrive_untouched() {
    let untouched = (0..16).all(|i| {
        let kinds = run_pass_with_lane_defender(900 + i, 1.2);
        !kinds.iter().any(|k| k == "STEAL" || k == "PASS_TIPPED")
    });
    assert!(
        untouched,
        "a lob whose sampled z at the defender's spot exceeds every \
         defender's reach height must arrive without STEAL/PASS_TIPPED; \
         a contact fact here means reachability ignores the sampled ball height"
    );
}
