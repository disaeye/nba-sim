//! 只读投影：渲染帧与快照视图必须忠实反映引擎真相。
//!
//! 守卫对象：决策追踪出现在流里、球队元数据、自定义场地几何、战术档案槽位
//! 决定进攻站位、底角三分几何、`FrameRules` 与 `GameRules` 的默认投影一致。
//!
//! 对应 `docs/architecture.md` §2 [12]（快照输出）与 `docs/quality.md` §1.1
//! （限值一律取自帧内 `FrameRules`，不得有第二份副本）。

#![allow(clippy::field_reassign_with_default)]

mod support;

use glam::Vec2;
use nba_engine::MatchEngine;

#[test]
fn test_render_frame_contains_potential_field_projection() {
    let mut engine = MatchEngine::new(42);
    let mut observed = false;
    for _ in 0..200 {
        let tick = engine.step();
        if !tick.frame.potential_field.is_empty() {
            observed = true;
            assert!(tick
                .frame
                .potential_field
                .iter()
                .all(|sample| sample.x.is_finite()
                    && sample.y.is_finite()
                    && sample.target_x.is_finite()
                    && sample.target_y.is_finite()
                    && sample.pressure >= 0.0
                    && sample.pressure <= 1.0
                    && sample.drive_x.is_finite()
                    && sample.drive_y.is_finite()));
            break;
        }
    }
    assert!(
        observed,
        "potential field samples should reach the render frame"
    );
}

#[test]
fn test_simulation_decision_trace_present_in_stream() {
    let mut engine = MatchEngine::new(7);
    let mut traces = 0;
    let mut events = std::collections::HashSet::new();
    for _ in 0..2500 {
        let tick = engine.step();
        if tick.frame.debug.is_some() {
            traces += 1;
        }
        events.extend(tick.frame.events);
        if traces > 2 && events.contains("SHOT_RELEASE") {
            break;
        }
    }
    assert!(traces > 0);
    assert!(events.contains("PHASE_TRANSITION"));
}

#[test]
fn test_render_frame_preserves_setup_team_metadata() {
    let mut setup = nba_engine::setup::MatchSetup::builtin(nba_domain::GameRules::default());
    setup.home_team.name = "Custom Home Club".to_string();
    setup.home_team.short_name = "CHC".to_string();
    setup.away_team.name = "Custom Away Club".to_string();
    setup.away_team.short_name = "CAC".to_string();

    let tick = MatchEngine::with_setup(setup, 910).step();
    assert_eq!(tick.frame.home_team.name, "Custom Home Club");
    assert_eq!(tick.frame.home_team.short_name, "CHC");
    assert_eq!(tick.frame.away_team.name, "Custom Away Club");
    assert_eq!(tick.frame.away_team.short_name, "CAC");

    let encoded = serde_json::to_value(&tick).expect("stream tick should serialize");
    assert_eq!(encoded["home_team"]["short_name"], "CHC");
    assert_eq!(encoded["away_team"]["short_name"], "CAC");
}

#[test]
fn test_decision_uses_custom_hoop_geometry() {
    let mut rules = nba_domain::GameRules::default();
    rules.court.hoop_right_x_ft = 70.0;
    rules.court.hoop_left_x_ft = 10.0;
    let mut engine = MatchEngine::with_rules(906, rules);
    for _ in 0..100 {
        let tick = engine.step();
        assert!(tick
            .frame
            .players
            .iter()
            .all(|player| player.x >= 0.0 && player.x <= 1.0));
    }
}

/// D5.1b 红测试：战术档案的槽位必须真正决定进攻站位。
///
/// 根因：`data/tactics/*.json` 的 `base_offset_x/y` 从未被消费，全队挤在
/// 弧顶三分线外（实测进攻方 90% 球员距篮筐 > 23.75 ft），中距离出手没有
/// 机会产生（2PA 均值 10.8，真实 ~55）。
#[test]
fn test_tactical_spec_slots_drive_offensive_spacing() {
    let rules = nba_domain::GameRules::default();
    let three = rules.league.three_point_distance_ft;
    let mut inside = 0usize;
    let mut total = 0usize;
    for seed in [0u64, 42] {
        let mut engine = MatchEngine::with_rules(seed, rules.clone());
        engine.set_scope("1q").unwrap();
        let mut ticks = 0usize;
        while !engine.is_finished() && ticks < 60_000 {
            let tick = engine.step();
            ticks += 1;
            for p in &tick.frame.players {
                if !p.on_court {
                    continue;
                }
                // 只统计进攻方（possession_team）
                if p.team != tick.frame.possession_team {
                    continue;
                }
                let x = p.x * rules.court.width_ft;
                let y = p.y * rules.court.height_ft;
                let hoop = rules.court.hoop_pos(tick.frame.possession_team == "home");
                total += 1;
                if (Vec2::new(x, y) - hoop).length() < three {
                    inside += 1;
                }
            }
        }
    }
    assert!(total > 0, "no offensive player samples collected");
    let ratio = inside as f64 / total as f64;
    // 修复前该比例约 9.9%；档案槽位生效后应显著提升（底角/内线槽位在三分线内）。
    assert!(
        ratio > 0.25,
        "offensive players inside the arc: {:.1}% (expected > 25%; \
         tactical spec slots are not driving spacing)",
        ratio * 100.0
    );
}

/// D5.1b：底角三分必须按「更近的底角线」判定，而不是弧顶半径。
///
/// NBA 底角线距边线 3 ft，最近点距篮筐 22 ft；弧顶是 23.75 ft。
#[test]
fn test_corner_three_geometry() {
    let rules = nba_domain::GameRules::default();
    let court = rules.court;
    let arc = rules.league.three_point_distance_ft;
    let corner = rules.league.corner_three_distance_ft;
    assert!(
        corner > 0.0 && corner < arc,
        "NBA must declare corner < arc"
    );

    // 底角带内、距篮筐 22.4 ft → 三分
    let corner_spot = Vec2::new(court.hoop_right_x_ft - 6.0, 2.5);
    let d = (corner_spot - court.hoop_pos(true)).length();
    assert!(
        d >= corner && d < arc,
        "probe must sit between corner and arc"
    );
    assert!(
        court.is_three_point_attempt(corner_spot, true, arc, corner),
        "corner spot at {d:.2} ft must count as a three"
    );

    // 同一距离但在弧顶区域（y 居中）→ 两分
    let top_spot = Vec2::new(court.hoop_right_x_ft - 6.0, court.hoop_y_ft);
    assert!(
        (top_spot - court.hoop_pos(true)).length() < arc,
        "probe must be inside the arc"
    );
    assert!(
        !court.is_three_point_attempt(top_spot, true, arc, corner),
        "top-of-key spot inside the arc must not count as a three"
    );

    // 底角特例只"放宽"近处的判定，不得"收紧"远处的判定：
    // 后场远投（距篮筐 > 弧顶半径）本就该是三分。
    let backcourt = Vec2::new(court.width_ft * 0.2, 2.5);
    let bd = (backcourt - court.hoop_pos(true)).length();
    assert!(bd > arc, "probe must be beyond the arc ({bd:.1} ft)");
    assert!(
        court.is_three_point_attempt(backcourt, true, arc, corner),
        "a shot beyond the arc is a three regardless of court zone"
    );

    // 底角特例不适用于【弧顶区域】内、比底角线更近的点。
    let near_top = Vec2::new(court.hoop_right_x_ft - 6.0, court.hoop_y_ft);
    assert!(
        !court.is_three_point_attempt(near_top, true, arc, corner),
        "corner exception must not apply away from the sideline"
    );
}

/// C6.4：protocol `FrameRules::default()` 必须与引擎从 `GameRules::default()`
/// 投影出的 `FrameRules` 全字段一致。
///
/// 历史上两处独立手抄已实际漂移（`player_radius_ft` 1.8 vs 1.0）。此测试让
/// 任何一侧的漂移必红——新增规则字段时同步更新投影函数与 `Default`。
#[test]
fn frame_rules_default_matches_game_rules_default_projection() {
    let projected = nba_engine::frame_rules_from_game_rules(&nba_domain::GameRules::default());
    let fallback = nba_protocol::FrameRules::default();
    assert_eq!(projected.tick_seconds, fallback.tick_seconds);
    assert_eq!(projected.court_width_ft, fallback.court_width_ft);
    assert_eq!(projected.court_height_ft, fallback.court_height_ft);
    assert_eq!(projected.hoop_left_x_ft, fallback.hoop_left_x_ft);
    assert_eq!(projected.hoop_right_x_ft, fallback.hoop_right_x_ft);
    assert_eq!(projected.hoop_y_ft, fallback.hoop_y_ft);
    assert_eq!(projected.player_radius_ft, fallback.player_radius_ft);
    assert_eq!(
        projected.body_contact_radius_ft,
        fallback.body_contact_radius_ft
    );
    assert_eq!(
        projected.min_player_separation_ft,
        fallback.min_player_separation_ft
    );
    assert_eq!(
        projected.separation_safety_margin_ft,
        fallback.separation_safety_margin_ft
    );
    assert_eq!(
        projected.max_player_speed_ftps,
        fallback.max_player_speed_ftps
    );
    assert_eq!(
        projected.max_player_accel_ftps2,
        fallback.max_player_accel_ftps2
    );
    assert_eq!(projected.ball_max_speed_ftps, fallback.ball_max_speed_ftps);
    assert_eq!(
        projected.three_point_distance_ft,
        fallback.three_point_distance_ft
    );
    assert_eq!(projected.shot_clock_seconds, fallback.shot_clock_seconds);
    assert_eq!(projected.holder_leash_ft, fallback.holder_leash_ft);
    assert_eq!(
        projected.speed_tolerance_ftps,
        fallback.speed_tolerance_ftps
    );
    assert_eq!(projected.ball_z_max_ft, fallback.ball_z_max_ft);
}
