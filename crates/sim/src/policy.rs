//! 策略层：从决策视图里选动作。
//!
//! 效用是状态特征的线性组合，权重全部来自 `policy.json`，由矩匹配拟合。
//! 动作产出的是意图，不是结果；结果由结算层给出。

use glam::Vec2;

use crate::action::{ActionIntent, ActionKind, DefenderRoute, PassKind, ScreenKind, ShotKind};
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
    // 防守人过掩护的路线：钻下沿是外线窗口，挤过是突破走廊。
    let route = state.primary_defender.and_then(|defender| defender.route);
    // 读球只在掩护后的第一个决策步生效；之后回合回到常规候选集。
    let reacting = world
        .possession
        .last_screen
        .map(|read| now - read.at)
        .map(|elapsed| {
            elapsed <= crate::world::DECISION_INTERVAL + crate::world::TICK_SECONDS
        })
        .unwrap_or(false);

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
    let endgame_shot = crate::endgame::shot_constraint(world);
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
        // 残局指令下没有「等更好的机会」这回事，早出手的机会成本归零。
        let early_penalty = if just_received || endgame_shot != crate::endgame::EndgameShot::Free {
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
            + weights.w("shoot_three_preference")
                * (world.players[actor].data.tendencies.three_point_preference - 0.5)
                * if shot_zone == "three" {
                    f32::from(1u8)
                } else {
                    -f32::from(1u8)
                }
            - early_penalty
            + weights.w("shoot_catch_bonus") * state.openness() * f32::from(just_received)
            + weights.w("shoot_putback_bonus") * f32::from(putback_ready(world))
            - weights.w("shoot_off_dribble_penalty") * f32::from(!just_received)
            + weights.w("shoot_tendency")
                * (world.players[actor].data.tendencies.shoot_frequency - 0.5);
        // 出手门槛：时钟充裕时，只有期望得分跨过门槛的机会才值得出手。
        // 防守人高速扑防时按合速度折价，因为出手瞬间的干扰会高于当下。
        let closing = state
            .primary_defender
            .map(|defender| defender.closing_speed_ftps.max(f32::default()))
            .unwrap_or_default();
        // 出手瞬间的干扰要按防守人的合速度外推：扑防越猛，出手时越难。
        let predicted_contest = crate::resolve::clamp01(
            contest + closing * world.params.policy.g("closeout_contest_weight"),
        );
        let shot_clock_seconds = world.rules.league.shot_clock_seconds as f64;
        let gate = weights.w("shot_gate_base")
            + weights.w("shot_gate_span")
                * ((state.shot_clock / shot_clock_seconds) as f32)
                    .powf(weights.w("shot_gate_exponent"));
        // 门槛按该区最高可得分的归一化分值比较，否则上限较低的两分机会会被一刀切掉。
        let max_points = if shot_zone == "three" {
            weights.w("gate_points_three")
        } else {
            weights.w("gate_points_two")
        };
        let normalized = expected_points * weights.w("gate_points_reference") / max_points;
        // 时钟充裕时不投受干扰的球：受干扰的机会留给时钟紧迫时拆炸弹。
        let too_contested_early = state.shot_clock
            > world.params.policy.p("early_shot_block_seconds") as f64
            && predicted_contest > world.params.policy.p("contested_threshold");
        let endgame_allows = match endgame_shot {
            crate::endgame::EndgameShot::Free => true,
            crate::endgame::EndgameShot::ThreeOnly => shot_zone == "three",
            crate::endgame::EndgameShot::Hold => false,
        };
        // 落后三分且时间不够时门槛失效：这一球必须出手，哪怕是被干扰的三分。
        let gate_passes =
            normalized >= gate || endgame_shot == crate::endgame::EndgameShot::ThreeOnly;
        if gate_passes && !too_contested_early && endgame_allows {
            candidates.push((
                ActionKind::Shoot(shot_kind),
                shoot_score.max(0.001),
                None,
                state.actor_pos,
            ));
        }
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
    let drive_min_zone = world.params.policy.g("drive_min_zone_ft");
    let finish_zone = world.params.policy.g("finish_zone_ft");
    let drive_max_zone = world.params.policy.g("drive_max_zone_ft");
    if state.distance_to_hoop() < drive_max_zone && state.distance_to_hoop() > drive_min_zone {
        candidates.push((
            ActionKind::Drive,
            drive_score.max(0.001),
            None,
            state.hoop - (state.hoop - state.actor_pos).normalize_or_zero() * 3.0,
        ));
    }
    // 收球步：运球到终结距离之内后，用两步把球收到手上。走步判定
    // 就落在这里，收球点取上一次运球的触球位置。
    if state.distance_to_hoop() <= finish_zone
        && world.possession.dribble_state == crate::state::DribbleState::Dribbling
    {
        candidates.push((
            ActionKind::Finish,
            drive_score.max(f32::EPSILON) + weights.w("finish_priority"),
            None,
            state.hoop,
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
            - weights.w("pass_risk") * risk
            - weights.w("pass_react_penalty") * f32::from(reacting);
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

    let dribble_score = weights.w("dribble_base")
        + weights.w("dribble_setup") * state.early_clock_slack()
        - weights.w("dribble_react_penalty")
            * f32::from(reacting && route == Some(DefenderRoute::Under));
    // 已收球就不能再运球：二次运球是犯规，不是候选。
    if world.possession.dribble_state == crate::state::DribbleState::Dribbling {
        candidates.push((
            ActionKind::Dribble,
            dribble_score.max(0.001),
            None,
            state.hoop,
        ));
    }

    // 读到掩护路线后，候选收敛到该路线的应答集：钻下沿只能拔或传，
    // 挤过只能攻框或传。读球决定行动，而不是把权重叠加到已有候选上。
    // 残局指令优先于读球：必须抢三分或压时间时，路线应答集不再收窄候选。
    if reacting && endgame_shot == crate::endgame::EndgameShot::Free {
        if let Some(read) = route {
            candidates.retain(|(kind, _, _, _)| match read {
                DefenderRoute::Under => matches!(
                    kind,
                    ActionKind::Shoot(_) | ActionKind::Pass(_) | ActionKind::Dribble
                ),
                DefenderRoute::Over => matches!(
                    kind,
                    ActionKind::Drive | ActionKind::Finish | ActionKind::Pass(_)
                ),
                DefenderRoute::Switch | DefenderRoute::Trapped => true,
            });
            assert!(!candidates.is_empty(), "掩护路线的应答集不能为空");
        }
    }

    // 选板窗口内的持球人偏好与候选抑制。
    crate::tactics::apply_play_effects(world, &mut candidates);

    // 劣势动作强制截断：分数低于最强候选固定比例的候选直接出局，
    // 否则比例抽样会给明显更差的动作保留非零概率。
    let best = candidates
        .iter()
        .map(|(_, score, _, _)| *score)
        .fold(f32::default(), f32::max);
    let ratio = weights.w("truncation_ratio");
    candidates.retain(|(_, score, _, _)| *score >= best * ratio);
    assert!(!candidates.is_empty(), "截断不能清空候选集");

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
        ActionKind::Finish => 0.9,
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
        && world.now() - world.possession.started_at
            < world.params.policy.g("transition_window_seconds") as f64;
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
    // 落后三分且时间不够时，语境不匹配也要出手：这一球只能是三分。
    if zone == "three"
        && crate::endgame::shot_constraint(world) == crate::endgame::EndgameShot::ThreeOnly
    {
        return Some((ShotKind::PullUp, zone));
    }
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
    let base = spots[index];
    // 投射区域倾向决定站位离篮筐的远近：低倾向向内线收缩，高倾向向外线扩张到三分线为止。
    let hoop = world.hoop_for(team);
    let offset = base - hoop;
    let distance = offset.length();
    let preference = world.players[player].data.tendencies.three_point_preference;
    let rim = world.params.policy.g("spacing_rim_ft");
    let center = world.params.policy.g("spacing_preference_center");
    let span = world.params.policy.g("spacing_preference_span");
    let arc = world.rules.league.three_point_distance_ft
        + world.params.policy.g("spacing_arc_margin_ft");
    let shift = if preference < center {
        -(distance - rim).max(f32::default()) * (center - preference) * span
    } else {
        (arc - distance).max(f32::default()) * (preference - center) * span
    };
    hoop + offset.normalize_or_zero() * (distance + shift).max(rim)
}

pub fn move_toward_targets(world: &mut World) {
    crate::defense::refresh_matchup(world);
    crate::defense::apply_defense_targets(world);
    let offense = world.possession.team;
    let holder = world.holder();
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
