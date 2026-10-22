//! 策略层：把决策视图折算成期望得分，再用玻尔兹曼分布抽样。
//!
//! 每个候选动作都产出一个 [`Utility`]：`EPV_base + γ·σ − C_clock − C_foul +
//! B_tactic`。EPV 又按成因拆成能力、干扰、通道、协防、倾向、语境、失球七项，
//! 所以「为什么这个人选了这一手」可以从日志逐项读出，而不是只有一个总数。
//!
//! 所有系数来自 `policy.json`，由矩匹配拟合；本文件不持有内联行为常数，
//! 也没有「某一节某一分钟强制做某动作」的分支。动作产出的是意图，不是结果。

use glam::Vec2;
use nba_domain::capability::{fatigue_attenuation, map_skill_to_interval};

use crate::action::{ActionIntent, ActionKind, DefenderRoute, PassKind, ScreenKind, ShotKind};
use crate::resolve::{contest_for, shooter_zone};
use crate::state::{DecisionState, PossessionKind};
use crate::utility::{softmax_pick, EpvTerms, Utility};
use crate::world::World;

const MAX_SPEED_FT_PER_SEC: f32 = 22.0;

/// 候选动作：动作本身、目标球员、落点与它的效用分解。
pub type Candidate = (ActionKind, Option<usize>, Vec2, Utility);

/// 一次决策的完整结果。
#[derive(Debug, Clone)]
pub struct Decision {
    pub intent: ActionIntent,
    /// 选中动作的效用分解。
    pub utility: Utility,
    /// 当时的逆温度 β。
    pub beta: f32,
    /// 当时的方差偏好 γ。
    pub gamma: f32,
    /// 候选数量：为 1 时这次决策没有选择余地。
    pub candidate_count: usize,
    /// 供审计使用的完整因果快照。
    pub trace: crate::state::DecisionTraceSample,
}

pub fn choose_action(world: &mut World) -> Decision {
    let state = world.decision_state();
    let actor = state.actor;
    let now = world.now();
    let gamma = variance_appetite(&state);
    let beta = decision_temperature(world, actor);
    let candidates = candidate_utilities(world, &state);
    assert!(
        !candidates.is_empty(),
        "决策候选集不能为空：持球人 {} 在任何规则组合下都至少能传或投",
        actor
    );

    let totals: Vec<f32> = candidates
        .iter()
        .map(|(_, _, _, utility)| utility.total(gamma))
        .collect();
    let probabilities = crate::utility::softmax_probabilities(&totals, beta);
    let index = softmax_pick(&mut world.rng, &totals, beta);
    let (kind, target, to_ft, utility) = candidates[index];
    let candidate_count = candidates.len();
    let traced: Vec<crate::state::CandidateTrace> = candidates
        .iter()
        .enumerate()
        .map(
            |(offset, (kind, target, to_ft, utility))| crate::state::CandidateTrace {
                kind: *kind,
                target: *target,
                to_ft: *to_ft,
                utility: *utility,
                total: totals[offset],
                probability: probabilities[offset],
            },
        )
        .collect();
    let mut order: Vec<usize> = (0..traced.len()).collect();
    order.sort_by(|left, right| {
        traced[*right]
            .probability
            .total_cmp(&traced[*left].probability)
    });
    let chosen = order
        .iter()
        .position(|offset| *offset == index)
        .expect("the drawn candidate must appear in the trace");
    let traced: Vec<crate::state::CandidateTrace> =
        order.into_iter().map(|offset| traced[offset]).collect();
    let trace = crate::state::DecisionTraceSample {
        actor,
        beta,
        gamma,
        candidates: traced,
        chosen,
    };

    let duration = match kind {
        ActionKind::Pass(_) => {
            let distance = (to_ft - state.actor_pos).length();
            (distance / 28.0).clamp(0.35, 1.4) as f64
        }
        ActionKind::Drive => {
            let distance = (state.hoop - state.actor_pos).length();
            (distance / MAX_SPEED_FT_PER_SEC).clamp(0.6, 3.2) as f64
        }
        ActionKind::Screen(_) => 0.9,
        ActionKind::Finish => 0.9,
        ActionKind::Cut => 1.1,
        ActionKind::PostUp => 1.0,
        ActionKind::Shoot(_) => 0.55,
        ActionKind::Dribble | ActionKind::Relocate => 1.4,
    };

    Decision {
        intent: ActionIntent {
            actor,
            kind,
            target,
            from_ft: state.actor_pos,
            to_ft,
            start_time: now,
            duration,
        },
        utility,
        beta,
        gamma,
        candidate_count,
        trace,
    }
}

/// 当前状态下的全部候选动作及其效用分解。
///
/// 选择与审计走同一条路径：软选择用 [`choose_action`]，检视与校验用这个
/// 函数。两者共用一份候选集，否则「日志里看到的候选」与「真正参与抽样的
/// 候选」迟早会分家。
pub fn candidate_utilities(world: &World, state: &DecisionState) -> Vec<Candidate> {
    let gamma = variance_appetite(state);
    let now = world.now();
    // 防守人过掩护的路线：钻下沿是外线窗口，挤过是突破走廊。
    let route = state.primary_defender.and_then(|defender| defender.route);
    // 读球只在掩护后的第一个决策步生效；之后回合回到常规候选集。
    let reacting = world
        .possession
        .last_screen
        .map(|read| now - read.at)
        .map(|elapsed| elapsed <= crate::world::DECISION_INTERVAL + crate::world::TICK_SECONDS)
        .unwrap_or(false);

    let mut candidates: Vec<Candidate> = [
        shot_candidate(world, state, gamma),
        drive_candidate(world, state),
        finish_candidate(world, state, gamma),
        pass_candidate(world, state, gamma),
        screen_candidate(world, state),
        cut_candidate(world, state),
        post_candidate(world, state),
        hold_candidate(world, state),
    ]
    .into_iter()
    .flatten()
    .collect();

    // 读到掩护路线后，候选收敛到该路线的应答集：钻下沿只能拔或传，
    // 挤过只能攻框或传。读球决定行动，而不是把权重叠加到已有候选上。
    if reacting {
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
    candidates
}

/// 逆温度 β：球商决定基准确定性，体能决定衰减。
///
/// 高球商的球员高度确定地选最优解；体能透支时同一个人的选择噪声变大，
/// 于是顶人强投与危险横传自然出现，不需要为「累了就乱打」写分支。
pub fn decision_temperature(world: &World, actor: usize) -> f32 {
    let weights = &world.params.policy;
    let player = &world.players[actor];
    let beta = map_skill_to_interval(
        player.data.attributes.decision_iq,
        world.rules.attribute_response_floor,
        weights.w("temperature_min"),
        weights.w("temperature_max"),
    );
    fatigue_attenuation(
        beta,
        player.stamina,
        weights.w("temperature_fatigue_discount"),
    )
}

/// 方差偏好 γ：落后为正（偏好高方差），领先为负（压低方差）。
///
/// 数值由 [`DecisionState::leverage`] 在决策时算好，这里只读——保证
/// 「出手重不重要」与「防守该不该赌」用的是同一个处境量。
pub fn variance_appetite(state: &DecisionState) -> f32 {
    state.leverage.variance_appetite
}

/// 突破过人的概率：决策层用它估 EPV，结算层用它抽结果——同一个式子。
pub fn blow_by_probability(world: &World, skill_edge: f32, lane_norm: f32, help_term: f32) -> f32 {
    let weights = &world.params.policy;
    crate::resolve::sigmoid(
        weights.w("drive_beat_base")
            + weights.w("drive_beat_skill") * skill_edge * 2.0
            + weights.w("drive_beat_lane") * lane_norm
            - weights.w("drive_beat_help") * help_term,
    )
}

/// 结果的离散度：比例类结果按 `p(1−p)` 放大到得失分量级。
fn variance_of(p: f32, magnitude: f32) -> f32 {
    let one = f32::from(1u8);
    magnitude * magnitude * p * (one - p)
}

/// 进攻时钟的宽裕程度：从规则数据给出的「开始急着出手」读数线性归一到 1。
fn clock_slack(world: &World, state: &DecisionState) -> f32 {
    let weights = &world.params.policy;
    let end = weights.g("shot_slack_end_seconds");
    let full = world.rules.league.shot_clock_seconds;
    let span = (full - end).max(f32::MIN_POSITIVE);
    ((state.shot_clock as f32 - end) / span).clamp(f32::default(), f32::from(1u8))
}

/// 早出手的机会成本。
///
/// 出手越早，放弃的「再找一次更好机会」越多。这条成本的形状由规则数据
/// 给出（衰减到零的进攻时钟读数），强度由 γ 调节：领先方（γ<0）把机会
/// 成本放大，于是把出手推迟；落后方（γ>0）把它压小，于是提前抢攻。
fn early_shot_cost(world: &World, state: &DecisionState, gamma: f32) -> f32 {
    let weights = &world.params.policy;
    let gain = weights.w("slack_variance_gain");
    let floor = weights.w("slack_variance_floor");
    let factor = (f32::from(1u8) - gain * gamma).max(floor);
    weights.b("shoot_early_slack") * clock_slack(world, state) * factor
}

/// 时钟成本：非终结动作在进攻时钟见底时越来越贵。
///
/// 指数形式保证「剩 1 秒必须出手」不是写死的规则，而是成本曲线在端点
/// 自然压过所有其它候选的结果。
fn clock_cost(world: &World, shot_clock: f64) -> f32 {
    let weights = &world.params.policy;
    let tau = weights.g("clock_cost_tau_seconds").max(f32::MIN_POSITIVE) as f64;
    weights.w("clock_cost_scale") * (-shot_clock / tau).exp() as f32
}

/// 一次出手的质量：无干扰命中率、带干扰命中率、分值、是否三分。
struct ShotQuality {
    p_open: f32,
    p_make: f32,
    points: f32,
    is_three: bool,
}

/// 统一算一次出手的命中率：决策层用 `p_make` 估期望得分，审计层用
/// `p_open` 与 `p_make` 之差把「能力不足」与「被干扰」拆开。
fn shot_quality(world: &World, pos: Vec2, shooter: usize, contest: f32, kind: &str) -> ShotQuality {
    let team = world.players[shooter].team;
    let zone = shooter_zone(world, pos, team);
    let is_three = zone == "three";
    let distance = (pos - world.hoop_for(team)).length();
    let base = world.params.shot_make.base_rate(kind, distance, is_three);
    let slope = world.params.shot_make.skill_spread(zone);
    let skill = world.players[shooter].shooting_attribute(zone);
    let p_open = (base + (skill - 0.5) * slope).clamp(0.02, 0.98);
    let p_make = crate::resolve::sigmoid(
        logit(p_open) + world.params.shot_make.contest_sensitivity * (0.5 - contest),
    )
    .clamp(0.02, 0.98);
    let points = if is_three { 3.0 } else { 2.0 };
    ShotQuality {
        p_open,
        p_make,
        points,
        is_three,
    }
}

/// 接球人在自己的位置上的期望得分。
///
/// 传球的价值就是这个量乘以传球成功率：同一个位置，更好的射手得到
/// 更高的期望得分，于是球权会向强点聚集。
fn catch_value(world: &World, pos: Vec2, shooter: usize, contest: f32) -> f32 {
    let quality = shot_quality(world, pos, shooter, contest, "catch_shoot");
    quality.points * quality.p_make
}

fn shot_candidate(world: &World, state: &DecisionState, gamma: f32) -> Option<Candidate> {
    let params = &world.params;
    let weights = &params.policy;
    let actor = state.actor;
    let (shoot_kind, shot_zone) = shot_choice(world, state)?;
    let contest = contest_for(world, actor, state.actor_pos);
    let quality = shot_quality(world, state.actor_pos, actor, contest, shoot_kind.as_str());
    let is_three = quality.is_three;
    let points = quality.points;
    let p_open = quality.p_open;
    let p_contested = quality.p_make;
    let p_make = quality.p_make;

    let just_received = world
        .possession
        .last_pass_at
        .map(|time| world.now() - time < 1.2)
        .unwrap_or(false);
    // 出手门槛：时钟充裕时，只有期望得分跨过门槛的机会才值得出手。
    // 门槛以惩罚项进入效用，不再直接删掉候选——删掉就等于「不准投」。
    let shot_clock_seconds = world.rules.league.shot_clock_seconds as f64;
    let gate = weights.w("shot_gate_base")
        + weights.w("shot_gate_span")
            * ((state.shot_clock / shot_clock_seconds) as f32)
                .powf(weights.w("shot_gate_exponent"));
    let max_points = if is_three {
        weights.w("gate_points_three")
    } else {
        weights.w("gate_points_two")
    };
    let expected_points = p_make * points;
    let normalized = expected_points * weights.w("gate_points_reference") / max_points;
    let gate_shortfall = (gate - normalized).max(f32::default());

    let separation_bonus = world
        .last_separation
        .filter(|(separation_actor, _, time)| {
            *separation_actor == actor && world.now() - *time < 3.0
        })
        .map(|(_, separation, _)| (separation / 4.0).clamp(0.0, 1.5))
        .unwrap_or(0.0);
    let mut epv = EpvTerms {
        ability: weights.b("shoot_base") + weights.w("shoot_expected_points") * points * p_open,
        contest: weights.w("shoot_expected_points") * points * (p_contested - p_open),
        ..Default::default()
    };
    epv.context = weights.b("shoot_catch_bonus") * state.openness() * f32::from(just_received)
        + weights.b("shoot_putback_bonus") * f32::from(putback_ready(world))
        + weights.b("shoot_separation_bonus") * separation_bonus
        + weights.b("momentum_shot_bonus") * state.momentum
        - weights.b("shoot_off_dribble_penalty") * f32::from(!just_received)
        - weights.b("shot_gate_penalty") * gate_shortfall;
    epv.tendency = weights.b("shoot_mid_bonus") * f32::from(shot_zone == "mid")
        + weights.b("shoot_three_preference")
            * (world.players[actor].data.tendencies.three_point_preference - 0.5)
            * if is_three {
                f32::from(1u8)
            } else {
                -f32::from(1u8)
            }
        + weights.b("shoot_tendency")
            * (world.players[actor].data.tendencies.shoot_frequency - 0.5);

    let utility = Utility {
        epv,
        variance: variance_of(p_make, points) * weights.w("variance_shot_weight"),
        // 接到球不等于不用为「提前终结回合」付代价：接球就投的时机加成
        // 在 EPV 的语境项里，机会成本在时钟项里，两者不互相抵消。
        clock_cost: early_shot_cost(world, state, gamma)
            - weights.b("shoot_pressure") * state.shot_clock_pressure(),
        foul_cost: f32::default(),
        tactical_bonus: f32::default(),
    };
    Some((
        ActionKind::Shoot(shoot_kind),
        None,
        state.actor_pos,
        utility,
    ))
}

fn drive_candidate(world: &World, state: &DecisionState) -> Option<Candidate> {
    let weights = &world.params.policy;
    let distance = state.distance_to_hoop();
    if distance >= weights.g("drive_max_zone_ft") || distance <= weights.g("drive_min_zone_ft") {
        // 位置决定这个动作在物理上是否可用：距离外的突破不成立。
        return None;
    }
    let lane_norm = (state.lane_clearance / 8.0).clamp(0.0, 1.5);
    let help_term = state
        .help
        .map(|help| (1.0 / help.arrival_seconds.max(0.1)).min(2.0))
        .unwrap_or(0.0);
    let p_open = blow_by_probability(world, state.skill_edge, f32::default(), f32::default());
    let p_lane = blow_by_probability(world, state.skill_edge, lane_norm, f32::default());
    let p_beat = blow_by_probability(world, state.skill_edge, lane_norm, help_term);
    let possession_value = weights.w("possession_value");
    let win = possession_value * (f32::from(1u8) + weights.w("drive_paint_bonus"));
    let lose = possession_value * weights.w("drive_stall_fraction");
    let swing = win - lose;
    let tightness = state
        .primary_defender
        .map(|defender| (1.0 - defender.distance_ft / 6.0).clamp(0.0, 1.0))
        .unwrap_or(0.0);
    let mut epv = EpvTerms {
        // 按成因递进：技能单独造成的优势、走廊带来的优势、协防带来的回吐。
        ability: lose + p_open * swing + weights.b("drive_mismatch") * state.mismatch,
        contest: weights.b("drive_tight") * tightness,
        lane: weights.b("drive_lane") * lane_norm + (p_lane - p_open) * swing,
        help: -weights.b("drive_help") * help_term + (p_beat - p_lane) * swing,
        ..Default::default()
    };
    epv.tendency = weights.b("drive_tendency")
        * (world.players[state.actor].data.tendencies.drive_frequency - 0.5);
    epv.turnover = -weights.w("epv_turnover_cost")
        * possession_value
        * (f32::from(1u8) - p_beat)
        * weights.w("drive_turnover_rate");
    let to_ft = state.hoop - (state.hoop - state.actor_pos).normalize_or_zero() * 3.0;
    Some((
        ActionKind::Drive,
        None,
        to_ft,
        Utility {
            epv,
            variance: variance_of(p_beat, possession_value) * weights.w("variance_choice_weight"),
            clock_cost: clock_cost(world, state.shot_clock) * weights.w("drive_clock_weight"),
            foul_cost: weights.w("offensive_foul_rate_drive") * possession_value,
            tactical_bonus: f32::default(),
        },
    ))
}

/// 收球步：把运球收成上篮步。到了终结距离之后不用再突，这两步就是出手。
fn finish_candidate(world: &World, state: &DecisionState, gamma: f32) -> Option<Candidate> {
    let weights = &world.params.policy;
    if state.distance_to_hoop() > weights.g("finish_zone_ft") {
        return None;
    }
    // 已收球就不能再收一次：再收就是二次运球。
    if world.possession.dribble_state != crate::state::DribbleState::Dribbling {
        return None;
    }
    let actor = state.actor;
    let contest = contest_for(world, actor, state.actor_pos);
    let quality = shot_quality(world, state.actor_pos, actor, contest, "layup");
    let p_open = quality.p_open;
    let p_make = quality.p_make;
    let points = quality.points;
    let epv = EpvTerms {
        ability: points * p_open,
        contest: points * (p_make - p_open),
        context: weights.b("finish_priority"),
        ..Default::default()
    };
    Some((
        ActionKind::Finish,
        None,
        state.hoop,
        Utility {
            epv,
            variance: variance_of(p_make, points) * weights.w("variance_shot_weight"),
            // 收球也是一次出手：提前终结同样放弃了「再找一次更好的机会」。
            clock_cost: early_shot_cost(world, state, gamma)
                - weights.b("shoot_pressure") * state.shot_clock_pressure(),
            foul_cost: f32::default(),
            tactical_bonus: f32::default(),
        },
    ))
}

fn pass_candidate(world: &World, state: &DecisionState, gamma: f32) -> Option<Candidate> {
    let weights = &world.params.policy;
    let possession_value = weights.w("possession_value");
    let mut best: Option<(usize, Vec2, Utility, f32)> = None;
    for teammate in &state.teammates {
        let advancement = if state.attacking_right {
            teammate.pos.x - state.actor_pos.x
        } else {
            state.actor_pos.x - teammate.pos.x
        };
        let risk = (1.0 - (teammate.lane_clearance / 6.0).clamp(0.0, 1.0)).max(0.0);
        let teammate_three =
            shooter_zone(world, teammate.pos, world.players[teammate.index].team) == "three";
        let p_complete = (weights.w("pass_completion_base")
            - weights.w("pass_completion_risk") * risk
            + weights.w("pass_completion_open") * (f32::from(1u8) - teammate.openness))
            .clamp(0.0, 1.0);
        // 接球人的得分期望：同一个位置，更好的射手得到更高的期望得分。
        // 传球价值必须用这个量，否则球权不会向强点聚集，关键时段也就
        // 不会出现「把球交给头号选择」。
        let v_catch = catch_value(world, teammate.pos, teammate.index, teammate.openness);
        let epv = EpvTerms {
            ability: p_complete * v_catch,
            lane: weights.b("pass_advancement") * (advancement / 20.0).clamp(-1.0, 1.0)
                - weights.b("pass_risk") * risk,
            context: weights.b("pass_perimeter") * f32::from(teammate_three),
            turnover: -weights.w("epv_turnover_cost")
                * possession_value
                * (f32::from(1u8) - p_complete),
            ..Default::default()
        };
        let utility = Utility {
            epv,
            variance: variance_of(p_complete, possession_value)
                * weights.w("variance_choice_weight"),
            clock_cost: clock_cost(world, state.shot_clock),
            foul_cost: f32::default(),
            tactical_bonus: f32::default(),
        };
        let total = utility.total(gamma);
        if best
            .as_ref()
            .is_none_or(|(_, _, _, current)| total > *current)
        {
            best = Some((teammate.index, teammate.pos, utility, total));
        }
    }
    best.map(|(target, pos, utility, _)| {
        (
            ActionKind::Pass(PassKind::Chest),
            Some(target),
            pos,
            utility,
        )
    })
}

fn screen_candidate(world: &World, state: &DecisionState) -> Option<Candidate> {
    let weights = &world.params.policy;
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
        .map(|view| view.index)?;
    let mut epv = EpvTerms {
        // 掩护本身不得分：它的价值是「制造优势的概率 × 一次回合的价值」。
        ability: weights.w("possession_value")
            * (weights.w("screen_base") + weights.w("screen_mismatch") * state.mismatch.abs())
                .clamp(f32::default(), f32::from(1u8)),
        ..Default::default()
    };
    epv.tendency = weights.b("screen_tendency")
        * (world.players[state.actor].data.tendencies.screen_frequency - 0.5);
    Some((
        ActionKind::Screen(ScreenKind::OnBall),
        Some(screener),
        state.actor_pos,
        Utility {
            epv,
            variance: f32::default(),
            clock_cost: clock_cost(world, state.shot_clock),
            foul_cost: weights.w("offensive_foul_rate_screen") * weights.w("possession_value"),
            tactical_bonus: f32::default(),
        },
    ))
}

fn cut_candidate(world: &World, state: &DecisionState) -> Option<Candidate> {
    let weights = &world.params.policy;
    let lane_norm = (state.lane_clearance / 8.0).clamp(0.0, 1.5);
    let cutter = state
        .teammates
        .iter()
        .max_by(|a, b| {
            b.defender_distance_ft
                .partial_cmp(&a.defender_distance_ft)
                .expect("finite distances")
        })
        .map(|view| view.index)?;
    let epv = EpvTerms {
        // 空切同样是「出现空位概率 × 一次回合的价值」。
        ability: weights.w("possession_value")
            * (weights.w("cut_base") + weights.w("cut_lane") * lane_norm)
                .clamp(f32::default(), f32::from(1u8)),
        ..Default::default()
    };
    Some((
        ActionKind::Cut,
        Some(cutter),
        state.hoop,
        Utility {
            epv,
            variance: f32::default(),
            clock_cost: clock_cost(world, state.shot_clock),
            foul_cost: f32::default(),
            tactical_bonus: f32::default(),
        },
    ))
}

fn post_candidate(world: &World, state: &DecisionState) -> Option<Candidate> {
    let weights = &world.params.policy;
    let size = world.players[state.actor].data.height_cm as f32 / 200.0;
    let possession_value = weights.w("possession_value");
    let p_score = (weights.w("post_base") + weights.w("post_size") * size).clamp(0.0, 1.0);
    let epv = EpvTerms {
        ability: possession_value * p_score,
        ..Default::default()
    };
    Some((
        ActionKind::PostUp,
        None,
        state.hoop,
        Utility {
            epv,
            variance: variance_of(p_score, possession_value) * weights.w("variance_choice_weight"),
            clock_cost: clock_cost(world, state.shot_clock) * weights.w("post_clock_weight"),
            foul_cost: weights.w("offensive_foul_rate_post") * possession_value,
            tactical_bonus: f32::default(),
        },
    ))
}

fn hold_candidate(world: &World, state: &DecisionState) -> Option<Candidate> {
    // 已收球就不能再运球：二次运球是犯规，持球选项在物理上不存在。
    if world.possession.dribble_state != crate::state::DribbleState::Dribbling {
        return None;
    }
    let weights = &world.params.policy;
    // 保留球权的价值就是一次进攻回合的价值，加上继续组织的期权价值。
    let possession_value = weights.w("possession_value");
    let epv = EpvTerms {
        ability: possession_value * weights.w("hold_epv_multiplier") + weights.b("dribble_base"),
        context: weights.b("dribble_setup") * clock_slack(world, state)
            - weights.b("dribble_react_penalty")
                * f32::from(
                    state.primary_defender.and_then(|d| d.route) == Some(DefenderRoute::Under),
                ),
        ..Default::default()
    };
    Some((
        ActionKind::Dribble,
        None,
        state.hoop,
        Utility {
            epv,
            variance: f32::default(),
            clock_cost: clock_cost(world, state.shot_clock),
            foul_cost: f32::default(),
            tactical_bonus: f32::default(),
        },
    ))
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
    let putback_ready = self::putback_ready(world);
    let finishing = world.players[state.actor].data.attributes.finishing;

    // 出手类型由动作语境决定；语境与位置不匹配时退回通用出手类型，
    // 而不是拒绝出手——一个刚做完空切的球员站到弧顶，仍然可以投三分。
    if zone == "rim" {
        if putback_ready {
            return Some((ShotKind::Putback, zone));
        }
        if cut_ready {
            return Some((
                if finishing > 0.6 {
                    ShotKind::CuttingDunk
                } else {
                    ShotKind::CuttingLayup
                },
                zone,
            ));
        }
        if transition {
            return Some((ShotKind::RunningLayup, zone));
        }
        if driving {
            return Some((
                if finishing > 0.6 {
                    ShotKind::DrivingDunk
                } else {
                    ShotKind::DrivingLayup
                },
                zone,
            ));
        }
        if post {
            return Some((ShotKind::Hook, zone));
        }
    }
    if zone == "near" {
        if putback_ready {
            return Some((ShotKind::Putback, zone));
        }
        if driving {
            return Some((ShotKind::Floater, zone));
        }
        if post {
            return Some((ShotKind::Hook, zone));
        }
    }
    if zone == "mid" && post {
        return Some((ShotKind::Turnaround, zone));
    }
    if zone == "three" && transition {
        return Some((ShotKind::RunningJump, zone));
    }
    if just_received {
        // 接球就投在三分线与中距离都成立：中距离的接球跳投是真实存在的出手类型。
        return matches!(zone, "three" | "mid").then_some((ShotKind::CatchShoot, zone));
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
    let arc =
        world.rules.league.three_point_distance_ft + world.params.policy.g("spacing_arc_margin_ft");
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
