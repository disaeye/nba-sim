//! 会话行为：`MatchService` 的生命周期命令与增量事件游标。
//!
//! 守卫对象：暂停/运行/就绪三态、固定步累加器、非法命令的拒绝、`events_since`
//! 使用跨 tick 唯一的 `event_id` 作为游标。
//!
//! 对应 `docs/gap.md` §16.3（运行/会话分离、增量游标必须全局唯一）。

#![allow(clippy::field_reassign_with_default)]

mod support;

use nba_engine::MatchEngine;

#[test]
fn match_service_owns_lifecycle_and_fixed_step_accumulator() {
    use nba_engine::{MatchService, SessionState};

    let setup = nba_engine::MatchSetup::builtin(nba_domain::GameRules::default());
    let mut service = MatchService::with_seed(911);
    assert_eq!(service.state(), SessionState::Ready);
    assert!(service.tick(nba_domain::FixedDt(0.1)).is_err());
    let info = service
        .setup_match(setup)
        .expect("setup should be accepted");
    assert_eq!(info.seed, 911);
    let initial = service.snapshot().expect("snapshot after setup");
    service
        .tick(nba_domain::FixedDt(0.01))
        .expect("paused tick");
    assert_eq!(service.snapshot().unwrap().frame.t, initial.frame.t);

    service.start().expect("configured session starts");
    let before = service.snapshot().unwrap();
    let partial = service
        .tick(nba_domain::FixedDt(0.02))
        .expect("partial tick");
    assert_eq!(partial.frame.t, before.frame.t);
    let stepped = service.tick(nba_domain::FixedDt(0.02)).expect("fixed step");
    assert!(stepped.frame.t > before.frame.t);
    assert_eq!(service.state(), SessionState::Running);
    service.pause();
    let paused = service.snapshot().unwrap();
    service
        .tick(nba_domain::FixedDt(1.0))
        .expect("paused tick is read-only");
    assert_eq!(service.snapshot().unwrap().frame.t, paused.frame.t);
}

#[test]
fn match_service_next_possession_preserves_pause_state_and_history() {
    use nba_engine::{MatchService, SessionState};

    let mut service = MatchService::with_seed(912);
    service
        .setup_match(nba_engine::MatchSetup::builtin(
            nba_domain::GameRules::default(),
        ))
        .expect("setup should be accepted");
    let before = service.snapshot().unwrap();
    let after = service
        .next_possession()
        .expect("next possession should advance");
    assert_eq!(service.state(), SessionState::Ready);
    assert!(after.frame.possession_id != before.frame.possession_id);
    assert!(!service.events_since(0).is_empty());
}

#[test]
fn match_service_rejects_invalid_lifecycle_commands_and_preserves_state() {
    use nba_engine::{MatchService, SessionState};

    let mut service = MatchService::with_seed(913);
    assert!(service.start().is_err());
    assert!(service.resume().is_err());
    assert!(service.fast_forward(1.0).is_err());
    assert!(service.next_possession().is_err());

    service
        .setup_match(nba_engine::MatchSetup::builtin(
            nba_domain::GameRules::default(),
        ))
        .expect("setup should be accepted");
    assert_eq!(service.state(), SessionState::Ready);
    assert!(service.tick(nba_domain::FixedDt(-0.01)).is_err());
    assert!(service.tick(nba_domain::FixedDt(f32::NAN)).is_err());
    assert!(service.fast_forward(-1.0).is_err());
    assert!(service.fast_forward(f32::INFINITY).is_err());
    assert_eq!(service.state(), SessionState::Ready);
}

#[test]
fn match_service_fast_forward_preserves_running_and_paused_lifecycle() {
    use nba_engine::{MatchService, SessionState};

    let mut service = MatchService::with_seed(914);
    service
        .setup_match(nba_engine::MatchSetup::builtin(
            nba_domain::GameRules::default(),
        ))
        .expect("setup should be accepted");
    service.start().expect("configured session starts");
    let running_before = service.snapshot().unwrap().frame.t;
    let running_after = service.fast_forward(0.08).expect("running fast-forward");
    assert!(running_after.frame.t > running_before);
    assert_eq!(service.state(), SessionState::Running);

    service.pause();
    let paused_before = service.snapshot().unwrap().frame.t;
    let paused_after = service.fast_forward(0.08).expect("paused fast-forward");
    assert!(paused_after.frame.t > paused_before);
    assert_eq!(service.state(), SessionState::Paused);
}

/// `events_since` 的游标必须用跨 tick 唯一的 `event_id`。
///
/// 原实现按 tick 内局部 `sequence` 过滤：后续 tick 里 sequence ≤ 游标的事件
/// 全部漏取，增量语义失效。本测试驱动多个 tick 后：
/// 1. 用最后一个 event_id 作游标，增量必须为空（无重复）；
/// 2. 用中途 event_id 作游标，增量必须恰好是其后的全部事件（无漏取）；
/// 3. 去重按 event_id：历史里不得有重复 event_id。
#[test]
fn match_service_events_since_cursor_uses_global_event_id() {
    use nba_engine::MatchService;

    let mut service = MatchService::with_seed(641);
    service
        .setup_match(nba_engine::MatchSetup::builtin(
            nba_domain::GameRules::default(),
        ))
        .expect("setup should be accepted");
    service.start().expect("start");
    for _ in 0..400 {
        service
            .tick(nba_domain::FixedDt(0.1))
            .expect("tick advances");
    }
    let history = service.events_since(0);
    assert!(
        history.len() >= 2,
        "expected multi-event history, got {}",
        history.len()
    );

    // 游标语义 1：尾部游标 → 空增量。
    let last_id = history.iter().map(|e| e.event_id).max().unwrap();
    assert!(
        service.events_since(last_id).is_empty(),
        "tail event_id cursor must yield no events"
    );

    // 游标语义 2：中途游标 → 恰好是其后的全部事件（按 event_id 全序）。
    let mid = history[history.len() / 2].event_id;
    let expected: Vec<u64> = history
        .iter()
        .filter(|e| e.event_id > mid)
        .map(|e| e.event_id)
        .collect();
    let got: Vec<u64> = service
        .events_since(mid)
        .iter()
        .map(|e| e.event_id)
        .collect();
    assert_eq!(got, expected, "mid cursor must return exactly the suffix");
    assert!(
        !got.is_empty(),
        "mid cursor must not drop events (the old sequence cursor did)"
    );

    // 去重语义：event_history 内无重复 event_id。
    let mut seen = std::collections::HashSet::new();
    for e in &history {
        assert!(
            seen.insert(e.event_id),
            "duplicate event_id {} in history",
            e.event_id
        );
    }
}

/// `MatchService` 的 setup JSON 路径必须接受完整 setup 并投影到运行时。
#[test]
fn test_setup_json_uses_selected_roster_for_runtime_state() {
    let setup = nba_engine::MatchSetup::builtin(nba_domain::GameRules::default());
    // 首发身份来自名册的 `starter` 标记，而非数组位置（P-2）。
    let starter_id = setup
        .home_team
        .players
        .iter()
        .find(|p| p.starter)
        .map(|p| p.id.clone())
        .expect("roster must declare a starter");
    let request = serde_json::json!({ "setup": setup, "seed": 916 });
    let mut service = nba_engine::MatchService::new();
    service
        .setup_match_json(&request.to_string())
        .expect("setup JSON should be accepted");
    let snapshot = service.snapshot().expect("snapshot after setup");
    assert_eq!(snapshot.frame.players.len(), 16);
    assert!(snapshot
        .frame
        .players
        .iter()
        .any(|player| player.id == starter_id));
}

/// 名册之外的初始站位必须被拒绝（几何校验在构造时执行）。
#[test]
fn setup_rejects_players_outside_configured_geometry() {
    let mut setup = nba_engine::MatchSetup::builtin(nba_domain::GameRules::default());
    setup.home_team.players[0].initial_position_ft = (200.0, 25.0);
    let error = setup
        .validate()
        .expect_err("invalid player position must be rejected");
    assert!(error.contains("starts outside the court geometry"));
}

/// 名册顺序不携带身份：交换首发数组位置不得改变持球人选择。
#[test]
fn test_configured_starter_order_drives_target_binding() {
    let mut setup = nba_engine::MatchSetup::builtin(nba_domain::GameRules::default());
    setup.home_lineup.starters.swap(0, 4);
    let first = setup
        .home_team
        .players
        .iter()
        .find(|p| p.starter)
        .map(|p| p.id.clone())
        .expect("roster must declare a starter");
    let engine = MatchEngine::with_setup(setup, 909);
    assert_eq!(engine.new_possession_pg_for_test(), first);
}

/// 只有被选中的首发参与运行时；替补在物理世界里有实体但不在场。
#[test]
fn test_selected_lineup_controls_runtime_roster() {
    let mut setup = nba_engine::MatchSetup::builtin(nba_domain::GameRules::default());
    let home_bench_id = setup.home_lineup.bench.pop().expect("builtin bench");
    let away_bench_id = setup.away_lineup.bench.pop().expect("builtin bench");
    let engine = MatchEngine::with_setup(setup, 915);
    assert!(engine.physics().get_player(&home_bench_id).is_none());
    assert!(engine.physics().get_player(&away_bench_id).is_none());
    assert_eq!(engine.physics().get_players().len(), 14);
    assert!(engine.physics().get_players().values().all(|player| {
        player.action == "Bench" || player.action == "SetPosition" || player.action == "Initiate"
    }));
}

/// 内置名册的属性必须真实进入运行态（最大速度、加速与体力容量）。
#[test]
fn test_builtin_player_attributes_reach_runtime_state() {
    let engine = MatchEngine::new(907);
    let home = engine
        .physics()
        .get_player("H_01")
        .expect("builtin home player");
    let away = engine
        .physics()
        .get_player("A_01")
        .expect("builtin away player");
    assert_eq!(home.max_speed_ftps, away.max_speed_ftps);
    assert_eq!(home.max_accel_ftps2, away.max_accel_ftps2);
    assert_eq!(home.max_stamina, 50.0);
    assert_eq!(home.stamina, home.max_stamina);
}

/// 自定义时钟参数必须改变引擎的实际推进量，而不是只存在规则结构里。
#[test]
fn test_custom_rules_drive_engine_tick_and_clock() {
    let rules = nba_domain::GameRules {
        tick_seconds: 0.1,
        tactical_initiation_seconds: 0.1,
        tip_off_duration_seconds: 0.0,
        ..nba_domain::GameRules::default()
    };
    let mut engine = MatchEngine::with_rules(3, rules);
    let before = engine.game_clock();
    engine.step();
    assert!((engine.current_time() - 0.1).abs() < f32::EPSILON);
    assert!(engine.game_clock() < before);
}
