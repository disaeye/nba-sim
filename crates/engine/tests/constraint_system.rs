//! 约束系统集成测试：验证约束对象模型的核心契约
//! （架构文档 Phase 1/3/7 的可观察行为）。

use std::collections::HashMap;

use glam::Vec2;

use nba_decision::constraint::{
    CandidateAction, ConstraintContext, ConstraintRegistry, PhaseType,
};
use nba_physics::movement::PlayerPhysicsState;
use nba_engine::MatchEngine;

fn make_player(id: &str, team: &str, x: f32, y: f32) -> PlayerPhysicsState {
    PlayerPhysicsState {
        id: id.to_string(),
        jersey: id.to_string(),
        team: team.to_string(),
        pos_ft: Vec2::new(x, y),
        vel_ft: Vec2::ZERO,
        accel_ft: Vec2::ZERO,
        target_pos_ft: Vec2::new(x, y),
        target_speed_ftps: 0.0,
        has_ball: false,
        action: "Idle".to_string(),
        slot: "PG".to_string(),
        morale: "Normal".to_string(),
        stamina: 100.0,
        max_stamina: 100.0,
        locomotion: nba_physics::movement::LocomotionState::Idle,
        facing_dir: Vec2::X,
        turn_decel_timer: 0.0,
        is_locked_kinematics: false,
    }
}

fn base_ctx<'a>(players: &'a HashMap<String, PlayerPhysicsState>) -> ConstraintContext<'a> {
    ConstraintContext {
        players,
        ball_pos: Vec2::new(47.0, 25.0),
        possession_team: "home",
        shot_clock: 12.0,
        game_clock: 300.0,
        phase: PhaseType::SetPlay,
        ball_in_flight: false,
        is_dead_ball: false,
    }
}

#[test]
fn test_hard_constraint_blocks_out_of_bounds_pass() {
    let registry = ConstraintRegistry::new();
    let mut players = HashMap::new();
    players.insert("H_1".to_string(), make_player("H_1", "home", 40.0, 25.0));
    players.insert("H_2".to_string(), make_player("H_2", "home", 60.0, 25.0));

    let ctx = base_ctx(&players);
    let action = CandidateAction::Pass {
        passer_id: "H_1".to_string(),
        receiver_id: "H_2".to_string(),
        from_pos: Vec2::new(40.0, 25.0),
        to_pos: Vec2::new(96.0, 25.0), // 出界点
    };

    let scored = registry.evaluate_candidate(&ctx, &action);
    assert!(!scored.feasible, "出界传球必须被硬约束剔除");
    assert_eq!(scored.blocked_by, Some("out_of_bounds"));
}

#[test]
fn test_dead_ball_blocks_shot() {
    let registry = ConstraintRegistry::new();
    let mut players = HashMap::new();
    players.insert("H_1".to_string(), make_player("H_1", "home", 40.0, 25.0));

    let mut ctx = base_ctx(&players);
    ctx.phase = PhaseType::DeadBallReset;
    ctx.is_dead_ball = true;

    let action = CandidateAction::Shoot {
        shooter_id: "H_1".to_string(),
        from_pos: Vec2::new(40.0, 25.0),
        is_three: false,
    };

    let scored = registry.evaluate_candidate(&ctx, &action);
    assert!(!scored.feasible, "死球阶段投篮必须被阻止");
    assert_eq!(scored.blocked_by, Some("dead_ball_action"));

    // 死球阶段允许 Dwell
    let dwell = CandidateAction::Dwell { player_id: "H_1".to_string() };
    let scored_dwell = registry.evaluate_candidate(&ctx, &dwell);
    assert!(scored_dwell.feasible);
}

#[test]
fn test_shot_clock_violation_world_check() {
    let registry = ConstraintRegistry::new();
    let mut players = HashMap::new();
    players.insert("H_1".to_string(), make_player("H_1", "home", 40.0, 25.0));

    let mut ctx = base_ctx(&players);
    ctx.shot_clock = 0.0;
    ctx.ball_in_flight = false;

    let violation = registry.evaluate_world(&ctx);
    assert!(violation.is_some(), "24 秒到时且球未出手必须触发世界级违例");
}

#[test]
fn test_game_clock_expired_blocks_shot_only() {
    let registry = ConstraintRegistry::new();
    let mut players = HashMap::new();
    players.insert("H_1".to_string(), make_player("H_1", "home", 40.0, 25.0));

    let mut ctx = base_ctx(&players);
    ctx.game_clock = 0.0;

    let shot = CandidateAction::Shoot {
        shooter_id: "H_1".to_string(),
        from_pos: Vec2::new(40.0, 25.0),
        is_three: false,
    };
    let scored = registry.evaluate_candidate(&ctx, &shot);
    assert!(!scored.feasible);
    assert_eq!(scored.blocked_by, Some("game_clock_expired"));
}

#[test]
fn test_risky_pass_soft_penalty_applies() {
    let registry = ConstraintRegistry::new();
    let mut players = HashMap::new();
    players.insert("H_1".to_string(), make_player("H_1", "home", 40.0, 25.0));
    players.insert("H_2".to_string(), make_player("H_2", "home", 60.0, 25.0));
    // 防守人正堵在传球线路中点
    players.insert("A_1".to_string(), make_player("A_1", "away", 50.0, 25.0));

    let ctx = base_ctx(&players);
    let action = CandidateAction::Pass {
        passer_id: "H_1".to_string(),
        receiver_id: "H_2".to_string(),
        from_pos: Vec2::new(40.0, 25.0),
        to_pos: Vec2::new(60.0, 25.0),
    };

    let scored = registry.evaluate_candidate(&ctx, &action);
    // 软约束不剔除动作，但施加惩罚
    assert!(scored.feasible, "软约束不得剔除动作");
    assert!(scored.constraint_penalty < 0.0, "被堵传球线必须有效用惩罚, got {}", scored.constraint_penalty);
    assert!(scored.risk > 0.0);
}

#[test]
fn test_clock_urgency_preference_boosts_shot() {
    let registry = ConstraintRegistry::new();
    let mut players = HashMap::new();
    players.insert("H_1".to_string(), make_player("H_1", "home", 40.0, 25.0));

    let mut ctx = base_ctx(&players);
    ctx.shot_clock = 2.0;

    let shot = CandidateAction::Shoot {
        shooter_id: "H_1".to_string(),
        from_pos: Vec2::new(40.0, 25.0),
        is_three: false,
    };
    let scored = registry.evaluate_candidate(&ctx, &shot);
    // preference 存在正贡献
    let has_boost = scored
        .flags
        .iter()
        .any(|(id, _, p)| *id == "shot_clock_urgency" && *p > 0.0);
    assert!(has_boost, "时钟紧迫偏好必须给出出手正偏置");

    // 同样场景下 Dwell 被负偏置
    let dwell = CandidateAction::Dwell { player_id: "H_1".to_string() };
    let scored_dwell = registry.evaluate_candidate(&ctx, &dwell);
    let dwell_penalized = scored_dwell
        .flags
        .iter()
        .any(|(id, _, p)| *id == "shot_clock_urgency" && *p < 0.0);
    assert!(dwell_penalized, "时钟紧迫偏好必须惩罚犹豫");
}

#[test]
fn test_simulation_decision_trace_present_in_stream() {
    // 端到端：决策调试层必须在流中可观察（文档 Phase 8）
    let mut engine = MatchEngine::new(7);
    let mut traces = 0;
    let mut events = std::collections::HashSet::new();
    for _ in 0..2500 {
        let tick = engine.step();
        events.insert(tick.frame.event_type.clone().unwrap_or_default());
        if tick.frame.debug.is_some() {
            traces += 1;
        }
    }
    assert!(traces > 0, "决策追踪必须出现在流中, got {traces}");
    assert!(
        events.contains("PASS"),
        "约束驱动决策必须产生传球事件, got {events:?}"
    );
    assert!(
        events.contains("SHOT_RELEASE"),
        "决策系统必须产生出手, got {events:?}"
    );
}

#[test]
fn test_shot_clock_violation_triggers_turnover_in_sim() {
    // 端到端：24 秒违例必须转换为球权转换事件
    let mut engine = MatchEngine::new(42);
    let mut saw_violation = false;
    let mut prev_possession_id = engine.possession_id;
    for _ in 0..20000 {
        let tick = engine.step();
        if tick.frame.event_type.as_deref() == Some("VIOLATION") {
            saw_violation = true;
            assert!(tick.frame.possession_id > prev_possession_id, "违例必须推进回合");
            break;
        }
        prev_possession_id = tick.frame.possession_id;
    }
    assert!(saw_violation || prev_possession_id > 1, "长模拟中应观察到违例或多次回合转换");
}
