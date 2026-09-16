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
fn test_match_world_clone_determinism() {
    let rules = GameRules::default();
    let world1 = MatchWorld::new_initial(rules);
    let world2 = world1.clone();

    assert_eq!(world1.clock.game_clock, world2.clock.game_clock);
    assert_eq!(world1.ledger.home_score, world2.ledger.home_score);
}
