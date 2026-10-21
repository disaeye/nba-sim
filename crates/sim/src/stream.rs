//! 协议投影：把动作链世界发布成 `StreamTick`。
//!
//! 表现层只消费这里的事实与几何，不参与裁定。事件载荷使用球员 id，
//! 索引只在本进程内部作为句柄使用。

use nba_domain::court::Court;
use nba_protocol::{
    FrameEvent, FrameRules, RenderBall, RenderFrame, RenderPlayer, RenderScore, RenderTeam,
    StreamTick,
};
use serde_json::json;

use crate::action::Fact;
use crate::state::{BallState, Phase};
use crate::world::{TimedFact, World};

impl Fact {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::PossessionStarted { .. } => "POSSESSION_STARTED",
            Self::Pass { .. } => "PASS",
            Self::Screen { .. } => "SCREEN",
            Self::Drive { .. } => "DRIVE",
            Self::Shot { .. } => "SHOT",
            Self::Foul { .. } => "FOUL",
            Self::FreeThrow { .. } => "FREE_THROW",
            Self::Rebound { .. } => "REBOUND",
            Self::Turnover { .. } => "TURNOVER",
            Self::Substitution { .. } => "SUBSTITUTION",
            Self::PeriodEnd { .. } => "PERIOD_END",
            Self::PlayActivated { .. } => "PLAY_ACTIVATED",
            Self::PlayWindowEnded { .. } => "PLAY_WINDOW_ENDED",
            Self::GameEnd => "GAME_END",
        }
    }
}

impl World {
    fn player_id(&self, handle: usize) -> String {
        self.players[handle].id.clone()
    }

    pub fn fact_payload(&self, fact: &Fact) -> serde_json::Value {
        match fact {
            Fact::PossessionStarted { team, live } => {
                json!({ "team": team.as_str(), "live": live })
            }
            Fact::Pass {
                pass_id,
                from,
                to,
                kind,
                completed,
            } => json!({
                "pass_id": format!("pass-{pass_id:06}"),
                "passer_id": self.player_id(*from),
                "receiver_id": self.player_id(*to),
                "kind": format!("{kind:?}"),
                "completed": completed,
            }),
            Fact::Screen {
                screener,
                on_ball,
                contact,
                route,
                separation_ft,
            } => json!({
                "screener_id": self.player_id(*screener),
                "on_ball": on_ball,
                "contact": contact,
                "route": format!("{route:?}"),
                "separation_ft": separation_ft,
            }),
            Fact::Drive {
                driver,
                reached,
                defender_beaten,
            } => json!({
                "driver_id": self.player_id(*driver),
                "reached": reached,
                "defender_beaten": defender_beaten,
            }),
            Fact::Shot {
                shot_id,
                shooter,
                kind,
                zone,
                distance_ft,
                contest,
                made,
                points,
                assisted_by,
            } => json!({
                "shot_id": format!("shot-{shot_id:06}"),
                "shooter_id": self.player_id(*shooter),
                "shot_kind": kind.as_str(),
                "creation": kind.creation(),
                "zone": zone,
                "distance_ft": distance_ft,
                "contest": contest,
                "made": made,
                "points": points,
                "assisted_by": assisted_by.map(|handle| self.player_id(handle)),
            }),
            Fact::Foul {
                fouler,
                fouled,
                shooting,
                free_throws,
                and_one,
            } => json!({
                "fouler_id": self.player_id(*fouler),
                "fouled_id": self.player_id(*fouled),
                "shooting": shooting,
                "free_throws": free_throws,
                "and_one": and_one,
            }),
            Fact::FreeThrow {
                shooter,
                made,
                index,
                total,
            } => json!({
                "shooter_id": self.player_id(*shooter),
                "made": made,
                "index": index,
                "total": total,
            }),
            Fact::Rebound {
                player,
                offensive,
                distance_ft,
            } => json!({
                "rebounder_id": self.player_id(*player),
                "offensive": offensive,
                "distance_ft": distance_ft,
            }),
            Fact::Turnover { player, kind } => json!({
                "player_id": self.player_id(*player),
                "turnover_kind": kind.as_str(),
            }),
            Fact::Substitution {
                team,
                incoming,
                outgoing,
            } => json!({
                "team": team.as_str(),
                "incoming_id": self.player_id(*incoming),
                "outgoing_id": self.player_id(*outgoing),
            }),
            Fact::PlayActivated { play, possession } => {
                json!({ "play_id": play, "possession": possession })
            }
            Fact::PlayWindowEnded { play, possession } => {
                json!({ "play_id": play, "possession": possession })
            }
            Fact::PeriodEnd { period } => json!({ "period": period }),
            Fact::GameEnd => json!({}),
        }
    }

    pub fn stream_tick(&self) -> StreamTick {
        let mut players = Vec::new();
        for player in self.players.iter().filter(|player| player.on_court) {
            let norm = Court::ft_to_norm_with_geometry(player.pos, self.rules.court);
            let target = Court::ft_to_norm_with_geometry(player.target, self.rules.court);
            players.push(RenderPlayer {
                id: player.id.clone(),
                jersey: player.jersey.clone(),
                team: player.team.as_str().to_string(),
                position: player.data.position.as_str().to_string(),
                offensive_role: player.data.offensive_role.as_str().to_string(),
                defensive_role: player.data.defensive_role.as_str().to_string(),
                x: norm.x,
                y: norm.y,
                zone: format!(
                    "{:?}",
                    self.rules.court.region(
                        player.pos,
                        player.team == crate::action::TeamSide::Home,
                        self.rules.league.three_point_distance_ft,
                    )
                ),
                has_ball: self.holder() == Some(self.player_index(&player.id)),
                on_court: true,
                action: player.slot.as_str().to_string(),
                action_phase: None,
                slot: player.slot.as_str().to_string(),
                morale: "steady".to_string(),
                stm: player.stamina * 10.0,
                stm_max: 10.0,
                foul_count: player.fouls,
                target_x: Some(target.x),
                target_y: Some(target.y),
                facing_x: Some(player.facing.x),
                facing_y: Some(player.facing.y),
                orientation: None,
                potential_target_x: None,
                potential_target_y: None,
                potential_action: None,
                potential_threat_ratio: None,
                potential_void_ratio: None,
            });
        }

        let holder = self.holder();
        let (ball_pos, ball_z, ball_status) = match &self.ball {
            BallState::Held(handle) => (self.players[*handle].pos, 4.0, "held"),
            BallState::Pass { to_pos, .. } => (*to_pos, 6.0, "pass"),
            BallState::Shot { to_pos, .. } => (*to_pos, 12.0, "shot"),
            BallState::Loose { pos, .. } => (*pos, 0.5, "loose"),
        };
        let ball_norm = Court::ft_to_norm_with_geometry(ball_pos, self.rules.court);

        let recent: Vec<FrameEvent> = self
            .events
            .iter()
            .rev()
            .take_while(|event| self.wall - event.time < 0.051)
            .enumerate()
            .map(|(offset, event)| self.frame_event(event, offset as u64))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        let event_names: Vec<String> = recent.iter().map(|event| event.kind.clone()).collect();
        let primary = event_names.first().cloned();

        StreamTick {
            frame: RenderFrame {
                t: self.clock.elapsed as f32,
                t_game: self.wall as f32,
                tick_index: self.clock.tick,
                shot_clock: self.clock.shot_clock as f32,
                period: self.clock.period,
                phase: format!("{:?}", self.phase),
                possession_id: self.possession.id as u32,
                possession_team: self.possession.team.as_str().to_string(),
                home_team: RenderTeam {
                    id: "home".to_string(),
                    name: self.home_team_name.clone(),
                    short_name: self.home_team_name.clone(),
                },
                away_team: RenderTeam {
                    id: "away".to_string(),
                    name: self.away_team_name.clone(),
                    short_name: self.away_team_name.clone(),
                },
                score: RenderScore {
                    home: self.score[0],
                    away: self.score[1],
                },
                players,
                ball: RenderBall {
                    x: ball_norm.x,
                    y: ball_norm.y,
                    z: ball_z,
                    status: ball_status.to_string(),
                    holder_id: holder.map(|handle| self.player_id(handle)),
                },
                event_type: primary,
                events: event_names,
                game_flow: match self.phase {
                    Phase::Live => "LiveBall",
                    Phase::Finished => "Finished",
                    _ => "DeadBall",
                }
                .to_string(),
                team_fouls_home: self.fouls[0].period as u32,
                team_fouls_away: self.fouls[1].period as u32,
                free_throws_remaining: match self.phase {
                    Phase::FreeThrows { remaining, .. } => remaining,
                    _ => 0,
                },
                defensive_tactic: None,
                event_sequence: self.clock.tick,
                event_log: recent,
                simulation_complete: self.finished,
                completed_possessions: self.possession_log.len(),
                target_possessions: 0,
                callout: None,
                intensity: None,
                debug: None,
                rules: frame_rules(&self.rules),
                stream_projection: "facts".to_string(),
                potential_field: Vec::new(),
            },
            tactical_set: String::new(),
            game_clock: self.clock.game_clock as f32,
            keyframe_index: None,
        }
    }

    fn frame_event(&self, event: &TimedFact, offset: u64) -> FrameEvent {
        let (shot_id, pass_id) = match &event.fact {
            Fact::Shot { shot_id, .. } => (Some(format!("shot-{shot_id:06}")), None),
            Fact::Pass { pass_id, .. } => (None, Some(format!("pass-{pass_id:06}"))),
            _ => (None, None),
        };
        FrameEvent {
            sequence: offset,
            time: event.time as f32,
            kind: event.fact.kind().to_string(),
            data: Some(self.fact_payload(&event.fact)),
            event_id: 0,
            parent_event_id: None,
            tick_index: self.clock.tick,
            action_id: None,
            shot_id,
            pass_id,
            loose_ball_id: None,
            foul_program_id: None,
            responsibility_id: None,
            movement_intent_id: None,
            play_instance_id: None,
            relations: Vec::new(),
        }
    }

    fn player_index(&self, id: &str) -> usize {
        self.players
            .iter()
            .position(|player| player.id == id)
            .expect("render projection must reference a known player")
    }
}

pub fn frame_rules(rules: &nba_domain::GameRules) -> FrameRules {
    FrameRules {
        tick_seconds: rules.tick_seconds,
        court_width_ft: rules.court.width_ft,
        court_height_ft: rules.court.height_ft,
        hoop_left_x_ft: rules.court.hoop_left_x_ft,
        hoop_right_x_ft: rules.court.hoop_right_x_ft,
        hoop_y_ft: rules.court.hoop_y_ft,
        player_radius_ft: rules.player_radius_ft,
        body_contact_radius_ft: rules.body_contact_radius_ft,
        min_player_separation_ft: rules.min_player_separation_ft,
        separation_safety_margin_ft: rules.separation_safety_margin_ft,
        max_player_speed_ftps: rules.max_player_speed_ftps,
        max_player_accel_ftps2: rules.max_player_accel_ftps2,
        ball_max_speed_ftps: rules.ball_max_speed_ftps,
        three_point_distance_ft: rules.league.three_point_distance_ft,
        shot_clock_seconds: rules
            .league
            .shot_clock_seconds
            .max(rules.league.offensive_rebound_shot_clock_seconds),
        holder_leash_ft: rules.invariant_holder_leash_ft,
        speed_tolerance_ftps: rules.invariant_speed_tolerance_ftps,
        ball_z_max_ft: rules.ball_z_max_ft,
        max_personal_fouls: rules.league.max_personal_fouls,
        bonus_fouls_per_period: rules.league.bonus_fouls_per_period,
    }
}
