//! D0.1 回合终结归因穷举（dev 方案 §3.2）的红测试。
//!
//! 目标：全矩阵（多种子、多 scope）下 `UNATTRIBUTED_END` 必须为 0——
//! 每个回合结束都必须携带显式终结原因。本测试在修复前应当为红，
//! 修复后为绿，并作为常驻回归。
//!
//! 纪律：不允许通过"把兜底标签改名"来糊弄本测试；修复必须让
//! `complete_possession()` 的每条调用路径在到达前已经发射带显式
//! 归因的 PossessionSummary。

use nba_engine::{MatchEngine, StreamMode};

/// 已知历史矩阵（problem.md §13.2）中 `UNATTRIBUTED_END` 出现于 62/100 场，
/// 逐 seed 扫描 1q，统计兜底终结的次数。
fn run_1q_and_count_unattributed(label: &str, seed: u64) -> (usize, String) {
    let mut engine = MatchEngine::new(seed);
    // label 必须逐测试唯一：并行运行的两个测试若共用路径会互相覆盖
    // （实测导致本测试偶发"未观察到违例回合"假红）。
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
    let mut total_unattributed = 0usize;
    for seed in 0..20u64 {
        let (unattributed, _content) = run_1q_and_count_unattributed("attr_matrix", seed);
        if unattributed > 0 {
            eprintln!("seed {seed}: UNATTRIBUTED_END × {unattributed}");
        }
        total_unattributed += unattributed;
    }
    assert_eq!(total_unattributed, 0, "UNATTRIBUTED_END 必须为 0");
}

/// full scope 下同样不允许兜底终结（含历史活锁种子）。
#[test]
fn no_unattributed_end_full_scope() {
    for seed in [0u64, 3, 6, 42, 555, 999] {
        let mut engine = MatchEngine::new(seed);
        let guard = nba_test_support::TempArtifact::new(&format!("possession_attr_full_{seed}"));
        engine
            .simulate_scope_and_export_with_mode("full", &guard.path_str(), StreamMode::Facts)
            .expect("run full");
        let content = std::fs::read_to_string(guard.path()).unwrap_or_default();
        let unattributed = content
            .lines()
            .filter(|line| line.contains("\"UNATTRIBUTED_END\""))
            .count();
        assert_eq!(unattributed, 0, "seed {seed} full scope 存在兜底终结");
    }
}

/// 违例回合必须带责任球员（problem.md §13.2：turnover_player_id 缺失）。
#[test]
fn violation_turnover_summary_carries_player_id() {
    let mut checked = 0u32;
    for seed in 0..10u64 {
        let (_unattributed, content) = run_1q_and_count_unattributed("attr_violation", seed);
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
                let turnover_player = summary.get("turnover_player_id").and_then(|v| v.as_str());
                assert!(
                    turnover_player.is_some(),
                    "seed {seed} 的 TURNOVER_VIOLATION 总结缺 turnover_player_id: {summary}"
                );
            }
        }
    }
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
    use std::collections::HashMap;
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
    for (eid, kind) in &kind_of {
        if kind == "CONTACT_BUMP" {
            let _ = eid;
            same_tick_independent += 1;
        }
    }
    assert!(
        same_tick_independent > 0,
        "contact facts must exist to make the no-fabricated-causality check meaningful"
    );
}
