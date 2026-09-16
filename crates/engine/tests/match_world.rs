//! D22 纯数据实体世界 (MatchWorld) 契约验证测试
//!
//! 验证 MatchWorld 纯数据组件布局、无隐藏状态突变以及确定性克隆特性。

use glam::Vec2;
use nba_domain::GameRules;
use nba_engine::world::{
    MatchWorld, PhysicalLimitComponent, PlayerRuntimeComponent, TransformComponent,
};
use nba_physics::movement::LocomotionState;

#[test]
fn test_match_world_initial_layout() {
    let rules = GameRules::default();
    let world = MatchWorld::new_initial(rules);

    assert_eq!(world.player_count(), 0);
    assert_eq!(world.clock.period, 1);
    assert_eq!(world.clock.game_clock, 720.0);
    assert_eq!(world.clock.shot_clock, 24.0);
    assert_eq!(world.ledger.home_score, 0);
    assert_eq!(world.ledger.away_score, 0);
}

#[test]
fn test_match_world_player_components_query() {
    let rules = GameRules::default();
    let mut world = MatchWorld::new_initial(rules);

    world.transforms.push(TransformComponent {
        pos_ft: Vec2::new(10.0, 20.0),
        vel_ft: Vec2::ZERO,
        accel_ft: Vec2::ZERO,
        facing: Vec2::new(1.0, 0.0),
    });
    world.limits.push(PhysicalLimitComponent {
        max_speed_ftps: 28.0,
        max_accel_ftps2: 20.0,
        traction_limit: 1.2,
    });
    world.states.push(PlayerRuntimeComponent {
        id: "p1".to_string(),
        team: "home".to_string(),
        jersey: "23".to_string(),
        on_court: true,
        stamina: 100.0,
        max_stamina: 100.0,
        foul_count: 0,
        morale: "normal".to_string(),
        action: "idle".to_string(),
        slot: "PG".to_string(),
        locomotion: LocomotionState::Idle,
    });

    assert_eq!(world.player_count(), 1);
    let on_court: Vec<_> = world.on_court_players().collect();
    assert_eq!(on_court.len(), 1);
    assert_eq!(on_court[0].0.id, "p1");
    assert_eq!(on_court[0].1.pos_ft, Vec2::new(10.0, 20.0));
}

#[test]
fn test_match_world_step_pipeline() {
    let rules = GameRules::default();
    let mut world = MatchWorld::new_initial(rules);

    world.transforms.push(TransformComponent {
        pos_ft: Vec2::new(10.0, 20.0),
        vel_ft: Vec2::ZERO,
        accel_ft: Vec2::ZERO,
        facing: Vec2::X,
    });
    world.limits.push(PhysicalLimitComponent {
        max_speed_ftps: 20.0,
        max_accel_ftps2: 15.0,
        traction_limit: 1.0,
    });
    world.states.push(PlayerRuntimeComponent {
        id: "p1".to_string(),
        team: "home".to_string(),
        jersey: "23".to_string(),
        on_court: true,
        stamina: 100.0,
        max_stamina: 100.0,
        foul_count: 0,
        morale: "normal".to_string(),
        action: "idle".to_string(),
        slot: "PG".to_string(),
        locomotion: LocomotionState::Idle,
    });

    let initial_time = world.clock.current_time;
    let initial_clock = world.clock.game_clock;
    let dt = 0.1_f32;

    let events = world.step_pipeline(dt);
    assert_eq!(world.clock.current_time, initial_time + dt);
    assert_eq!(world.clock.game_clock, initial_clock - dt);
    assert!(events.is_empty());
}

#[test]
fn test_spatial_perception_monotonicity() {
    use nba_engine::world::PerceptionSystem;

    let rules = GameRules::default();
    let mut world = MatchWorld::new_initial(rules);

    // 进攻球员 (Home) 位于 (25.0, 25.0)
    world.transforms.push(TransformComponent {
        pos_ft: Vec2::new(25.0, 25.0),
        vel_ft: Vec2::ZERO,
        accel_ft: Vec2::ZERO,
        facing: Vec2::X,
    });
    world.states.push(PlayerRuntimeComponent {
        id: "offense".to_string(),
        team: "home".to_string(),
        jersey: "1".to_string(),
        on_court: true,
        stamina: 100.0,
        max_stamina: 100.0,
        foul_count: 0,
        morale: "normal".to_string(),
        action: "idle".to_string(),
        slot: "PG".to_string(),
        locomotion: LocomotionState::Idle,
    });

    // 防守球员 (Away) 初始距离 3 英尺 (28.0, 25.0)
    world.transforms.push(TransformComponent {
        pos_ft: Vec2::new(28.0, 25.0),
        vel_ft: Vec2::ZERO,
        accel_ft: Vec2::ZERO,
        facing: Vec2::NEG_X, // 正面迎对进攻人
    });
    world.states.push(PlayerRuntimeComponent {
        id: "defense".to_string(),
        team: "away".to_string(),
        jersey: "2".to_string(),
        on_court: true,
        stamina: 100.0,
        max_stamina: 100.0,
        foul_count: 0,
        morale: "normal".to_string(),
        action: "idle".to_string(),
        slot: "PG".to_string(),
        locomotion: LocomotionState::Idle,
    });

    let p_close = PerceptionSystem::evaluate(&world);
    let density_close = p_close.defensive_contest_density[0];
    let openness_close = p_close.voronoi_openness[0];

    // 防守球员后撤到 10 英尺 (35.0, 25.0)
    world.transforms[1].pos_ft = Vec2::new(35.0, 25.0);
    let p_mid = PerceptionSystem::evaluate(&world);
    let density_mid = p_mid.defensive_contest_density[0];
    let openness_mid = p_mid.voronoi_openness[0];

    // 防守球员继续后撤到 20 英尺 (45.0, 25.0)
    world.transforms[1].pos_ft = Vec2::new(45.0, 25.0);
    let p_far = PerceptionSystem::evaluate(&world);
    let density_far = p_far.defensive_contest_density[0];
    let openness_far = p_far.voronoi_openness[0];

    // 严密单调性检验：距离越大，压迫密度严格单调递减，空间开阔度严格单调递增！
    assert!(
        density_close > density_mid && density_mid > density_far,
        "Contest density must be strictly monotonic: close={density_close}, mid={density_mid}, far={density_far}"
    );
    assert!(
        openness_close < openness_mid && openness_mid < openness_far,
        "Voronoi openness must be strictly monotonic: close={openness_close}, mid={openness_mid}, far={openness_far}"
    );
}

#[test]
fn test_traction_envelope_lateral_limit() {
    use nba_engine::world::{MatchIntentions, PhysicalLimitComponent, PhysicsSystem, PlayerIntentAction};

    let rules = GameRules::default();
    let mut world = MatchWorld::new_initial(rules);

    // 球员高速向正前方运动 (15.0 ft/s)
    world.transforms.push(TransformComponent {
        pos_ft: Vec2::new(10.0, 10.0),
        vel_ft: Vec2::new(15.0, 0.0),
        accel_ft: Vec2::ZERO,
        facing: Vec2::X,
    });
    world.limits.push(PhysicalLimitComponent {
        max_speed_ftps: 25.0,
        max_accel_ftps2: 40.0,
        traction_limit: 12.0, // 抓地力上限为 12.0 ft/s^2
    });
    world.states.push(PlayerRuntimeComponent {
        id: "runner".to_string(),
        team: "home".to_string(),
        jersey: "7".to_string(),
        on_court: true,
        stamina: 100.0,
        max_stamina: 100.0,
        foul_count: 0,
        morale: "normal".to_string(),
        action: "run".to_string(),
        slot: "SG".to_string(),
        locomotion: LocomotionState::Sprinting,
    });

    // 企图瞬间向 90 度垂直侧方全力变向 (y = 50.0)
    let intentions = MatchIntentions {
        actions: vec![PlayerIntentAction::MoveTo {
            target: Vec2::new(10.0, 50.0),
        }],
    };

    PhysicsSystem::step(&mut world, &intentions, 0.1);

    // 侧向加速度分量必须被限制在 traction_limit 附近（允许极小浮点公差）
    let lateral_accel_y = world.transforms[0].accel_ft.y.abs();
    assert!(
        lateral_accel_y <= 12.001,
        "Lateral acceleration must be constrained by traction limit 12.0, got: {lateral_accel_y}"
    );
}

#[test]
fn test_action_kinematics_phase_transitions_and_windows() {
    use nba_engine::world::{ActionPhase, ActionPhaseTransition};

    let mut phase = ActionPhase::Gather {
        elapsed: 0.0,
        duration: 0.15,
    };

    // 1. Gather 期：不可封盖
    assert!(!phase.is_blockable_window());
    assert!(!phase.is_shooting_foul_protected_window());

    // 步进 0.15 秒，跨入 Elevate 阶段
    let trans = phase.step(0.15);
    assert_eq!(trans, ActionPhaseTransition::Elevating);
    assert!(matches!(phase, ActionPhase::Elevate { .. }));

    // 2. Elevate 期：合法盖帽窗口开启，圆柱体保护开启
    assert!(phase.is_blockable_window());
    assert!(phase.is_shooting_foul_protected_window());

    // 步进 0.25 秒，极点出手
    let trans = phase.step(0.25);
    assert_eq!(trans, ActionPhaseTransition::ReadyToRelease);
    assert!(matches!(phase, ActionPhase::Release { .. }));
    assert!(phase.is_blockable_window());

    // 出手瞬间转移至 Landing
    let trans = phase.step(0.01);
    assert_eq!(trans, ActionPhaseTransition::Released);
    assert!(matches!(phase, ActionPhase::Landing { .. }));

    // 3. Landing 期：球已离手不可封盖，但圆柱体防守垫脚保护仍然生效！
    assert!(!phase.is_blockable_window());
    assert!(phase.is_shooting_foul_protected_window());

    // 落地缓冲完成，重回 Idle
    let trans = phase.step(0.20);
    assert_eq!(trans, ActionPhaseTransition::Completed);
    assert_eq!(phase, ActionPhase::Idle);
}
