//! 策略层：从决策视图里选动作。
//!
//! 效用是状态特征的线性组合，权重全部来自 `policy.json`，由矩匹配拟合。
//! 动作产出的是意图，不是结果；结果由结算层给出。

use glam::Vec2;

use crate::action::{ActionIntent, ActionKind, PassKind, ScreenKind, ShotKind};
use crate::resolve::{contest_for, shooter_zone};
use crate::state::{DecisionState, PossessionKind};
use crate::world::World;

const MAX_SPEED_FT_PER_SEC: f32 = 22.0;

pub fn choose_action(world: &mut World) -> ActionIntent {
    let state = world.decision_state();
    let params = &world.params;
    let weights = &params.policy;
    let actor = state.actor;
    let now = world.now();

    let mut candidates: Vec<(ActionKind, f32, Option<usize>, Vec2)> = Vec::new();

    let chosen_shot = shot_choice(world, &state).or_else(|| {
        if state.shot_clock < 2.5 {
            Some((
                ShotKind::PullUp,
                shooter_zone(world, state.actor_pos, world.players[actor].team),
            ))
        } else {
            None
        }
    });
    let just_received = world
        .possession
        .last_pass_at
        .map(|time| now - time < 1.2)
        .unwrap_or(false);
    if let Some((shot_kind, shot_zone)) = chosen_shot {
        let shot_distance = (state.actor_pos - state.hoop).length();
        let is_three = shot_zone == "three";
        let contest = contest_for(world, actor, state.actor_pos);
        let base = params
            .shot_make
            .base_rate(shot_kind.as_str(), shot_distance, is_three);
        let zone_slope = params.shot_make.skill_spread(shot_zone);
        let skill = world.players[actor].shooting_attribute(shot_zone);
        let p_skill = (base + (skill - 0.5) * zone_slope).clamp(0.02, 0.98);
        let p_make = (crate::resolve::sigmoid(
            logit(p_skill) + params.shot_make.contest_sensitivity * (0.5 - contest),
        ))
        .clamp(0.02, 0.98);
        let expected_points = p_make * if is_three { 3.0 } else { 2.0 };
        let early_penalty = if just_received {
            0.0
        } else {
            weights.w("shoot_early_slack") * state.early_clock_slack()
        };
        let separation_bonus = world
            .last_separation
            .filter(|(separation_actor, _, time)| {
                *separation_actor == state.actor && now - *time < 3.0
            })
            .map(|(_, separation, _)| (separation / 4.0).clamp(0.0, 1.5))
            .unwrap_or(0.0);
        let shoot_score = weights.w("shoot_base")
            + weights.w("shoot_open") * state.openness()
            + weights.w("shoot_expected_points") * expected_points
            + weights.w("shoot_pressure") * state.shot_clock_pressure()
            + weights.w("shoot_separation_bonus") * separation_bonus
            + weights.w("shoot_mid_bonus") * f32::from(shot_zone == "mid")
            - early_penalty
            + weights.w("shoot_catch_bonus") * state.openness() * f32::from(just_received)
            + weights.w("shoot_putback_bonus") * f32::from(putback_ready(world))
            - weights.w("shoot_off_dribble_penalty") * f32::from(!just_received)
            + weights.w("shoot_tendency")
                * (world.players[actor].data.tendencies.shoot_frequency - 0.5);
        candidates.push((
            ActionKind::Shoot(shot_kind),
            shoot_score.max(0.001),
            None,
            state.actor_pos,
        ));
    }

    let lane_norm = (state.lane_clearance / 8.0).clamp(0.0, 1.5);
    let help_term = state
        .help
        .map(|help| (1.0 / help.arrival_seconds.max(0.1)).min(2.0))
        .unwrap_or(0.0);
    let drive_score = weights.w("drive_base")
        + weights.w("drive_lane") * lane_norm
        + weights.w("drive_mismatch") * state.mismatch
        + weights.w("drive_tight")
            * state
                .primary_defender
                .map(|defender| (1.0 - defender.distance_ft / 6.0).clamp(0.0, 1.0))
                .unwrap_or(0.0)
        - weights.w("drive_help") * help_term
        + weights.w("drive_tendency")
            * (world.players[actor].data.tendencies.drive_frequency - 0.5);
    if state.distance_to_hoop() < 32.0 {
        candidates.push((
            ActionKind::Drive,
            drive_score.max(0.001),
            None,
            state.hoop - (state.hoop - state.actor_pos).normalize_or_zero() * 3.0,
        ));
    }

    let mut best_pass: Option<(usize, f32, Vec2, f32)> = None;
    let mut pass_options: Vec<(usize, f32, Vec2)> = Vec::new();
    for teammate in &state.teammates {
        let advancement = if state.attacking_right {
            teammate.pos.x - state.actor_pos.x
        } else {
            state.actor_pos.x - teammate.pos.x
        };
        let risk = (1.0 - (teammate.lane_clearance / 6.0).clamp(0.0, 1.0)).max(0.0);
        let teammate_three =
            crate::resolve::shooter_zone(world, teammate.pos, world.players[teammate.index].team)
                == "three";
        let score = weights.w("pass_base")
            + weights.w("pass_open") * teammate.openness
            + weights.w("pass_advancement") * (advancement / 20.0).clamp(-1.0, 1.0)
            + weights.w("pass_perimeter") * f32::from(teammate_three)
            - weights.w("pass_risk") * risk;
        let score = score.max(0.001);
        pass_options.push((teammate.index, score, teammate.pos));
        if best_pass.is_none_or(|(_, value, _, _)| score > value) {
            best_pass = Some((teammate.index, score, teammate.pos, risk));
        }
    }
    if let Some((_, best_score, _, _)) = best_pass {
        let weights_by_option: Vec<f32> = pass_options.iter().map(|(_, score, _)| *score).collect();
        let pick = crate::resolve::weighted_pick(&mut world.rng, &weights_by_option);
        let (target, _, pos) = pass_options[pick];
        candidates.push((
            ActionKind::Pass(PassKind::Chest),
            best_score.max(0.001),
            Some(target),
            pos,
        ));
    }

    let screen_score = weights.w("screen_base")
        + weights.w("screen_mismatch") * state.mismatch.abs()
        + weights.w("screen_tendency")
            * (world.players[actor].data.tendencies.screen_frequency - 0.5);
    let screener = state
        .teammates
        .iter()
        .max_by(|a, b| {
            world.players[a.index]
                .data
                .attributes
                .strength
                .partial_cmp(&world.players[b.index].data.attributes.strength)
                .expect("finite strength")
        })
        .map(|view| view.index);
    candidates.push((
        ActionKind::Screen(ScreenKind::OnBall),
        screen_score.max(0.001),
        screener,
        state.actor_pos,
    ));

    let cut_score = weights.w("cut_base") + weights.w("cut_lane") * lane_norm;
    let cutter = state
        .teammates
        .iter()
        .max_by(|a, b| {
            b.defender_distance_ft
                .partial_cmp(&a.defender_distance_ft)
                .expect("finite distances")
        })
        .map(|view| view.index);
    candidates.push((ActionKind::Cut, cut_score.max(0.001), cutter, state.hoop));

    let size = world.players[actor].data.height_cm as f32 / 200.0;
    let post_score = weights.w("post_base") + weights.w("post_size") * size;
    candidates.push((ActionKind::PostUp, post_score.max(0.001), None, state.hoop));

    let dribble_score =
        weights.w("dribble_base") + weights.w("dribble_setup") * state.early_clock_slack();
    // 已收球就不能再运球：二次运球是犯规，不是候选。
    if world.possession.dribble_state == crate::state::DribbleState::Dribbling {
        candidates.push((
            ActionKind::Dribble,
            dribble_score.max(0.001),
            None,
            state.hoop,
        ));
    }

    let scores: Vec<f32> = candidates.iter().map(|(_, score, _, _)| *score).collect();
    let index = crate::resolve::weighted_pick(&mut world.rng, &scores);
    let (kind, _, target, to_ft) = candidates[index].clone();

    let duration = match kind {
        ActionKind::Pass(_) => {
            let distance = (to_ft - state.actor_pos).length();
            (distance / 28.0).clamp(0.35, 1.4) as f64
        }
        ActionKind::Drive => {
            let distance = (state.hoop - state.actor_pos).length();
            (distance / MAX_SPEED_FT_PER_SEC as f32).clamp(0.6, 3.2) as f64
        }
        ActionKind::Screen(_) => 0.9,
        ActionKind::Cut => 1.1,
        ActionKind::PostUp => 1.0,
        ActionKind::Shoot(_) => 0.55,
        ActionKind::Dribble | ActionKind::Relocate => 1.4,
    };

    ActionIntent {
        actor,
        kind,
        target,
        from_ft: state.actor_pos,
        to_ft,
        start_time: now,
        duration,
    }
}

fn logit(p: f32) -> f32 {
    let p = p.clamp(1e-4, 1.0 - 1e-4);
    (p / (1.0 - p)).ln()
}

/// 出手类型由动作链语境决定：刚接到球是接球投，突破到篮下是上篮，
/// 背身是勾手，转换是跑动终结。
pub fn putback_ready(world: &World) -> bool {
    world
        .possession
        .last_offensive_rebound_at
        .map(|time| world.now() - time < 2.5)
        .unwrap_or(false)
}

pub fn shot_choice(world: &World, state: &DecisionState) -> Option<(ShotKind, &'static str)> {
    let zone = shooter_zone(world, state.actor_pos, world.players[state.actor].team);
    let just_received = world
        .possession
        .last_pass_at
        .map(|time| world.now() - time < 1.2)
        .unwrap_or(false);
    let driving = world.possession.last_drive_actor == Some(state.actor)
        && world
            .possession
            .last_drive_at
            .map(|time| world.now() - time < 2.5)
            .unwrap_or(false);
    let post = world.possession.last_post_actor == Some(state.actor)
        && world
            .possession
            .last_post_at
            .map(|time| world.now() - time < 2.5)
            .unwrap_or(false);
    let transition = world.possession.kind == PossessionKind::Live
        && world.now() - world.possession.started_at < 7.6;
    let cut_ready = world.possession.last_cut_actor == Some(state.actor)
        && world
            .possession
            .last_cut_at
            .map(|time| world.now() - time < 1.5)
            .unwrap_or(false);
    let putback_ready = world
        .possession
        .last_offensive_rebound_at
        .map(|time| world.now() - time < 2.5)
        .unwrap_or(false);
    let finishing = world.players[state.actor].data.attributes.finishing;

    // 出手类型由动作语境决定，位置必须与语境匹配；不匹配时不出手。
    if putback_ready {
        return matches!(zone, "rim" | "near").then_some((ShotKind::Putback, zone));
    }
    if cut_ready {
        return (zone == "rim").then_some((
            if finishing > 0.6 {
                ShotKind::CuttingDunk
            } else {
                ShotKind::CuttingLayup
            },
            zone,
        ));
    }
    if transition && zone == "rim" {
        return Some((ShotKind::RunningLayup, zone));
    }
    if transition && zone == "three" {
        return Some((ShotKind::RunningJump, zone));
    }
    if driving {
        return match zone {
            "rim" => Some((
                if finishing > 0.6 {
                    ShotKind::DrivingDunk
                } else {
                    ShotKind::DrivingLayup
                },
                zone,
            )),
            "near" => Some((ShotKind::Floater, zone)),
            _ => None,
        };
    }
    if post {
        return match zone {
            "rim" => Some((ShotKind::Hook, zone)),
            "near" => Some((ShotKind::Hook, zone)),
            "mid" => Some((ShotKind::Turnaround, zone)),
            _ => None,
        };
    }
    if just_received {
        return (zone == "three").then_some((ShotKind::CatchShoot, zone));
    }
    match zone {
        "three" | "mid" => Some((ShotKind::PullUp, zone)),
        _ => None,
    }
}

pub fn spot_target(world: &World, player: usize) -> Vec2 {
    let team = world.players[player].team;
    let spots = crate::world::offense_spots(
        &world.params.formations,
        world.attacking_right(team),
        world.alternate_spacing,
    );
    let slot = world.players[player].slot;
    let index = match slot {
        crate::state::Slot::Handler => 0,
        crate::state::Slot::Wing => 1,
        crate::state::Slot::Corner => 2,
        crate::state::Slot::Post => 3,
        crate::state::Slot::DunkerSpot => 4,
        crate::state::Slot::Top => 0,
    };
    spots[index]
}

pub fn move_toward_targets(world: &mut World) {
    let offense = world.possession.team;
    let defense = offense.other();
    let holder = world.holder();
    let hoop = world.hoop_for(offense);
    let ball_pos = holder
        .map(|handle| world.players[handle].pos)
        .unwrap_or(Vec2::new(47.0, 25.0));

    // 防守：离球最近的人贴在球与篮筐之间，其余人从自己的基准位置朝球侧收缩。
    let mut defenders = world.on_court(defense);
    defenders.sort_by(|a, b| {
        (world.players[*a].pos - ball_pos)
            .length_squared()
            .partial_cmp(&(world.players[*b].pos - ball_pos).length_squared())
            .expect("finite distances")
    });
    for (rank, index) in defenders.iter().enumerate() {
        let target = if rank == 0 {
            let toward_hoop = (hoop - ball_pos).normalize_or_zero();
            ball_pos + toward_hoop * 2.8
        } else {
            let anchor = world.players[*index].home_spot;
            anchor.lerp(ball_pos, 0.22)
        };
        world.players[*index].target = target;
    }

    for handle in 0..world.players.len() {
        if !world.players[handle].on_court {
            continue;
        }
        if world.players[handle].team == offense {
            if Some(handle) == holder {
                continue;
            }
            world.players[handle].target = spot_target(world, handle);
        }
    }
}
