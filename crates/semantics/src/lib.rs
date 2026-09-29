//! Basketball semantic facts derived from physical facts and match context.
//!
//! This crate deliberately stops before rule enforcement: physics reports what
//! happened, semantics explains the basketball context, and the engine/rules
//! layer decides what changes the game state.

use glam::Vec2;
use nba_domain::{GameRules, Possession, SubPhase};
use nba_physics::{RawContact, SpatialPhysics};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactKind {
    Incidental,
    LegalScreen,
    IllegalScreenCandidate,
    BlockingCandidate,
    ChargingCandidate,
    ShootingContactCandidate,
    ReboundContact,
    LooseBallContact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactSeverity {
    None,
    Minor,
    Positional,
    FoulCandidate,
}

#[derive(Debug, Clone)]
pub struct ContactContext {
    pub possessor: Option<String>,
    pub action_a: String,
    pub action_b: String,
    pub relative_speed: f32,
    /// Time for which the screener or defender has held its current position.
    /// The engine supplies this observation; semantics does not own a clock.
    pub setup_duration: f32,
    pub legal_position: bool,
}

#[derive(Debug, Clone)]
pub struct SemanticContact {
    pub tick: u64,
    pub raw: RawContact,
    pub kind: ContactKind,
    pub severity: ContactSeverity,
    pub context: ContactContext,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpacingEvaluation {
    pub offensive_team: Possession,
    pub rim_pressure: f32,
    pub lane_openness: f32,
    pub corner_spacing: f32,
    pub weak_side_space: f32,
    pub paint_crowding: f32,
    pub shot_quality_bonus: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShotEvaluation {
    pub distance_ft: f32,
    pub contest_intensity: f32,
    pub open_factor: f32,
    pub shot_quality: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PassEvaluation {
    pub lane_risk: f32,
    pub target_openness: f32,
    pub success_probability: f32,
}

/// Read-only basketball interpretation of the current physical world.
pub struct SemanticEvaluator;

impl SemanticEvaluator {
    pub fn spacing(
        offense: Possession,
        ball_pos: Vec2,
        physics: &dyn SpatialPhysics,
        rules: &GameRules,
    ) -> SpacingEvaluation {
        let team = team_name(offense);
        let hoop = rules.court.hoop_pos(offense == Possession::Home);
        let paint_crowding = normalized_density(
            hoop,
            team,
            physics,
            rules.teammate_density_radius_ft,
            rules.teammate_density_capacity,
            rules,
        );
        let lane_openness = (1.0
            - opponent_density(
                ball_pos,
                team,
                physics,
                rules.teammate_density_radius_ft,
                rules.semantics.opponent_density_capacity,
            ))
        .clamp(0.0, 1.0);
        let corner_spacing = perimeter_spacing(team, ball_pos, hoop, physics, true, rules);
        let weak_side_space = perimeter_spacing(team, ball_pos, hoop, physics, false, rules);
        SpacingEvaluation {
            offensive_team: offense,
            rim_pressure: (1.0 - (ball_pos - hoop).length() / rules.court.width_ft).clamp(0.0, 1.0),
            lane_openness,
            corner_spacing,
            weak_side_space,
            paint_crowding,
            shot_quality_bonus: (lane_openness * rules.semantics.spacing_lane_weight
                + corner_spacing * rules.semantics.spacing_corner_weight
                + weak_side_space * rules.semantics.spacing_weak_side_weight
                - paint_crowding * rules.semantics.spacing_paint_penalty)
                .clamp(-1.0, 1.0),
        }
    }

    /// 位置 `pos` 上进攻方承担的**多人**防守压迫：把 `pressure_radius_ft` 内
    /// 每名防守人的朝向投影与距离衰减求和。
    ///
    /// 与 `contest_intensity` 的分工（ADR-016）：后者是**最近一人**的距离、
    /// 朝向与速度合成的单人对位干扰，用于出手与传球的直接对抗；本量是
    /// **多人**覆盖叠加，用于判断一个位置是否处在协防网之内。
    pub fn defensive_pressure(
        pos: Vec2,
        offense: Possession,
        physics: &dyn SpatialPhysics,
        rules: &GameRules,
    ) -> f32 {
        let team = team_name(offense);
        let pressure = &rules.semantics;
        let ids = physics.query_nearby(
            pos,
            pressure.pressure_radius_ft,
            &nba_physics::EntityFilter::OpposingTeam(team.to_string()),
        );
        let players = physics.get_players();
        let defenders = ids
            .into_iter()
            .filter_map(|id| players.get(&id))
            .map(|player| (player.pos_ft, player.facing_dir));
        pressure.defensive_pressure(pos, defenders)
    }

    pub fn shot(
        shooter_id: &str,
        physics: &dyn SpatialPhysics,
        rules: &GameRules,
    ) -> Option<ShotEvaluation> {
        let shooter = physics.get_player(shooter_id)?;
        let hoop = rules.court.hoop_pos(shooter.team == "home");
        let distance_ft = (shooter.pos_ft - hoop).length();
        let openness = physics.openness(shooter_id);
        let open_factor = openness.contest_free_score();
        Some(ShotEvaluation {
            distance_ft,
            contest_intensity: openness.contest_intensity,
            open_factor,
            shot_quality: (open_factor
                * (1.0 - (distance_ft / rules.court.width_ft).clamp(0.0, 1.0)))
            .clamp(0.0, 1.0),
        })
    }

    pub fn pass(
        from: Vec2,
        to: Vec2,
        passer_id: &str,
        receiver_id: &str,
        physics: &dyn SpatialPhysics,
        rules: &GameRules,
    ) -> PassEvaluation {
        let corridor = physics.pass_corridor(
            from,
            to,
            rules.pass_corridor_radius_ft,
            passer_id,
            receiver_id,
        );
        let target_openness = physics.openness(receiver_id).contest_free_score();
        let semantic_rules = &rules.semantics;
        let lane_risk = if corridor.is_blocked {
            1.0
        } else {
            (1.0 - corridor
                .corridor_clearance
                .clamp(0.0, rules.pass_lane_clearance_reference_ft)
                / rules.pass_lane_clearance_reference_ft)
                .clamp(0.0, 1.0)
        };
        PassEvaluation {
            lane_risk,
            target_openness,
            success_probability: (semantic_rules.pass_base_probability
                + target_openness * semantic_rules.pass_openness_weight
                - lane_risk * semantic_rules.pass_lane_risk_weight)
                .clamp(0.0, 1.0),
        }
    }

    /// 传球时空走廊几何干涉分析（docs/protocol.md §5）：
    /// 计算防守人坐标与传球起点到目标点线段的最短空间投影距离。
    pub fn pass_corridor_distance(from: Vec2, to: Vec2, interceptor_pos: Vec2) -> f32 {
        let segment = to - from;
        let len_sq = segment.length_squared();
        if len_sq < 1e-4 {
            return (interceptor_pos - from).length();
        }
        let t = ((interceptor_pos - from).dot(segment) / len_sq).clamp(0.0, 1.0);
        let projection = from + segment * t;
        (interceptor_pos - projection).length()
    }
    /// Converts a raw contact into a contextual semantic fact. No foul, score,
    /// turnover, or possession mutation occurs here.
    #[allow(clippy::too_many_arguments)]
    pub fn contact(
        raw: RawContact,
        physics: &dyn SpatialPhysics,
        _possession: Possession,
        phase: SubPhase,
        shot_shooter: Option<&str>,
        ball_holder_id: Option<&str>,
        tick: u64,
        rules: &GameRules,
    ) -> SemanticContact {
        let players = physics.get_players();
        let a = players.get(&raw.entity_a);
        let b = players.get(&raw.entity_b);
        let action_a = raw
            .entity_a_action
            .clone()
            .or_else(|| a.map(|p| p.action.clone()))
            .unwrap_or_default();
        let action_b = raw
            .entity_b_action
            .clone()
            .or_else(|| b.map(|p| p.action.clone()))
            .unwrap_or_default();
        let relative_speed = raw.relative_velocity.map(|v| v.length()).unwrap_or(0.0);
        let screen_a = is_screen_action(&action_a);
        let screen_b = is_screen_action(&action_b);
        let possessor = if phase == SubPhase::ShotAttempt {
            shot_shooter.map(str::to_string)
        } else {
            ball_holder_id.map(str::to_string)
        };
        let screen = screen_a || screen_b;
        let possessor_is_a = possessor.as_deref() == Some(raw.entity_a.as_str());
        let possessor_is_b = possessor.as_deref() == Some(raw.entity_b.as_str());
        let defender = if possessor_is_a {
            b
        } else if possessor_is_b {
            a
        } else {
            None
        };
        let defender_speed = defender.map(|player| player.vel_ft.length()).unwrap_or(0.0);
        let defender_established = defender.is_some()
            && defender_speed
                <= rules.max_player_speed_ftps * rules.semantics.screen_stationary_speed_ratio;
        let contact_kind = if screen {
            let screener = if screen_a { a } else { b };
            let screener_speed = screener
                .map(|s| s.vel_ft.length())
                .unwrap_or(relative_speed);
            if screener_speed
                <= rules.max_player_speed_ftps * rules.semantics.screen_stationary_speed_ratio
            {
                ContactKind::LegalScreen
            } else {
                ContactKind::IllegalScreenCandidate
            }
        } else if phase == SubPhase::FlightAndRebound {
            ContactKind::ReboundContact
        } else if (phase == SubPhase::ActionExecution || phase == SubPhase::ShotAttempt)
            && (possessor_is_a || possessor_is_b)
        {
            let possessor_player = if possessor_is_a { a } else { b };
            let is_shooting_act = phase == SubPhase::ShotAttempt
                || possessor_player.is_some_and(|p| {
                    p.is_driving_to_rim
                        || p.action.contains("Layup")
                        || p.action.contains("Dunk")
                        || p.action.contains("Floater")
                        || p.action.contains("Shot")
                });
            if is_shooting_act && !defender_established {
                ContactKind::ShootingContactCandidate
            } else if defender_established {
                ContactKind::ChargingCandidate
            } else {
                ContactKind::BlockingCandidate
            }
        } else {
            ContactKind::Incidental
        };
        let severity = if relative_speed
            > rules.max_player_speed_ftps * rules.semantics.contact_foul_candidate_speed_ratio
        {
            ContactSeverity::FoulCandidate
        } else if relative_speed
            > rules.max_player_speed_ftps * rules.semantics.contact_positional_speed_ratio
        {
            ContactSeverity::Positional
        } else if relative_speed
            > rules.max_player_speed_ftps * rules.semantics.contact_minor_speed_ratio
        {
            ContactSeverity::Minor
        } else {
            ContactSeverity::None
        };
        SemanticContact {
            tick,
            raw,
            kind: contact_kind,
            severity,
            context: ContactContext {
                possessor,
                action_a,
                action_b,
                relative_speed,
                setup_duration: 0.0,
                legal_position: defender_established,
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn event_facts(
        raws: impl IntoIterator<Item = RawContact>,
        physics: &dyn SpatialPhysics,
        possession: Possession,
        phase: SubPhase,
        shot_shooter: Option<&str>,
        ball_holder_id: Option<&str>,
        tick: u64,
        rules: &GameRules,
    ) -> Vec<SemanticContact> {
        raws.into_iter()
            .map(|raw| {
                Self::contact(
                    raw,
                    physics,
                    possession,
                    phase,
                    shot_shooter,
                    ball_holder_id,
                    tick,
                    rules,
                )
            })
            .collect()
    }
}

fn team_name(possession: Possession) -> &'static str {
    match possession {
        Possession::Home => "home",
        Possession::Away => "away",
    }
}

fn normalized_density(
    center: Vec2,
    team: &str,
    physics: &dyn SpatialPhysics,
    radius: f32,
    capacity: f32,
    rules: &GameRules,
) -> f32 {
    let radius = radius.max(f32::EPSILON);
    let players = physics.get_players();
    let ids = physics.query_nearby(
        center,
        radius,
        &nba_physics::EntityFilter::Team(team.to_string()),
    );
    let density = ids
        .into_iter()
        .filter_map(|id| players.get(&id))
        .map(|player| (player.pos_ft - center).length())
        .filter(|distance| *distance > rules.semantics.minimum_entity_distance_ft)
        .map(|distance| 1.0 - distance / radius * rules.semantics.density_distance_weight)
        .sum::<f32>();
    (density / capacity.max(1.0)).clamp(0.0, 1.0)
}

fn opponent_density(
    center: Vec2,
    own_team: &str,
    physics: &dyn SpatialPhysics,
    radius: f32,
    capacity: f32,
) -> f32 {
    let ids = physics.query_nearby(
        center,
        radius,
        &nba_physics::EntityFilter::OpposingTeam(own_team.to_string()),
    );
    (ids.len() as f32 / capacity.max(1.0)).clamp(0.0, 1.0)
}

fn perimeter_spacing(
    own_team: &str,
    ball_pos: Vec2,
    hoop: Vec2,
    physics: &dyn SpatialPhysics,
    strong_side: bool,
    rules: &GameRules,
) -> f32 {
    let semantic = &rules.semantics;
    let side_width = rules.court.width_ft * semantic.perimeter_side_margin_ratio;
    let depth = rules.court.height_ft * semantic.perimeter_depth_ratio;
    let ball_side = if ball_pos.y < hoop.y { -1.0 } else { 1.0 };
    let side = if strong_side { ball_side } else { -ball_side };
    let side_center = hoop.y + side * rules.court.height_ft * 0.25;
    let players = physics.get_players();
    let own_ids = physics.query_nearby(
        Vec2::new(hoop.x, side_center),
        rules.court.width_ft.hypot(rules.court.height_ft),
        &nba_physics::EntityFilter::Team(own_team.to_string()),
    );
    let mut quality = 0.0;
    let mut count = 0.0;
    for id in own_ids {
        let Some(player) = players.get(&id) else {
            continue;
        };
        if !player.on_court || (player.pos_ft.y - side_center).abs() > depth {
            continue;
        }
        let horizontal = (player.pos_ft.x - hoop.x).abs();
        if horizontal < side_width {
            continue;
        }
        let openness = physics.openness(&player.id).contest_free_score();
        let defender_ids = physics.query_nearby(
            player.pos_ft,
            rules.teammate_density_radius_ft,
            &nba_physics::EntityFilter::OpposingTeam(own_team.to_string()),
        );
        let defender_pressure = (defender_ids.len() as f32
            / semantic.perimeter_density_capacity.max(1.0))
        .clamp(0.0, 1.0);
        let side_weight = if strong_side {
            semantic.strong_side_ball_weight * (1.0 - defender_pressure)
                + semantic.strong_side_defender_weight * openness
        } else {
            semantic.weak_side_defender_weight * openness
        };
        quality += side_weight.clamp(0.0, 1.0);
        count += 1.0;
    }
    if count == 0.0 {
        0.0
    } else {
        (quality / count).clamp(0.0, 1.0)
    }
}

fn is_screen_action(action: &str) -> bool {
    action
        .split(|character: char| !character.is_ascii_alphanumeric())
        .any(|token| token.eq_ignore_ascii_case("screen") || token.eq_ignore_ascii_case("pick"))
}

/// Compile-time assertion that semantic consumers depend on data, not a
/// concrete physics implementation.
fn _requires_backend(_physics: &dyn nba_physics::SpatialPhysics) {}
