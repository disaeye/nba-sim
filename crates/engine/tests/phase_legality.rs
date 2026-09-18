//! 子阶段迁移合法性回归（`nba.v2` 的 `phase_transitions` 契约）。
//!
//! ## 为何单独立一条测试
//!
//! 该契约原先只由 G-STATS 16-seed 矩阵在 CLI 批处理里检查，而那个矩阵
//! **不属于** `run-tests.sh`：日常回归看不到它。实测因此漏过一条 Hard 缺陷
//! （seed 12：非投篮犯规在球飞行中把子阶段重置为 `Initiation`，
//! 球落地后再执行 `Initiation -> FlightAndRebound`）。
//!
//! 本测试把同一条契约搬进测试套件：跑多种子全场，断言相邻两个
//! `frame.phase` 一定落在契约允许的集合内。判定用的是 fixture 里的
//! 原始表（`include_str!` 读同一份文件），不重抄一份，避免两处漂移。

use nba_domain::GameRules;
use nba_engine::MatchEngine;

/// `nba.v2` 的合法性表。键为来源阶段，值为允许的目标阶段。
fn allowed_transitions() -> std::collections::HashMap<String, Vec<String>> {
    let raw: serde_json::Value =
        serde_json::from_str(include_str!("../../evaluator/fixtures/nba.v2.json"))
            .expect("nba.v2 fixture must parse");
    let table = raw
        .get("phase_transitions")
        .expect("nba.v2 must declare phase_transitions")
        .as_object()
        .expect("phase_transitions must be an object");
    table
        .iter()
        .map(|(from, targets)| {
            let list = targets
                .as_array()
                .expect("each transition entry must be an array")
                .iter()
                .map(|t| t.as_str().expect("target must be a string").to_string())
                .collect();
            (from.clone(), list)
        })
        .collect()
}

#[test]
fn phase_transitions_are_legal_across_seeds() {
    let allowed = allowed_transitions();
    // 全场约 87000 tick；用多种子覆盖不同比赛分支。
    let seeds: [u64; 4] = [42, 12, 7, 31337];
    for seed in seeds {
        let mut engine = MatchEngine::new(seed);
        let mut prev: Option<String> = None;
        for tick in 0..90000u32 {
            let frame = engine.step().frame;
            let phase = frame.phase.clone();
            if let Some(from) = &prev {
                if from != &phase {
                    // 表里没有的来源阶段按契约视为允许（与评判器同一口径）。
                    if let Some(targets) = allowed.get(from) {
                        assert!(
                            targets.contains(&phase),
                            "seed {seed} tick {tick}: illegal phase transition \
                             {from} -> {phase}; allowed from {from}: {targets:?} \
                             (contract: crates/evaluator/fixtures/nba.v2.json)"
                        );
                    }
                }
            }
            prev = Some(phase);
        }
    }
}

/// 空规则下的迁移同样必须合法：契约不依赖具体规则档案。
#[test]
fn phase_transitions_are_legal_under_default_rules() {
    let allowed = allowed_transitions();
    let mut engine = MatchEngine::with_rules(12, GameRules::default());
    let mut prev: Option<String> = None;
    for tick in 0..90000u32 {
        let phase = engine.step().frame.phase.clone();
        if let Some(from) = &prev {
            if from != &phase {
                if let Some(targets) = allowed.get(from) {
                    assert!(
                        targets.contains(&phase),
                        "tick {tick}: illegal phase transition {from} -> {phase}"
                    );
                }
            }
        }
        prev = Some(phase);
    }
}
