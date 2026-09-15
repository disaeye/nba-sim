use glam::Vec2;
use rand::Rng;
use std::collections::HashMap;

use nba_domain::event::GameEvent;
use nba_physics::movement::PlayerPhysicsState;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DriveResolution {
    pub successful: bool,
    pub finish_made: bool,
    pub shooting_foul: bool,
    pub finish_kind: nba_domain::action_window::RimFinishKind,
}
impl DriveResolution {
    /// Resolve the semantic outcome of a drive from spatial facts and policy.
    ///
    /// The officiating layer owns stochastic adjudication, while the caller
    /// supplies facts measured by physics and attributes supplied by setup.
    /// This keeps geometry out of the rules calculation and keeps outcomes
    /// independent from presentation movement.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve(
        base_success: f32,
        finish_rate: f32,
        foul_rate: f32,
        finishing_skill: f32,
        stamina: f32,
        lane_density: f32,
        contest_intensity: f32,
        finish_block_bias: f32,
        finishing_weight: f32,
        policy: &nba_domain::resolve::DrivePolicy,
        rng: &mut impl Rng,
    ) -> Self {
        let skill_delta = (finishing_skill - 0.5) * finishing_weight * 2.0;
        let fatigue_delta = (stamina - 1.0) * finishing_weight;
        let reach_probability = (base_success + skill_delta + fatigue_delta
            - lane_density * policy.lane_density_penalty
            - contest_intensity * policy.contest_penalty)
            .clamp(0.0, 1.0);
        let successful = rng.gen_bool(reach_probability as f64);

        let foul_probability =
            (foul_rate * (0.5 + contest_intensity * policy.foul_contest_weight)).clamp(0.0, 1.0);
        let shooting_foul = rng.gen_bool(foul_probability as f64);

        let finish_probability = (finish_rate + skill_delta + fatigue_delta
            - contest_intensity * policy.finish_contest_penalty * finish_block_bias)
            .clamp(0.0, 1.0);
        let finish_made = successful && !shooting_foul && rng.gen_bool(finish_probability as f64);
        let finish_kind = if contest_intensity > 0.65 {
            nba_domain::action_window::RimFinishKind::Floater
        } else if finishing_skill > 0.75 && lane_density < 0.3 {
            nba_domain::action_window::RimFinishKind::Dunk
        } else {
            nba_domain::action_window::RimFinishKind::Layup
        };

        Self {
            successful,
            finish_made,
            shooting_foul,
            finish_kind,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ResolutionOutcome {
    NoChange,
    Score {
        points: u32,
        shooter_id: String,
    },
    Miss {
        rebound_spot: Vec2,
    },
    PassIntercepted {
        defender_id: String,
    },
    PassTipped {
        defender_id: String,
    },
    PassReceived {
        receiver_id: String,
    },
    Foul {
        fouled_player_id: String,
        fouler_id: String,
        is_shooting: bool,
    },
    ReboundSecured {
        rebounder_id: String,
        is_offensive: bool,
    },
}

pub struct ResolutionLayer;

impl ResolutionLayer {
    /// Resolve one physical pass-intersection opportunity using the policy
    /// accepted at the match boundary and the defender's supplied ability.
    pub fn resolve_pass_intersection_with_policy(
        event: &GameEvent,
        players: &HashMap<String, PlayerPhysicsState>,
        policy: &nba_domain::resolve::BaseRates,
        rng: &mut impl Rng,
    ) -> ResolutionOutcome {
        if let GameEvent::PathIntersection {
            defender_id,
            clearance_dist,
            ..
        } = event
        {
            let base_contest =
                (1.0 - (clearance_dist / policy.intercept_clearance_scale_ft)).clamp(0.0, 1.0);
            let steal_skill = players
                .get(defender_id)
                .map(|player| player.attributes.steal)
                .unwrap_or(0.5)
                .clamp(0.0, 1.0);
            let skill_factor = 0.5 + steal_skill;
            let steal_prob = (base_contest * policy.intercept_steal_slope * skill_factor)
                .clamp(policy.intercept_steal_floor, policy.intercept_steal_ceiling);
            let tip_prob = (base_contest * policy.intercept_tip_slope * skill_factor)
                .clamp(policy.intercept_tip_floor, policy.intercept_tip_ceiling);

            let roll = rng.gen::<f32>();
            if roll < steal_prob {
                return ResolutionOutcome::PassIntercepted {
                    defender_id: defender_id.clone(),
                };
            } else if roll < steal_prob + tip_prob {
                return ResolutionOutcome::PassTipped {
                    defender_id: defender_id.clone(),
                };
            }
        }
        ResolutionOutcome::NoChange
    }

    /// Resolves whether a released pass is secured by its intended receiver.
    /// Lane and openness are physical/semantic facts; abilities and policy
    /// provide the receiver-specific part of the probability.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_pass_arrival(
        passer: &PlayerPhysicsState,
        receiver: &PlayerPhysicsState,
        target_openness: f32,
        catch_equilibrium: f32,
        policy: &nba_domain::resolve::PassPolicy,
        base_success: f32,
        rng: &mut impl Rng,
    ) -> ResolutionOutcome {
        // Fix（round-15）：删除 lane_risk 项 —— 走廊风险的**双重计费**。
        //
        // 走廊拥堵是否化为事实，由 `resolve_pass_interception` 在飞行中
        // 独立裁决（拦截/点掉）；球既然干净到达接球人身旁（实测层 B 失败
        // 的 37 例中球距 p50=0.94 ft），就不应再因「出发时走廊拥挤」被
        // 降低接球概率。该项删除后，接球概率由接球当下的因素决定：
        // 空位程度、传球/控球技术、接球稳定性调制。
        let probability = (base_success
            + target_openness.clamp(0.0, 1.0) * policy.openness_weight
            + passer.attributes.passing * policy.passer_skill_weight
            + receiver.attributes.ball_handling * policy.receiver_control_weight
            + catch_equilibrium.clamp(0.0, 1.0) * policy.catch_equilibrium_weight)
            .clamp(0.0, 1.0);
        if rng.gen_bool(probability as f64) {
            ResolutionOutcome::PassReceived {
                receiver_id: receiver.id.clone(),
            }
        } else {
            ResolutionOutcome::NoChange
        }
    }

    /// Resolves an offensive-versus-defensive rebound contest from measured
    /// distances and player capabilities. The caller owns the physical facts;
    /// this layer owns only the semantic probability and random draw.
    pub fn resolve_rebound(
        offensive: &PlayerPhysicsState,
        defensive: &PlayerPhysicsState,
        offensive_distance: f32,
        defensive_distance: f32,
        policy: &nba_domain::resolve::ReboundPolicy,
        rng: &mut impl Rng,
    ) -> ResolutionOutcome {
        let total_distance = (offensive_distance + defensive_distance).max(f32::EPSILON);
        let distance_advantage = (defensive_distance - offensive_distance) / total_distance;
        let attribute_advantage =
            offensive.attributes.offensive_rebound - defensive.attributes.defensive_rebound;
        let stamina_advantage = normalized_stamina(offensive) - normalized_stamina(defensive);
        let positioning_advantage =
            offensive.attributes.off_ball_sense - defensive.attributes.off_ball_sense;
        let score = policy.base_offensive_rate
            + distance_advantage * policy.distance_weight
            + attribute_advantage * policy.attribute_weight
            + stamina_advantage * policy.stamina_weight
            + positioning_advantage * policy.positioning_weight;
        let offensive_probability = score.clamp(0.0, 1.0);
        let is_offensive = rng.gen_bool(offensive_probability as f64);
        ResolutionOutcome::ReboundSecured {
            rebounder_id: if is_offensive {
                offensive.id.clone()
            } else {
                defensive.id.clone()
            },
            is_offensive,
        }
    }

    /// Chooses among one side's candidates when the other side has no player
    /// in the landing window, preserving deterministic id ordering upstream.
    pub fn resolve_uncontested_rebound(
        candidates: &[PlayerPhysicsState],
        policy: &nba_domain::resolve::ReboundPolicy,
        rng: &mut impl Rng,
    ) -> ResolutionOutcome {
        let Some(winner) = candidates.iter().max_by(|left, right| {
            rebound_strength(left, policy)
                .partial_cmp(&rebound_strength(right, policy))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| right.id.cmp(&left.id))
        }) else {
            return ResolutionOutcome::NoChange;
        };
        let _ = rng;
        ResolutionOutcome::ReboundSecured {
            rebounder_id: winner.id.clone(),
            is_offensive: false,
        }
    }

    /// Adjudicates shot outcome at hoop arrival
    pub fn resolve_shot_arrival(
        is_made: bool,
        is_three: bool,
        shooter_id: &str,
    ) -> ResolutionOutcome {
        if is_made {
            let points = if is_three { 3 } else { 2 };
            ResolutionOutcome::Score {
                points,
                shooter_id: shooter_id.to_string(),
            }
        } else {
            ResolutionOutcome::Miss {
                rebound_spot: Vec2::ZERO,
            }
        }
    }

    /// Adjudicate a semantic contact after the semantic layer has classified
    /// its participants, role, severity, and legal position.
    pub fn resolve_semantic_contact(
        contact: &nba_semantics::SemanticContact,
        players: &HashMap<String, PlayerPhysicsState>,
        policy: &nba_domain::resolve::ContactPolicy,
        rng: &mut impl Rng,
    ) -> ResolutionOutcome {
        let is_foul_candidate = matches!(
            contact.severity,
            nba_semantics::ContactSeverity::FoulCandidate
        );
        let is_screen = matches!(
            contact.kind,
            nba_semantics::ContactKind::LegalScreen
                | nba_semantics::ContactKind::IllegalScreenCandidate
        );
        if !is_foul_candidate {
            return ResolutionOutcome::NoChange;
        }
        let (fouled_id, fouler_id) = match contact.kind {
            nba_semantics::ContactKind::BlockingCandidate => (
                contact
                    .context
                    .possessor
                    .clone()
                    .unwrap_or_else(|| contact.raw.entity_a.clone()),
                if contact.context.possessor.as_deref() == Some(contact.raw.entity_a.as_str()) {
                    contact.raw.entity_b.clone()
                } else {
                    contact.raw.entity_a.clone()
                },
            ),
            nba_semantics::ContactKind::ChargingCandidate => (
                contact
                    .context
                    .possessor
                    .clone()
                    .unwrap_or_else(|| contact.raw.entity_a.clone()),
                if contact.context.possessor.as_deref() == Some(contact.raw.entity_a.as_str()) {
                    contact.raw.entity_b.clone()
                } else {
                    contact.raw.entity_a.clone()
                },
            ),
            nba_semantics::ContactKind::IllegalScreenCandidate => {
                let (screener, defender) = if is_screen_action(&contact.context.action_a) {
                    (contact.raw.entity_a.clone(), contact.raw.entity_b.clone())
                } else {
                    (contact.raw.entity_b.clone(), contact.raw.entity_a.clone())
                };
                (defender, screener)
            }
            nba_semantics::ContactKind::ShootingContactCandidate => (
                contact
                    .context
                    .possessor
                    .clone()
                    .unwrap_or_else(|| contact.raw.entity_a.clone()),
                if contact.context.possessor.as_deref() == Some(contact.raw.entity_a.as_str()) {
                    contact.raw.entity_b.clone()
                } else {
                    contact.raw.entity_a.clone()
                },
            ),
            _ => {
                return Self::resolve_contact_with_policy(
                    &contact.raw.entity_a,
                    &contact.raw.entity_b,
                    contact.context.relative_speed,
                    is_screen,
                    players,
                    policy,
                    rng,
                )
            }
        };
        let impact_factor = ((contact.context.relative_speed - policy.threshold_speed_ftps)
            / policy.impact_scale_ftps.max(f32::EPSILON))
        .clamp(0.0, 1.0);
        let fouler_skill = players
            .get(&fouler_id)
            .map(|player| player.attributes.defense_perimeter)
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);
        let semantic_multiplier = if contact.context.legal_position {
            policy.legal_position_foul_multiplier
        } else {
            policy.illegal_position_foul_multiplier
        };
        let probability = (policy.foul_rate
            * (0.5 + impact_factor * 0.5)
            * semantic_multiplier
            * (1.0 + (0.5 - fouler_skill) * policy.defender_skill_foul_scale))
            .clamp(0.0, 1.0);
        if rng.gen::<f32>() < probability {
            if let (Some(fouled_p), Some(fouler_p)) =
                (players.get(&fouled_id), players.get(&fouler_id))
            {
                if fouled_p.team == fouler_p.team {
                    return ResolutionOutcome::NoChange;
                }
            }
            ResolutionOutcome::Foul {
                fouled_player_id: fouled_id,
                fouler_id,
                is_shooting: matches!(
                    contact.kind,
                    nba_semantics::ContactKind::BlockingCandidate
                        | nba_semantics::ContactKind::ChargingCandidate
                        | nba_semantics::ContactKind::ShootingContactCandidate
                ),
            }
        } else {
            ResolutionOutcome::NoChange
        }
    }

    /// Adjudicates soft contact / screen collisions for legacy physical facts.
    pub fn resolve_contact_with_policy(
        player_a_id: &str,
        player_b_id: &str,
        impact_speed: f32,
        is_screen: bool,
        players: &HashMap<String, PlayerPhysicsState>,
        policy: &nba_domain::resolve::ContactPolicy,
        rng: &mut impl Rng,
    ) -> ResolutionOutcome {
        if impact_speed > policy.threshold_speed_ftps {
            let impact_factor = ((impact_speed - policy.threshold_speed_ftps)
                / policy.impact_scale_ftps.max(f32::EPSILON))
            .clamp(0.0, 1.0);
            let screen_factor = if is_screen {
                policy.screen_foul_multiplier
            } else {
                1.0
            };
            let defender_skill = players
                .get(player_b_id)
                .map(|player| {
                    let is_interior = player.pos_ft.y.abs() < 12.0 && player.pos_ft.x.abs() < 10.0;
                    if is_interior {
                        player.attributes.defense_interior
                    } else {
                        player.attributes.defense_perimeter
                    }
                })
                .unwrap_or(0.5)
                .clamp(0.0, 1.0);
            let foul_probability = (policy.foul_rate
                * (0.5 + impact_factor * 0.5)
                * screen_factor
                * (1.0 + (0.5 - defender_skill) * policy.defender_skill_foul_scale))
                .clamp(0.0, 1.0);
            if rng.gen::<f32>() < foul_probability {
                if let (Some(p_a), Some(p_b)) = (players.get(player_a_id), players.get(player_b_id))
                {
                    if p_a.team == p_b.team {
                        return ResolutionOutcome::NoChange;
                    }
                }
                let (fouled, fouler) = if is_screen {
                    (player_a_id.to_string(), player_b_id.to_string())
                } else {
                    (player_b_id.to_string(), player_a_id.to_string())
                };
                return ResolutionOutcome::Foul {
                    fouled_player_id: fouled,
                    fouler_id: fouler,
                    is_shooting: false,
                };
            }
        }
        ResolutionOutcome::NoChange
    }
}

fn is_screen_action(action: &str) -> bool {
    action
        .split(|character: char| !character.is_ascii_alphanumeric())
        .any(|token| token.eq_ignore_ascii_case("screen") || token.eq_ignore_ascii_case("pick"))
}

fn normalized_stamina(player: &PlayerPhysicsState) -> f32 {
    (player.stamina / player.max_stamina.max(f32::EPSILON)).clamp(0.0, 1.0)
}

fn rebound_strength(
    player: &PlayerPhysicsState,
    policy: &nba_domain::resolve::ReboundPolicy,
) -> f32 {
    player.attributes.defensive_rebound * policy.strength_rebound_weight
        + player.attributes.off_ball_sense * policy.strength_positioning_weight
        + normalized_stamina(player) * policy.strength_stamina_weight
}

#[cfg(test)]
mod tests {
    use super::{ResolutionLayer, ResolutionOutcome};
    use glam::Vec2;
    use nba_domain::resolve::{ContactPolicy, PassPolicy, ReboundPolicy};
    use nba_domain::PlayerAttributes;
    use nba_physics::movement::{LocomotionState, PlayerPhysicsState};
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    use std::collections::HashMap;
    fn player(id: &str, passing: f32, ball_handling: f32, rebounding: f32) -> PlayerPhysicsState {
        let attributes = PlayerAttributes {
            passing,
            ball_handling,
            defensive_rebound: rebounding,
            offensive_rebound: rebounding,
            ..Default::default()
        };
        PlayerPhysicsState {
            id: id.to_string(),
            jersey: id.to_string(),
            team: "home".to_string(),
            pos_ft: Vec2::ZERO,
            vel_ft: Vec2::ZERO,
            accel_ft: Vec2::ZERO,
            target_pos_ft: Vec2::ZERO,
            target_speed_ftps: 0.0,
            max_speed_ftps: 22.0,
            max_accel_ftps2: 35.0,
            has_ball: false,
            on_court: true,
            action: "Idle".to_string(),
            slot: "Guard".to_string(),
            morale: "Normal".to_string(),
            stamina: 100.0,
            max_stamina: 100.0,
            foul_count: 0,
            locomotion: LocomotionState::Idle,
            facing_dir: Vec2::X,
            turn_decel_timer: 0.0,
            is_locked_kinematics: false,
            out_of_bounds_placement: false,
            is_receiving_pass: false,
            is_driving_to_rim: false,
            boundary_cross_latched: false,
            attributes,
            tendencies: Default::default(),
        }
    }

    #[test]
    fn pass_policy_can_make_release_outcome_deterministic() {
        let passer = player("passer", 1.0, 0.5, 0.5);
        let receiver = player("receiver", 0.5, 1.0, 0.5);
        let policy = PassPolicy {
            openness_weight: 0.0,
            passer_skill_weight: 0.0,
            receiver_control_weight: 0.0,
            catch_equilibrium_weight: 0.0,
        };
        let mut rng = StdRng::seed_from_u64(7);
        let received = ResolutionLayer::resolve_pass_arrival(
            &passer, &receiver, 0.0, 0.0, &policy, 1.0, &mut rng,
        );
        assert!(matches!(
            received,
            ResolutionOutcome::PassReceived { receiver_id } if receiver_id == "receiver"
        ));

        let mut rng = StdRng::seed_from_u64(7);
        let dropped = ResolutionLayer::resolve_pass_arrival(
            &passer, &receiver, 0.0, 0.0, &policy, 0.0, &mut rng,
        );
        assert!(matches!(dropped, ResolutionOutcome::NoChange));
    }

    #[test]
    fn contact_policy_uses_defender_skill_without_roster_specific_logic() {
        let mut players = HashMap::new();
        let mut defender = player("defender", 0.5, 0.5, 0.5);
        defender.attributes.defense_perimeter = 1.0;
        players.insert(defender.id.clone(), defender);
        let policy = ContactPolicy {
            foul_rate: 1.0,
            threshold_speed_ftps: 0.0,
            impact_scale_ftps: 1.0,
            screen_foul_multiplier: 1.0,
            defender_skill_foul_scale: 1.0,
            legal_position_foul_multiplier: 1.0,
            illegal_position_foul_multiplier: 1.0,
        };
        let mut rng = StdRng::seed_from_u64(12);
        let outcome = ResolutionLayer::resolve_contact_with_policy(
            "attacker", "defender", 1.0, false, &players, &policy, &mut rng,
        );
        assert!(matches!(outcome, ResolutionOutcome::Foul { .. }));
    }

    #[test]
    fn invalid_contact_policy_is_rejected_before_match_creation() {
        let mut config = nba_domain::resolve::ResolveConfig::default();
        config.contact.impact_scale_ftps = 0.0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn rebound_policy_uses_distance_and_capability_facts() {
        let offensive = player("offense", 0.5, 0.5, 1.0);
        let defensive = player("defense", 0.5, 0.5, 0.0);
        let policy = ReboundPolicy {
            base_offensive_rate: 0.0,
            distance_weight: 0.0,
            attribute_weight: 1.0,
            stamina_weight: 0.0,
            positioning_weight: 0.0,
            ..Default::default()
        };
        let mut rng = StdRng::seed_from_u64(8);
        let outcome =
            ResolutionLayer::resolve_rebound(&offensive, &defensive, 5.0, 5.0, &policy, &mut rng);
        assert!(matches!(
            outcome,
            ResolutionOutcome::ReboundSecured {
                rebounder_id,
                is_offensive: true
            } if rebounder_id == "offense"
        ));
    }
}
