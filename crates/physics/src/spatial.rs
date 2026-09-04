use crate::movement::{EntityFilter, PlayerPhysicsState};
use glam::Vec2;
use nba_domain::GameRules;
use std::collections::HashMap;

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
    pub contest_intensity: f32,
    pub is_open_shot: bool,
}

pub struct SpatialGeometry;

impl SpatialGeometry {
    pub fn check_pass_corridor(
        from: Vec2,
        to: Vec2,
        corridor_radius: f32,
        players: &HashMap<String, PlayerPhysicsState>,
        passer_id: &str,
        receiver_id: &str,
        rules: &GameRules,
    ) -> PassCorridorStatus {
        let segment = to - from;
        let seg_len_sq = segment.length_squared();
        if seg_len_sq < 0.001 {
            return PassCorridorStatus {
                is_blocked: false,
                nearest_interceptor_id: None,
                corridor_clearance: f32::INFINITY,
                intercept_point: None,
            };
        }
        let passer_team = players
            .get(passer_id)
            .map(|p| p.team.as_str())
            .unwrap_or_default();
        let mut min_dist_to_line = f32::MAX;
        let mut nearest_interceptor = None;
        let mut intercept_point = None;
        let effective_radius = corridor_radius + rules.player_radius_ft + rules.defender_reach_ft;
        for player in players.values() {
            if !player.on_court
                || player.id == passer_id
                || player.id == receiver_id
                || player.team == passer_team
            {
                continue;
            }

            let t = ((player.pos_ft - from).dot(segment) / seg_len_sq).clamp(0.0, 1.0);
            let closest = from + segment * t;
            let distance = (player.pos_ft - closest).length();
            if distance < min_dist_to_line {
                min_dist_to_line = distance;
                if distance < effective_radius {
                    nearest_interceptor = Some(player.id.clone());
                    intercept_point = Some(closest);
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

    pub fn check_pass_corridor_default(
        from: Vec2,
        to: Vec2,
        corridor_radius: f32,
        players: &HashMap<String, PlayerPhysicsState>,
        passer_id: &str,
        receiver_id: &str,
    ) -> PassCorridorStatus {
        Self::check_pass_corridor(
            from,
            to,
            corridor_radius,
            players,
            passer_id,
            receiver_id,
            &GameRules::default(),
        )
    }

    /// Finds the nearest opponent through the supplied backend-facing view.
    /// The view is deliberately abstract so geometry never depends on a
    /// concrete physics implementation.
    pub fn nearest_opponent(
        player_id: &str,
        players: &HashMap<String, PlayerPhysicsState>,
        rules: &GameRules,
    ) -> Option<(String, f32)> {
        let player = players.get(player_id)?;
        let filter = EntityFilter::OpposingTeam(player.team.clone());
        let radius = rules.court.width_ft.hypot(rules.court.height_ft);
        let ids = SimpleSpatialView::new(players).query_nearby(player.pos_ft, radius, &filter);
        ids.into_iter()
            .filter_map(|id| {
                players
                    .get(&id)
                    .map(|opponent| (id, (opponent.pos_ft - player.pos_ft).length()))
            })
            .min_by(|left, right| {
                left.1
                    .partial_cmp(&right.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| left.0.cmp(&right.0))
            })
    }

    pub fn get_openness(
        player_id: &str,
        players: &HashMap<String, PlayerPhysicsState>,
        rules: &GameRules,
    ) -> OpennessMetric {
        let player = match players.get(player_id) {
            Some(p) => p,
            None => {
                return OpennessMetric {
                    closest_defender_id: None,
                    closest_defender_dist: f32::INFINITY,
                    defender_closing_speed: 0.0,
                    contest_intensity: 0.0,
                    is_open_shot: true,
                }
            }
        };
        let Some((closest_id, closest_dist)) = Self::nearest_opponent(player_id, players, rules)
        else {
            return OpennessMetric {
                closest_defender_id: None,
                closest_defender_dist: f32::INFINITY,
                defender_closing_speed: 0.0,
                contest_intensity: 0.0,
                is_open_shot: true,
            };
        };
        let defender = &players[&closest_id];
        let to_player = player.pos_ft - defender.pos_ft;
        let closing_speed = if closest_dist > rules.semantics.minimum_entity_distance_ft {
            defender.vel_ft.dot(to_player.normalize()).max(0.0)
        } else {
            0.0
        };
        let contest_distance =
            (rules.open_shot_distance_ft + rules.player_radius_ft).max(f32::EPSILON);
        let dist_factor =
            (1.0 - closest_dist / (contest_distance + rules.defender_reach_ft)).clamp(0.0, 1.0);
        let speed_factor = (closing_speed / rules.max_player_speed_ftps.max(f32::EPSILON))
            .clamp(0.0, rules.semantics.contest_speed_factor_cap);
        let facing_dot = if closest_dist > rules.semantics.minimum_entity_distance_ft {
            defender.facing_dir.dot(to_player.normalize())
        } else {
            1.0
        };
        let facing_factor = if facing_dot < 0.0 { 0.25 } else { (0.5 + 0.5 * facing_dot).clamp(0.25, 1.0) };
        let contest_intensity = ((dist_factor * rules.semantics.contest_dist_weight
            + speed_factor * rules.semantics.contest_speed_weight) * facing_factor)
            .clamp(0.0, 1.0);
        OpennessMetric {
            closest_defender_id: Some(closest_id),
            closest_defender_dist: closest_dist,
            defender_closing_speed: closing_speed,
            contest_intensity,
            is_open_shot: closest_dist >= rules.open_shot_distance_ft
                && contest_intensity < rules.semantics.open_shot_contest_threshold,
        }
    }

    pub fn get_openness_default(
        player_id: &str,
        players: &HashMap<String, PlayerPhysicsState>,
    ) -> OpennessMetric {
        Self::get_openness(player_id, players, &GameRules::default())
    }

    pub fn teammate_density(
        pos: Vec2,
        team: &str,
        players: &HashMap<String, PlayerPhysicsState>,
        radius_ft: f32,
        capacity: f32,
        rules: &GameRules,
    ) -> f32 {
        let radius_ft = radius_ft.max(rules.semantics.minimum_entity_distance_ft);
        let mut count = 0.0;
        for player in players.values() {
            if !player.on_court || player.team != team {
                continue;
            }
            let distance = (player.pos_ft - pos).length();
            if distance < radius_ft && distance > rules.semantics.minimum_entity_distance_ft {
                count += 1.0 - distance / radius_ft * rules.semantics.density_distance_weight;
            }
        }
        (count / capacity.max(1.0)).clamp(0.0, 1.0)
    }
    pub fn teammate_density_default(
        pos: Vec2,
        team: &str,
        players: &HashMap<String, PlayerPhysicsState>,
        radius_ft: f32,
    ) -> f32 {
        let rules = GameRules::default();
        Self::teammate_density(
            pos,
            team,
            players,
            radius_ft,
            rules.teammate_density_capacity,
            &rules,
        )
    }

    pub fn is_in_bounds_with_rules(pos: Vec2, margin: f32, rules: &GameRules) -> bool {
        rules.court.contains(pos, margin)
    }
    pub fn is_in_bounds(pos: Vec2, margin: f32) -> bool {
        Self::is_in_bounds_with_rules(pos, margin, &GameRules::default())
    }
}

impl OpennessMetric {
    pub fn contest_free_score(&self) -> f32 {
        (1.0 - self.contest_intensity).clamp(0.0, 1.0)
    }
}

struct SimpleSpatialView<'a> {
    players: &'a HashMap<String, PlayerPhysicsState>,
}
impl<'a> SimpleSpatialView<'a> {
    fn new(players: &'a HashMap<String, PlayerPhysicsState>) -> Self {
        Self { players }
    }
    fn query_nearby(&self, center: Vec2, radius: f32, filter: &EntityFilter) -> Vec<String> {
        let mut ids: Vec<_> = self
            .players
            .values()
            .filter(|p| p.on_court)
            .filter(|p| match filter {
                EntityFilter::Any => true,
                EntityFilter::Team(team) => &p.team == team,
                EntityFilter::OpposingTeam(team) => &p.team != team,
            })
            .filter_map(|p| {
                let d = (p.pos_ft - center).length();
                (d <= radius).then(|| (d, p.id.clone()))
            })
            .collect();
        ids.sort_by(|left, right| {
            left.0
                .partial_cmp(&right.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.1.cmp(&right.1))
        });
        ids.into_iter().map(|(_, id)| id).collect()
    }
}
