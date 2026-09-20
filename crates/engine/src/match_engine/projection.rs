//! 只读投影与协议输出：`EngineSnapshot` 借用视图与 `StreamTick` 渲染帧。
//!
//! 依据 `docs/architecture.md` §2 [12] EmitPhase：快照阶段无副作用，且是
//! InvariantPhase 之后唯一允许的动作。

use glam::Vec2;
use nba_domain::court::Court;
use nba_domain::{GameEvent, Possession};
use nba_physics::ballistics::BallTrajectoryKind;
use nba_protocol::{
    DebugFlag, DebugProb, DebugUtility, DecisionDebug, RenderBall, RenderFrame, RenderPlayer,
    RenderScore, RenderTeam, StreamTick,
};
use nba_semantics::SemanticContact;

use crate::snapshot::{BallStateView, EngineSnapshot, GameStateView, LineupStateView};

use super::stream::frame_rules_from_game_rules;
use super::MatchEngine;

impl MatchEngine {
    /// 返回对引擎内部状态的零拷贝借用只读投影（D14）。
    ///
    /// 供 CLI、评判器、测试、回放等统一获取只读视图，不产生任何副作用。
    pub fn engine_snapshot(&self) -> EngineSnapshot<'_> {
        EngineSnapshot {
            game: GameStateView {
                period: self.clock.period,
                game_clock: self.clock.game_clock,
                shot_clock: self.clock.shot_clock,
                current_time: self.clock.current_time,
                home_score: self.ledger.home_score,
                away_score: self.ledger.away_score,
                possession: self.flow.possession,
                possession_id: self.flow.possession_id,
                sub_phase: self.clock.sub_phase,
                sub_phase_timer: self.clock.sub_phase_timer,
                tactical_set: self.config.tactical_set,
                game_flow: &self.flow.game_flow,
                possession_arrow: self.flow.possession_arrow,
            },
            ball: BallStateView {
                pos_3d: self.ball.ball_pos_3d,
                state: &self.ball.ball_state,
                active_carrier_or_focus_id: self.active_carrier_or_focus_id(),
            },
            lineups: LineupStateView {
                home_roster_order: &self.config.home_roster_order,
                away_roster_order: &self.config.away_roster_order,
            },
            rules: &self.config.rules,
            box_score: &self.ledger.box_score,
            physics: &self.systems.physics,
        }
    }

    /// 构建面向前端/协议传输的完整渲染帧（包含克隆与序列化准备）。
    pub fn render_frame(&self) -> StreamTick {
        self.build_tick()
    }

    /// Returns a protocol snapshot without advancing the simulation.
    pub fn snapshot(&self) -> StreamTick {
        self.render_frame()
    }

    pub(crate) fn build_tick(&self) -> StreamTick {
        let active_carrier = match &self.ball.ball_state {
            BallTrajectoryKind::Held { carrier_id }
            | BallTrajectoryKind::InboundReady {
                inbounder_id: carrier_id,
                ..
            }
            | BallTrajectoryKind::Drive {
                driver_id: carrier_id,
                ..
            } => Some(carrier_id.clone()),
            _ => None,
        };
        let mut render_players = self
            .systems
            .physics
            .get_players()
            .values()
            .map(|p| {
                let norm = Court::ft_to_norm_with_geometry(p.pos_ft, self.config.rules.court);
                let target_norm = if p.on_court {
                    Some(Court::ft_to_norm_with_geometry(
                        p.target_pos_ft,
                        self.config.rules.court,
                    ))
                } else {
                    None
                };
                RenderPlayer {
                    id: p.id.clone(),
                    jersey: p.jersey.clone(),
                    team: p.team.clone(),
                    x: (norm.x * 1_000_000.0).round() / 1_000_000.0,
                    y: (norm.y * 1_000_000.0).round() / 1_000_000.0,
                    zone: format!(
                        "{:?}",
                        self.config.rules.court.region(
                            p.pos_ft,
                            p.team == "home",
                            self.config.rules.league.three_point_distance_ft,
                        )
                    ),
                    has_ball: active_carrier.as_deref() == Some(p.id.as_str()),
                    on_court: p.on_court,
                    action: p.action.clone(),
                    slot: p.slot.clone(),
                    morale: p.morale.clone(),
                    stm: (p.stamina * 10.0).round() / 10.0,
                    stm_max: p.max_stamina,
                    foul_count: p.foul_count,
                    target_x: target_norm.map(|t| (t.x * 1_000_000.0).round() / 1_000_000.0),
                    target_y: target_norm.map(|t| (t.y * 1_000_000.0).round() / 1_000_000.0),
                    facing_x: if p.on_court {
                        Some((p.facing_dir.x * 1000.0).round() / 1000.0)
                    } else {
                        None
                    },
                    facing_y: if p.on_court {
                        Some((p.facing_dir.y * 1000.0).round() / 1000.0)
                    } else {
                        None
                    },
                }
            })
            .collect::<Vec<_>>();
        render_players.sort_by(|a, b| a.id.cmp(&b.id));
        let ball_norm =
            Court::ft_to_norm_with_geometry(self.ball.ball_pos_3d.0, self.config.rules.court);

        let ball_status = match &self.ball.ball_state {
            BallTrajectoryKind::Held { .. } => "HELD",
            BallTrajectoryKind::ControlTransfer { .. } => "CONTROL_TRANSFER",
            BallTrajectoryKind::InboundTransfer { .. } => "INBOUND_TRANSFER",
            BallTrajectoryKind::InboundReady { .. } => "INBOUND_READY",
            BallTrajectoryKind::Pass { .. } => "PASS",
            BallTrajectoryKind::Drive { .. } => "DRIVE",
            BallTrajectoryKind::Shot { .. } => "SHOT",
            BallTrajectoryKind::RimRebound { .. } => "REBOUND",
            BallTrajectoryKind::LooseBall { .. } => "LOOSE_BALL",
            BallTrajectoryKind::Dead { .. } => "DEAD",
        };
        let home_team = RenderTeam {
            id: self.config.home_team.id.clone(),
            name: self.config.home_team.name.clone(),
            short_name: self.config.home_team.short_name.clone(),
        };
        let away_team = RenderTeam {
            id: self.config.away_team.id.clone(),
            name: self.config.away_team.name.clone(),
            short_name: self.config.away_team.short_name.clone(),
        };
        let possession_team = match self.flow.possession {
            Possession::Home => "home",
            Possession::Away => "away",
        };
        let frame = RenderFrame {
            t: (self.clock.current_time * 100.0).round() / 100.0,
            t_game: (self.clock.game_clock * 10.0).round() / 10.0,
            shot_clock: (self.clock.shot_clock * 10.0).round() / 10.0,
            period: self.clock.period,
            phase: format!("{:?}", self.clock.sub_phase),
            possession_id: self.flow.possession_id,
            possession_team: possession_team.to_string(),
            home_team,
            away_team,
            score: RenderScore {
                home: self.ledger.home_score,
                away: self.ledger.away_score,
            },
            players: render_players,
            ball: RenderBall {
                x: ball_norm.x,
                y: ball_norm.y,
                z: (self.ball.ball_pos_3d.1 * 100.0).round() / 100.0,
                status: ball_status.to_string(),
                holder_id: active_carrier.clone(),
            },
            event_type: self.journal.current_event.clone(),
            events: self.journal.current_event_types.clone(),
            game_flow: format!("{:?}", self.flow.game_flow),
            team_fouls_home: self.ledger.team_fouls_home,
            team_fouls_away: self.ledger.team_fouls_away,
            free_throws_remaining: self.ledger.free_throws_remaining,
            defensive_tactic: Some(self.defensive_tactic_name()),
            event_sequence: self.journal.event_sequence,
            event_log: self.journal.current_event_log.clone(),
            simulation_complete: self.flow.simulation_complete,
            completed_possessions: self.flow.completed_possessions,
            target_possessions: self.flow.target_possessions,
            callout: self.journal.current_callout.clone(),
            intensity: self.journal.current_intensity.clone(),
            rules: frame_rules_from_game_rules(&self.config.rules),
            stream_projection: "full".to_string(),
            debug: self.observations.last_decision_trace.clone(),
        };

        StreamTick {
            frame,
            tactical_set: self.config.tactical_set.name_zh().to_string(),
            game_clock: (self.clock.game_clock * 10.0).round() / 10.0,
            keyframe_index: None,
        }
    }
}

/// 领域事实到协议事件的适配层。
pub(crate) fn physics_fact_to_event(fact: nba_physics::PhysicsFact) -> GameEvent {
    match fact {
        nba_physics::PhysicsFact::BoundaryCross {
            entity_id,
            attempted_pos,
            boundary_name,
        } => GameEvent::BoundaryCross {
            player_id: entity_id,
            pos: (attempted_pos.x, attempted_pos.y),
            boundary_name,
        },
    }
}

pub(crate) fn semantic_contact_to_event(contact: &SemanticContact) -> GameEvent {
    let relative_velocity = contact.raw.relative_velocity.unwrap_or(Vec2::ZERO);
    GameEvent::Contact {
        player_a: contact.raw.entity_a.clone(),
        player_b: contact.raw.entity_b.clone(),
        impact_speed: relative_velocity.length(),
        contact_normal: (contact.raw.normal.x, contact.raw.normal.y),
        is_screen: matches!(
            contact.kind,
            nba_semantics::ContactKind::LegalScreen
                | nba_semantics::ContactKind::IllegalScreenCandidate
        ),
        semantic_kind: format!("{:?}", contact.kind),
        semantic_severity: format!("{:?}", contact.severity),
        possessor_id: contact.context.possessor.clone(),
        legal_position: contact.context.legal_position,
    }
}

/// 与 physics 层 `is_inbound_role` 对应的发球角色判定（引擎侧）。
pub(crate) fn is_inbound_role_action(action: &str) -> bool {
    action == "INBOUND_SETUP" || action == "InboundPositioning" || action == "INBOUND_READY"
}

pub(crate) fn opposite(p: Possession) -> Possession {
    match p {
        Possession::Home => Possession::Away,
        Possession::Away => Possession::Home,
    }
}

/// 转换决策追踪到协议调试层。
pub(crate) fn convert_trace(trace: &nba_decision::pipeline::DecisionTrace) -> DecisionDebug {
    DecisionDebug {
        player: trace.player_id.clone(),
        chosen: trace.chosen_label.clone(),
        utilities: trace
            .utilities
            .iter()
            .map(|(k, u)| DebugUtility {
                kind: k.clone(),
                utility: *u,
            })
            .collect(),
        probabilities: trace
            .probabilities
            .iter()
            .map(|(k, p)| DebugProb {
                kind: k.clone(),
                prob: *p,
            })
            .collect(),
        flags: trace
            .flags_full
            .iter()
            .map(|(id, reason, penalty)| DebugFlag {
                constraint: id.to_string(),
                reason: reason.clone(),
                penalty: *penalty,
            })
            .collect(),
        blocked: trace
            .blocked
            .iter()
            .map(|(cand, why)| format!("{} ✗ {}", cand, why))
            .collect(),
        active_constraints: trace
            .active_constraints
            .iter()
            .map(|s| s.to_string())
            .collect(),
        enforcement: trace.enforcement.clone(),
    }
}
