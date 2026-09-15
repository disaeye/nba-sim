//! 归因正确性回归（round-5 审计发现的 98 条 `TURNOVER_ATTRIBUTION` Hard 的根因）。
//!
//! ## 缺陷
//!
//! 实测 seed 42 full：32 个 `TURNOVER_VIOLATION` 回合中 **15 个**窗口内没有任何
//! `VIOLATION` 事实，而窗口尾部是 `PASS_DROPPED` / `PASS_TIPPED` /
//! `LOOSE_BALL_SECURED`。即**回合终结原因与窗口内事实类别不符**。
//!
//! 根因（`match_engine.rs::start_out_of_bounds_transition`）：松球出界时无条件
//! 以 `PossessionEndCause::TurnoverViolation` 结算，**覆盖**了本已成立的
//! 传球失误原因；且 `OUT_OF_BOUNDS` 在事件流中出现 **0 次**——出界这一事实
//! 在因果账本里不存在。
//!
//! 另一个实例：`PossessionEndCause::PeriodEnd` 在 domain 中已定义，但引擎
//! **从未 emit**；跨节回合（实测 3 个/场，其中一个 41.6s）被记在上一节名下。
//!
//! ## 本测试的判定口径
//!
//! 不是"字段非空"（D0.1 只做到这一层，因此 98 条错配全部漏过），而是
//! **"终结原因类别必须与窗口内存在的事实类别一致"**。
//!
//! 每个种子在完整全场模拟中逐回合断言，失败时打印回合号、终结原因与窗口
//! 内实际出现的事件种类，便于定位。

use nba_engine::MatchEngine;

/// 一个回合窗口内观察到的事件种类计数。
#[derive(Default, Debug)]
struct WindowKinds {
    violation: usize,
    dropped: usize,
    tipped: usize,
    steal: usize,
    loose_secured: usize,
    made: usize,
    shot_release: usize,
    /// 该窗口是否跨越了节边界（period 变化）。
    crossed_period: bool,
}

/// 判定口径：终结原因 ∈ {Steal, PassTipped, PassDropped, Violation, LooseBall}
/// 时，窗口内必须有同类别的事实。
fn classify_mismatch(terminal: &str, w: &WindowKinds) -> Option<String> {
    let ok = match terminal {
        "TURNOVER_STEAL" => w.steal > 0,
        "TURNOVER_PASS_TIPPED" => w.tipped > 0,
        "TURNOVER_PASS_DROPPED" => w.dropped > 0,
        "TURNOVER_VIOLATION" => w.violation > 0,
        "TURNOVER_LOOSE_BALL" => w.loose_secured > 0,
        _ => true,
    };
    if ok {
        None
    } else {
        Some(format!(
            "terminal {terminal} but window facts are \
             {{violation:{}, dropped:{}, tipped:{}, steal:{}, loose:{}}}",
            w.violation, w.dropped, w.tipped, w.steal, w.loose_secured
        ))
    }
}

fn audit_seed(seed: u64) -> Vec<(u64, String)> {
    let mut engine = MatchEngine::new(seed);
    // 结构性质（归因类别一致性）与比赛长度无关；用 `1q` 而非 `full`
    // 使单个测试从约 85s 降到约 20s，同时保留 8 种子判定口径
    // （quality.md §2.5）。分布类指标才需要 full。
    engine.set_scope("1q").expect("1q scope is valid");

    let mut mismatches: Vec<(u64, String)> = Vec::new();
    let mut w = WindowKinds::default();
    let mut idx: u64 = 0;
    let mut prev_period: u32 = 0;
    let mut ticks = 0usize;

    while !engine.is_finished() && ticks < 300_000 {
        let tick = engine.step();
        let period = tick.frame.period;
        if prev_period != 0 && period != prev_period {
            w.crossed_period = true;
        }
        prev_period = period;
        for ev in &tick.frame.event_log {
            match ev.kind.as_str() {
                "VIOLATION" => w.violation += 1,
                "PASS_DROPPED" => w.dropped += 1,
                "PASS_TIPPED" => w.tipped += 1,
                "STEAL" => w.steal += 1,
                "LOOSE_BALL_SECURED" => w.loose_secured += 1,
                // 带球被切掉是 TURNOVER_LOOSE_BALL 的**原因事实**（round-14/15）：
                // 球被拨离后可能直接出界或被防守方收下，此时没有
                // LOOSE_BALL_SECURED，但 BALL_POKED_LOOSE 事实仍在。
                "BALL_POKED_LOOSE" => w.loose_secured += 1,
                "SCORE" => w.made += 1,
                "SHOT_RELEASE" => w.shot_release += 1,
                "POSSESSION_SUMMARY" => {
                    // POSSESSION_SUMMARY 关闭窗口；其前的归本回合。
                    let terminal = ev
                        .data
                        .as_ref()
                        .and_then(|d| d.get("PossessionSummary"))
                        .and_then(|s| s.get("terminal_event"))
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    if let Some(reason) = classify_mismatch(&terminal, &w) {
                        mismatches.push((idx, reason));
                    }
                    idx += 1;
                    w = WindowKinds::default();
                }
                _ => {}
            }
        }
        ticks += 1;
    }
    mismatches
}

#[test]
fn turnover_terminal_reason_matches_window_facts() {
    // 8 个种子（quality.md §2.5 判定口径 ≥8）。
    let seeds: [u64; 8] = [42, 1, 7, 100, 999, 31337, 2024, 555];
    let mut total = 0usize;
    for seed in seeds {
        let mism = audit_seed(seed);
        if !mism.is_empty() {
            total += mism.len();
            eprintln!("seed {seed}: {} mismatched possessions", mism.len());
            for (i, reason) in mism.iter().take(5) {
                eprintln!("   possession {i}: {reason}");
            }
        }
    }
    assert_eq!(
        total, 0,
        "turnover terminal cause must match the fact category present in its own \
         possession window (round-5 audit: 98 Hard defects across 8 seeds; root cause \
         is start_out_of_bounds_transition overwriting the existing turnover cause)"
    );
}

#[test]
fn possession_windows_do_not_span_period_boundaries() {
    // `PossessionEndCause::PeriodEnd` 已定义且在节末发射：跨节回合被显式结算，
    // 不再记在上一节名下（修复前实测 3 个/场，最长 41.6s，越出回合时长上界）。
    //
    // 判定口径：周期变化发生时，若自上一个回合总结以来**又积累了事实**，
    // 说明有回合跨过了节边界。仅凭“period 变了”会误报——节末总结在
    // period 仍是旧值的 tick 发出，而新周期从下一 tick 才开始。
    let seeds: [u64; 8] = [42, 1, 7, 100, 999, 31337, 2024, 555];
    let mut spanning = 0usize;
    for seed in seeds {
        let mut engine = MatchEngine::new(seed);
        engine.set_scope("1q").unwrap();
        let mut prev_period: u32 = 0;
        let mut ticks = 0usize;
        let mut idx = 0u64;
        // 自上一个 POSSESSION_SUMMARY 以来是否又发生了事实。
        let mut facts_since_summary = false;

        while !engine.is_finished() && ticks < 300_000 {
            let tick = engine.step();
            let period = tick.frame.period;
            if prev_period != 0 && period != prev_period && facts_since_summary {
                spanning += 1;
                eprintln!("seed {seed}: possession {idx} was still open when the period changed");
            }
            prev_period = period;
            for ev in &tick.frame.event_log {
                match ev.kind.as_str() {
                    "POSSESSION_SUMMARY" => {
                        idx += 1;
                        facts_since_summary = false;
                    }
                    // 节间布置与阶段迁移不属于“回合仍在进行”。
                    "PLACEMENT_APPLIED" | "PHASE_TRANSITION" => {}
                    _ => facts_since_summary = true,
                }
            }
            ticks += 1;
        }
    }
    assert_eq!(
        spanning, 0,
        "a possession must not span a period boundary: it must be settled by \
         PossessionEndCause::PeriodEnd (defined in domain/event.rs, emitted in \
         finish_period)"
    );
}
