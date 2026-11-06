//! 协议投影：把动作链世界发布成 `StreamTick`。
//!
//! 表现层只消费这里的事实与几何，不参与裁定。事件载荷使用球员 id，
//! 索引只在本进程内部作为句柄使用。

use nba_domain::court::Court;
use nba_protocol::{
    DebugEpv, DebugUtility, DecisionDebug, DecisionTrace, FrameEvent, FrameRules, RenderAction,
    RenderBall, RenderFrame, RenderPlayer, RenderScore, RenderTeam, StreamTick, TraceCandidate,
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
            Self::FreeThrowLaneViolation { .. } => "FREE_THROW_LANE_VIOLATION",
            Self::Rebound { .. } => "REBOUND",
            Self::ReboundOpportunity { .. } => "REBOUND_OPPORTUNITY",
            Self::LooseBallRecovered { .. } => "LOOSE_BALL_RECOVERED",
            Self::SecondChanceStarted { .. } => "SECOND_CHANCE_STARTED",
            Self::Cut { .. } => "CUT",
            Self::DribbleMove { .. } => "DRIBBLE_MOVE",
            Self::FakeAction { .. } => "FAKE_ACTION",
            Self::BallProtection { .. } => "BALL_PROTECTION",
            Self::PokePrevented { .. } => "POKE_PREVENTED",
            Self::BoxOut { .. } => "BOX_OUT",
            Self::DefenseResponsibilityChanged { .. } => "DEFENSE_RESPONSIBILITY_CHANGED",
            Self::Turnover { .. } => "TURNOVER",
            Self::Steal { .. } => "STEAL",
            Self::Substitution { .. } => "SUBSTITUTION",
            Self::BenchDepleted { .. } => "BENCH_DEPLETED",
            Self::FoulOutRetention { .. } => "FOUL_OUT_RETENTION",
            Self::TeamTechnicalFoul { .. } => "TEAM_TECHNICAL_FOUL",
            Self::Ejected { .. } => "EJECTED",
            Self::OnBallPositioningChanged { .. } => "ON_BALL_POSITIONING_CHANGED",
            Self::Timeout { .. } => "TIMEOUT",
            Self::JumpBall { .. } => "JUMP_BALL",
            Self::PeriodEnd { .. } => "PERIOD_END",
            Self::PlayActivated { .. } => "PLAY_ACTIVATED",
            Self::PlayWindowEnded { .. } => "PLAY_WINDOW_ENDED",
            Self::BallContact { .. } => "BALL_CONTACT",
            Self::Goaltending { .. } => "GOALTENDING",
            Self::Review { .. } => "REVIEW",
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
            Fact::ActionIntentRejected {
                player,
                kind,
                reason,
            } => json!({
                "player_id": self.player_id(*player),
                "action": kind,
                "reason": reason,
            }),
            Fact::ActionPhaseChanged {
                action_id,
                player,
                from,
                to,
            } => json!({
                "action_id": action_id,
                "player_id": self.player_id(*player),
                "from": format!("{from:?}"),
                "to": format!("{to:?}"),
            }),
            Fact::ActionCompleted {
                action_id,
                player,
                kind,
            }
            | Fact::ActionFailed {
                action_id,
                player,
                kind,
            } => json!({
                "action_id": action_id,
                "player_id": self.player_id(*player),
                "action": kind,
            }),
            Fact::ActionCancelled {
                action_id,
                player,
                kind,
                reason,
            } => json!({
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
            Fact::DefensiveActionPhaseChanged {
                action_id,
                defender,
                from,
                to,
            } => json!({
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
            Fact::DefensiveActionCancelled {
                action_id,
                defender,
                shooter,
                kind,
                reason,
            } => json!({
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
                kind,
            } => json!({
                "driver_id": self.player_id(*driver),
                "reached": reached,
                "defender_beaten": defender_beaten,
                "distance_ft": distance_ft,
                "separation_ft": separation_ft,
                "kind": kind,
            }),
            Fact::PostUp { player, kind } => json!({
                "player_id": self.player_id(*player),
                "kind": kind,
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
            Fact::FreeThrowLaneViolation { side } => json!({
                "side": side,
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
                kind,
            } => json!({
                "cutter_id": self.player_id(*cutter),
                "arrived": arrived,
                "defender_distance_ft": defender_distance_ft,
                "kind": kind,
            }),
            Fact::DribbleMove {
                player,
                kind,
                separation_ft,
                exposure,
            } => json!({
                "player_id": self.player_id(*player),
                "dribble": format!("{kind:?}"),
                "separation_ft": separation_ft,
                "exposure": exposure,
            }),
            Fact::FakeAction {
                player,
                defender,
                kind,
                bitten,
                displacement_ft,
            } => json!({
                "player_id": self.player_id(*player),
                "defender_id": defender.map(|handle| self.player_id(handle)),
                "fake": kind.as_str(),
                "bitten": bitten,
                "displacement_ft": displacement_ft,
            }),
            Fact::BallProtection {
                player,
                kind,
                seconds,
            } => json!({
                "player_id": self.player_id(*player),
                "protection": kind.as_str(),
                "seconds": seconds,
            }),
            Fact::PokePrevented {
                player,
                defender,
                kind,
            } => json!({
                "player_id": self.player_id(*player),
                "defender_id": self.player_id(*defender),
                "protection": kind.as_str(),
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
            Fact::Ejected {
                team,
                player,
                reason,
            } => json!({
                "team": team.as_str(),
                "player_id": self.player_id(*player),
                "reason": reason,
            }),
            Fact::OnBallPositioningChanged { defender, kind } => json!({
                "defender_id": self.player_id(*defender),
                "kind": kind,
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
            Fact::Goaltending {
                defender,
                shooter,
                shot_id,
                time,
            } => json!({
                "defender_id": self.player_id(*defender),
                "shooter_id": self.player_id(*shooter),
                "shot_id": shot_id,
                "time": time,
            }),
            Fact::Review {
                team,
                call,
                overturned,
                cost_timeout,
                time,
            } => json!({
                "team": team,
                "call": call,
                "overturned": overturned,
                "cost_timeout": cost_timeout,
                "time": time,
            }),
        }
    }

    fn player_active_actions(
        &self,
        player_index: usize,
        player: &crate::state::PlayerState,
    ) -> Vec<RenderAction> {
        let mut actions = Vec::new();

        if let Some(runtime) = &player.action {
            let (family, kind, variant) = match runtime.intent.kind {
                crate::action::ActionKind::Dribble(move_kind) => {
                    let v = match move_kind {
                        nba_domain::action_window::DribbleMoveKind::DirectDrive => "direct_drive",
                        nba_domain::action_window::DribbleMoveKind::Crossover => "crossover",
                        nba_domain::action_window::DribbleMoveKind::BetweenTheLegs => {
                            "between_the_legs"
                        }
                        nba_domain::action_window::DribbleMoveKind::BehindTheBack => {
                            "behind_the_back"
                        }
                        nba_domain::action_window::DribbleMoveKind::SpinMove => "spin_move",
                        nba_domain::action_window::DribbleMoveKind::Hesitation => "hesitation",
                        nba_domain::action_window::DribbleMoveKind::Retreat => "retreat",
                    };
                    ("ACTION-DRIBBLE", "dribble", v)
                }
                crate::action::ActionKind::Drive(drive_kind) => {
                    ("ACTION-DRIVE", "drive", drive_kind.as_str())
                }
                crate::action::ActionKind::Finish => ("ACTION-FINISH", "finish", "rim_finish"),
                crate::action::ActionKind::Pass(pass_kind) => {
                    let v = match pass_kind {
                        crate::action::PassKind::Chest => "chest",
                        crate::action::PassKind::Bounce => "bounce",
                        crate::action::PassKind::Lob => "lob",
                        crate::action::PassKind::Outlet => "outlet",
                        crate::action::PassKind::Overhead => "overhead",
                        crate::action::PassKind::OneHand => "one_hand",
                        crate::action::PassKind::BehindBack => "behind_back",
                        crate::action::PassKind::Handoff => "handoff",
                    };
                    ("ACTION-PASS", "pass", v)
                }
                crate::action::ActionKind::Screen(screen_kind) => {
                    let v = match screen_kind {
                        crate::action::ScreenKind::OnBall => "on_ball",
                        crate::action::ScreenKind::OffBall => "off_ball",
                        crate::action::ScreenKind::HandOff => "handoff",
                    };
                    ("ACTION-SCREEN", "screen", v)
                }
                crate::action::ActionKind::Cut(cut_kind) => {
                    ("ACTION-CUT", "cut", cut_kind.as_str())
                }
                crate::action::ActionKind::PostUp => ("ACTION-POST", "post_up", "post_up"),
                crate::action::ActionKind::Shoot(shot_kind) => {
                    if shot_kind.is_rim_finish() {
                        ("ACTION-FINISH", "finish", shot_kind.as_str())
                    } else {
                        ("ACTION-SHOT", "jump_shot", shot_kind.as_str())
                    }
                }
                crate::action::ActionKind::Relocate => ("ACTION-RELOCATE", "relocate", "relocate"),
                crate::action::ActionKind::Fake(fake_kind) => {
                    ("ACTION-FAKE", "fake", fake_kind.as_str())
                }
                crate::action::ActionKind::Protect(protect_kind) => {
                    ("ACTION-BALL-PROTECTION", "protect", protect_kind.as_str())
                }
            };

            let role_str = match runtime.role {
                crate::action::ActionIntentRole::Primary => "primary",
                crate::action::ActionIntentRole::ScreenPartner => "screen_partner",
                crate::action::ActionIntentRole::CutPartner => "cut_partner",
            };

            let phase_str = match runtime.phase {
                nba_domain::action_window::ActionPhase::Preparation => "preparation",
                nba_domain::action_window::ActionPhase::Execution => "execution",
                nba_domain::action_window::ActionPhase::FollowThrough => "follow_through",
            };

            let phase_started = match runtime.phase {
                nba_domain::action_window::ActionPhase::Preparation => runtime.intent.start_time,
                nba_domain::action_window::ActionPhase::Execution => runtime.preparation_ends_at,
                nba_domain::action_window::ActionPhase::FollowThrough => runtime.execution_ends_at,
            };

            let mut participants = Vec::new();
            let mut target_id = None;
            if let Some(target) = runtime.intent.target {
                if target < self.players.len() {
                    let pid = self.player_id(target);
                    participants.push(pid.clone());
                    target_id = Some(pid);
                }
            }

            actions.push(RenderAction {
                action_id: runtime.action_id,
                interaction_id: Some(runtime.interaction_id),
                source: "action_window".to_string(),
                family: family.to_string(),
                kind: kind.to_string(),
                variant: variant.to_string(),
                role: role_str.to_string(),
                phase: phase_str.to_string(),
                started_at: runtime.intent.start_time as f32,
                phase_started_at: phase_started as f32,
                ends_at: Some(runtime.ends_at as f32),
                participants,
                target_id,
            });
        }

        if let Some(Some(def_action)) = self.defensive_actions.get(player_index) {
            let (family, kind, variant) = match def_action.kind {
                crate::action::DefensiveActionKind::CloseoutContest => {
                    ("ACTION-CLOSEOUT", "closeout_contest", "contest")
                }
                crate::action::DefensiveActionKind::BlockJump => {
                    ("ACTION-CLOSEOUT", "block_jump", "block")
                }
                crate::action::DefensiveActionKind::StealAttempt => {
                    ("ACTION-DEFENSE-ON-BALL", "steal_attempt", "steal")
                }
                crate::action::DefensiveActionKind::Slide => {
                    ("ACTION-DEFENSE-ON-BALL", "slide", "slide")
                }
                crate::action::DefensiveActionKind::Contain => {
                    ("ACTION-DEFENSE-ON-BALL", "contain", "contain")
                }
                crate::action::DefensiveActionKind::ChargeTake => {
                    ("ACTION-DEFENSE-ON-BALL", "charge_take", "charge_take")
                }
            };

            let phase_str = match def_action.phase {
                nba_domain::action_window::ActionPhase::Preparation => "preparation",
                nba_domain::action_window::ActionPhase::Execution => "execution",
                nba_domain::action_window::ActionPhase::FollowThrough => "follow_through",
            };

            let phase_started = match def_action.phase {
                nba_domain::action_window::ActionPhase::Preparation => {
                    (def_action.preparation_ends_at - 0.2).max(0.0)
                }
                nba_domain::action_window::ActionPhase::Execution => def_action.preparation_ends_at,
                nba_domain::action_window::ActionPhase::FollowThrough => {
                    def_action.execution_ends_at
                }
            };

            let shooter_id = if def_action.shooter < self.players.len() {
                Some(self.player_id(def_action.shooter))
            } else {
                None
            };
            let participants = shooter_id.clone().into_iter().collect();

            actions.push(RenderAction {
                action_id: def_action.action_id,
                interaction_id: Some(def_action.interaction_id),
                source: "defensive_action".to_string(),
                family: family.to_string(),
                kind: kind.to_string(),
                variant: variant.to_string(),
                role: "defender".to_string(),
                phase: phase_str.to_string(),
                started_at: (def_action.preparation_ends_at - 0.2).max(0.0) as f32,
                phase_started_at: phase_started as f32,
                ends_at: Some(def_action.ends_at as f32),
                participants,
                target_id: shooter_id,
            });
        } else if let Some(pos) = player.on_ball_positioning {
            let (kind, variant) = match pos {
                crate::action::OnBallPositioning::Slide => ("slide", "slide"),
                crate::action::OnBallPositioning::Contain => ("contain", "contain"),
                crate::action::OnBallPositioning::ChargeTake => ("charge_take", "charge_take"),
            };
            let holder_id = self.holder().map(|h| self.player_id(h));
            let participants = holder_id.clone().into_iter().collect();
            actions.push(RenderAction {
                action_id: 0x8000_0000_0000_0000 | (player_index as u64),
                interaction_id: None,
                source: "defense_posture".to_string(),
                family: "ACTION-DEFENSE-ON-BALL".to_string(),
                kind: kind.to_string(),
                variant: variant.to_string(),
                role: "on_ball_defender".to_string(),
                phase: "sustained".to_string(),
                started_at: player.assignment_since as f32,
                phase_started_at: player.assignment_since as f32,
                ends_at: None,
                participants,
                target_id: holder_id,
            });
        }

        if player.defense_responsibility == crate::state::DefenseResponsibility::BoxOut {
            let partner_id = if player.target_partner < self.players.len() {
                Some(self.player_id(player.target_partner))
            } else {
                None
            };
            let participants = partner_id.clone().into_iter().collect();
            actions.push(RenderAction {
                action_id: 0x8100_0000_0000_0000 | (player_index as u64),
                interaction_id: None,
                source: "defense_posture".to_string(),
                family: "ACTION-REBOUND".to_string(),
                kind: "box_out".to_string(),
                variant: "box_out".to_string(),
                role: "rebounder".to_string(),
                phase: "sustained".to_string(),
                started_at: player.assignment_since as f32,
                phase_started_at: player.assignment_since as f32,
                ends_at: None,
                participants,
                target_id: partner_id,
            });
        }

        actions
    }

    pub fn stream_tick(&self) -> StreamTick {
        let mut players = Vec::new();
        for (player_index, player) in self
            .players
            .iter()
            .enumerate()
            .filter(|(_, player)| player.on_court)
        {
            let norm = Court::ft_to_norm_with_geometry(player.pos, self.rules.court);
            let target = Court::ft_to_norm_with_geometry(player.target, self.rules.court);
            let active_actions = self.player_active_actions(player_index, player);
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
                action_phase: player.action.as_ref().map(|runtime| {
                    match runtime.phase {
                        nba_domain::action_window::ActionPhase::Preparation => "preparation",
                        nba_domain::action_window::ActionPhase::Execution => "execution",
                        nba_domain::action_window::ActionPhase::FollowThrough => "follow_through",
                    }
                    .to_string()
                }),
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
                active_actions,
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
                    Phase::Live => "LiveBall",
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
            .clocks
            .shot_clock_seconds
            .max(rules.league.clocks.offensive_rebound_shot_clock_seconds),
        holder_leash_ft: rules.invariant_holder_leash_ft,
        speed_tolerance_ftps: rules.invariant_speed_tolerance_ftps,
        ball_z_max_ft: rules.ball_z_max_ft,
        max_personal_fouls: rules.league.fouls.max_personal_fouls,
        bonus_fouls_per_period: rules.league.fouls.bonus_fouls_per_period,
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
