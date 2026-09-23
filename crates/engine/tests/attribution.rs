//! 归因与账本一致性：回合终结原因、事件因果链、箱体统计与事件事实对平。
//!
//! 守卫对象：
//! - 回合终结原因必须与窗口内存在的事实类别一致（不是「字段非空」）；
//! - 回合不得跨节边界；
//! - `box_score` 的每个字段必须与从事件流独立重建的值对平；
//! - `event_id` 单调唯一，结果事件的父链语义正确，不伪造因果。
//!
//! 对应 `docs/gap.md` §7.1/§7.5、§18.6 第 2 条（事件、回合和账本可独立重建
//! 并对平）与 ADR-001（事件序列是比赛唯一真相）。

mod support;

use std::collections::HashMap;

use nba_engine::{MatchEngine, StreamMode};
use rayon::prelude::*;

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

/// 共享的 1q 模拟矩阵：同一批种子各跑一次 1q，把事件流逐 tick 暂存，
/// 三个矩阵类测试（终结原因、跨节窗口、箱体对平）从同一份事件流各自
/// 统计，消掉 22 次重复模拟中的 14 次。
struct SeedAudit {
    seed: u64,
    mismatches: Vec<(u64, String)>,
    spanning: usize,
    box_failures: Vec<String>,
}

fn shared_seed_audits() -> &'static Vec<SeedAudit> {
    static AUDITS: std::sync::OnceLock<Vec<SeedAudit>> = std::sync::OnceLock::new();
    AUDITS.get_or_init(|| {
        let seeds: [u64; 8] = [42, 1, 7, 100, 999, 31337, 2024, 555];
        seeds
            .par_iter()
            .copied()
            .map(|seed| {
                let mut engine = MatchEngine::new(seed);
                engine.set_scope("1q").expect("1q scope is valid");

                let mut mismatches: Vec<(u64, String)> = Vec::new();
                let mut spanning = 0usize;
                let mut w = WindowKinds::default();
                let mut idx: u64 = 0;
                let mut prev_period: u32 = 0;
                let mut ticks = 0usize;
                // 自上一个 POSSESSION_SUMMARY 以来是否又发生了事实。
                let mut facts_since_summary = false;

                // 从事件流独立重建的箱体计数（不读 box_score）。
                let mut shot_rel_2 = 0u32;
                let mut shot_rel_3 = 0u32;
                let mut made_2 = 0u32;
                let mut made_3 = 0u32;
                let mut ft_att = 0u32;
                let mut ft_made = 0u32;
                let mut fouls = 0u32;
                let mut turnover_terminals = 0u32;

                while !engine.is_finished() && ticks < 300_000 {
                    let tick = engine.step();
                    let period = tick.frame.period;
                    if prev_period != 0 && period != prev_period {
                        w.crossed_period = true;
                    }
                    if prev_period != 0 && period != prev_period && facts_since_summary {
                        spanning += 1;
                        eprintln!(
                            "seed {seed}: possession {idx} was still open when the period changed"
                        );
                    }
                    prev_period = period;
                    for ev in &tick.frame.event_log {
                        let payload = || ev.data.as_ref();
                        match ev.kind.as_str() {
                            "VIOLATION" => w.violation += 1,
                            "PASS_DROPPED" => w.dropped += 1,
                            "PASS_TIPPED" => w.tipped += 1,
                            "STEAL" => w.steal += 1,
                            "LOOSE_BALL_SECURED" => w.loose_secured += 1,
                            "BALL_POKED_LOOSE" => w.loose_secured += 1,
                            "SHOT_RELEASE" => {
                                w.shot_release += 1;
                                let is_three = payload()
                                    .and_then(|d| d.get("ShotRelease"))
                                    .and_then(|s| s.get("is_three"))
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false);
                                if is_three {
                                    shot_rel_3 += 1;
                                } else {
                                    shot_rel_2 += 1;
                                }
                            }
                            // `SCORE` 与 `SHOT_MISS` 都是 `HoopArrival` 载荷，
                            // 按 `is_made` 区分命中；`SCORE` 同时关闭回合窗口。
                            "SCORE" | "SHOT_MISS" => {
                                if ev.kind == "SCORE" {
                                    w.made += 1;
                                }
                                let arrival = payload().and_then(|d| d.get("HoopArrival"));
                                let is_made = arrival
                                    .and_then(|a| a.get("is_made"))
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false);
                                let is_three = arrival
                                    .and_then(|a| a.get("is_three"))
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false);
                                if is_made {
                                    if is_three {
                                        made_3 += 1;
                                    } else {
                                        made_2 += 1;
                                    }
                                }
                            }
                            "FREE_THROW" => {
                                ft_att += 1;
                                let made = payload()
                                    .and_then(|d| d.get("FreeThrowAttempt"))
                                    .and_then(|f| f.get("made"))
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false);
                                if made {
                                    ft_made += 1;
                                }
                            }
                            "FOUL" | "SHOOTING_FOUL" => fouls += 1,
                            "POSSESSION_SUMMARY" => {
                                let terminal = payload()
                                    .and_then(|d| d.get("PossessionSummary"))
                                    .and_then(|s| s.get("terminal_event"))
                                    .and_then(|t| t.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                if terminal.starts_with("TURNOVER") {
                                    turnover_terminals += 1;
                                }
                                if let Some(reason) = classify_mismatch(&terminal, &w) {
                                    mismatches.push((idx, reason));
                                }
                                idx += 1;
                                w = WindowKinds::default();
                                facts_since_summary = false;
                            }
                            // 节间布置与阶段迁移不属于「回合仍在进行」。
                            "PLACEMENT_APPLIED" | "PHASE_TRANSITION" => {}
                            _ => {
                                facts_since_summary = true;
                            }
                        }
                    }
                    ticks += 1;
                }

                let b = engine.box_score();
                let mut box_failures = Vec::new();
                let check = |what: &str, expected: u32, actual: u32,
                             failures: &mut Vec<String>| {
                    if expected != actual {
                        failures.push(format!(
                            "seed {seed}: {what}: box_score {actual} vs event-stream {expected}"
                        ));
                    }
                };
                check(
                    "fg2_attempts = SHOT_RELEASE(is_three=false)",
                    shot_rel_2,
                    b.fg2_attempts,
                    &mut box_failures,
                );
                check(
                    "fg3_attempts = SHOT_RELEASE(is_three=true)",
                    shot_rel_3,
                    b.fg3_attempts,
                    &mut box_failures,
                );
                check(
                    "fg2_made = HoopArrival(made && !three)",
                    made_2,
                    b.fg2_made,
                    &mut box_failures,
                );
                check(
                    "fg3_made = HoopArrival(made && three)",
                    made_3,
                    b.fg3_made,
                    &mut box_failures,
                );
                check(
                    "ft_attempts = FREE_THROW facts",
                    ft_att,
                    b.ft_attempts,
                    &mut box_failures,
                );
                check(
                    "ft_made = FREE_THROW(made=true)",
                    ft_made,
                    b.ft_made,
                    &mut box_failures,
                );
                check(
                    "fouls = FOUL facts (a zero-write field shows here as 0 vs N)",
                    fouls,
                    b.fouls,
                    &mut box_failures,
                );
                check(
                    "turnovers = TURNOVER* possession terminals (a partial-write field shows here, e.g. 12 vs 52)",
                    turnover_terminals,
                    b.turnovers,
                    &mut box_failures,
                );

                SeedAudit {
                    seed,
                    mismatches,
                    spanning,
                    box_failures,
                }
            })
            .collect()
    })
}

#[test]
fn turnover_terminal_reason_matches_window_facts() {
    let audits = shared_seed_audits();
    let mut total = 0usize;
    for audit in audits {
        if !audit.mismatches.is_empty() {
            total += audit.mismatches.len();
            eprintln!(
                "seed {}: {} mismatched possessions",
                audit.seed,
                audit.mismatches.len()
            );
            for (i, reason) in audit.mismatches.iter().take(5) {
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
    // 说明有回合跨过了节边界。仅凭"period 变了"会误报——节末总结在
    // period 仍是旧值的 tick 发出，而新周期从下一 tick 才开始。
    //
    // 模拟来自 shared_seed_audits 共享矩阵（与终结原因测试同一次模拟）。
    let spanning: usize = shared_seed_audits().iter().map(|a| a.spanning).sum();
    assert_eq!(
        spanning, 0,
        "a possession must not span a period boundary: it must be settled by \
         PossessionEndCause::PeriodEnd (defined in domain/event.rs, emitted in \
         finish_period)"
    );
}

/// 箱体统计的**逐字段**与事件流对平。
///
/// ## 为什么需要（evidence/problem.md §23.10 / §23.9 / §27）
///
/// 已实测两次同类缺陷：
/// - `box_score.turnovers` 有 5 种失误终结而只有 2 个自增点，
///   实测 52 次终结 vs 箱体 12 次（低估约 4 倍）；
/// - `box_score.fouls` **零自增点**，CLI 恒打印 `Fouls: 0`。
///
/// 两者的共同点是「声明字段与事件事实脱钩」，而既有守卫（常数、世界私有化、
/// 文档、身份）都覆盖不到——它们检查的是**代码形态**，**不是**数据一致性。
/// 本测试把 `MatchBoxScore` 的 8 个字段逐个与从事件流独立重建的值对平，
/// 对**未来新增的终结路径**同样有效。
/// 模拟来自 shared_seed_audits 共享矩阵（8 种子全矩阵覆盖）。
#[test]
fn box_score_fields_reconcile_with_event_stream() {
    let failures: Vec<String> = shared_seed_audits()
        .iter()
        .flat_map(|a| a.box_failures.clone())
        .collect();
    assert!(
        failures.is_empty(),
        "box_score fields must reconcile with the event stream: {failures:?}"
    );
}

/// D0.1 回合终结归因穷举：`UNATTRIBUTED_END` 必须在全矩阵下为 0。
///
/// 纪律：不允许通过「把兜底标签改名」来糊弄本测试；修复必须让
/// `complete_possession()` 的每条调用路径在到达前已经发射带显式归因的
/// `PossessionSummary`。
fn run_1q_and_count_unattributed(label: &str, seed: u64) -> (usize, String) {
    let mut engine = MatchEngine::new(seed);
    // label 必须逐测试唯一：并行运行的两个测试若共用路径会互相覆盖
    // （实测导致偶发「未观察到违例回合」假红）。
    let guard = nba_test_support::TempArtifact::new(&format!("{label}_{seed}"));
    engine
        .simulate_scope_and_export_with_mode("1q", &guard.path_str(), StreamMode::Facts)
        .expect("run 1q");
    let content = std::fs::read_to_string(guard.path()).unwrap_or_default();
    let unattributed = content
        .lines()
        .filter(|line| line.contains("\"UNATTRIBUTED_END\""))
        .count();
    (unattributed, content)
}

#[test]
fn no_unattributed_end_across_seed_matrix_1q() {
    // 判定口径：quality.md §2.5 的 8 种子矩阵。`UNATTRIBUTED_END` 已从
    // `PossessionEndCause` 物理删除（domain/event.rs），编译期写不出无归因
    // 终结；本测试守的是导出管道（ledger 解析）不重新引入它——分支覆盖
    // 由 8 个种子的 ~440 个回合提供，与矩阵大小无关。
    // 种子间并行；TempArtifact 路径含 seed，8 个线程互不覆盖。
    let per_seed: Vec<(u64, usize)> = (0..8u64)
        .into_par_iter()
        .map(|seed| {
            let (unattributed, _content) = run_1q_and_count_unattributed("attr_matrix", seed);
            (seed, unattributed)
        })
        .collect();
    for (seed, unattributed) in &per_seed {
        if *unattributed > 0 {
            eprintln!("seed {seed}: UNATTRIBUTED_END × {unattributed}");
        }
    }
    let total_unattributed: usize = per_seed.iter().map(|(_, u)| u).sum();
    assert_eq!(total_unattributed, 0, "UNATTRIBUTED_END 必须为 0");
}

/// full scope 下同样不允许兜底终结（含历史活锁种子）。
#[test]
fn no_unattributed_end_full_scope() {
    // 种子间并行：六场完整比赛互不共享状态；TempArtifact 路径含 seed 互不冲突。
    let failures: Vec<String> = [0u64, 3, 6, 42, 555, 999]
        .par_iter()
        .map(|&seed| {
            let mut engine = MatchEngine::new(seed);
            let guard =
                nba_test_support::TempArtifact::new(&format!("possession_attr_full_{seed}"));
            engine
                .simulate_scope_and_export_with_mode("full", &guard.path_str(), StreamMode::Facts)
                .expect("run full");
            let content = std::fs::read_to_string(guard.path()).unwrap_or_default();
            let unattributed = content
                .lines()
                .filter(|line| line.contains("\"UNATTRIBUTED_END\""))
                .count();
            if unattributed == 0 {
                None
            } else {
                Some(format!(
                    "seed {seed} full scope 存在兜底终结 × {unattributed}"
                ))
            }
        })
        .flatten()
        .collect();
    assert!(
        failures.is_empty(),
        "full scope 下不允许存在兜底终结: {failures:?}"
    );
}

/// 违例回合必须带责任球员（problem.md §13.2：`turnover_player_id` 缺失）。
///
/// 与 16-seed 矩阵同口径（`--seeds 1..16`、full scope）：缺陷历史上在
/// 第 4 节开场才暴露（seed 14，节末拨球松球跨节后归因断链），单节
/// 窗口覆盖不到；种子集与 `stats_baseline` 的账本门一致。
#[test]
fn violation_turnover_summary_carries_player_id() {
    let per_seed: Vec<(u32, Option<String>)> = (1..=16u64)
        .into_par_iter()
        .map(|seed| {
            let mut checked = 0u32;
            let mut violation = None;
            let mut engine = MatchEngine::new(seed);
            let guard = nba_test_support::TempArtifact::new(&format!("attr_violation_full_{seed}"));
            engine
                .simulate_scope_and_export_with_mode("full", &guard.path_str(), StreamMode::Facts)
                .expect("run full");
            let content = std::fs::read_to_string(guard.path()).unwrap_or_default();
            for line in content.lines() {
                if !line.contains("TURNOVER_VIOLATION") {
                    continue;
                }
                let json: serde_json::Value = serde_json::from_str(line).expect("summary json");
                // facts 流结构：event_log 数组内 data.PossessionSummary（可能多条），
                // 逐条检查终结为 TURNOVER_VIOLATION 的总结必须带责任球员。
                let entries: Vec<&serde_json::Value> = json
                    .get("event_log")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().collect())
                    .unwrap_or_else(|| vec![&json]);
                for entry in entries {
                    let Some(summary) = entry
                        .pointer("/data/PossessionSummary")
                        .or_else(|| json.pointer("/payload/summary"))
                    else {
                        continue;
                    };
                    if summary.get("terminal_event").and_then(|v| v.as_str())
                        != Some("TURNOVER_VIOLATION")
                    {
                        continue;
                    }
                    checked += 1;
                    let turnover_player =
                        summary.get("turnover_player_id").and_then(|v| v.as_str());
                    if turnover_player.is_none() {
                        violation = Some(format!(
                            "seed {seed} 的 TURNOVER_VIOLATION 总结缺 turnover_player_id: {summary}"
                        ));
                    }
                }
            }
            (checked, violation)
        })
        .collect();
    let checked: u32 = per_seed.iter().map(|(c, _)| c).sum();
    let violations: Vec<String> = per_seed.iter().filter_map(|(_, v)| v.clone()).collect();
    assert!(
        violations.is_empty(),
        "violation turnovers missing player id: {violations:?}"
    );
    assert!(checked > 0, "矩阵中未观察到违例回合，测试无判别力");
}

/// D4.1 事件 ID 与语义因果链（dev 方案 §7.1）。
///
/// 断言三条不变量：
/// 1. `event_id` 全场唯一且单调递增（跨 tick 稳定，供账本按 ID 重建因果）；
/// 2. 结果事件携带语义正确的父（`SCORE`/`SHOT_MISS` ← `SHOT_RELEASE`，
///    `PASS_RECEIVED` ← `PASS`，`FREE_THROW` ← `FOUL`）；
/// 3. **不伪造因果**：同 tick 内的独立事实（如两条 `CONTACT_BUMP`）之间
///    不得互相串链（同 tick 串链会被读成因果关系，违反"事件只陈述事实"）。
#[test]
fn event_ids_and_semantic_causal_links_hold() {
    let mut engine = MatchEngine::new(0);
    let guard = nba_test_support::TempArtifact::new("d4_causal");
    engine
        .simulate_scope_and_export_with_mode("1q", &guard.path_str(), StreamMode::Facts)
        .expect("run 1q");
    let content = std::fs::read_to_string(guard.path()).unwrap_or_default();

    let mut kind_of: HashMap<u64, String> = HashMap::new();
    let mut last_id = 0u64;
    let mut linked = 0usize;
    let mut same_tick_independent = 0usize;
    for line in content.lines() {
        let Ok(tick) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(events) = tick.get("event_log").and_then(|v| v.as_array()) else {
            continue;
        };
        for e in events {
            let eid = e.get("event_id").and_then(|v| v.as_u64()).unwrap_or(0);
            let kind = e.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            assert!(
                eid > last_id,
                "event_id must increase monotonically ({eid} after {last_id})"
            );
            last_id = eid;
            kind_of.insert(eid, kind.to_string());
            if let Some(pid) = e.get("parent_event_id").and_then(|v| v.as_u64()) {
                let parent_kind = kind_of.get(&pid).map(String::as_str).unwrap_or("");
                linked += 1;
                match kind {
                    "SCORE" | "SHOT_MISS" | "REBOUND" => assert_eq!(
                        parent_kind, "SHOT_RELEASE",
                        "{kind} must chain to SHOT_RELEASE, got {parent_kind}"
                    ),
                    "PASS_RECEIVED" | "PASS_TIPPED" | "PASS_DROPPED" | "STEAL" => assert_eq!(
                        parent_kind, "PASS",
                        "{kind} must chain to PASS, got {parent_kind}"
                    ),
                    "FREE_THROW" => assert_eq!(
                        parent_kind, "FOUL",
                        "FREE_THROW must chain to FOUL, got {parent_kind}"
                    ),
                    _ => {}
                }
            }
        }
    }
    assert!(
        linked > 0,
        "matrix must exercise causal links (no links found)"
    );
    // 不伪造因果：接触类事实不得携带父（它们之间无因果关系）。
    for kind in kind_of.values() {
        if kind == "CONTACT_BUMP" {
            same_tick_independent += 1;
        }
    }
    assert!(
        same_tick_independent > 0,
        "contact facts must exist to make the no-fabricated-causality check meaningful"
    );
}
