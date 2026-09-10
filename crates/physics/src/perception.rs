use glam::Vec2;
use nba_domain::GameRules;
use std::collections::HashMap;

use crate::movement::PlayerPhysicsState;

/// Read-only spatial facts made available to decision and replay consumers.
///
/// This type deliberately contains no action choice. It answers where players
/// are, how they are moving, and what is physically reachable at the current
/// tick; decision layers combine those facts with tactics, tendencies, and
/// rules.
#[derive(Debug, Clone, PartialEq)]
pub struct PerceptionSnapshot {
    pub ball_pos: Vec2,
    pub carrier_id: Option<String>,
    pub players: Vec<PlayerPerception>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerPerception {
    pub id: String,
    pub team: String,
    pub pos_ft: Vec2,
    pub vel_ft: Vec2,
    pub facing_dir: Vec2,
    pub speed_ftps: f32,
    pub distance_to_ball_ft: f32,
    pub time_to_ball_seconds: f32,
    pub distance_to_hoop_ft: f32,
    pub nearest_opponent_id: Option<String>,
    pub nearest_opponent_distance_ft: f32,
    pub nearest_opponent_closing_speed_ftps: f32,
}

impl PerceptionSnapshot {
    pub fn from_players(
        ball_pos: Vec2,
        carrier_id: Option<&str>,
        players: &HashMap<String, PlayerPhysicsState>,
        rules: &GameRules,
    ) -> Self {
        let mut ids: Vec<&String> = players.keys().collect();
        ids.sort();
        let mut perceptions = Vec::with_capacity(ids.len());

        for id in ids {
            let Some(player) = players.get(id) else {
                continue;
            };
            if !player.on_court {
                continue;
            }
            let hoop = rules.court.hoop_pos(player.team == "home");
            let to_ball = ball_pos - player.pos_ft;
            let distance_to_ball_ft = to_ball.length();
            let max_speed = player.max_speed_ftps.max(f32::EPSILON);
            let mut nearest_opponent_id = None;
            let mut nearest_opponent_distance_ft = f32::INFINITY;
            let mut nearest_opponent_closing_speed_ftps = 0.0;

            for opponent in players.values() {
                if !opponent.on_court || opponent.team == player.team {
                    continue;
                }
                let from_opponent = player.pos_ft - opponent.pos_ft;
                let distance = from_opponent.length();
                if distance < nearest_opponent_distance_ft
                    || (distance == nearest_opponent_distance_ft
                        && opponent.id.as_str() < nearest_opponent_id.as_deref().unwrap_or(""))
                {
                    nearest_opponent_id = Some(opponent.id.clone());
                    nearest_opponent_distance_ft = distance;
                    nearest_opponent_closing_speed_ftps = if distance > f32::EPSILON {
                        opponent.vel_ft.dot(from_opponent / distance).max(0.0)
                    } else {
                        0.0
                    };
                }
            }

            perceptions.push(PlayerPerception {
                id: player.id.clone(),
                team: player.team.clone(),
                pos_ft: player.pos_ft,
                vel_ft: player.vel_ft,
                facing_dir: player.facing_dir,
                speed_ftps: player.vel_ft.length(),
                distance_to_ball_ft,
                time_to_ball_seconds: distance_to_ball_ft / max_speed,
                distance_to_hoop_ft: (hoop - player.pos_ft).length(),
                nearest_opponent_id,
                nearest_opponent_distance_ft,
                nearest_opponent_closing_speed_ftps,
            });
        }

        Self {
            ball_pos,
            carrier_id: carrier_id.map(str::to_owned),
            players: perceptions,
        }
    }

    pub fn player(&self, id: &str) -> Option<&PlayerPerception> {
        self.players.iter().find(|player| player.id == id)
    }
}
