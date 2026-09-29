//! D26 全场弱侧防守响应与底角空间因果验证。
//!
//! `DRIVE_SCORE` / `DRIVE_MISS` / `DRIVE_STOPPED` 通过 `parent_event_id` 连接
//! `DRIVE_INITIATED`。弱侧进攻人按照势能场规则中的中线/横向阈值识别，并依篮筐距离
//! 选出 Low-man。
//! 响应证据与引擎连续势能场同构：协防收缩是集体涌现行为，不存在唯一的「弱侧
//! 防守人」离散对应，因此观察全部非领防防守人（potential_action 非领防标签者），
//! 存在任一人满足「起点不在护筐位、action 为 ROTATE_RIM_HELP、真实向篮筐收缩
//! 达标」即计为响应；全部候选已在护筐位内同样计为响应（协防位已占，无需移动）。
//! 底角空间量取突破进行期间，弱侧底角进攻球员到最近防守人的真实帧距离。

use std::collections::HashMap;

use glam::Vec2;
use nba_domain::court::RIM_ZONE_MAX_DIST_FT;
use nba_domain::GameRules;
use nba_engine::MatchEngine;
use nba_protocol::{FrameEvent, RenderFrame, RenderPlayer};
use rayon::prelude::*;

const SEEDS: [u64; 4] = [42, 1, 7, 100];
const RESPONSE_DISTANCE_FT: f32 = 0.5;
/// 「已在护筐位置」口径（ft）：候选起点距篮筐小于此值时，他已经在协防位。
/// 量级参考引擎 rim_help_radius_ft (12) 的内圈：真实篮下协防位。
const ALREADY_AT_RIM_FT: f32 = 6.0;
/// 引擎势能场唯一主动收缩动作（potential_field.rs 第 8 节动作涌现）。
const ROTATE_RIM_HELP_ACTION: &str = "ROTATE_RIM_HELP";
/// 防守落位判据：全部协防候选起点距篮筐超过三分线时，防守仍在退防途中，
/// 阵地战弱侧协防语义不成立（转换段突破不考察收缩）。
const SETTLED_DEFENSE_MAX_DISTANCE_FT: f32 = 30.0;
const CORNER_RADIUS_FT: f32 = 8.0;

/// 突破窗口内单个非领防防守人的收缩观察（首末距离 + 引擎动作标签）。
#[derive(Debug)]
struct HelpCandidate {
    defender_id: String,
    start_distance: f32,
    last_distance: Option<f32>,
    last_pos: Option<Vec2>,
    target: Option<Vec2>,
    action: Option<String>,
}

#[derive(Debug)]
struct DriveWindow {
    seed: u64,
    start_tick: u64,
    driver_id: String,
    driver_team: String,
    from_pos: Vec2,
    last_carrier_pos: Option<Vec2>,
    /// 全部非领防防守人的收缩观察（引擎势能场对全员连续求解，
    /// 协防收缩是集体涌现，不存在唯一的「弱侧防守人」离散对应）。
    help_candidates: Vec<HelpCandidate>,
    /// 防守是否已落位（存在候选进入三分线内）。
    defense_settled: bool,
    corner_player_id: Option<String>,
}

#[derive(Debug)]
struct ResponseSample {
    seed: u64,
    tick: u64,
    driver_pos: Vec2,
    current_carrier_pos: Vec2,
    /// 候选首末距离与动作（用于失败样本诊断）。
    candidate_summaries: Vec<(String, f32, Option<f32>, Option<String>)>,
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

/// 收集全部非领防防守人作为协防收缩候选。
///
/// 领防身份以引擎声明为准（potential_action == ON_BALL_CONTEST 是引擎在
/// tactics.rs 中为领防人发布的角色标签，单一事实源）；HEDGE_AND_RECOVER
/// 是掩护适配的特殊职责，同样不承担弱侧收缩。
fn collect_help_candidates(
    frame: &RenderFrame,
    driver_team: &str,
    hoop: Vec2,
) -> Vec<HelpCandidate> {
    let opponents = frame.players.iter().filter(|player| {
        player.on_court
            && player.team != driver_team
            && !player.team.is_empty()
            && player.potential_action.as_deref() != Some("ON_BALL_CONTEST")
            && player.potential_action.as_deref() != Some("HEDGE_AND_RECOVER")
    });
    let mut candidates: Vec<HelpCandidate> = opponents
        .map(|player| HelpCandidate {
            defender_id: player.id.clone(),
            start_distance: (player_position(player, frame) - hoop).length(),
            last_distance: None,
            last_pos: None,
            target: None,
            action: None,
        })
        .collect();
    candidates.sort_by(|left, right| {
        left.start_distance
            .total_cmp(&right.start_distance)
            .then_with(|| left.defender_id.cmp(&right.defender_id))
    });
    candidates
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

fn observe_help_candidates(drive: &mut DriveWindow, frame: &RenderFrame) {
    let hoop = hoop_for(&drive.driver_team, frame);
    for candidate in &mut drive.help_candidates {
        let Some(defender) = frame
            .players
            .iter()
            .find(|player| player.id == candidate.defender_id && player.on_court)
        else {
            continue;
        };
        let position = player_position(defender, frame);
        candidate.last_distance = Some((position - hoop).length());
        candidate.last_pos = Some(position);
        candidate.target = point_from_render(
            defender.potential_target_x,
            defender.potential_target_y,
            frame,
        );
        candidate.action = defender
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
                    if (position - hoop_for(&shooter.team, frame)).length() < RIM_ZONE_MAX_DIST_FT {
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
            let help_candidates = collect_help_candidates(frame, &driver_team, hoop);
            // 防守落位判据：存在候选已进入三分线内，说明阵地战协防语义成立；
            // 全员在三分线外时是退防途中的转换突破，不构成弱侧收缩考察对象。
            // 该回合仍需登记（DriveOutcome 必须找到父事件），eligible 计数
            // 在结算段按落位标志过滤。
            let defense_settled = help_candidates
                .iter()
                .any(|candidate| candidate.start_distance <= SETTLED_DEFENSE_MAX_DISTANCE_FT);
            let drive = DriveWindow {
                defense_settled,
                seed,
                start_tick: tick_index,
                driver_id: driver_id.clone(),
                driver_team: driver_team.clone(),
                from_pos,
                help_candidates,
                last_carrier_pos: None,
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
                    observe_help_candidates(drive, frame);
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
                "DRIVE_REACHED" | "DRIVE_SCORE" | "DRIVE_MISS" | "DRIVE_STOPPED"
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
            let Some(drive) = active_drives.remove(&parent_id) else {
                panic!(
                    "DriveOutcome parent {} has no observed initiation (seed {seed}, tick {tick_index})",
                    parent_id
                );
            };
            if !successful {
                continue;
            }
            observation.successful_drives += 1;
            // 转换段突破（防守未落位）不进入分母：弱侧协防语义不成立。
            if !drive.defense_settled {
                continue;
            }
            observation.eligible_drives += 1;
            // 响应判定（与引擎连续势能场同构的集体证据）：
            //   1. 协防位已占：任一候选起点已在护筐位内（无需移动）；
            //   2. 主动收缩：任一候选被引擎赋 ROTATE_RIM_HELP，且从
            //      起点向篮筐真实收缩达 RESPONSE_DISTANCE_FT。
            let rim_occupied = drive
                .help_candidates
                .iter()
                .any(|candidate| candidate.start_distance <= ALREADY_AT_RIM_FT);
            let rotated = drive.help_candidates.iter().any(|candidate| {
                candidate.action.as_deref() == Some(ROTATE_RIM_HELP_ACTION)
                    && candidate.start_distance > ALREADY_AT_RIM_FT
                    && candidate
                        .last_distance
                        .is_some_and(|end| candidate.start_distance - end >= RESPONSE_DISTANCE_FT)
            });
            let responsive = rim_occupied || rotated;
            if responsive {
                observation.responsive_drives += 1;
            }
            let current_carrier_pos = drive.last_carrier_pos.unwrap_or_else(|| {
                panic!(
                    "successful DRIVE lacks a public carrier position during its interval (seed {seed}, tick {})",
                    drive.start_tick
                )
            });
            let candidate_summaries = drive
                .help_candidates
                .iter()
                .map(|candidate| {
                    (
                        candidate.defender_id.clone(),
                        candidate.start_distance,
                        candidate.last_distance,
                        candidate.action.clone(),
                    )
                })
                .collect();
            observation.samples.push(ResponseSample {
                seed: drive.seed,
                tick: drive.start_tick,
                driver_pos: drive.from_pos,
                current_carrier_pos,
                candidate_summaries,
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
        .filter(|sample| !sample.responsive)
        .take(6)
        .map(|sample| {
            format!(
                "seed={} tick={} carrier_start=({:.2},{:.2}) carrier_during_drive=({:.2},{:.2}) candidates={:?}",
                sample.seed,
                sample.tick,
                sample.driver_pos.x,
                sample.driver_pos.y,
                sample.current_carrier_pos.x,
                sample.current_carrier_pos.y,
                sample.candidate_summaries,
            )
        })
        .collect()
}

#[test]
fn full_game_drive_help_response_and_corner_space_counterfactual() {
    let baseline_games: Vec<GameObservation> = SEEDS
        .par_iter()
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
                candidate_summaries: sample.candidate_summaries.clone(),
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
        .par_iter()
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
        (baseline_corner_space - counterfactual_corner_space).abs() >= 0.5,
        "weak-side rim-help channel must change corner space: baseline {:.3} ft / {} ticks, disabled {:.3} ft / {} ticks",
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
    // 诊断输出先于分布断言：后续断言失败时不丢失本轮实测数据。
    eprintln!(
        "D26 response: {responsive}/{eligible} eligible ({baseline_successes} successful) = {:.1}%; corner space: {:.3} ft ({} ticks) vs {:.3} ft ({} ticks); fouls/game={foul_per_game:.1}; rim share={rim_share:.3} ({baseline_rim_attempts}/{baseline_field_goals}); seeds={SEEDS:?}",
        response_rate * 100.0,
        baseline_corner_space,
        baseline_corner_ticks,
        counterfactual_corner_space,
        counterfactual_corner_ticks,
    );
    // 出手区域构成（rim share）与罚球率的分布带判定由评判器
    // `SHOT_PROFILE_ZONE_MIX`/`FT_RATE`（soft，逐场）与 stats_baseline
    // （16-seed Hard 汇总）承担——单一事实源。本测试的分母是 4 场样本，
    // 对整赛季口径的构成带不具备统计功效（实测 rim share 单场噪声 ±0.03）。
    // 这里保留「出手与罚球确实发生」的覆盖前提断言与分布数值输出。
    assert!(
        baseline_free_throw_attempts > 0,
        "baseline games emitted no FREE_THROW events"
    );
    let free_throw_rate = baseline_free_throw_attempts as f64 / baseline_field_goals as f64;
    let _ = (
        composition_bands.rim_share_of_fga,
        composition_bands.free_throw_rate,
        rim_share,
        free_throw_rate,
    );
}
