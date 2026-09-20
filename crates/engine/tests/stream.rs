//! 流输出行为：流模式、字节预算、事件帧的作用域与单调性。
//!
//! 守卫对象：默认有界输出、超预算必须报错而不是写满磁盘、事件种类不跨 tick
//! 泄漏、事件序号与时间戳自洽、事件载荷可回放。
//!
//! 对应 `docs/gap.md` §16.4（资源治理）与 `docs/quality.md` §1.1。

#![allow(clippy::field_reassign_with_default)]

mod support;

use nba_engine::MatchEngine;

/// 默认（facts）流必须远小于逐 tick 帧流，且仍然可评判。
///
/// 根因：此前默认逐 tick 全量帧，单场 full scope 达数百 MB
/// （`gap.md` §16.4 / `problem.md` §14.4）。
#[test]
fn test_bounded_stream_modes_are_much_smaller() {
    use nba_engine::StreamMode;

    let run = |mode: StreamMode, label: &str| -> u64 {
        let mut engine = MatchEngine::new(42);
        // RAII：panic 展开也会删除临时流（test-support）。
        let artifact = nba_test_support::TempArtifact::new(&format!("stream_mode_{label}"));
        let result = engine.simulate_scope_and_export_with_mode("1q", &artifact.path_str(), mode);
        assert!(result.is_ok(), "mode {:?} failed: {:?}", mode, result.err());
        artifact.assert_within_limit()
    };

    let frames = run(StreamMode::Frames, "frames");
    let facts = run(StreamMode::Facts, "facts");
    let summary = run(StreamMode::Summary, "summary");

    assert!(frames > 0 && facts > 0 && summary > 0);
    // facts 模式必须比帧模式小一个数量级以上。
    assert!(
        facts * 10 < frames,
        "facts stream ({facts} B) must be at least 10x smaller than frames ({frames} B)"
    );
    assert!(
        summary < facts,
        "summary ({summary} B) must be smaller than facts ({facts} B)"
    );
    // 一场 full scope 的 facts 流必须处于 MB 级（< 64 MiB）。
    let mut engine = MatchEngine::new(42);
    let artifact = nba_test_support::TempArtifact::with_limit("full_facts", 64 * 1024 * 1024);
    engine
        .simulate_scope_and_export_with_mode("full", &artifact.path_str(), StreamMode::Facts)
        .expect("full facts export");
    let full_facts = artifact.assert_within_limit();
    assert!(
        full_facts < 64 * 1024 * 1024,
        "full-scope facts stream must stay in the MB range, got {} bytes",
        full_facts
    );
}

/// 超过字节预算必须报错，而不是静默写满磁盘。
#[test]
fn test_stream_byte_budget_fails_closed() {
    use nba_engine::StreamMode;
    let mut rules = nba_domain::GameRules::default();
    rules.stream_max_bytes = 1024; // 1 KiB：必然超限
    let mut engine = MatchEngine::with_rules(42, rules);
    let artifact = nba_test_support::TempArtifact::new("budget");
    let result =
        engine.simulate_scope_and_export_with_mode("1q", &artifact.path_str(), StreamMode::Facts);
    assert!(
        result.is_err(),
        "exceeding the byte budget must fail closed"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("budget"),
        "error should mention the budget, got: {err}"
    );
}

/// 事件种类必须严格属于本 tick，不得把上一 tick 的残留带过来。
#[test]
fn test_event_types_are_scoped_to_one_tick() {
    let mut rules = nba_domain::GameRules::default();
    rules.tick_seconds = 0.1;
    rules.tactical_initiation_seconds = 0.1;
    let mut engine = MatchEngine::with_rules(905, rules);
    let first = engine.step();
    let second = engine.step();
    assert!(first.frame.events.len() <= 10);
    assert!(second.frame.events.len() <= 10);
    assert!(engine.pending_events().is_empty());
}

/// 事件日志的序号必须单调，时间戳必须与本 tick 一致。
#[test]
fn test_event_log_is_monotonic_and_tick_scoped() {
    let mut engine = MatchEngine::new(908);
    let mut prior_sequence = 0;
    let mut logged = 0;
    for _ in 0..200 {
        let tick = engine.step();
        assert!(tick
            .frame
            .event_log
            .windows(2)
            .all(|events| events[0].sequence < events[1].sequence));
        for event in &tick.frame.event_log {
            assert!(event.sequence > prior_sequence);
            assert!((event.time - tick.frame.t).abs() < 0.001);
            prior_sequence = event.sequence;
            logged += 1;
        }
        assert_eq!(tick.frame.event_sequence, prior_sequence);
    }
    assert!(logged > 0);
}

/// 事件日志必须携带可回放的领域载荷（`data` 字段为对象）。
#[test]
fn event_log_retains_replayable_domain_payloads() {
    let mut engine = MatchEngine::new(917);
    let tick = (0..200)
        .map(|_| engine.step())
        .find(|tick| !tick.frame.event_log.is_empty())
        .expect("simulation should publish a domain event");
    assert!(tick
        .frame
        .event_log
        .iter()
        .all(|event| event.data.is_some()));
    let encoded = serde_json::to_value(&tick).expect("event payload should serialize");
    let first = encoded["event_log"]
        .as_array()
        .and_then(|events| events.first())
        .expect("serialized event log should contain an entry");
    assert!(first["data"].is_object());
}

/// `StreamTick` 必须序列化出与引擎规则一致的审计余量。
#[test]
fn audit_margin_is_part_of_serialized_rules_contract() {
    let mut engine = MatchEngine::new(1204);
    let value = serde_json::to_value(engine.step()).expect("frame should serialize");
    assert_eq!(
        value["rules"]["separation_safety_margin_ft"],
        serde_json::json!(engine.rules().separation_safety_margin_ft)
    );
}
