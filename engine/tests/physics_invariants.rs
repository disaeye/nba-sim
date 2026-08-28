use nba_sim_engine::simulation::MatchEngine;
use nba_sim_engine::movement::MAX_PLAYER_SPEED_FTPS;
use std::collections::HashMap;

#[test]
fn test_physics_zero_anomalies_full_match() {
    let mut engine = MatchEngine::new(42);
    let total_ticks = 1500; // ~60 seconds at 25Hz (dt = 0.04s)

    let mut prev_positions: HashMap<String, (f32, f32)> = HashMap::new();
    let mut speed_violations = 0;
    let mut spacing_violations = 0;
    let dt = 0.04;

    for tick_idx in 0..total_ticks {
        let tick = engine.step();

        // 1. Verify speed constraints for every player
        for p in &tick.frame.players {
            let curr_x = p.x;
            let curr_y = p.y;

            if let Some(&(prev_x, prev_y)) = prev_positions.get(&p.jersey) {
                let dx_ft = (curr_x - prev_x) * 94.0;
                let dy_ft = (curr_y - prev_y) * 50.0;
                let disp_ft = (dx_ft * dx_ft + dy_ft * dy_ft).sqrt();
                let speed_ftps = disp_ft / dt;

                // Max sprint limit with numerical tolerance for integration
                if speed_ftps > MAX_PLAYER_SPEED_FTPS + 1.5 {
                    speed_violations += 1;
                    if speed_violations <= 5 {
                        eprintln!(
                            "Tick {}: Player {} exceeded max speed! speed = {:.2} ft/s (limit = {:.2})",
                            tick_idx, p.jersey, speed_ftps, MAX_PLAYER_SPEED_FTPS
                        );
                    }
                }
            }

            prev_positions.insert(p.jersey.clone(), (curr_x, curr_y));
        }

        // 2. Verify anti-penetration / spacing constraints between all 10 players
        for i in 0..tick.frame.players.len() {
            for j in (i + 1)..tick.frame.players.len() {
                let p1 = &tick.frame.players[i];
                let p2 = &tick.frame.players[j];

                let dx_ft = (p1.x - p2.x) * 94.0;
                let dy_ft = (p1.y - p2.y) * 50.0;
                let dist_ft = (dx_ft * dx_ft + dy_ft * dy_ft).sqrt();

                // 2.0 * PLAYER_RADIUS_FT = 2.4 ft minimum physical contact distance
                // With soft penalty and spring force, check no extreme overlaps (< 1.6 ft)
                if dist_ft < 1.6 {
                    spacing_violations += 1;
                    if spacing_violations <= 5 {
                        eprintln!(
                            "Tick {}: Spacing collapse between {} and {}! dist = {:.2} ft",
                            tick_idx, p1.jersey, p2.jersey, dist_ft
                        );
                    }
                }
            }
        }
    }

    assert_eq!(
        speed_violations, 0,
        "Found {} speed/teleport violations",
        speed_violations
    );
    assert_eq!(
        spacing_violations, 0,
        "Found {} body overlap/spacing violations",
        spacing_violations
    );
}

#[test]
fn test_court_boundaries_invariants() {
    let mut engine = MatchEngine::new(108);
    for _ in 0..1500 {
        let tick = engine.step();
        for p in &tick.frame.players {
            assert!(p.x >= 0.0 && p.x <= 1.0, "Player {} x out of bounds: {}", p.jersey, p.x);
            assert!(p.y >= 0.0 && p.y <= 1.0, "Player {} y out of bounds: {}", p.jersey, p.y);
        }
    }
}
