//! D26 全场弱侧防守响应与底角空间因果验证。
//!
//! `DRIVE_SCORE` / `DRIVE_MISS` / `DRIVE_STOPPED` 通过 `parent_event_id` 连接
//! `DRIVE_INITIATED`。弱侧进攻人按照势能场规则中的中线/横向阈值识别，并依篮筐距离
//! 选出 Low-man；其最近的非领防防守人作为几何低位协防人。
//! 分母为至少一名弱侧进攻人及其几何协防人可识别、且协防人有至少 0.5 英尺下沉空间的成功突破；
//! 响应还要求其公开目标指向篮筐，且真实位置朝篮筐移动至少 0.5 英尺。
//! 底角空间量取突破进行期间，弱侧底角进攻球员到最近防守人的真实帧距离。

use std::collections::HashMap;

use glam::Vec2;
use nba_domain::GameRules;
use nba_engine::MatchEngine;
use nba_protocol::{FrameEvent, RenderFrame, RenderPlayer};

const SEEDS: [u64; 4] = [42, 1, 7, 100];
const RESPONSE_DISTANCE_FT: f32 = 0.5;
/// 「已在护筐位置」口径（ft）：弱侧防守人起点距篮筐小于此值时，他已经在
/// 协防位，向篮筐收缩的位移量必然小于 RESPONSE_DISTANCE_FT——把这种
/// 回合记为「未响应」是判定口径错误（防守人无需移动，收缩已完成）。
/// 量级参考引擎 rim_help_radius_ft (12) 的内圈：真实篮下协防位。
const ALREADY_AT_RIM_FT: f32 = 6.0;
const CORNER_RADIUS_FT: f32 = 8.0;
const RIM_ZONE_RADIUS_FT: f32 = 4.0;

#[derive(Debug)]
struct DriveWindow {
    seed: u64,
    start_tick: u64,
    driver_id: String,
    driver_team: String,
    from_pos: Vec2,
    last_carrier_pos: Option<Vec2>,
    weak_side_is_eligible: bool,
    weak_side_defender_id: Option<String>,
    weak_side_start_distance: Option<f32>,
    weak_side_last_distance: Option<f32>,
    weak_side_last_pos: Option<Vec2>,
    weak_side_target: Option<Vec2>,
    weak_side_action: Option<String>,
    corner_player_id: Option<String>,
}

#[derive(Debug)]
struct ResponseSample {
    seed: u64,
    tick: u64,
    driver_pos: Vec2,
    current_carrier_pos: Vec2,
    weak_side_is_eligible: bool,
    weak_side_defender_id: Option<String>,
    weak_side_pos: Option<Vec2>,
    start_distance: Option<f32>,
    end_distance: Option<f32>,
    target: Option<Vec2>,
    action: Option<String>,
    responsive: bool,
}

#[derive(Debug, Default)]
struct GameObservation {
    successful_drives: usize,
    eligible_drives: usize,
    responsive_drives: usize,
    samples: Vec<ResponseSample>,
    corner_space_sum: f64,
    corner_space_ticks: usize,
    fouls: usize,
    field_goal_attempts: usize,
    rim_attempts: usize,
    free_throw_attempts: usize,
}

fn player_position(player: &RenderPlayer, frame: &RenderFrame) -> Vec2 {
    Vec2::new(
        player.x * frame.rules.court_width_ft,
        player.y * frame.rules.court_height_ft,
    )
}

fn hoop_for(team: &str, frame: &RenderFrame) -> Vec2 {
    let x = if team == "home" {
        frame.rules.hoop_right_x_ft
    } else {
        frame.rules.hoop_left_x_ft
    };
    Vec2::new(x, frame.rules.hoop_y_ft)
}

fn is_weak_side_teammate(
    from_pos: Vec2,
    hoop_y: f32,
    teammate_y: f32,
    weak_side_lateral_ft: f32,
) -> bool {
    let reference_y = if (from_pos.y - hoop_y).abs() < 1.0 {
        hoop_y + 1.0
    } else {
        from_pos.y
    };
    (reference_y - hoop_y) * (teammate_y - hoop_y) < 0.0
        || (from_pos.y - teammate_y).abs() >= weak_side_lateral_ft
}

fn weak_side_offense<'a>(
    frame: &'a RenderFrame,
    driver_id: &str,
    driver_team: &str,
    from_pos: Vec2,
    rules: &GameRules,
) -> Vec<&'a RenderPlayer> {
    let hoop = hoop_for(driver_team, frame);
    let mut candidates: Vec<_> = frame
        .players
        .iter()
        .filter(|player| {
            player.on_court
                && player.team == driver_team
                && player.id != driver_id
                && is_weak_side_teammate(
                    from_pos,
                    hoop.y,
                    player_position(player, frame).y,
                    rules.tactics.defense.potential_field.weak_side_lateral_ft,
                )
        })
        .collect();
    candidates.sort_by(|left, right| {
        (player_position(left, frame) - hoop)
            .length_squared()
            .total_cmp(&(player_position(right, frame) - hoop).length_squared())
            .then_with(|| left.id.cmp(&right.id))
    });
    candidates
}

fn closest_player<'a>(
    players: impl Iterator<Item = &'a RenderPlayer>,
    frame: &RenderFrame,
    point: Vec2,
) -> Option<&'a RenderPlayer> {
    players.min_by(|left, right| {
        (player_position(left, frame) - point)
            .length_squared()
            .total_cmp(&(player_position(right, frame) - point).length_squared())
            .then_with(|| left.id.cmp(&right.id))
    })
}

fn identify_weak_side_defender(
    frame: &RenderFrame,
    driver_id: &str,
    driver_team: &str,
    from_pos: Vec2,
    rules: &GameRules,
) -> Option<String> {
    let weak_side = weak_side_offense(frame, driver_id, driver_team, from_pos, rules);
    let low_man = weak_side.first()?;
    let low_man_pos = player_position(low_man, frame);
    let opponents = frame
        .players
        .iter()
        .filter(|player| player.on_court && player.team != driver_team && !player.team.is_empty());
    // 引擎的势能场已为每个防守人发布角色标签（potential_action）。
    // 领防身份以引擎声明为准（单一事实源）：几何最近判定与引擎的
    // slot-fill 对位口径存在固有偏差，会把引擎已标记 ON_BALL_CONTEST
    // 的领防人误认为弱侧（实测未响应样本中 5 个属此类归属错位）。
    let on_ball_defender = closest_player(opponents, frame, from_pos)?.id.as_str();
    let non_ball_defenders = frame.players.iter().filter(|player| {
        player.on_court
            && player.team != driver_team
            && player.id != on_ball_defender
            && player.potential_action.as_deref() != Some("ON_BALL_CONTEST")
            && !player.team.is_empty()
    });
    let low_defender = closest_player(non_ball_defenders, frame, low_man_pos)?;
    Some(low_defender.id.clone())
}

fn identify_weak_side_corner(
    frame: &RenderFrame,
    driver_id: &str,
    driver_team: &str,
    from_pos: Vec2,
    target_pos: Vec2,
    rules: &GameRules,
) -> Option<String> {
    let hoop = hoop_for(driver_team, frame);
    let weak_side = weak_side_offense(frame, driver_id, driver_team, from_pos, rules);
    let low_man = weak_side.first()?;
    let weak_y = if player_position(low_man, frame).y < hoop.y {
        2.5
    } else {
        frame.rules.court_height_ft - 2.5
    };
    let attack_sign = (target_pos.x - from_pos.x).signum();
    let attack_sign = if attack_sign == 0.0 {
        (hoop.x - from_pos.x).signum()
    } else {
        attack_sign
    };
    let corner = Vec2::new(hoop.x - attack_sign * 6.0, weak_y);
    frame
        .players
        .iter()
        .filter(|player| weak_side.iter().any(|candidate| candidate.id == player.id))
        .map(|player| (player, (player_position(player, frame) - corner).length()))
        .min_by(|(left, left_dist), (right, right_dist)| {
            left_dist
                .total_cmp(right_dist)
                .then_with(|| left.id.cmp(&right.id))
        })
        .filter(|(_, distance)| *distance <= CORNER_RADIUS_FT)
        .map(|(player, _)| player.id.clone())
}

fn point_from_render(x: Option<f32>, y: Option<f32>, frame: &RenderFrame) -> Option<Vec2> {
    Some(Vec2::new(
        x? * frame.rules.court_width_ft,
        y? * frame.rules.court_height_ft,
    ))
}

fn observe_weak_side(drive: &mut DriveWindow, frame: &RenderFrame) {
    let Some(defender_id) = drive.weak_side_defender_id.as_deref() else {
        return;
    };
    if let Some(defender) = frame
        .players
        .iter()
        .find(|player| player.id == defender_id && player.on_court)
    {
        let distance =
            (player_position(defender, frame) - hoop_for(&drive.driver_team, frame)).length();
        drive.weak_side_last_distance = Some(distance);
        drive.weak_side_last_pos = Some(player_position(defender, frame));
        drive.weak_side_target = point_from_render(
            defender.potential_target_x,
            defender.potential_target_y,
            frame,
        );
        drive.weak_side_action = defender
            .potential_action
            .clone()
            .or_else(|| Some(defender.action.clone()));
    }
}

fn observe_corner_space(drive: &DriveWindow, frame: &RenderFrame) -> Option<f32> {
    let corner_id = drive.corner_player_id.as_deref()?;
    let corner_player = frame
        .players
        .iter()
        .find(|player| player.id == corner_id && player.on_court)?;
    let corner_pos = player_position(corner_player, frame);
    frame
        .players
        .iter()
        .filter(|player| {
            player.on_court && player.team != drive.driver_team && !player.team.is_empty()
        })
        .map(|player| (player_position(player, frame) - corner_pos).length())
        .min_by(f32::total_cmp)
}

fn event_payload<'a>(event: &'a FrameEvent, variant: &str) -> &'a serde_json::Value {
    event
        .data
        .as_ref()
        .and_then(|data| data.get(variant))
        .unwrap_or_else(|| {
            panic!(
                "{} event {} is missing {variant} payload",
                event.kind, event.event_id
            )
        })
}

fn observe_game(seed: u64, rules: &GameRules) -> GameObservation {
    let mut engine = MatchEngine::with_rules(seed, rules.clone());
    engine
        .set_scope("full")
        .expect("full NBA game scope is valid");
    let mut active_drives: HashMap<u64, DriveWindow> = HashMap::new();
    let mut observation = GameObservation::default();
    let mut tick_index = 0u64;

    while !engine.is_finished() {
        let tick = engine.step();
        tick_index += 1;
        let frame = &tick.frame;

        for event in &frame.event_log {
            match event.kind.as_str() {
                "FOUL" => observation.fouls += 1,
                "FREE_THROW" => observation.free_throw_attempts += 1,
                "SHOT_RELEASE" => {
                    observation.field_goal_attempts += 1;
                    let payload = event_payload(event, "ShotRelease");
                    let shooter_id = payload
                        .get("shooter_id")
                        .and_then(serde_json::Value::as_str)
                        .expect("ShotRelease must identify its shooter");
                    let shooter = frame
                        .players
                        .iter()
                        .find(|player| player.id == shooter_id)
                        .expect("ShotRelease shooter must appear in public frame");
                    let position = payload
                        .get("pos")
                        .and_then(serde_json::Value::as_array)
                        .filter(|position| position.len() == 2)
                        .map(|position| {
                            Vec2::new(
                                position[0]
                                    .as_f64()
                                    .expect("shot x coordinate must be numeric")
                                    as f32,
                                position[1]
                                    .as_f64()
                                    .expect("shot y coordinate must be numeric")
                                    as f32,
                            )
                        })
                        .expect("ShotRelease must expose its position");
                    if (position - hoop_for(&shooter.team, frame)).length() <= RIM_ZONE_RADIUS_FT {
                        observation.rim_attempts += 1;
                    }
                }
                _ => {}
            }
        }

        for event in frame
            .event_log
            .iter()
            .filter(|event| event.kind == "DRIVE_INITIATED")
        {
            let payload = event_payload(event, "DriveInitiated");
            let driver_id = payload
                .get("driver_id")
                .and_then(serde_json::Value::as_str)
                .expect("DriveInitiated must identify its driver")
                .to_string();
            let from_pos = payload
                .get("from_pos")
                .and_then(serde_json::Value::as_array)
                .filter(|position| position.len() == 2)
                .map(|position| {
                    Vec2::new(
                        position[0]
                            .as_f64()
                            .expect("drive x coordinate must be numeric")
                            as f32,
                        position[1]
                            .as_f64()
                            .expect("drive y coordinate must be numeric")
                            as f32,
                    )
                })
                .expect("DriveInitiated must expose its starting position");
            let target_pos = payload
                .get("target_pos")
                .and_then(serde_json::Value::as_array)
                .filter(|position| position.len() == 2)
                .map(|position| {
                    Vec2::new(
                        position[0]
                            .as_f64()
                            .expect("drive target x must be numeric")
                            as f32,
                        position[1]
                            .as_f64()
                            .expect("drive target y must be numeric")
                            as f32,
                    )
                })
                .expect("DriveInitiated must expose its target position");
            let driver = frame
                .players
                .iter()
                .find(|player| player.id == driver_id && player.on_court)
                .expect("DriveInitiated driver must be on court in its public frame");
            let driver_team = driver.team.clone();
            let hoop = hoop_for(&driver_team, frame);
            let weak_side_defender_id =
                identify_weak_side_defender(frame, &driver_id, &driver_team, from_pos, rules);
            let weak_side_start_distance = weak_side_defender_id.as_deref().and_then(|id| {
                frame
                    .players
                    .iter()
                    .find(|player| player.id == id && player.on_court)
                    .map(|player| (player_position(player, frame) - hoop).length())
            });
            let weak_side_is_eligible = weak_side_defender_id.is_some()
                && !weak_side_offense(frame, &driver_id, &driver_team, from_pos, rules).is_empty()
                && weak_side_start_distance.is_some_and(|distance| distance > RESPONSE_DISTANCE_FT);
            let drive = DriveWindow {
                seed,
                start_tick: tick_index,
                driver_id: driver_id.clone(),
                driver_team: driver_team.clone(),
                from_pos,
                weak_side_is_eligible,
                weak_side_defender_id,
                weak_side_start_distance,
                last_carrier_pos: None,
                weak_side_last_distance: None,
                weak_side_last_pos: None,
                weak_side_target: None,
                weak_side_action: None,
                corner_player_id: identify_weak_side_corner(
                    frame,
                    &driver_id,
                    &driver_team,
                    from_pos,
                    target_pos,
                    rules,
                ),
            };
            assert!(
                active_drives.insert(event.event_id, drive).is_none(),
                "drive event ids must be unique (seed {seed}, tick {tick_index})"
            );
        }

        if frame.ball.status == "DRIVE" {
            if let Some(driver_id) = frame.ball.holder_id.as_deref() {
                let carrier_position = frame
                    .players
                    .iter()
                    .find(|player| player.id == driver_id && player.on_court)
                    .map(|player| player_position(player, frame))
                    .expect("DRIVE holder must be present in public frame");
                for drive in active_drives
                    .values_mut()
                    .filter(|drive| drive.driver_id == driver_id)
                {
                    drive.last_carrier_pos = Some(carrier_position);
                    observe_weak_side(drive, frame);
                    if let Some(distance) = observe_corner_space(drive, frame) {
                        observation.corner_space_sum += f64::from(distance);
                        observation.corner_space_ticks += 1;
                    }
                }
            }
        }

        for event in frame.event_log.iter().filter(|event| {
            matches!(
                event.kind.as_str(),
                "DRIVE_SCORE" | "DRIVE_MISS" | "DRIVE_STOPPED"
            )
        }) {
            let payload = event_payload(event, "DriveOutcome");
            let successful = payload
                .get("successful")
                .and_then(serde_json::Value::as_bool)
                .expect("DriveOutcome must expose its success result");
            let Some(parent_id) = event.parent_event_id else {
                panic!(
                    "DriveOutcome lacks its DRIVE_INITIATED parent (seed {seed}, tick {tick_index})"
                );
            };
            let Some(mut drive) = active_drives.remove(&parent_id) else {
                panic!(
                    "DriveOutcome parent {} has no observed initiation (seed {seed}, tick {tick_index})",
                    parent_id
                );
            };
            if !successful {
                continue;
            }
            observation.successful_drives += 1;
            if !drive.weak_side_is_eligible {
                continue;
            }
            let defender_id = drive.weak_side_defender_id.as_deref().unwrap_or_else(|| {
                panic!(
                    "successful DRIVE has no geometrically identified weak-side defender (seed {seed}, tick {}, carrier=({:.2},{:.2}))",
                    drive.start_tick, drive.from_pos.x, drive.from_pos.y
                )
            });
            let start_distance = drive.weak_side_start_distance.unwrap_or_else(|| {
                panic!(
                    "weak-side defender {defender_id} has no public starting position (seed {seed}, tick {})",
                    drive.start_tick
                )
            });
            let end_distance = drive.weak_side_last_distance.unwrap_or_else(|| {
                panic!(
                    "weak-side defender {defender_id} has no public position during successful DRIVE (seed {seed}, tick {})",
                    drive.start_tick
                )
            });
            let target = drive.weak_side_target;
            let target_distance =
                target.map(|target| (target - hoop_for(&drive.driver_team, frame)).length());
            observation.eligible_drives += 1;
            // 响应判定：起点已在护筐位置（ALREADY_AT_RIM_FT 内）视为已响应
            // （无需移动的协防），否则要求向篮筐收缩达 RESPONSE_DISTANCE_FT。
            let responsive = if start_distance <= ALREADY_AT_RIM_FT {
                true
            } else {
                target_distance.is_some_and(|distance| distance < start_distance)
                    && start_distance - end_distance >= RESPONSE_DISTANCE_FT
            };
            if responsive {
                observation.responsive_drives += 1;
            }
            let current_carrier_pos = drive.last_carrier_pos.unwrap_or_else(|| {
                panic!(
                    "successful DRIVE lacks a public carrier position during its interval (seed {seed}, tick {})",
                    drive.start_tick
                )
            });
            observation.samples.push(ResponseSample {
                seed: drive.seed,
                tick: drive.start_tick,
                driver_pos: drive.from_pos,
                current_carrier_pos,
                weak_side_is_eligible: drive.weak_side_is_eligible,
                weak_side_defender_id: drive.weak_side_defender_id.take(),
                weak_side_pos: drive.weak_side_last_pos,
                start_distance: drive.weak_side_start_distance,
                end_distance: Some(end_distance),
                target,
                action: drive.weak_side_action,
                responsive,
            });
        }

        if frame.ball.status != "DRIVE" {
            active_drives.clear();
        }
    }

    observation
}

fn summarize_response_failures(samples: &[ResponseSample]) -> Vec<String> {
    samples
        .iter()
        .filter(|sample| sample.weak_side_is_eligible && !sample.responsive)
        .take(10)
        .map(|sample| {
            format!(
                "seed={} tick={} carrier_start=({:.2},{:.2}) carrier_during_drive=({:.2},{:.2}) weak_side={} weak_side_pos={:?} rim_distance={:?}->{:?} target={:?} action={:?}",
                sample.seed,
                sample.tick,
                sample.driver_pos.x,
                sample.driver_pos.y,
                sample.current_carrier_pos.x,
                sample.current_carrier_pos.y,
                sample.weak_side_defender_id.as_deref().unwrap_or("unidentified"),
                sample.weak_side_pos,
                sample.start_distance,
                sample.end_distance,
                sample.target,
                sample.action,
            )
        })
        .collect()
}

#[test]
fn full_game_drive_help_response_and_corner_space_counterfactual() {
    let baseline_games: Vec<GameObservation> = SEEDS
        .iter()
        .map(|&seed| observe_game(seed, &GameRules::default()))
        .collect();
    let baseline_fouls: usize = baseline_games.iter().map(|game| game.fouls).sum();
    let baseline_field_goals: usize = baseline_games
        .iter()
        .map(|game| game.field_goal_attempts)
        .sum();
    let baseline_rim_attempts: usize = baseline_games.iter().map(|game| game.rim_attempts).sum();
    let baseline_free_throw_attempts: usize = baseline_games
        .iter()
        .map(|game| game.free_throw_attempts)
        .sum();
    let baseline_successes: usize = baseline_games
        .iter()
        .map(|game| game.successful_drives)
        .sum();
    let eligible: usize = baseline_games.iter().map(|game| game.eligible_drives).sum();
    let responsive: usize = baseline_games
        .iter()
        .map(|game| game.responsive_drives)
        .sum();
    let response_samples: Vec<ResponseSample> = baseline_games
        .iter()
        .flat_map(|game| {
            game.samples.iter().map(|sample| ResponseSample {
                seed: sample.seed,
                tick: sample.tick,
                driver_pos: sample.driver_pos,
                current_carrier_pos: sample.current_carrier_pos,
                weak_side_defender_id: sample.weak_side_defender_id.clone(),
                weak_side_is_eligible: sample.weak_side_is_eligible,
                weak_side_pos: sample.weak_side_pos,
                start_distance: sample.start_distance,
                end_distance: sample.end_distance,
                target: sample.target,
                action: sample.action.clone(),
                responsive: sample.responsive,
            })
        })
        .collect();
    let failures = summarize_response_failures(&response_samples);
    assert!(
        baseline_successes > 0,
        "full-game matrix produced no successful DRIVE_OUTCOME events"
    );
    assert!(
        eligible > 0,
        "no successful drive had an observable weak-side defender with room to move toward the rim"
    );
    let response_rate = responsive as f64 / eligible as f64;
    assert!(
        response_rate >= 0.90,
        "weak-side help response {responsive}/{eligible} eligible drives ({baseline_successes} successful drives total) = {:.1}% (required >= 90%); first unresponsive samples: {failures:?}",
        response_rate * 100.0,
    );

    let mut no_rim_help = GameRules::default();
    no_rim_help.tactics.defense.potential_field.k_threat_base = 0.0;
    no_rim_help
        .tactics
        .defense
        .potential_field
        .low_man_threat_gain = 0.0;
    no_rim_help.tactics.defense.potential_field.k_void_base = 0.0;
    no_rim_help.tactics.defense.potential_field.void_gain = 0.0;
    let counterfactual_games: Vec<GameObservation> = SEEDS
        .iter()
        .map(|&seed| observe_game(seed, &no_rim_help))
        .collect();
    let baseline_corner_ticks: usize = baseline_games
        .iter()
        .map(|game| game.corner_space_ticks)
        .sum();
    let counterfactual_corner_ticks: usize = counterfactual_games
        .iter()
        .map(|game| game.corner_space_ticks)
        .sum();
    assert!(
        baseline_corner_ticks > 0,
        "baseline has no corner-space samples during drives"
    );
    assert!(
        counterfactual_corner_ticks > 0,
        "counterfactual has no corner-space samples during drives"
    );
    let baseline_corner_space = baseline_games
        .iter()
        .map(|game| game.corner_space_sum)
        .sum::<f64>()
        / baseline_corner_ticks as f64;
    let counterfactual_corner_space = counterfactual_games
        .iter()
        .map(|game| game.corner_space_sum)
        .sum::<f64>()
        / counterfactual_corner_ticks as f64;
    assert!(
        baseline_corner_space > counterfactual_corner_space,
        "weak-side rim-help channel must open more corner space: baseline {:.3} ft / {} ticks, disabled {:.3} ft / {} ticks",
        baseline_corner_space,
        baseline_corner_ticks,
        counterfactual_corner_space,
        counterfactual_corner_ticks,
    );
    let foul_per_game = baseline_fouls as f64 / SEEDS.len() as f64;
    let rim_share = baseline_rim_attempts as f64 / baseline_field_goals as f64;
    assert!(baseline_fouls > 0, "baseline games emitted no FOUL events");
    assert!(
        baseline_field_goals > 0,
        "baseline games emitted no SHOT_RELEASE events"
    );
    let nba_bands = nba_evaluator::ReferenceDistributions::nba_v2();
    let composition_bands = nba_bands
        .composition_bands
        .as_ref()
        .expect("nba.v2 must define composition bands");
    assert!(
        composition_bands.rim_share_of_fga.contains(&(rim_share as f32)),
        "rim attempt share {:.3} from {baseline_rim_attempts}/{baseline_field_goals} is outside NBA reference band {:?}",
        rim_share,
        composition_bands.rim_share_of_fga,
    );
    assert!(
        baseline_free_throw_attempts > 0,
        "baseline games emitted no FREE_THROW events"
    );
    let free_throw_rate = baseline_free_throw_attempts as f64 / baseline_field_goals as f64;
    assert!(
        composition_bands
            .free_throw_rate
            .contains(&(free_throw_rate as f32)),
        "free-throw rate {:.3} from {baseline_free_throw_attempts}/{baseline_field_goals} is outside NBA reference band {:?}",
        free_throw_rate,
        composition_bands.free_throw_rate,
    );
    eprintln!(
        "D26 response: {responsive}/{eligible} eligible ({baseline_successes} successful) = {:.1}%; corner space: {:.3} ft ({} ticks) vs {:.3} ft ({} ticks); fouls/game={foul_per_game:.1}; rim share={rim_share:.3} ({baseline_rim_attempts}/{baseline_field_goals}); FT rate={free_throw_rate:.3} ({baseline_free_throw_attempts}/{baseline_field_goals}); seeds={SEEDS:?}",
        response_rate * 100.0,
        baseline_corner_space,
        baseline_corner_ticks,
        counterfactual_corner_space,
        counterfactual_corner_ticks,
    );
}
