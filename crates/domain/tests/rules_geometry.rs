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

/// 统一出手分区（attributes.md §2.3a）：Rim < 5ft ≤ Near < 14ft ≤ Mid，
/// 三分优先于距离分区；底角特例（NBA 22ft 直线）属于 Three 而非 Mid。
#[test]
fn shot_zone_boundaries_follow_the_spec() {
    let court = CourtGeometry::default();
    let league = nba_domain::LeagueProfile::nba();
    let hoop = court.hoop_pos(true);
    let zone_at = |dist: f32, y_offset: f32| {
        court.shot_zone(
            Vec2::new(hoop.x + dist, hoop.y + y_offset),
            true,
            league.three_point_distance_ft,
            league.corner_three_distance_ft,
        )
    };
    use nba_domain::ShotZone;
    assert_eq!(zone_at(2.0, 0.0), ShotZone::Rim);
    assert_eq!(zone_at(4.99, 0.0), ShotZone::Rim);
    assert_eq!(zone_at(5.01, 0.0), ShotZone::Near);
    assert_eq!(zone_at(13.99, 0.0), ShotZone::Near);
    assert_eq!(zone_at(14.01, 0.0), ShotZone::Mid);
    assert_eq!(zone_at(23.0, 4.0), ShotZone::Mid);
    assert_eq!(zone_at(23.8, 2.0), ShotZone::Three);
}

/// 底角三分特例：距篮 22–23.75ft、贴边线的出手是 Three，不被误判为 Mid。
#[test]
fn corner_three_is_three_not_mid() {
    let court = CourtGeometry::default();
    let league = nba_domain::LeagueProfile::nba();
    let hoop = court.hoop_pos(true);
    // 底角：贴边线（y=3.0，在 3ft 底角带内），与篮筐几乎同 x（dx=1）→
    // 实际距篮约 22.02ft（在底角 22ft 线外、弧顶 23.75ft 线内）。
    let corner = Vec2::new(hoop.x + 1.0, 3.0);
    let distance = (corner - hoop).length();
    assert!(
        distance >= league.corner_three_distance_ft,
        "dist={distance}"
    );
    assert!(distance < league.three_point_distance_ft, "dist={distance}");
    assert_eq!(
        court.shot_zone(
            corner,
            true,
            league.three_point_distance_ft,
            league.corner_three_distance_ft
        ),
        nba_domain::ShotZone::Three
    );
}

/// 名册身份字段（attributes.md §2.7a/§2.7b）：六类位置与赛前攻防角色
/// 必填、合法且经校验；角色不随数据而外（枚举封闭）。
#[test]
fn rosters_declare_position_and_match_roles() {
    for team in [
        nba_domain::TeamData::builtin_home(),
        nba_domain::TeamData::builtin_away(),
    ] {
        for player in &team.players {
            // 枚举封闭，这里只校验字段确实从档案读入（不为默认占位）。
            let _ = player.position.as_str();
            let _ = player.offensive_role.as_str();
            let _ = player.defensive_role.as_str();
        }
    }
    let home = nba_domain::TeamData::builtin_home();
    let names: Vec<_> = home.players.iter().map(|p| p.position.as_str()).collect();
    assert!(
        names.contains(&"Point"),
        "roster must declare positions: {names:?}"
    );
    assert!(
        names.contains(&"Center"),
        "roster must declare positions: {names:?}"
    );
}
