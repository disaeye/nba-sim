use nba_domain::GameRules;
use nba_engine::MatchEngine;
use std::collections::HashMap;

#[test]
fn test_physics_zero_anomalies_full_match() {
    let mut engine = MatchEngine::new(42);
    let total_ticks = 1500;
    let mut prev_positions: HashMap<String, (f32, f32)> = HashMap::new();
    let mut speed_violations = 0;
    let mut spacing_violations = 0;
    let rules = engine.rules.clone();
    let dt = rules.tick_seconds;
    for tick_idx in 0..total_ticks {
        let tick = engine.step();
        for p in &tick.frame.players {
            if let Some(&(prev_x, prev_y)) = prev_positions.get(&p.id) {
                let dx_ft = (p.x - prev_x) * rules.court.width_ft;
                let dy_ft = (p.y - prev_y) * rules.court.height_ft;
                let speed_ftps = (dx_ft * dx_ft + dy_ft * dy_ft).sqrt() / dt;
                if speed_ftps > rules.max_player_speed_ftps + 1.5 {
                    speed_violations += 1;
                    if speed_violations <= 5 {
                        eprintln!(
                            "Tick {}: Player {} exceeded max speed: {:.2} ft/s",
                            tick_idx, p.id, speed_ftps
                        );
                    }
                }
            }
            prev_positions.insert(p.id.clone(), (p.x, p.y));
        }
        for i in 0..tick.frame.players.len() {
            for j in (i + 1)..tick.frame.players.len() {
                let p1 = &tick.frame.players[i];
                let p2 = &tick.frame.players[j];
                let dx_ft = (p1.x - p2.x) * rules.court.width_ft;
                let dy_ft = (p1.y - p2.y) * rules.court.height_ft;
                if (dx_ft * dx_ft + dy_ft * dy_ft).sqrt() < rules.player_radius_ft * 0.9 {
                    spacing_violations += 1;
                }
            }
        }
    }
    assert_eq!(speed_violations, 0);
    assert_eq!(spacing_violations, 0);
}

#[test]
fn test_custom_geometry_boundaries_invariants() {
    let mut rules = GameRules::default();
    rules.court.width_ft = 80.0;
    rules.court.height_ft = 40.0;
    rules.court.hoop_left_x_ft = 4.0;
    rules.court.hoop_right_x_ft = 76.0;
    rules.court.hoop_y_ft = 20.0;
    let mut engine = MatchEngine::with_rules(108, rules.clone());
    for _ in 0..1500 {
        let tick = engine.step();
        for p in &tick.frame.players {
            // 边界不变量只约束在场球员（与 L1 PLAYER_IN_BOUNDS 同口径）：
            // 替补严格位于场外替补席（data.rs），其归一化坐标允许越出
            // [0,1]——物理层不为替补伪造 BoundaryCross，也不钳回场内。
            if !p.on_court {
                continue;
            }
            assert!(
                (0.0..=1.0).contains(&p.x),
                "Player {} x out of bounds: {}",
                p.id,
                p.x
            );
            assert!(
                (0.0..=1.0).contains(&p.y),
                "Player {} y out of bounds: {}",
                p.id,
                p.y
            );
        }
    }
}
