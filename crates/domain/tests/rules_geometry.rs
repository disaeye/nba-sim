use glam::Vec2;
use nba_domain::{CourtGeometry, GameRules};

#[test]
fn custom_geometry_is_used_for_playable_bounds_and_hoops() {
    let rules = GameRules {
        court: CourtGeometry {
            width_ft: 80.0,
            height_ft: 40.0,
            hoop_left_x_ft: 4.0,
            hoop_right_x_ft: 76.0,
            hoop_y_ft: 20.0,
        },
        ..GameRules::default()
    };
    assert_eq!(rules.court.hoop_pos(true), Vec2::new(76.0, 20.0));
    assert_eq!(
        rules.court.clamp_playable(Vec2::new(100.0, -5.0), 2.0),
        Vec2::new(78.0, 2.0)
    );
    assert!(rules.court.contains(Vec2::new(40.0, 20.0), 2.0));
    assert!(!rules.court.contains(Vec2::new(1.0, 20.0), 2.0));
}

#[test]
fn builtin_teams_expose_data_driven_rosters() {
    let home = nba_domain::TeamData::builtin_home();
    let away = nba_domain::TeamData::builtin_away();
    assert!(home.players.len() >= 8);
    assert!(away.players.len() >= 8);
    assert!(home.players.iter().all(|player| player.team_id == home.id));
    assert!(away.players.iter().all(|player| player.team_id == away.id));
    assert!(home.players.iter().all(|player| !player.name.is_empty()));
}

#[test]
fn custom_geometry_passes_validation() {
    let mut rules = GameRules::default();
    rules.court.width_ft = 80.0;
    rules.court.height_ft = 40.0;
    rules.court.hoop_left_x_ft = 4.0;
    rules.court.hoop_right_x_ft = 76.0;
    rules.court.hoop_y_ft = 20.0;
    assert!(rules.validate().is_ok());
}
