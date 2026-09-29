use glam::Vec2;
use nba_physics::movement::PlayerPhysicsState;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub(super) struct DriveGeometryResolution {
    pub(super) successful: bool,
    pub(super) primary_defender_id: Option<String>,
    pub(super) bypass_target: Option<Vec2>,
    pub(super) lateral_direction: f32,
}

/// 归一到 [0,1]（数值边界模式，非行为系数）。
pub(super) fn clamp_unit(value: f32) -> f32 {
    value.clamp(f32::from(0u8), f32::from(1u8))
}

/// 负值归零（数值边界模式，非行为系数）。
pub(super) fn non_negative(value: f32) -> f32 {
    value.max(f32::from(0u8))
}

pub(super) fn drive_speed_and_duration(
    driver: &PlayerPhysicsState,
    drive_dist: f32,
    rules: &nba_domain::GameRules,
) -> (f32, f32) {
    let drive_speed = (driver.max_speed_ftps * rules.tactics.drive_speed_ratio).max(f32::from(1u8));
    let accel = rules.max_player_accel_ftps2.max(f32::EPSILON);
    let duration = (drive_dist / drive_speed + drive_speed / accel).clamp(
        rules.tactics.drive_min_duration_seconds,
        rules.tactics.drive_max_duration_seconds,
    );
    (drive_speed, duration)
}

/// 突破接触技巧的加权组合：控球 + 敏捷 + 力量（攻方用 ball_handling，
/// 守方用 defense_perimeter），权重来自 DriveGeometryPolicy。
fn drive_contact_skill(
    primary: f32,
    agility: f32,
    strength: f32,
    policy: &nba_domain::DriveGeometryPolicy,
) -> f32 {
    primary * policy.contact_skill_primary_weight
        + agility * policy.contact_skill_agility_weight
        + strength * policy.contact_skill_strength_weight
}

struct DrivePathCheck<'a> {
    points: &'a [Vec2],
    offense_team: &'a str,
    contact_skill: f32,
    players: &'a HashMap<String, PlayerPhysicsState>,
    drive_speed: f32,
    duration: f32,
    minimum_separation: f32,
    beaten_defender: Option<(&'a str, usize)>,
    policy: &'a nba_domain::DriveGeometryPolicy,
}

fn drive_path_margin(check: &DrivePathCheck<'_>) -> Option<f32> {
    let path_distance: f32 = check
        .points
        .windows(2)
        .map(|pair| pair[0].distance(pair[1]))
        .sum();
    if path_distance > check.drive_speed * check.duration {
        return None;
    }
    let mut minimum_margin = f32::INFINITY;
    let mut elapsed = f32::from(0u8);
    for (segment_index, segment) in check.points.windows(2).enumerate() {
        let start = segment[0];
        let vector = segment[1] - start;
        let length_squared = vector.length_squared();
        if length_squared <= f32::EPSILON {
            continue;
        }
        let length = length_squared.sqrt();
        for defender in check.players.values().filter(|player| {
            player.on_court
                && player.team != check.offense_team
                && !check.beaten_defender.is_some_and(|(id, after_segment)| {
                    player.id == id && segment_index >= after_segment
                })
        }) {
            let initial_fraction =
                clamp_unit((defender.pos_ft - start).dot(vector) / length_squared);
            let arrival_time = (elapsed + length * initial_fraction / check.drive_speed)
                .clamp(f32::from(0u8), check.duration);
            let max_speed = non_negative(defender.max_speed_ftps);
            let velocity_speed = defender.vel_ft.length();
            let velocity = if velocity_speed > max_speed && velocity_speed > f32::EPSILON {
                defender.vel_ft * (max_speed / velocity_speed)
            } else {
                defender.vel_ft
            };
            let predicted_pos = defender.pos_ft + velocity * arrival_time;
            let fraction = clamp_unit((predicted_pos - start).dot(vector) / length_squared);
            let closest = start + vector * fraction;
            let defender_can_close = fraction > f32::from(0u8) && fraction < f32::from(1u8);
            let distance = (predicted_pos - closest).length();
            let stamina = clamp_unit(defender.stamina / defender.max_stamina.max(f32::EPSILON));
            let defensive_skill = drive_contact_skill(
                defender.attributes.defense_perimeter,
                defender.attributes.agility,
                defender.attributes.strength,
                check.policy,
            );
            let response_time =
                non_negative(arrival_time - check.policy.defender_response_delay_seconds);
            let acceleration_reach =
                (non_negative(defender.max_accel_ftps2) * response_time * response_time
                    / f32::from(2u8))
                .min(non_negative(defender.max_speed_ftps) * response_time)
                    * (check.policy.reach_base_factor
                        + defensive_skill * check.policy.reach_skill_gain)
                    * stamina;
            let contact_margin = if defender_can_close {
                distance - check.minimum_separation - acceleration_reach
                    + (check.contact_skill - defensive_skill) * check.policy.contact_skill_scale
            } else {
                f32::INFINITY
            };
            minimum_margin = minimum_margin.min(contact_margin);
        }
        elapsed += length / check.drive_speed;
    }
    Some(minimum_margin)
}

pub(super) fn resolve_drive_geometry(
    driver: &PlayerPhysicsState,
    from_pos: Vec2,
    target_pos: Vec2,
    players: &HashMap<String, PlayerPhysicsState>,
    drive_speed: f32,
    duration: f32,
    rules: &nba_domain::GameRules,
) -> DriveGeometryResolution {
    let policy = &rules.resolve.drive_geometry;
    let contact_skill = drive_contact_skill(
        driver.attributes.ball_handling,
        driver.attributes.agility,
        driver.attributes.strength,
        policy,
    );
    let drive_vector = target_pos - from_pos;
    let drive_dist = drive_vector.length();
    if drive_dist <= f32::EPSILON {
        return DriveGeometryResolution {
            successful: false,
            primary_defender_id: None,
            bypass_target: None,
            lateral_direction: f32::from(0u8),
        };
    }
    let drive_dir = drive_vector / drive_dist;
    let perp = Vec2::new(-drive_dir.y, drive_dir.x);
    let minimum_separation = rules.min_player_separation_ft;
    let max_path_distance = drive_speed * duration;
    let lane_width = rules.tactics.drive_lane_offset_ft.max(minimum_separation);
    let mut primary_defender: Option<(&PlayerPhysicsState, (f32, f32))> = None;
    for candidate in players
        .values()
        .filter(|player| player.on_court && player.team != driver.team && player.id != driver.id)
    {
        let relative = candidate.pos_ft - from_pos;
        let along = relative.dot(drive_dir);
        let lateral = (relative - drive_dir * along).length();
        if along <= f32::from(0u8)
            || along >= drive_dist
            || lateral > lane_width + minimum_separation
        {
            continue;
        }
        let is_primary = primary_defender.as_ref().is_none_or(|(current, distance)| {
            (along, lateral, candidate.id.as_str()).partial_cmp(&(
                distance.0,
                distance.1,
                current.id.as_str(),
            )) == Some(std::cmp::Ordering::Less)
        });
        if is_primary {
            primary_defender = Some((candidate, (along, lateral)));
        }
    }
    let direct_path = [from_pos, target_pos];
    let direct_check = DrivePathCheck {
        points: &direct_path,
        offense_team: driver.team.as_str(),
        contact_skill,
        players,
        drive_speed,
        duration,
        minimum_separation,
        beaten_defender: None,
        policy,
    };
    if let Some(margin) = drive_path_margin(&direct_check) {
        if margin > f32::from(0u8) {
            if let Some((defender, _)) = primary_defender {
                let defender_side = (defender.pos_ft - from_pos).dot(perp);
                return DriveGeometryResolution {
                    successful: true,
                    primary_defender_id: Some(defender.id.clone()),
                    bypass_target: None,
                    lateral_direction: if defender_side >= f32::from(0u8) {
                        -f32::from(1u8)
                    } else {
                        f32::from(1u8)
                    },
                };
            }
            return DriveGeometryResolution {
                successful: true,
                primary_defender_id: None,
                bypass_target: None,
                lateral_direction: f32::from(0u8),
            };
        }
    }

    let mut best_route: Option<(f32, f32, String, Vec2, f32)> = None;
    for defender in players
        .values()
        .filter(|player| player.on_court && player.team != driver.team && player.id != driver.id)
    {
        let relative = defender.pos_ft - from_pos;
        let along = relative.dot(drive_dir);
        let lateral = (relative - drive_dir * along).length();
        if along <= f32::from(0u8)
            || along >= drive_dist
            || lateral > lane_width + minimum_separation
        {
            continue;
        }
        let projected = from_pos + drive_dir * along;
        let clearance = minimum_separation + minimum_separation + rules.player_radius_ft;
        for lateral_direction in [-f32::from(1u8), f32::from(1u8)] {
            let bypass_target = rules.court.clamp_playable(
                projected + perp * lateral_direction * clearance,
                rules.player_radius_ft,
            );
            let route = [from_pos, bypass_target, target_pos];
            let route_check = DrivePathCheck {
                points: &route,
                offense_team: driver.team.as_str(),
                contact_skill,
                players,
                drive_speed,
                duration,
                minimum_separation,
                beaten_defender: Some((&defender.id, 1)),
                policy,
            };
            let Some(margin) = drive_path_margin(&route_check) else {
                continue;
            };
            let path_distance: f32 = route.windows(2).map(|pair| pair[0].distance(pair[1])).sum();
            if margin <= f32::from(0u8) || path_distance > max_path_distance {
                continue;
            }
            let candidate = (
                path_distance,
                -margin,
                defender.id.clone(),
                bypass_target,
                lateral_direction,
            );
            let replace = best_route.as_ref().is_none_or(|best| {
                candidate
                    .0
                    .partial_cmp(&best.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        candidate
                            .1
                            .partial_cmp(&best.1)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .then_with(|| candidate.2.cmp(&best.2))
                    .then_with(|| {
                        candidate
                            .4
                            .partial_cmp(&best.4)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .is_lt()
            });
            if replace {
                best_route = Some(candidate);
            }
        }
    }

    if let Some((_, _, defender_id, bypass_target, lateral_direction)) = best_route {
        DriveGeometryResolution {
            successful: true,
            primary_defender_id: Some(defender_id),
            bypass_target: Some(bypass_target),
            lateral_direction,
        }
    } else if let Some((defender, _)) = primary_defender {
        DriveGeometryResolution {
            successful: false,
            primary_defender_id: Some(defender.id.clone()),
            bypass_target: None,
            lateral_direction: f32::from(0u8),
        }
    } else {
        DriveGeometryResolution {
            successful: false,
            primary_defender_id: None,
            bypass_target: None,
            lateral_direction: f32::from(0u8),
        }
    }
}
