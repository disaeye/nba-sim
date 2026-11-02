//! 协议投影：把动作链世界发布成 `StreamTick`。
//!
//! 表现层只消费这里的事实与几何，不参与裁定。事件载荷使用球员 id，
//! 索引只在本进程内部作为句柄使用。

use nba_domain::court::Court;
use nba_protocol::{
    DebugEpv, DebugUtility, DecisionDebug, DecisionTrace, FrameEvent, FrameRules, RenderBall,
    RenderFrame, RenderPlayer, RenderScore, RenderTeam, StreamTick, TraceCandidate,
};
use serde_json::json;

use crate::action::Fact;
use crate::state::{BallState, Phase};
use crate::tactics::PLAY_SOURCES;
use crate::world::{TimedFact, World};

impl World {
    pub fn project_event(&self, event: &TimedFact) -> FrameEvent {
        self.frame_event(event)
    }
}

impl Fact {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ActionStarted { .. } => "ACTION_STARTED",
            Self::ActionIntentRejected { .. } => "ACTION_INTENT_REJECTED",
            Self::ActionPhaseChanged { .. } => "ACTION_PHASE_CHANGED",
            Self::ActionCompleted { .. } => "ACTION_COMPLETED",
            Self::ActionFailed { .. } => "ACTION_FAILED",
            Self::ActionCancelled { .. } => "ACTION_CANCELLED",
            Self::DefensiveActionStarted { .. } => "DEFENSIVE_ACTION_STARTED",
            Self::DefensiveActionPhaseChanged { .. } => "DEFENSIVE_ACTION_PHASE_CHANGED",
            Self::DefensiveActionCompleted { .. } => "DEFENSIVE_ACTION_COMPLETED",
            Self::DefensiveActionCancelled { .. } => "DEFENSIVE_ACTION_CANCELLED",
            Self::PossessionStarted { .. } => "POSSESSION_STARTED",
            Self::PossessionEnded { .. } => "POSSESSION_ENDED",
            Self::Pass { .. } => "PASS",
            Self::Screen { .. } => "SCREEN",
            Self::Drive { .. } => "DRIVE",
            Self::PostUp { .. } => "POST_UP",
            Self::Shot { .. } => "SHOT",
            Self::Foul { .. } => "FOUL",
            Self::FreeThrow { .. } => "FREE_THROW",
            Self::Rebound { .. } => "REBOUND",
            Self::ReboundOpportunity { .. } => "REBOUND_OPPORTUNITY",
            Self::LooseBallRecovered { .. } => "LOOSE_BALL_RECOVERED",
            Self::SecondChanceStarted { .. } => "SECOND_CHANCE_STARTED",
            Self::Cut { .. } => "CUT",
            Self::BoxOut { .. } => "BOX_OUT",
            Self::DefenseResponsibilityChanged { .. } => "DEFENSE_RESPONSIBILITY_CHANGED",
            Self::Turnover { .. } => "TURNOVER",
            Self::Steal { .. } => "STEAL",
            Self::Substitution { .. } => "SUBSTITUTION",
            Self::BenchDepleted { .. } => "BENCH_DEPLETED",
            Self::FoulOutRetention { .. } => "FOUL_OUT_RETENTION",
            Self::TeamTechnicalFoul { .. } => "TEAM_TECHNICAL_FOUL",
            Self::Timeout { .. } => "TIMEOUT",
            Self::JumpBall { .. } => "JUMP_BALL",
            Self::PeriodEnd { .. } => "PERIOD_END",
            Self::PlayActivated { .. } => "PLAY_ACTIVATED",
            Self::PlayWindowEnded { .. } => "PLAY_WINDOW_ENDED",
            Self::BallContact { .. } => "BALL_CONTACT",
            Self::PlacementApplied { .. } => "PLACEMENT_APPLIED",
            Self::GameEnd => "GAME_END",
        }
    }
}

impl World {
    fn player_id(&self, handle: usize) -> String {
        self.players[handle].id.clone()
    }

    fn zone_for_position(&self, pos: glam::Vec2, team: crate::action::TeamSide) -> String {
        let zone = self.rules.court.region(
            pos,
            self.attacking_right(team),
            self.rules.league.three_point_distance_ft,
        );
        match zone {
            nba_domain::court::CourtRegion::Rim => "Rim",
            nba_domain::court::CourtRegion::RestrictedArea => "RestrictedArea",
            nba_domain::court::CourtRegion::Paint => "Paint",
            nba_domain::court::CourtRegion::LowPost => "LowPost",
            nba_domain::court::CourtRegion::Elbow => "Elbow",
            nba_domain::court::CourtRegion::MidRange => "MidRange",
            nba_domain::court::CourtRegion::CornerThree => "CornerThree",
            nba_domain::court::CourtRegion::WingThree => "WingThree",
            nba_domain::court::CourtRegion::TopThree => "TopThree",
            nba_domain::court::CourtRegion::Backcourt => "Backcourt",
        }
        .to_owned()
    }

    pub fn fact_payload(&self, fact: &Fact) -> serde_json::Value {
        match fact {
            Fact::ActionStarted {
                action_id,
                interaction_id,
                player,
                kind,
                target,
            } => json!({
                "action_id": action_id,
                "interaction_id": interaction_id,
                "player_id": self.player_id(*player),
                "action": kind,
                "target_id": target.map(|handle| self.player_id(handle)),
            }),
            Fact::ActionIntentRejected { player, kind, reason } => json!({
                "player_id": self.player_id(*player),
                "action": kind,
                "reason": reason,
            }),
            Fact::ActionPhaseChanged { action_id, player, from, to } => json!({
                "action_id": action_id,
                "player_id": self.player_id(*player),
                "from": format!("{from:?}"),
                "to": format!("{to:?}"),
            }),
            Fact::ActionCompleted { action_id, player, kind }
            | Fact::ActionFailed { action_id, player, kind } => json!({
                "action_id": action_id,
                "player_id": self.player_id(*player),
                "action": kind,
            }),
            Fact::ActionCancelled { action_id, player, kind, reason } => json!({
                "action_id": action_id,
                "player_id": self.player_id(*player),
                "action": kind,
                "reason": reason,
            }),
            Fact::DefensiveActionStarted {
                action_id,
                interaction_id,
                defender,
                shooter,
                kind,
                start_position,
            } => json!({
                "action_id": action_id,
                "interaction_id": interaction_id,
                "defender_id": self.player_id(*defender),
                "shooter_id": self.player_id(*shooter),
                "action": kind,
                "start_position_ft": [start_position.x, start_position.y],
            }),
            Fact::DefensiveActionPhaseChanged { action_id, defender, from, to } => json!({
                "action_id": action_id,
                "defender_id": self.player_id(*defender),
                "from": format!("{from:?}"),
                "to": format!("{to:?}"),
            }),
            Fact::DefensiveActionCompleted {
                action_id,
                defender,
                shooter,
                kind,
                displacement_ft,
            } => json!({
                "action_id": action_id,
                "defender_id": self.player_id(*defender),
                "shooter_id": self.player_id(*shooter),
                "action": kind,
                "displacement_ft": displacement_ft,
            }),
            Fact::DefensiveActionCancelled { action_id, defender, shooter, kind, reason } => json!({
                "action_id": action_id,
                "defender_id": self.player_id(*defender),
                "shooter_id": self.player_id(*shooter),
                "action": kind,
                "reason": reason,
            }),
            Fact::PossessionStarted {
                team,
                live,
                possession_id,
            } => json!({
                "team": team.as_str(),
                "live": live,
                "possession_id": possession_id,
            }),
            Fact::PossessionEnded {
                team,
                possession_id,
                end_reason,
                points,
                termination_detail,
                start_game_clock,
                end_game_clock,
                start_shot_clock,
                end_shot_clock,
                duration,
                passes,
                screens,
                drives,
                possession_kind,
                had_shot,
                had_offensive_rebound,
                first_action_seconds,
            } => json!({
                "team": team.as_str(),
                "possession_id": possession_id,
                "cause": end_reason.as_str(),
                "termination_detail": termination_detail,
                "points": points,
                "start_game_clock": start_game_clock,
                "end_game_clock": end_game_clock,
                "start_shot_clock": start_shot_clock,
                "end_shot_clock": end_shot_clock,
                "duration": duration,
                "passes": passes,
                "screens": screens,
                "drives": drives,
                "possession_kind": possession_kind.as_str(),
                "had_shot": had_shot,
                "had_offensive_rebound": had_offensive_rebound,
                "first_action_seconds": first_action_seconds,
            }),
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
                team,
                on_ball,
                contact,
                route,
                separation_ft,
                strength,
                displacement_ft,
            } => json!({
                "screener_id": self.player_id(*screener),
                "team": team.as_str(),
                "on_ball": on_ball,
                "contact": contact,
                "route": format!("{route:?}"),
                "separation_ft": separation_ft,
                "strength": strength,
                "displacement_ft": displacement_ft,
            }),
            Fact::Drive {
                driver,
                reached,
                defender_beaten,
                distance_ft,
                separation_ft,
            } => json!({
                "driver_id": self.player_id(*driver),
                "reached": reached,
                "defender_beaten": defender_beaten,
                "distance_ft": distance_ft,
                "separation_ft": separation_ft,
            }),
            Fact::PostUp { player } => json!({
                "player_id": self.player_id(*player),
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
                is_three,
                blocked_by,
            } => json!({
                "shot_id": format!("shot-{shot_id:06}"),
                "shooter_id": self.player_id(*shooter),
                "shot_kind": kind.as_str(),
                "creation": kind.creation(),
                "zone": zone,
                "distance_ft": distance_ft,
                "contest": contest,
                "is_three": is_three,
                "blocked_by": blocked_by.map(|handle| self.player_id(handle)),
                "made": made,
                "points": points,
                "assisted_by": assisted_by.map(|handle| self.player_id(handle)),
            }),
            Fact::Foul {
                fouler,
                fouled,
                kind,
                free_throws,
                and_one,
                foul_id,
            } => json!({
                "fouler_id": self.player_id(*fouler),
                "fouled_id": self.player_id(*fouled),
                "foul_kind": format!("{:?}", kind).to_lowercase(),
                "free_throws": free_throws,
                "and_one": and_one,
                "foul_id": foul_id,
            }),
            Fact::FreeThrow {
                shooter,
                made,
                index,
                total,
                foul_id,
            } => json!({
                "shooter_id": self.player_id(*shooter),
                "made": made,
                "index": index,
                "total": total,
                "foul_id": foul_id,
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
            Fact::ReboundOpportunity {
                players,
                contestable,
            } => json!({
                "players": players,
                "contestable": contestable,
            }),
            Fact::LooseBallRecovered { player, kind } => json!({
                "player": player,
                "kind": kind,
            }),
            Fact::SecondChanceStarted {
                rebounder,
                possession,
            } => json!({
                "rebounder_id": self.player_id(*rebounder),
                "possession": possession,
            }),
            Fact::Cut {
                cutter,
                arrived,
                defender_distance_ft,
            } => json!({
                "cutter_id": self.player_id(*cutter),
                "arrived": arrived,
                "defender_distance_ft": defender_distance_ft,
            }),
            Fact::BoxOut {
                player,
                distance_ft,
            } => json!({
                "player_id": self.player_id(*player),
                "distance_ft": distance_ft,
            }),
            Fact::DefenseResponsibilityChanged { player, from, to } => json!({
                "player_id": self.player_id(*player),
                "from": from,
                "to": to,
            }),
            Fact::Turnover { player, kind } => json!({
                "player_id": self.player_id(*player),
                "turnover_kind": kind.as_str(),
            }),
            Fact::Steal { player, victim } => json!({
                "player_id": self.player_id(*player),
                "victim_id": self.player_id(*victim),
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
            Fact::BenchDepleted { team, player } => json!({
                "team": team.as_str(),
                "player_id": self.player_id(*player),
            }),
            Fact::FoulOutRetention {
                team,
                player,
                foul_id,
            } => json!({
                "team": team.as_str(),
                "player_id": self.player_id(*player),
                "foul_id": foul_id,
            }),
            Fact::TeamTechnicalFoul {
                team,
                reason,
                foul_id,
            } => json!({
                "team": team.as_str(),
                "reason": reason,
                "foul_id": foul_id,
            }),
            Fact::PlayActivated { play, possession } => {
                json!({ "play_id": PLAY_SOURCES[*play].0, "possession": possession })
            }
            Fact::Timeout { team, remaining } => json!({
                "team": team.as_str(),
                "home_remaining": remaining[0],
                "away_remaining": remaining[1],
            }),
            Fact::JumpBall {
                kind,
                home_jumper,
                away_jumper,
                tapper,
                tapped_team,
            } => json!({
                "kind": format!("{kind:?}").to_lowercase(),
                "home_jumper_id": self.player_id(*home_jumper),
                "away_jumper_id": self.player_id(*away_jumper),
                "tapper_id": self.player_id(*tapper),
                "tapped_team": tapped_team.as_str(),
            }),
            Fact::PlayWindowEnded { play, possession } => {
                json!({ "play_id": PLAY_SOURCES[*play].0, "possession": possession })
            }
            Fact::PeriodEnd { period } => json!({ "period": period }),
            Fact::BallContact {
                kind,
                entity,
                time,
                x,
                y,
                z,
                sub_tick,
            } => json!({
                "contact_kind": kind,
                "player_id": entity.map(|handle| self.player_id(handle)),
                "contact_time": time,
                "x": x,
                "y": y,
                "z": z,
                "sub_tick": sub_tick,
            }),
            Fact::PlacementApplied { player, reason } => json!({
                "player_id": self.player_id(*player),
                "reason": reason,
            }),
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
                zone: self.zone_for_position(player.pos, player.team),
                has_ball: self.holder() == Some(self.player_index(&player.id)),
                on_court: true,
                action: player.slot.as_str().to_string(),
                action_phase: player
                    .action
                    .as_ref()
                    .map(|runtime| match runtime.phase {
                        nba_domain::action_window::ActionPhase::Preparation => "preparation",
                        nba_domain::action_window::ActionPhase::Execution => "execution",
                        nba_domain::action_window::ActionPhase::FollowThrough => "follow_through",
                    }.to_string()),
                job: Some(player.offense_job.as_str().to_string()),
                responsibility: Some(player.defense_responsibility.as_str().to_string()),
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
        let state = self.ball.state();
        let (ball_pos, ball_z, ball_status) = match &state {
            BallState::Held { holder } => (self.players[*holder].pos, 4.0, "held"),
            BallState::Pass { .. } => (self.ball.pos, self.ball.z, "pass"),
            BallState::Shot { .. } => (self.ball.pos, self.ball.z, "shot"),
            BallState::Loose { .. } => (self.ball.pos, self.ball.z.max(0.5), "loose"),
        };
        let ball_norm = Court::ft_to_norm_with_geometry(ball_pos, self.rules.court);

        let recent_start = self
            .events
            .iter()
            .rposition(|event| self.now() - event.time >= 0.051)
            .map(|index| index + 1)
            .unwrap_or(0);
        let recent: Vec<FrameEvent> = self.events[recent_start..]
            .iter()
            .map(|event| self.frame_event(event))
            .collect();

        let event_names: Vec<String> = recent.iter().map(|event| event.kind.clone()).collect();
        let primary = event_names.first().cloned();

        StreamTick {
            frame: RenderFrame {
                t: self.now() as f32,
                t_game: self.clock.elapsed() as f32,
                tick_index: self.clock.tick(),
                shot_clock: self.clock.shot_clock() as f32,
                period: self.clock.period(),
                phase: format!("{:?}", self.phase()),
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
                    vx: self.ball.vel.x,
                    vy: self.ball.vel.y,
                    vz: self.ball.vz,
                    status: ball_status.to_string(),
                    holder_id: holder.map(|handle| self.player_id(handle)),
                },
                event_type: primary,
                events: event_names,
                game_flow: match self.phase() {
                    Phase::Live
                    | Phase::PassFlight { .. }
                    | Phase::ShotFlight { .. }
                    | Phase::JumpBallFlight => "LiveBall",
                    Phase::Finished => "Finished",
                    _ => "DeadBall",
                }
                .to_string(),
                team_fouls_home: self.fouls[0].period as u32,
                team_fouls_away: self.fouls[1].period as u32,
                free_throws_remaining: match self.phase() {
                    Phase::FreeThrows { remaining, .. } => remaining,
                    _ => 0,
                },
                defensive_tactic: None,
                event_sequence: self.clock.tick(),
                event_log: recent,
                simulation_complete: self.finished,
                completed_possessions: self.possession_log.len(),
                target_possessions: 0,
                callout: None,
                intensity: None,
                debug: self
                    .active_trace
                    .and_then(|index| {
                        self.decision_traces
                            .get(index as usize)
                            .map(|trace| (index, trace))
                    })
                    .map(|(index, trace)| Box::new(self.decision_debug(index, trace))),
                rules: frame_rules(&self.rules),
                stream_projection: "facts".to_string(),
                potential_field: Vec::new(),
            },
            tactical_set: String::new(),
            game_clock: self.clock.game_clock() as f32,
            keyframe_index: None,
        }
    }

    pub fn project_frame_event(&self, event: &TimedFact) -> FrameEvent {
        self.frame_event(event)
    }

    pub(crate) fn frame_event(&self, event: &TimedFact) -> FrameEvent {
        let (shot_id, pass_id) = match &event.fact {
            Fact::Shot { shot_id, .. } => (Some(format!("shot-{shot_id:06}")), None),
            Fact::Pass { pass_id, .. } => (None, Some(format!("pass-{pass_id:06}"))),
            _ => (None, None),
        };
        let contact = contact_geometry(&event.fact);
        FrameEvent {
            sequence: event.sequence,
            time: event.time as f32,
            kind: event.fact.kind().to_string(),
            x: contact.map(|geometry| geometry.0),
            y: contact.map(|geometry| geometry.1),
            z: contact.map(|geometry| geometry.2),
            sub_tick: contact.map_or(0, |geometry| geometry.3),
            contact_time: contact.map(|geometry| geometry.4 as f32),
            data: Some(self.fact_payload(&event.fact)),
            event_id: event.event_id,
            parent_event_id: None,
            tick_index: event.tick_index,
            action_id: None,
            shot_id,
            pass_id,
            loose_ball_id: None,
            foul_program_id: None,
            responsibility_id: None,
            movement_intent_id: None,
            play_instance_id: None,
            relations: Vec::new(),
            decision_trace: self.event_decision_trace(event),
        }
    }

    /// 动作事件带上产生它的那次决策的快照。
    ///
    /// 时钟、篮板、换人这类事件不在这里附带：它们不是一次决策的直接后果，
    /// 强行绑定会让「哪个决策导致了这次出手」这个问题失去区分度。
    fn event_decision_trace(&self, event: &TimedFact) -> Option<DecisionTrace> {
        let is_action = matches!(
            event.fact,
            Fact::Shot { .. } | Fact::Pass { .. } | Fact::Drive { .. } | Fact::PostUp { .. }
        );
        if !is_action {
            return None;
        }
        let index = event.trace?;
        let sample = self.decision_traces.get(index as usize)?;
        Some(self.decision_trace(index, sample))
    }

    fn decision_trace(
        &self,
        index: u32,
        sample: &crate::state::DecisionTraceSample,
    ) -> DecisionTrace {
        let chosen = sample
            .candidates
            .get(sample.chosen)
            .map(|candidate| candidate.kind.label())
            .unwrap_or_default()
            .to_string();
        DecisionTrace {
            trace_id: u64::from(index),
            actor_id: self.player_id(sample.actor),
            chosen,
            beta: sample.beta,
            gamma: sample.gamma,
            candidates: sample
                .candidates
                .iter()
                .map(|candidate| TraceCandidate {
                    action: candidate.kind.label().to_string(),
                    target_id: candidate.target.map(|handle| self.player_id(handle)),
                    total: candidate.total,
                    probability: candidate.probability,
                    epv: epv_of(&candidate.utility),
                    variance: candidate.utility.variance,
                    clock_cost: candidate.utility.clock_cost,
                    foul_cost: candidate.utility.foul_cost,
                    tactical_bonus: candidate.utility.tactical_bonus,
                })
                .collect(),
        }
    }

    fn decision_debug(
        &self,
        index: u32,
        sample: &crate::state::DecisionTraceSample,
    ) -> DecisionDebug {
        let chosen = sample
            .candidates
            .get(sample.chosen)
            .map(|candidate| candidate.kind.label())
            .unwrap_or_default()
            .to_string();
        let record = self
            .decision_records
            .iter()
            .rev()
            .find(|record| record.actor == sample.actor);
        let (decision_id, observation_digest, revalidation) = match record {
            Some(record) => (
                record.decision_id,
                record.observation_digest,
                Some(match &record.revalidation {
                    crate::state::Revalidation::Accepted => "accepted".to_string(),
                    crate::state::Revalidation::Cancelled(reason) => format!("cancelled:{reason}"),
                }),
            ),
            None => (0, 0, None),
        };
        DecisionDebug {
            player: self.player_id(sample.actor),
            chosen,
            utilities: sample
                .candidates
                .iter()
                .map(|candidate| DebugUtility {
                    kind: candidate.kind.label().to_string(),
                    utility: candidate.total,
                    epv: epv_of(&candidate.utility),
                    variance: candidate.utility.variance,
                    clock_cost: candidate.utility.clock_cost,
                    foul_cost: candidate.utility.foul_cost,
                    tactical_bonus: candidate.utility.tactical_bonus,
                    probability: candidate.probability,
                })
                .collect(),
            probabilities: sample
                .candidates
                .iter()
                .map(|candidate| nba_protocol::DebugProb {
                    kind: candidate.kind.label().to_string(),
                    prob: candidate.probability,
                })
                .collect(),
            flags: Vec::new(),
            blocked: Vec::new(),
            trace_id: index,
            decision_id,
            observation_digest,
            revalidation,
            beta: sample.beta,
            gamma: sample.gamma,
            active_constraints: Vec::new(),
            enforcement: Vec::new(),
            active_play_id: None,
            play_adjustments: Vec::new(),
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

/// 接触事实的几何与连续时刻。
fn contact_geometry(fact: &Fact) -> Option<(f32, f32, f32, u16, f64)> {
    match fact {
        Fact::BallContact {
            time,
            x,
            y,
            z,
            sub_tick,
            ..
        } => Some((*x, *y, *z, *sub_tick, *time)),
        _ => None,
    }
}

/// 把引擎内的效用分解投影成协议结构。
fn epv_of(utility: &crate::utility::Utility) -> DebugEpv {
    DebugEpv {
        ability: utility.epv.ability,
        contest: utility.epv.contest,
        lane: utility.epv.lane,
        help: utility.epv.help,
        tendency: utility.epv.tendency,
        context: utility.epv.context,
        turnover: utility.epv.turnover,
    }
}
