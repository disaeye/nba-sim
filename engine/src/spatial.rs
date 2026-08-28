use glam::Vec2;
use std::collections::HashMap;
use crate::court::{COURT_HEIGHT_FT, COURT_WIDTH_FT};
use crate::movement::{PlayerPhysicsState, PLAYER_RADIUS_FT};

#[derive(Debug, Clone)]
pub struct PassCorridorStatus {
    pub is_blocked: bool,
    pub nearest_interceptor_id: Option<String>,
    pub corridor_clearance: f32,
    pub intercept_point: Option<Vec2>,
}

#[derive(Debug, Clone)]
pub struct OpennessMetric {
    pub closest_defender_id: Option<String>,
    pub closest_defender_dist: f32,
    pub defender_closing_speed: f32,
    pub contest_intensity: f32, // [0.0, 1.0]
    pub is_open_shot: bool,
}

pub struct SpatialGeometry;

impl SpatialGeometry {
    /// Raycasts a capsule pass corridor from passer to receiver against defenders.
    /// Checks if any defender's cylinder (or reach) intersects the capsule corridor.
    pub fn check_pass_corridor(
        from: Vec2,
        to: Vec2,
        corridor_radius: f32,
        players: &HashMap<String, PlayerPhysicsState>,
        passer_id: &str,
        receiver_id: &str,
    ) -> PassCorridorStatus {
        let segment = to - from;
        let seg_len_sq = segment.length_squared();
        if seg_len_sq < 0.001 {
            return PassCorridorStatus {
                is_blocked: false,
                nearest_interceptor_id: None,
                corridor_clearance: 99.0,
                intercept_point: None,
            };
        }

        let passer_team = players.get(passer_id).map(|p| p.team.as_str()).unwrap_or("home");

        let mut min_dist_to_line = f32::MAX;
        let mut nearest_interceptor = None;
        let mut intercept_point = None;

        for player in players.values() {
            if player.id == passer_id || player.id == receiver_id {
                continue;
            }
            if player.team == passer_team {
                continue;
            }

            let to_player = player.pos_ft - from;
            let t = (to_player.dot(segment) / seg_len_sq).clamp(0.0, 1.0);
            let closest_on_segment = from + segment * t;
            let dist_to_segment = (player.pos_ft - closest_on_segment).length();

            // Total reach = player body radius + arm reach (~1.2 ft reach envelope)
            let defender_effective_radius = PLAYER_RADIUS_FT + 1.2;
            let combined_radius = corridor_radius + defender_effective_radius;

            if dist_to_segment < min_dist_to_line {
                min_dist_to_line = dist_to_segment;
                if dist_to_segment < combined_radius {
                    nearest_interceptor = Some(player.id.clone());
                    intercept_point = Some(closest_on_segment);
                }
            }
        }

        PassCorridorStatus {
            is_blocked: nearest_interceptor.is_some(),
            nearest_interceptor_id: nearest_interceptor,
            corridor_clearance: min_dist_to_line,
            intercept_point,
        }
    }

    /// Evaluates shooter/carrier openness by evaluating closest defender distance and closing velocity
    pub fn get_openness(
        player_id: &str,
        players: &HashMap<String, PlayerPhysicsState>,
    ) -> OpennessMetric {
        let player = match players.get(player_id) {
            Some(p) => p,
            None => {
                return OpennessMetric {
                    closest_defender_id: None,
                    closest_defender_dist: 99.0,
                    defender_closing_speed: 0.0,
                    contest_intensity: 0.0,
                    is_open_shot: true,
                };
            }
        };

        let mut closest_dist = f32::MAX;
        let mut closest_id = None;
        let mut closing_speed = 0.0;

        for other in players.values() {
            if other.team == player.team {
                continue;
            }

            let to_player = player.pos_ft - other.pos_ft;
            let dist = to_player.length();

            if dist < closest_dist {
                closest_dist = dist;
                closest_id = Some(other.id.clone());

                if dist > 0.1 {
                    let dir = to_player.normalize();
                    closing_speed = other.vel_ft.dot(dir).max(0.0);
                }
            }
        }

        // Contest intensity heuristic:
        // Wide open: dist > 6.0 ft (contest ~ 0.0)
        // Open: 4.0 - 6.0 ft (contest ~ 0.2 - 0.5)
        // Contested: 2.0 - 4.0 ft (contest ~ 0.5 - 0.85)
        // Smothered: < 2.5 ft (contest ~ 0.85 - 1.0)
        let dist_factor = (1.0 - (closest_dist / 7.0)).clamp(0.0, 1.0);
        let speed_factor = (closing_speed / 15.0).clamp(0.0, 0.4);
        let contest_intensity = (dist_factor * 0.8 + speed_factor).clamp(0.0, 1.0);

        let is_open_shot = closest_dist >= 4.5 && contest_intensity < 0.35;

        OpennessMetric {
            closest_defender_id: closest_id,
            closest_defender_dist: closest_dist,
            defender_closing_speed: closing_speed,
            contest_intensity,
            is_open_shot,
        }
    }

    /// 队友密度（约束系统 §3.2 空间占用查询）：
    /// 半径 radius_ft 内同队球员数（排除接球人自身）按容量归一到 [0,1]。
    /// capacity = 4 时，4 名队友全挤在半径内 → 1.0。
    pub fn teammate_density(
        pos: Vec2,
        team: &str,
        players: &HashMap<String, PlayerPhysicsState>,
        radius_ft: f32,
    ) -> f32 {
        let mut count = 0.0f32;
        for p in players.values() {
            if p.team != team {
                continue;
            }
            let d = (p.pos_ft - pos).length();
            if d < radius_ft && d > 0.001 {
                // 距离加权：越近权重越高
                count += 1.0 - (d / radius_ft) * 0.5;
            }
        }
        (count / 4.0).clamp(0.0, 1.0)
    }

    /// 点是否在场地界内（含 margin 缓冲）。
    pub fn is_in_bounds(pos: Vec2, margin: f32) -> bool {
        pos.x >= margin
            && pos.x <= COURT_WIDTH_FT - margin
            && pos.y >= margin
            && pos.y <= COURT_HEIGHT_FT - margin
    }
}
