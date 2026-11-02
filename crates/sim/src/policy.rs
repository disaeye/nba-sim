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
use crate::draw::{purpose, Stream};
use crate::resolve::shooter_zone;
use crate::state::{DecisionRecord, DecisionSample, DecisionState, Phase, Revalidation};
use crate::utility::{softmax_pick_at, EpvTerms, Utility};
use crate::world::World;

const MAX_SPEED_FT_PER_SEC: f32 = 22.0;

/// 出手评估的逐条轨迹（三分与压哨），仅在 `NBA_SIM_TRACE_SHOT` 设置时输出到 stderr。
/// 标准输出只承载机器可读的 JSON，诊断信息一律走标准错误。
fn shot_trace() -> bool {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRACE.get_or_init(|| std::env::var_os("NBA_SIM_TRACE_SHOT").is_some())
}

fn pass_trace() -> bool {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRACE.get_or_init(|| std::env::var_os("NBA_SIM_TRACE_PASS").is_some())
}

/// 决策调度：每 tick 调用，只在持球人的调度到期时执行一次完整决策。
///
/// 决策由动作完成、球权变化、接球、掩护接触、协防承诺等事件触发；
/// 没有事件时，`rules.redecide_interval_seconds` 是最小回退间隔。
pub fn tick(world: &mut World) {
    if !matches!(world.phase(), Phase::Live) {
        return;
    }
    let Some(holder) = world.holder() else {
        return;
    };
    // 时钟见底时节奏优先于调度：球还在手上，就不能等下一次回退间隔。
    let urgent = world.clock.shot_clock() <= 0.2;
    if world.players[holder].action.is_some()
        || (!urgent && world.now() < world.players[holder].schedule.next_redecide_at)
    {
        return;
    }
    // 选板与故意犯规都在决策时刻推进：两者都会发事实，
    // 而事实写入会分配内存，稳态 tick 必须保持零分配。
    crate::tactics::update_play_window(world);
    if crate::timeouts::coach_stop_run(world) {
        return;
    }
    if let Some(plan) = crate::endgame::intentional_foul(world) {
        crate::resolve::foul::apply_non_shooting_foul(world, plan.fouler, plan.target);
        return;
    }
    let decision = choose_action(world);
    let intent = decision.intent.clone();
    let actor = intent.actor;
    let since_screen = world
        .possession
        .last_screen
        .map(|read| world.now() - read.at)
        .unwrap_or(f64::INFINITY);
    let shooter_three = world.players[actor].data.attributes.shooting_three;
    let trace_index = world.decision_traces.len() as u32;
    world.decision_samples.push(DecisionSample {
        route: crate::views::screen_read_route(world),
        family: intent.kind.family(),
        shot_clock: world.clock.shot_clock(),
        since_screen,
        shooter_three,
        utility: decision.utility,
        beta: decision.beta,
        gamma: decision.gamma,
        candidates: decision.candidate_count.min(u8::MAX as usize) as u8,
        on_ball: true,
        trace: Some(trace_index),
    });
    world.active_trace = Some(trace_index);
    world.decision_traces.push(decision.trace.clone());
    world.decision_records.push(DecisionRecord {
        decision_id: u64::from(trace_index),
        tick: world.clock.tick(),
        actor,
        observation_digest: crate::views::observation_digest(world, actor),
        candidates: decision.trace.candidates.clone(),
        chosen: decision.trace.chosen,
        revalidation: Revalidation::Accepted,
    });
    world.players[actor].schedule.next_redecide_at =
        world.now() + f64::from(world.rules.redecide_interval_seconds);
    world.players[actor].schedule.pending = None;
    if world.possession.first_action_seconds.is_none() {
        world.possession.first_action_seconds = Some(world.now() - world.possession.started_at);
    }
    // 决策选出的动作决定接下来的持球阶段。出手类型的选择读当前阶段：
    // 阶段写在这里，出手结算读到的阶段与刚选定的动作一致。
    world.possession.ball_phase = match intent.kind {
        ActionKind::Drive => crate::state::BallPhase::Drive,
        ActionKind::Finish => crate::state::BallPhase::Gather,
        ActionKind::PostUp => crate::state::BallPhase::Post,
        ActionKind::Dribble(_)
        | ActionKind::Pass(_)
        | ActionKind::Cut(_)
        | ActionKind::Screen(_)
        | ActionKind::Fake(_)
        | ActionKind::Protect(_)
        | ActionKind::Relocate => crate::state::BallPhase::Dribble,
        ActionKind::Shoot(_) => world.possession.ball_phase,
    };
    match intent.kind {
        ActionKind::Drive
        | ActionKind::Finish
        | ActionKind::Dribble(_)
        | ActionKind::PostUp
        | ActionKind::Relocate => {
            world.players[actor].target = intent.to_ft;
            world.players[actor].target_kind = crate::state::TargetKind::Window;
            world.players[actor].target_partner = actor;
        }
        ActionKind::Cut(_) => {
            if let Some(cutter) = intent.target {
                world.players[cutter].target = crate::resolve::pass::cut_target(world, cutter);
                world.players[cutter].target_kind = crate::state::TargetKind::Window;
                world.players[cutter].target_partner = cutter;
            }
        }
        _ => {}
    }
    if let ActionKind::Pass(_) = intent.kind {
        let _ = intent.target;
    }
    let screen_defender = decision.screen_defender;
    world.screen_defender = screen_defender;
    let mut participants = vec![(actor, crate::action::ActionIntentRole::Primary)];
    match intent.kind {
        ActionKind::Screen(_) => {
            let screener = intent.target.expect("screen action requires a screener");
            participants.push((screener, crate::action::ActionIntentRole::ScreenPartner));
            let defender = screen_defender.expect("screen must have a defender");
            let defender_pos = world.players[defender].pos;
            let handler_pos = world.players[actor].pos;
            let contact_target = defender_pos
                + (handler_pos - defender_pos).normalize_or_zero()
                    * world.rules.min_player_separation_ft;
            world.players[screener].target = contact_target;
            world.players[screener].target_kind = crate::state::TargetKind::Contact;
            world.players[screener].target_partner = defender;
        }
        ActionKind::Cut(_) => {
            let cutter = intent.target.expect("cut action requires a cutter");
            participants.push((cutter, crate::action::ActionIntentRole::CutPartner));
        }
        _ => {}
    }
    assert!(world.start_action(intent, &participants));
}

/// 无球球员的独立决策：每名不在持球位置上的进攻球员按自己的候选集与
/// 自己的逆温度选择动作，并由他本人执行。
///
/// 这条路径与持球人的决策完全分开：动作的执行人是无球球员自己，动作的
/// 后果（跑位、切入、接球窗口）由他自己的运行时产出，不是持球人代选的。
pub fn off_ball_tick(world: &mut World) {
    if !matches!(world.phase(), Phase::Live) {
        return;
    }
    let Some(holder) = world.holder() else {
        return;
    };
    let team = world.possession.team;
    let now = world.now();
    // 逐 tick 走这条路径，不能分配：按句柄遍历，在循环内过滤。
    for actor in 0..world.players.len() {
        if actor == holder || !world.players[actor].on_court || world.players[actor].team != team {
            continue;
        }
        // 正在重选恢复段的球员先把自己的路线走完，不叠加新的无球动作。
        if world.players[actor].recovery_since > 0.0 {
            continue;
        }
        // 已经在做动作的球员不重新决策：同一时刻只有一个身体动作。
        if world.players[actor].action.is_some() {
            continue;
        }
        if now < world.players[actor].schedule.next_redecide_at {
            continue;
        }
        // 无球动作只发生在回合前段：进攻时钟进入后段时，球权和出手
        // 都由持球人的决策决定，无球球员不再另起动作。
        if world.clock.shot_clock() < 10.0 {
            continue;
        }
        // 剧本窗口内的动作由剧本编排：无球球员此时不另起动作，
        // 否则剧本的候选集与战术先验会被打散。
        if crate::tactics::active_play(world).is_some() {
            continue;
        }
        // 每个回合只允许一次无球自主动作：无球跑位是补充，不是主循环。
        if world.possession.off_ball_actions > 0 {
            continue;
        }
        // 回合开头的几秒属于战术发动：无球球员等剧本窗口有了结论再动，
        // 否则剧本还没被读到就被无球跑位顶掉。
        if now - world.possession.started_at < 3.0 {
            continue;
        }
        // 无球动作不需要和持球人一样频繁：间隔放长，避免每名球员
        // 每个决策周期都产生一套生命周期事实。已经评估过就后退调度，
        // 否则每 tick 都会重新评估同一个人。
        world.players[actor].schedule.next_redecide_at =
            now + f64::from(world.rules.redecide_interval_seconds) * 12.0;
        world.players[actor].schedule.pending = None;
        let Some(decision) = off_ball_decision(world, actor) else {
            continue;
        };
        let intent = decision.intent;
        world.possession.off_ball_actions += 1;
        // 掩护后的下一次动作已经兑现，顺下与外弹的偏好随之取消。
        world.players[actor].screened_at = 0.0;
        world.players[actor].target = intent.to_ft;
        world.players[actor].target_kind = crate::state::TargetKind::Window;
        world.players[actor].target_partner = actor;
        let trace_index = world.decision_traces.len() as u32;
        world.decision_samples.push(DecisionSample {
            route: crate::views::screen_read_route(world),
            family: intent.kind.family(),
            shot_clock: world.clock.shot_clock(),
            since_screen: f64::INFINITY,
            shooter_three: world.players[actor].data.attributes.shooting_three,
            utility: decision.utility,
            beta: decision.beta,
            gamma: decision.gamma,
            candidates: decision.candidate_count.min(u8::MAX as usize) as u8,
            on_ball: false,
            trace: Some(trace_index),
        });
        world.decision_traces.push(decision.trace.clone());
        world.decision_records.push(DecisionRecord {
            decision_id: u64::from(trace_index),
            tick: world.clock.tick(),
            actor,
            observation_digest: crate::views::observation_digest(world, actor),
            candidates: decision.trace.candidates.clone(),
            chosen: decision.trace.chosen,
            revalidation: Revalidation::Accepted,
        });
        let participants = [(actor, crate::action::ActionIntentRole::Primary)];
        assert!(
            world.start_action(intent, &participants),
            "无球球员 {actor} 的动作必须能开始"
        );
    }
}

/// 无球候选：跑位到编队空位，或切入篮下。
///
/// 两者都由该球员自己的能力与位置估值；选择用他自己的逆温度，因此
/// 不同球员在同一时刻会做出不同的无球决定。
fn off_ball_decision(world: &World, actor: usize) -> Option<Decision> {
    let team = world.players[actor].team;
    let now = world.now();
    let hoop = world.hoop_for(team);
    let pos = world.players[actor].pos;
    let shot_clock = world.clock.shot_clock();
    let beta = decision_temperature(world, actor);
    let weights = &world.params.policy;

    // 跑位：回到编队空位。价值是该位置的接球价值，成本是跑动占用的时钟。
    let spot = crate::formation::spot_target(world, actor);
    let travel = (spot - pos).length();
    let relocate: Candidate = (
        ActionKind::Relocate,
        None,
        spot,
        Utility {
            epv: EpvTerms {
                ability: crate::value::catch_value(world, spot, actor, 0.0),
                ..Default::default()
            },
            variance: f32::default(),
            clock_cost: clock_cost(world, shot_clock)
                * (travel / weights.g("relocate_reference_ft").max(f32::EPSILON)).clamp(0.0, 1.0),
            foul_cost: f32::default(),
            tactical_bonus: f32::default(),
        },
    );

    // 切入：先按几何选路线类型，再按类型取落点。
    let to_rim = (hoop - pos).normalize_or_zero();
    let lateral = Vec2::new(-to_rim.y, to_rim.x);
    let marker = world
        .players
        .iter()
        .filter(|player| player.on_court && player.team != team)
        .map(|player| (player.pos - pos).length())
        .fold(f32::INFINITY, f32::min);
    let openness = (marker / weights.g("cut_open_distance_ft").max(f32::EPSILON)).clamp(0.0, 1.0);
    let holder_pos = world.holder().map(|handle| world.players[handle].pos);
    // 防守人卡在持球人与自己之间：转身从背后切。
    let denied = holder_pos.is_some_and(|ball| {
        let segment = ball - pos;
        let length_squared = segment.length_squared();
        world
            .players
            .iter()
            .filter(|player| player.on_court && player.team != team)
            .any(|player| {
                let t = if length_squared > f32::MIN_POSITIVE {
                    ((player.pos - pos).dot(segment) / length_squared).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (player.pos - (pos + segment * t)).length() < 1.5 && (0.15..=0.85).contains(&t)
            })
    });
    let near_screen = world
        .possession
        .last_screen
        .map(|read| (world.players[read.screener].pos - pos).length())
        .unwrap_or(f32::INFINITY)
        < 8.0;
    let kind = if denied {
        crate::action::CutKind::Backdoor
    } else if world.possession.last_pass_from == Some(actor) {
        crate::action::CutKind::GiveAndGo
    } else if near_screen {
        crate::action::CutKind::Curl
    } else if marker < 3.0 {
        crate::action::CutKind::VCut
    } else if (pos - hoop).length() > 22.0 {
        crate::action::CutKind::PopUp
    } else {
        crate::action::CutKind::Basket
    };
    let cut_spot = match kind {
        crate::action::CutKind::Basket | crate::action::CutKind::GiveAndGo => {
            hoop - to_rim * weights.g("finish_min_distance_ft")
        }
        crate::action::CutKind::Backdoor => hoop - to_rim * 3.0 + lateral * 4.0,
        crate::action::CutKind::VCut => pos - to_rim * 2.0,
        crate::action::CutKind::Curl => world
            .possession
            .last_screen
            .map(|read| {
                let spot = world.players[read.screener].pos;
                spot + (hoop - spot).normalize_or_zero() * 4.0
            })
            .unwrap_or(hoop - to_rim * weights.g("finish_min_distance_ft")),
        crate::action::CutKind::PopUp => holder_pos
            .map(|ball| pos + (ball - pos).normalize_or_zero() * 6.0)
            .unwrap_or(pos - to_rim * 3.0),
    };
    let rim_attack = matches!(
        kind,
        crate::action::CutKind::Basket
            | crate::action::CutKind::Backdoor
            | crate::action::CutKind::GiveAndGo
    );
    // 空切是机会动作，不是常规站位：只有对位人确实跟丢、且篮下接球
    // 比留在自己的位置更值时才是候选；V 字与上提是拿球动作，不要求篮下机会。
    let cut_value = crate::value::catch_value(world, cut_spot, actor, 0.0);
    let spot_value = crate::value::catch_value(world, spot, actor, 0.0);
    let cut_open = marker >= weights.g("cut_open_distance_ft") * 0.6;
    let cut_pays = if rim_attack {
        cut_value > spot_value * 1.2
    } else {
        cut_value > spot_value
    };
    let mut cut_available = if rim_attack {
        cut_open && cut_pays && travel > 3.0
    } else {
        cut_pays && travel > 2.0
    };
    let mut relocate = relocate;
    let mut cut: Candidate = (
        ActionKind::Cut(kind),
        None,
        cut_spot,
        Utility {
            epv: EpvTerms {
                ability: cut_value * openness,
                tendency: weights.b("cut_tendency")
                    * (world.players[actor].data.tendencies.cut_frequency - 0.5),
                ..Default::default()
            },
            variance: f32::default(),
            clock_cost: clock_cost(world, shot_clock) * 0.5,
            foul_cost: f32::default(),
            tactical_bonus: f32::default(),
        },
    );

    // 候选集只放真正成立的动作：空切不成立时就只剩跑位。
    // 掩护后的顺下与外弹：掩护人刚才设了掩护，就必须用下一次动作
    // 兑现它。对位人往缩就顺下，对位人贴上就外弹。
    let post_screen = world.players[actor].screened_at > 0.0
        && now - world.players[actor].screened_at < 2.0;
    if post_screen {
        cut_available = true;
        if marker >= 4.0 {
            cut.3.tactical_bonus += weights.g("cut_open_distance_ft") * 0.5;
        } else {
            relocate.3.tactical_bonus += weights.g("cut_open_distance_ft") * 0.5;
        }
    }
    let mut candidates = [relocate; 2];
    let mut count = 1usize;
    if cut_available {
        candidates[count] = cut;
        count += 1;
    }
    let gamma = f32::default();
    // 固定数组，逐 tick 的热路径上不分配。
    let mut totals = [0.0f32; 2];
    for (slot, candidate) in candidates[..count].iter().enumerate() {
        totals[slot] = candidate.3.total(gamma);
    }
    assert!(beta > 0.0, "逆温度必须为正：{beta}");
    let peak = totals[..count]
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);
    let mut weights_soft = [0.0f32; 2];
    for slot in 0..count {
        weights_soft[slot] = (beta * (totals[slot] - peak)).exp();
    }
    // 归一化后才能作为候选概率写进审计快照：未归一化的权重会让
    // 「候选概率合计为 1」的判据失效。
    let total_weight: f32 = weights_soft[..count].iter().sum();
    assert!(total_weight > 0.0, "无球候选权重必须为正");
    for slot in 0..count {
        weights_soft[slot] /= total_weight;
    }
    let roll = world.draw(
        crate::draw::Stream::Decision,
        actor,
        crate::draw::purpose::ACTION_CHOICE,
        9,
    );
    let chosen = crate::resolve::weighted_pick_at(roll, &weights_soft[..count]);
    let candidate = candidates[chosen];
    let kind = candidate.0;
    let to_ft = candidate.2;
    // 已经站在该站的位置上就不必再跑一趟：没有位移的跑位不产生动作。
    if matches!(kind, ActionKind::Relocate) && travel < 4.0 {
        return None;
    }
    let speed = world.players[actor]
        .effective_speed(&world.params.policy)
        .max(1.0);
    let cut_travel = (cut_spot - pos).length();
    // 空切必须跑得到：超过一个动作窗口能跑的距离就不成立。
    if matches!(kind, ActionKind::Cut(_)) && cut_travel > speed * 2.6 * 0.85 {
        return None;
    }
    let duration = match kind {
        ActionKind::Cut(_) => (cut_travel / speed * 1.35).clamp(1.2, 2.6) as f64,
        _ => ((to_ft - pos).length() / speed * 1.2).clamp(0.8, 2.6) as f64,
    };
    let trace = crate::state::DecisionTraceSample {
        actor,
        beta,
        gamma,
        chosen,
        candidates: candidates
            .iter()
            .zip(&totals)
            .zip(&weights_soft)
            .map(
                |((candidate, total), probability)| crate::state::CandidateTrace {
                    kind: candidate.0,
                    target: candidate.1,
                    to_ft: candidate.2,
                    utility: candidate.3,
                    total: *total,
                    probability: *probability,
                },
            )
            .collect(),
    };
    Some(Decision {
        intent: ActionIntent {
            actor,
            kind,
            target: candidate.1,
            from_ft: pos,
            to_ft,
            start_time: now,
            duration,
        },
        utility: candidate.3,
        beta,
        gamma,
        candidate_count: count,
        screen_defender: None,
        trace,
    })
}

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
    /// 掩护候选所针对的持球防守人。
    pub screen_defender: Option<usize>,
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
    let mut candidates = candidates;
    // 后场推进：八秒计时过半后，优先选把球送过中线的动作。
    let mid = world.rules.court.width_ft * 0.5;
    let attacking_right = state.attacking_right;
    let ball_x = world.ball.pos.x;
    let in_backcourt = if attacking_right {
        ball_x < mid
    } else {
        ball_x > mid
    };
    if in_backcourt {
        if let Some(start) = world.possession.backcourt_started_game_clock {
            let elapsed = (start - world.clock.game_clock()).max(0.0);
            let limit = f64::from(world.rules.backcourt_seconds);
            if elapsed >= limit * 0.5 {
                let bonus = world.params.policy.b("advance_pressure_bonus");
                for candidate in &mut candidates {
                    let advances = if attacking_right {
                        candidate.2.x > mid
                    } else {
                        candidate.2.x < mid
                    };
                    if advances {
                        candidate.3.tactical_bonus += bonus;
                    }
                }
            }
        }
    }
    let screen_defender = state
        .primary_defender
        .map(|defender| defender.index)
        .filter(|_| {
            candidates
                .iter()
                .any(|candidate| matches!(candidate.0, ActionKind::Screen(_)))
        });
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
    if pass_trace() {
        static SEEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seen = SEEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let has_pass = candidates
            .iter()
            .any(|(kind, _, _, _)| matches!(kind, ActionKind::Pass(_)));
        if has_pass && seen < 60 {
            eprintln!(
                "DECIDE t={:.2} actor={} dist={:.2} clock={:.2} beta={beta:.3} gamma={gamma:.3} open={:.3}",
                now,
                actor,
                state.distance_to_hoop(),
                state.shot_clock,
                state.openness(),
            );
            for (offset, (other, target, pos, other_utility)) in candidates.iter().enumerate() {
                eprintln!(
                    "  {other:?} tgt={target:?} pos=({:.1},{:.1}) total={:.4} p={:.4} epv={:.4} var={:.4} clock={:.4} foul={:.4} tactic={:.4}",
                    pos.x,
                    pos.y,
                    totals[offset],
                    probabilities[offset],
                    other_utility.epv_base(),
                    other_utility.variance,
                    other_utility.clock_cost,
                    other_utility.foul_cost,
                    other_utility.tactical_bonus,
                );
            }
        }
    }
    let index = softmax_pick_at(
        world.draw(Stream::Decision, actor, purpose::ACTION_CHOICE, 0),
        &totals,
        beta,
    );
    let (kind, target, to_ft, utility) = candidates[index];
    if matches!(kind, ActionKind::Shoot(_)) && state.distance_to_hoop() > 30.0 && shot_trace() {
        eprintln!(
            "HEAVE_PICK t={:.2} dist={:.2} beta={beta:.3} gamma={gamma:.3} candidates={}",
            now,
            state.distance_to_hoop(),
            candidates.len(),
        );
        for (offset, (other, _, _, other_utility)) in candidates.iter().enumerate() {
            eprintln!(
                "  cand {other:?} total={:.4} p={:.4} epv={:.4} var={:.4} clock={:.4} foul={:.4} tactic={:.4}",
                totals[offset],
                probabilities[offset],
                other_utility.epv_base(),
                other_utility.variance,
                other_utility.clock_cost,
                other_utility.foul_cost,
                other_utility.tactical_bonus,
            );
        }
    }
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
    let duration = action_duration(world, &state, kind, target, to_ft);

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
        screen_defender,
        trace,
    }
}

/// 一个候选动作从决定到产生结果的时长。它也是「这个动作在剩下的进攻
/// 时间里做不做得完」的判据：进攻时钟不会等一个做不完的动作。
fn action_duration(
    world: &World,
    state: &DecisionState,
    kind: ActionKind,
    target: Option<usize>,
    to_ft: Vec2,
) -> f64 {
    match kind {
        ActionKind::Pass(_) => {
            let distance = (to_ft - state.actor_pos).length();
            (distance / 28.0).clamp(0.35, 1.4) as f64
        }
        ActionKind::Drive => {
            // 突破时长按「跑到落点」估算，而不是按到筐距离。落点在篮下
            // 四尺左右，从外线起步时到筐距离有二十多英尺；用到筐距离
            // 估时间会把动作压缩到跑不完，突破在半路就结束，收球步
            // 只能在中距离出手，近筐区域的出手分布整体塌到 near 区。
            let travel = (to_ft - state.actor_pos).length();
            let speed = world.players[state.actor]
                .effective_speed(&world.params.policy)
                .max(1.0);
            let accel = world.players[state.actor]
                .effective_acceleration(&world.params.policy)
                .max(1.0);
            let ramp = world
                .params
                .policy
                .g("advance_ramp_distance_ft")
                .max(f32::EPSILON);
            let startup = (speed / accel) * (-travel / ramp).exp();
            ((travel / speed + startup) * 1.3).clamp(0.8, 4.0) as f64
        }
        ActionKind::Screen(_) => {
            let screener = target.expect("screen requires a screener");
            let defender = state
                .primary_defender
                .expect("screen requires its primary defender")
                .index;
            let handler_pos = world.players[state.actor].pos;
            let defender_pos = world.players[defender].pos;
            let contact_point = defender_pos
                + (handler_pos - defender_pos).normalize_or_zero()
                    * world.rules.min_player_separation_ft;
            let distance = (contact_point - world.players[screener].pos).length();
            let speed = world.players[screener]
                .effective_speed(&world.params.policy)
                .max(1.0);
            (distance / speed * 1.25).max(0.6) as f64
        }
        ActionKind::Finish => 0.9,
        ActionKind::Cut(_) => {
            let cutter = target.expect("cut requires a cutter");
            let distance = (state.hoop - world.players[cutter].pos).length();
            (distance / MAX_SPEED_FT_PER_SEC * 1.8).clamp(1.5, 3.2) as f64
        }
        ActionKind::PostUp => 1.6,
        ActionKind::Shoot(_) => 0.55,
        ActionKind::Dribble(_) => 1.4,
        ActionKind::Relocate => 1.4,
        // 假动作与护球都是短技术动作，时长由规则档案的战术下限决定。
        ActionKind::Fake(_) => 0.45,
        ActionKind::Protect(_) => 0.35,
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
        .map(|elapsed| elapsed <= world.params.policy.g("screen_read_window_seconds") as f64)
        .unwrap_or(false);

    let mut candidates: Vec<Candidate> = [
        shot_candidate(world, state, gamma),
        drive_candidate(world, state),
        finish_candidate(world, state, gamma),
        pass_candidate(world, state, gamma),
        cut_candidate(world, state),
        post_candidate(world, state),
        screen_candidate(world, state),
        fake_candidate(world, state),
        protect_candidate(world, state),
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
                    ActionKind::Shoot(_) | ActionKind::Pass(_) | ActionKind::Dribble(_)
                ),
                DefenderRoute::Over => matches!(
                    kind,
                    ActionKind::Drive | ActionKind::Finish | ActionKind::Pass(_)
                ),
                DefenderRoute::Switch | DefenderRoute::Trapped => true,
            });
            assert!(!candidates.is_empty(), "掩护路线的应答集不能为空");
            if read == DefenderRoute::Over {
                let bonus = world.params.policy.w("screen_over_drive_bonus");
                for candidate in &mut candidates {
                    if matches!(candidate.0, ActionKind::Drive | ActionKind::Finish) {
                        candidate.3.tactical_bonus += bonus;
                    }
                }
            }
            if read == DefenderRoute::Under {
                let bonus = world.params.policy.w("screen_under_shoot_bonus");
                for candidate in &mut candidates {
                    if matches!(candidate.0, ActionKind::Shoot(_)) {
                        candidate.3.tactical_bonus += bonus;
                    }
                }
            }
        }
    }

    // 进攻时钟不会等一个做不完的动作：剩余时间装不下的候选不成立。若
    // 全部装不下，只留最短的那个——那一次违例已经是必然结果。
    let clock = state.shot_clock;
    let durations: Vec<f64> = candidates
        .iter()
        .map(|(kind, target, to_ft, _)| action_duration(world, state, *kind, *target, *to_ft))
        .collect();
    if durations.iter().any(|duration| *duration > clock) {
        let shortest = durations.iter().copied().fold(f64::INFINITY, f64::min);
        let mut kept = Vec::new();
        for (candidate, duration) in candidates.drain(..).zip(durations) {
            if matches!(candidate.0, ActionKind::Shoot(_))
                || duration <= clock
                || duration == shortest
            {
                kept.push(candidate);
            }
        }
        candidates = kept;
    }

    // 选板窗口内的持球人偏好与候选抑制。
    crate::tactics::apply_play_effects(world, &mut candidates);
    if std::env::var_os("NBA_TRACE_EPV").is_some() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static SEEN: AtomicU32 = AtomicU32::new(0);
        let seen = SEEN.fetch_add(1, Ordering::Relaxed);
        if seen < 60 {
            let listing: Vec<String> = candidates
                .iter()
                .map(|(kind, _, _, utility)| {
                    format!(
                        "{}={:.2}[e{:.2} v{:.2} c{:.2} f{:.2} t{:.2}]",
                        kind.family(),
                        utility.total(gamma),
                        utility.epv_base(),
                        gamma * utility.variance,
                        -utility.clock_cost,
                        -utility.foul_cost,
                        utility.tactical_bonus
                    )
                })
                .collect();
            eprintln!(
                "EPV dist={:.1} clock={:.1} gamma={:.3} {:?}",
                state.distance_to_hoop(),
                state.shot_clock,
                gamma,
                listing
            );
            if seen < 6 {
                for (kind, _, _, utility) in &candidates {
                    eprintln!("  {} {:?}", kind.family(), utility.epv);
                }
            }
        }
    }
    if std::env::var_os("NBA_TRACE_CAND").is_some() {
        eprintln!(
            "CANDCALL last_screen={:?} window={} now={:.3}",
            world
                .possession
                .last_screen
                .map(|read| (read.route, read.at)),
            world.params.policy.g("screen_read_window_seconds"),
            world.now()
        );
    }
    if std::env::var_os("NBA_TRACE_CAND").is_some()
        && world
            .possession
            .last_screen
            .map(|read| world.now() - read.at <= 2.2)
            .unwrap_or(false)
    {
        let listing: Vec<String> = candidates
            .iter()
            .map(|(kind, _, _, utility)| format!("{}={:.3}", kind.family(), utility.total(gamma)))
            .collect();
        eprintln!(
            "CAND route={:?} three={:.2} dist={:.1} clock={:.1} beta={:.2} {:?}",
            route,
            world.players[state.actor].data.attributes.shooting_three,
            state.distance_to_hoop(),
            state.shot_clock,
            decision_temperature(world, state.actor),
            listing
        );
    }
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
///
/// 代价只在「这次出手不如最好的替代动作」时出现：出手比替代动作好就不
/// 付代价（好机会没有代价），不如替代动作时才为「放弃了更好的机会」付钱。
/// 统一的固定惩罚会让传球永远压过出手，回合最后以进攻时钟违例结束。
fn early_shot_cost(
    world: &World,
    state: &DecisionState,
    gamma: f32,
    shot_value: f32,
    alternative_value: f32,
) -> f32 {
    let weights = &world.params.policy;
    let gain = weights.w("slack_variance_gain");
    let floor = weights.w("slack_variance_floor");
    let factor = (f32::from(1u8) - gain * gamma).max(floor);
    let worse_than = (alternative_value - shot_value).max(0.0);
    let ratio = worse_than / alternative_value.max(0.1);
    weights.b("shoot_early_slack") * clock_slack(world, state) * factor * ratio
}

/// 时钟成本：非终结动作要为自己烧掉的进攻时钟付钱。
///
/// 每一秒进攻时钟都对应一份回合价值：把回合价值除以一个回合的时长就是
/// 每秒的价格。烧掉的时间越多，代价越高；进攻时钟见底时可选的回合更少，
/// 每秒的价格急剧上升，于是「剩几秒必须出手」是成本曲线在端点自然压过
/// 其它候选的结果，而不是写死的规则。
fn clock_cost(world: &World, shot_clock: f64) -> f32 {
    let weights = &world.params.policy;
    let full = f64::from(world.rules.league.shot_clock_seconds);
    let remaining = (shot_clock / full).clamp(0.0, 1.0);
    let possession_seconds = weights.w("seconds_per_possession").max(1.0) as f64;
    let per_second = weights.w("possession_value") as f64 / possession_seconds;
    // 一个决策步消耗的时间：决策间隔。
    let step = world.rules.redecide_interval_seconds as f64;
    // 越接近终场，可选的回合越少，每秒的价格越贵。
    let urgency = 1.0 + 6.0 * (1.0 - remaining).powi(4);
    (weights.w("clock_cost_scale") as f64 * per_second * step * urgency) as f32
}

/// 一次出手的质量：唯一 ShotModel 给出的命中率、分值与区域。
struct ShotQuality {
    p_make: f32,
    points: f32,
    is_three: bool,
}

fn shot_quality(
    world: &World,
    pos: Vec2,
    shooter: usize,
    kind: ShotKind,
    phase: crate::state::BallPhase,
) -> ShotQuality {
    let team = world.players[shooter].team;
    let zone = shooter_zone(world, pos, team);
    let is_three = zone == "three";
    let model = crate::shot_quality::evaluate(world, shooter, kind, phase, pos);
    let points = if is_three { 3.0 } else { 2.0 };
    ShotQuality {
        p_make: model.p_make,
        points,
        is_three,
    }
}

/// 接球人在自己的位置上的期望得分与继续进攻价值。
#[cfg(test)]
fn catch_value(world: &World, pos: Vec2, shooter: usize, contest: f32) -> f32 {
    crate::value::catch_value(world, pos, shooter, contest)
}

fn shot_origin_legal(world: &World, actor: usize) -> bool {
    let team = world.players[actor].team;
    let pos = world.players[actor].pos;
    let hoop = world.hoop_for(team);
    let margin = world.rules.ball_radius_ft;
    let court = &world.rules.court;
    if pos.x < margin || pos.x > court.width_ft - margin {
        return false;
    }
    if pos.y < margin || pos.y > court.height_ft - margin {
        return false;
    }
    // 篮筐后方到篮板之间的位置没有出手角度；从那里投出的球会先撞板。
    if world.attacking_right(team) {
        pos.x <= hoop.x + margin
    } else {
        pos.x >= hoop.x - margin
    }
}

/// 末段抢投三分的处境加成。
///
/// 是否必须投三分由比分、剩余时间与暂停情况决定，不由出手距离决定：
/// - 落后且剩余时间少于 `endgame_three_required_seconds`；
/// - 落后分差需要三分才能追上（`endgame_hold_margin_max` 以内）；
/// - 本方已经没有暂停可用（不能停下来重新布置）。
///
/// 满足这些条件时，任何距离的三分都获得加成，越远加成越小但不为零；
/// 这让超远抛投成为处境驱动的选择，而不是被硬性距离上限删掉。
/// 当前状态下最好的非出手后续价值：传球、突破、保留球权里的最高值。
///
/// 这条值用来给「早出手的机会成本」一个参照：出手比最好的替代动作好
/// 多少，就少付多少机会成本。它只读价值模型，不读候选效用，因此不会
/// 与候选集形成循环依赖。
fn best_alternative_value(world: &World, state: &DecisionState) -> f32 {
    let mut best = crate::value::dribble_value(world, state.shot_clock);
    for teammate in &state.teammates {
        let value = crate::value::pass_value(world, state.actor, teammate.index);
        if value > best {
            best = value;
        }
    }
    let lane_norm = (state.lane_clearance / 8.0).clamp(0.0, 1.0);
    let help_term = state
        .help
        .map(|help| (1.0 / help.arrival_seconds.max(0.1)).min(2.0))
        .unwrap_or(0.0);
    let p_beat = blow_by_probability(world, state.skill_edge, lane_norm, help_term);
    let drive = crate::value::drive_value(world, state, p_beat);
    if drive > best {
        best = drive;
    }
    best
}

fn shot_candidate(world: &World, state: &DecisionState, gamma: f32) -> Option<Candidate> {
    let params = &world.params;
    let weights = &params.policy;
    let actor = state.actor;
    let trace = std::env::var_os("NBA_TRACE_SHOT").is_some()
        && world
            .possession
            .last_screen
            .map(|read| world.now() - read.at <= 2.2)
            .unwrap_or(false)
        && state.primary_defender.and_then(|d| d.route)
            == Some(crate::action::DefenderRoute::Under)
        && world.players[actor].data.attributes.shooting_three >= 0.7;
    if !shot_origin_legal(world, actor) {
        if trace {
            eprintln!("SHOTDROP origin_illegal pos={:?}", world.players[actor].pos);
        }
        return None;
    }
    // 出手必须发生在一个能出手的位置上：突破或收球被挡在远离篮筐的地方
    // 时，那个位置不是出手点——球手要么继续走，要么把球交出去。
    let (shoot_kind, shot_zone) = match shot_choice(world, state) {
        Some(choice) => choice,
        None => {
            if trace {
                eprintln!("SHOTDROP no_zone dist={:.2}", state.distance_to_hoop());
            }
            return None;
        }
    };
    let phase = world.possession.ball_phase;
    let remaining = crate::endgame::remaining_seconds(world);
    let required_margin = if remaining <= 8.0 { 3.0 } else { 9.0 };
    let no_timeouts = world.timeouts[World::team_index(world.players[actor].team)].remaining == 0;
    let three_needed = remaining <= 120.0
        && state.margin < 0
        && (-(state.margin as f32) <= required_margin || no_timeouts);
    let hold_window = weights.w("endgame_hold_seconds") as f64;
    let hold_clock = weights.w("endgame_hold_shot_clock") as f64;
    let just_received = world
        .possession
        .last_pass_at
        .map(|time| world.now() - time < 2.5)
        .unwrap_or(false);
    if state.margin > 0 && remaining <= hold_window && state.shot_clock > hold_clock {
        return None;
    }
    let quality = shot_quality(world, state.actor_pos, actor, shoot_kind, phase);

    if (quality.is_three || state.distance_to_hoop() > 30.0) && shot_trace() {
        let model = crate::shot_quality::evaluate(world, actor, shoot_kind, phase, state.actor_pos);
        eprintln!(
            "SHOT dist={:.2} three={} kind={:?} phase={:?} p={:.4} sigma={:.4} aper={:.5} speed={:.2} contest={:.3} block={:.3}",
            state.distance_to_hoop(),
            quality.is_three,
            shoot_kind,
            phase,
            model.p_make,
            model.sigma_ft,
            model.effective_aperture_ft,
            world.players[actor].vel.length(),
            model.contest,
            model.block_risk,
        );
    }
    let points = quality.points;
    let p_make = quality.p_make;
    let net = crate::value::shot_value_for(world, actor, state.actor_pos, p_make, points);

    let separation_bonus = world
        .last_separation
        .filter(|(separation_actor, _, time)| {
            *separation_actor == actor && world.now() - *time < 3.0
        })
        .map(|(_, separation, _)| (separation / 4.0).clamp(0.0, 1.5))
        .unwrap_or(0.0);
    let mut epv = EpvTerms {
        ability: weights.w("shoot_expected_points") * net,
        ..Default::default()
    };
    let mid_bonus = if shot_zone == "mid" {
        weights.b("shoot_mid_bonus")
    } else {
        0.0
    };
    let buzzer_urgency = if quality.is_three {
        if three_needed {
            (6.0 - state.shot_clock as f32).max(0.0) * 8.0
        } else {
            -(6.0 - state.shot_clock as f32).max(0.0) * 3.0
        }
    } else {
        (6.0 - state.shot_clock as f32).max(0.0) * 14.0
    };
    let preference = world.players[actor].data.tendencies.three_point_preference;
    let three_tendency = if quality.is_three {
        if preference <= 0.10 {
            -50.0
        } else {
            (preference - 0.5) * weights.b("shoot_tendency")
        }
    } else {
        0.0
    };
    let endgame_three = if three_needed && quality.is_three {
        weights.b("endgame_three_pressure")
    } else {
        0.0
    };
    let three_context = if quality.is_three {
        if endgame_three > 0.0 {
            0.0
        } else {
            weights.b("shoot_three_preference")
        }
    } else {
        0.0
    };
    let is_putback = putback_available(world, actor);
    epv.context = weights.b("shoot_catch_bonus") * state.openness() * f32::from(just_received)
        + weights.b("shoot_putback_bonus") * f32::from(is_putback)
        + weights.b("shoot_separation_bonus") * separation_bonus
        + weights.b("momentum_shot_bonus") * state.momentum
        + endgame_three
        + mid_bonus
        + buzzer_urgency
        + three_context
        - weights.b("shoot_off_dribble_penalty") * f32::from(!just_received && !is_putback);
    epv.tendency = weights.b("shoot_tendency")
        * (world.players[actor].data.tendencies.shoot_frequency - 0.5)
        * weights.w("shoot_tendency_gain")
        + three_tendency;

    let utility = Utility {
        epv,
        variance: variance_of(p_make, points) * weights.w("variance_shot_weight"),
        // 接到球不等于不用为「提前终结回合」付代价：接球就投的时机加成
        // 在 EPV 的语境项里，机会成本在时钟项里，两者不互相抵消。
        clock_cost: early_shot_cost(
            world,
            state,
            gamma,
            net,
            if just_received {
                best_alternative_value(world, state)
            } else {
                crate::value::dribble_value(world, state.shot_clock)
            },
        ) - weights.b("shoot_pressure") * state.shot_clock_pressure(),
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
    let screen_over = state.primary_defender.and_then(|defender| defender.route)
        == Some(crate::action::DefenderRoute::Over)
        && world
            .possession
            .last_screen
            .map(|read| {
                world.now() - read.at
                    <= f64::from(world.params.policy.g("screen_read_window_seconds"))
            })
            .unwrap_or(false);
    let max_zone = if screen_over {
        f32::INFINITY
    } else {
        weights.g("drive_max_zone_ft")
    };
    let min_zone = if screen_over {
        0.0
    } else {
        weights.g("drive_min_zone_ft")
    };
    if distance >= max_zone || distance <= min_zone {
        // 位置决定这个动作在物理上是否可用：距离外的突破不成立。
        return None;
    }
    let lane_norm = (state.lane_clearance / 8.0).clamp(0.0, 1.0);
    let help_term = state
        .help
        .map(|help| (1.0 / help.arrival_seconds.max(0.1)).min(2.0))
        .unwrap_or(0.0);
    let p_beat = blow_by_probability(world, state.skill_edge, lane_norm, help_term);
    let possession_value = weights.w("possession_value");
    // 突破价值按「推进成功后自己终结或急停」一条链计算；传球是决策点上的
    // 另一个候选，不在突破价值里重复计入。
    let net = crate::value::drive_value(world, state, p_beat);
    let tightness = state
        .primary_defender
        .map(|defender| (1.0 - defender.distance_ft / 6.0).clamp(0.0, 1.0))
        .unwrap_or(0.0);
    let mut epv = EpvTerms {
        ability: weights.b("drive_base") + net,
        contest: weights.b("drive_tight") * tightness,
        ..Default::default()
    };
    epv.tendency = weights.b("drive_tendency")
        * (world.players[state.actor].data.tendencies.drive_frequency - 0.5);
    // 掩护读完（防守人绕上沿）之后就是突破窗口：这一段按策略加成。
    if screen_over {
        epv.context += weights.b("drive_screen_bonus");
    }
    epv.turnover = -weights.w("epv_turnover_cost")
        * possession_value
        * (f32::from(1u8) - p_beat)
        * weights.w("drive_turnover_rate");
    // 突破的落点按终结能力决定：终结能力强的球员在篮下合球完成上篮，
    // 其余球员突到抛投区域完成近筐抛投。落点必须留在近筐区（4 ft 以外），
    // 否则每一条突破都变成篮下出手，出手的区域分布整段压在篮下。
    let finishing = world.players[state.actor].data.attributes.finishing;
    // 终结能力决定突破停在哪里：强的到篮下收球，弱的在抛投区提前收球。
    // 阈值与四个几何参数都在参数表里，不写死系数。
    let shortfall = (1.0 - finishing).clamp(0.0, 1.0);
    let drive_target_distance = if finishing > weights.g("drive_rim_finishing_threshold") {
        weights.g("drive_rim_finish_ft") + weights.g("drive_rim_spread_ft") * shortfall
    } else {
        weights.g("drive_finish_distance_ft") + weights.g("drive_finish_spread_ft") * shortfall
    }
    .max(weights.g("finish_min_distance_ft"));
    let to_ft =
        state.hoop + (state.actor_pos - state.hoop).normalize_or_zero() * drive_target_distance;
    Some((
        ActionKind::Drive,
        None,
        to_ft,
        Utility {
            epv,
            variance: variance_of(p_beat, net.max(0.1)) * weights.w("variance_choice_weight"),
            clock_cost: clock_cost(world, state.shot_clock) * weights.w("drive_clock_weight"),
            foul_cost: weights.w("offensive_foul_rate_drive") * crate::value::retained_value(world),
            tactical_bonus: f32::default(),
        },
    ))
}

/// 收球步：把运球收成上篮步。到了终结距离之后不用再突，这两步就是出手。
fn finish_candidate(world: &World, state: &DecisionState, gamma: f32) -> Option<Candidate> {
    let weights = &world.params.policy;
    let finish_limit = weights
        .g("finish_zone_ft")
        .min(weights.g("gather_travel_allowance_ft") - 1.0);
    if state.distance_to_hoop() > finish_limit {
        return None;
    }
    let phase = world.possession.ball_phase;
    let phase_allows_gather = matches!(
        phase,
        crate::state::BallPhase::Drive
            | crate::state::BallPhase::Post
            | crate::state::BallPhase::Cut
            | crate::state::BallPhase::Gather
    );
    if !phase_allows_gather
        && state.primary_defender.and_then(|defender| defender.route)
            != Some(crate::action::DefenderRoute::Over)
    {
        return None;
    }
    if world.possession.dribble_state != crate::state::DribbleState::Dribbling
        && state.primary_defender.and_then(|defender| defender.route)
            != Some(crate::action::DefenderRoute::Over)
    {
        return None;
    }
    let actor = state.actor;
    if !shot_origin_legal(world, actor) {
        return None;
    }
    let remaining = crate::endgame::remaining_seconds(world);
    let hold_window = weights.w("endgame_hold_seconds") as f64;
    let hold_clock = weights.w("endgame_hold_shot_clock") as f64;
    if state.margin > 0 && remaining <= hold_window && state.shot_clock > hold_clock {
        return None;
    }
    // 收球后的出手类型按当前阶段与实际距离决定：低位是勾手，靠近篮筐
    // 是上篮，外线是跳投。用通用的 Layup 估值会把远距离收球也当成
    // 篮下上篮，进攻于是全都冲到篮下。
    let finish_kind = match phase {
        crate::state::BallPhase::Post => crate::action::ShotKind::Hook,
        crate::state::BallPhase::Cut => crate::action::ShotKind::CuttingLayup,
        _ => {
            if state.distance_to_hoop() <= 7.8 {
                crate::action::ShotKind::DrivingLayup
            } else {
                crate::action::ShotKind::Floater
            }
        }
    };
    let quality = shot_quality(world, state.actor_pos, actor, finish_kind, phase);
    let p_make = quality.p_make;
    let points = quality.points;
    let net = crate::value::shot_value_for(world, actor, state.actor_pos, p_make, points);
    let buzzer_urgency = (6.0 - state.shot_clock as f32).max(0.0) * 18.0;
    let epv = EpvTerms {
        ability: net,
        context: weights.b("finish_priority") + buzzer_urgency,
        ..Default::default()
    };
    let finish_dist = match finish_kind {
        crate::action::ShotKind::Floater => 7.2,
        crate::action::ShotKind::Hook | crate::action::ShotKind::Turnaround => 6.5,
        _ => weights.g("finish_min_distance_ft"),
    };
    let to_ft = state.hoop + (state.actor_pos - state.hoop).normalize_or_zero() * finish_dist;
    Some((
        ActionKind::Finish,
        None,
        to_ft,
        Utility {
            epv,
            variance: variance_of(p_make, points) * weights.w("variance_shot_weight"),
            // 收球也是一次出手：提前终结同样放弃了「再找一次更好的机会」。
            clock_cost: early_shot_cost(
                world,
                state,
                gamma,
                net,
                best_alternative_value(world, state),
            ) - weights.b("shoot_pressure") * state.shot_clock_pressure(),
            foul_cost: f32::default(),
            tactical_bonus: f32::default(),
        },
    ))
}

fn pass_candidate(world: &World, state: &DecisionState, gamma: f32) -> Option<Candidate> {
    let late_clock_penalty = (5.0 - state.shot_clock as f32).max(0.0) * 8.0;
    let weights = &world.params.policy;
    // 球商决定风险容忍度：球商越高，越不容忍把球送进有人的地方。
    // 曲线走映射层（`effective_risk_tolerance` = base − iq × gain），
    // 球商只在这一处进入决策，管线其余部分不读它。
    //
    // 没有这一项时球商只抬高逆温度，高球商会更常选中「效用最高」的候选，
    // 而高风险深传恰好常常是效用最高的候选——球商于是与失误率同向，
    // 与属性的语义相反。实测把球商从 0.10 扰动到 0.95，失误率上升 21.8%。
    let risk_tolerance = nba_domain::capability::effective_risk_tolerance(
        &world.rules,
        &world.players[state.actor].data.attributes,
    );
    let mut best: Option<(usize, Vec2, Utility, f32)> = None;
    for teammate in &state.teammates {
        // 已建立前场控制后不把球传回后场：那是回场违例。
        if world.possession.frontcourt_established {
            let mid = world.rules.court.width_ft * 0.5;
            let backcourt = if state.attacking_right {
                teammate.pos.x < mid
            } else {
                teammate.pos.x > mid
            };
            if backcourt {
                continue;
            }
        }
        let advancement = if state.attacking_right {
            teammate.pos.x - state.actor_pos.x
        } else {
            state.actor_pos.x - teammate.pos.x
        };
        let risk = (1.0 - (teammate.lane_clearance / 6.0).clamp(0.0, 1.0)).max(0.0);
        let teammate_three =
            shooter_zone(world, teammate.pos, world.players[teammate.index].team) == "three";
        let p_complete = crate::value::pass_completion_probability(
            world,
            state.actor,
            teammate.index,
            crate::action::PassKind::Chest,
        );
        // 传球价值直接读取几何完成概率与接球后的完整后果；失败按失误
        // 成本进入同一个净值，不再由另一个独立线性权重估算概率。
        let net = crate::value::pass_value(world, state.actor, teammate.index);
        // 刚接到的球立刻原路回传属于无意义的乒乓传球，在效用中扣减，
        // 促使持球人转向防守弱侧或寻找空位队友。
        let ping_pong = world.possession.last_pass_from == Some(teammate.index);
        let epv = EpvTerms {
            ability: weights.b("pass_base") + net - if ping_pong { 1.2 } else { 0.0 },
            lane: weights.b("pass_advancement") * (advancement / 20.0).clamp(-1.0, 1.0)
                - weights.b("pass_risk") * risk * (1.0 - risk_tolerance),
            context: weights.b("pass_perimeter") * f32::from(teammate_three)
                + weights.b("pass_open") * teammate.openness
                - late_clock_penalty,
            ..Default::default()
        };
        let utility = Utility {
            epv,
            variance: variance_of(p_complete, crate::value::retained_value(world))
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
        // 传球技术跟着几何走：贴着手递手，抢断线路被占就改高吊或击地，
        // 长距离推进用单手推传，贴身且向前推进则从头顶过。
        let view = state.teammates.iter().find(|view| view.index == target);
        let gap = view
            .map(|view| (view.pos - state.actor_pos).length())
            .unwrap_or(f32::INFINITY);
        let lane = view.map(|view| view.lane_clearance).unwrap_or(f32::INFINITY);
        let advancement = view
            .map(|view| {
                if state.attacking_right {
                    view.pos.x - state.actor_pos.x
                } else {
                    state.actor_pos.x - view.pos.x
                }
            })
            .unwrap_or(f32::default());
        let defender_gap = world
            .players
            .iter()
            .filter(|player| player.on_court && player.team != world.players[state.actor].team)
            .map(|player| (player.pos - state.actor_pos).length())
            .fold(f32::INFINITY, f32::min);
        let kind = if gap <= 5.5 && defender_gap <= 3.0 {
            PassKind::Handoff
        } else if lane < 2.0 {
            PassKind::Bounce
        } else if defender_gap <= 3.0 && advancement > 8.0 {
            PassKind::Overhead
        } else if advancement > 20.0 {
            PassKind::OneHand
        } else if defender_gap <= 1.8 && advancement.abs() < 3.0 {
            PassKind::BehindBack
        } else {
            PassKind::Chest
        };
        (ActionKind::Pass(kind), Some(target), pos, utility)
    })
}

fn screen_candidate(world: &World, state: &DecisionState) -> Option<Candidate> {
    let weights = &world.params.policy;
    // 有球掩护是阵地进攻的起手，发生在三分线一带。后场推进途中也能做
    // 掩护，但那种掩护不为任何人制造机会，读球也就无从谈起。
    let screen_distance = state.distance_to_hoop();
    if screen_distance < weights.g("screen_min_distance_ft")
        || screen_distance > weights.g("screen_max_distance_ft")
    {
        return None;
    }
    // 正在执行自己动作的队友不能被拉来掩护：同一时刻只有一个身体动作。
    let screener = state
        .teammates
        .iter()
        .filter(|view| world.players[view.index].action.is_none())
        .max_by(|a, b| {
            world.players[a.index]
                .data
                .attributes
                .strength
                .partial_cmp(&world.players[b.index].data.attributes.strength)
                .expect("finite strength")
        })
        .map(|view| view.index)?;
    // 掩护本身不得分，它的价值是「制造出机会的概率 × 机会本身的价值」：
    // 掩护成功时持球人获得一个身位的空位，可以直接攻击篮下。
    let actor = state.actor;
    let hoop = state.hoop;
    let finish_spot = hoop
        + (state.actor_pos - hoop).normalize_or_zero()
            * world.params.policy.g("finish_min_distance_ft");
    let finish_value = crate::value::shot_value(
        world,
        actor,
        finish_spot,
        crate::action::ShotKind::DrivingLayup,
        crate::state::BallPhase::Drive,
    );
    // 掩护成功的机会：错位越大越容易形成空位。
    let p_success = (weights.w("screen_success_base")
        + weights.w("screen_mismatch") * state.mismatch.abs())
    .clamp(0.0, 0.9);
    // 掩护之后球还在自己手里，现在能做的选择一个都没丢，所以它的下限是
    // 「现在就出手」的价值，而不是零。成功的那部分只买一样东西：把这次
    // 出手换成一个更干净的版本。换给谁不固定——持球人可以自己终结，也
    // 可以把球交给顺下的大个子在篮下接球。
    let now_value = crate::value::shot_value(
        world,
        actor,
        state.actor_pos,
        crate::action::ShotKind::CatchShoot,
        crate::state::BallPhase::Catch,
    );
    let roll_spot = hoop
        + (world.players[screener].pos - hoop).normalize_or_zero()
            * world.params.policy.g("finish_min_distance_ft");
    let roll_value = crate::value::catch_value(world, roll_spot, screener, 0.0);
    let open_value = finish_value.max(roll_value);
    let premium = (open_value - now_value).max(0.0);
    let mut epv = EpvTerms {
        ability: now_value + p_success * premium * weights.w("screen_open_value_share"),
        ..Default::default()
    };
    // 掩护与空切创造机会的价值要和「一次出手」可比：出手值是净得分，
    // 掩护创造的机会也按净得分计。
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
            foul_cost: weights.w("offensive_foul_rate_screen")
                * crate::value::retained_value(world),
            tactical_bonus: f32::default(),
        },
    ))
}

fn cut_candidate(world: &World, state: &DecisionState) -> Option<Candidate> {
    let weights = &world.params.policy;
    let lane_norm = (state.lane_clearance / 8.0).clamp(0.0, 1.0);
    // 正在执行自己动作的队友不能被派去空切。
    let cutter = state
        .teammates
        .iter()
        .filter(|view| world.players[view.index].action.is_none())
        .max_by(|a, b| {
            b.defender_distance_ft
                .partial_cmp(&a.defender_distance_ft)
                .expect("finite distances")
        })
        .map(|view| view.index)?;
    // 空切只有在两件事同时成立时才存在：切入人能在球到达前跑到篮下，
    // 并且他的防守人跟不回来。第一条是物理可达性，第二条是「空位」本身：
    // 切入人被跟防时，高吊球就是一次普通的、很难完成的传球。
    let (cut_spot, flight_seconds) = crate::resolve::pass::cut_plan(world, cutter);
    let run_distance = (cut_spot - world.players[cutter].pos).length();
    let run_speed = world.players[cutter]
        .effective_speed(&world.params.policy)
        .max(1.0);
    let run_seconds = f64::from(run_distance / run_speed);
    if run_seconds > flight_seconds * 1.05 {
        return None;
    }
    // 切入人的防守人：离切入人最近的那名防守人。防守人贴得越近，空位越
    // 假；「有人看着」是空切不成立的常态。
    let mut defender_distance = f32::INFINITY;
    for player in world.players.iter().filter(|p| p.on_court) {
        if player.team == world.players[cutter].team {
            continue;
        }
        let distance = (player.pos - world.players[cutter].pos).length();
        if distance < defender_distance {
            defender_distance = distance;
        }
    }
    // 防守人贴得很近时不存在空位：切入人接球就是被贴身干扰的出手。
    // 距离门槛按切入人到篮下的实际距离折算：切入人离筐越近，防守人同样
    // 可以在篮下等他，所以空位要求随切入点一起收紧。
    if defender_distance < weights.g("cut_open_defender_ft") {
        return None;
    }
    // 空切的价值是「高吊球传到位并且切入人真的能出手」的两段结果：
    // 传到位概率 × (切入人干净接球的概率 × 篮下出手价值 + 被跟防时的
    // 保留球权价值)。只算「传到位」会把空切定价成必然的空位，于是每次
    // 进攻都变成空切。
    let cut_spot = crate::resolve::pass::cut_target(world, cutter);
    let p_complete = crate::value::pass_completion_probability(
        world,
        state.actor,
        cutter,
        crate::action::PassKind::Lob,
    );
    // 高吊球比平直传球更容易被断：它飞得高、落点固定，防守人站在线上就
    // 能碰到。完成概率必须反映这一点，否则空切会被定价成必然成功。
    // 完成概率直接读几何模型：它已经按高吊球的落点、飞行时间与防守人的
    // 移动算过，不再另乘一个经验系数。
    let contest = crate::resolve::contest_for(world, cutter, cut_spot);
    // 接球时的干扰要算上防守人在这段时间里的移动：他现在离得远，
    // 但球到时他可能已经站在切入人旁边。
    // 接球时的干扰：防守人现在离切入人越近，球到时干扰越大。
    let mut p_clean = (1.0 - contest).clamp(0.0, 1.0);
    {
        let near = world.rules.min_player_separation_ft + 1.5;
        p_clean = p_clean.min((defender_distance / near).clamp(0.0, 1.0));
    }
    for player in world.players.iter().filter(|p| p.on_court) {
        if player.team == world.players[cutter].team {
            continue;
        }
        // 防守人能不能在球到之前贴上切入人：距离除以他的速度就是到场时间。
        let gap_now = (player.pos - cut_spot).length();
        let speed = player.vel.length().max(1.0);
        let arrival = gap_now / speed;
        let _reach = world.rules.min_player_separation_ft + 1.0;
        if arrival < flight_seconds as f32 {
            // 防守人先到：切入人只能在贴身干扰下接球。
            let slack = (flight_seconds as f32 - arrival) / flight_seconds as f32;
            p_clean = p_clean.min(slack.clamp(0.0, 1.0));
        }
    }
    // 接球那一瞬间就是一次出手机会，接球价值按接球点的实际出手质量算。
    // 这里与传球候选调用同一个函数：两种选择都通向「某个人在某处接到球
    // 并且继续这次进攻」，用同一个口径才能互相比较。
    let finish = crate::value::catch_value(world, cut_spot, cutter, contest);
    // 接球不干净不等于丢掉球权：球还在切入人手里（或者被挡出来的球被
    // 同队捡回），进攻继续。继续持有球权的价值才是这一分支的下界。
    let blocked = crate::value::dribble_value(world, state.shot_clock);
    let continuation = p_clean * finish + (1.0 - p_clean) * blocked;
    let turnover = weights.w("epv_turnover_cost") * weights.w("possession_value");
    let net = p_complete * continuation - (1.0 - p_complete) * turnover;
    let epv = EpvTerms {
        ability: net,
        // 通道越开阔，空切越容易做出来。
        context: weights.w("cut_lane_scale") * lane_norm,
        ..Default::default()
    };
    Some((
        ActionKind::Cut(if lane_norm < 0.4 {
            crate::action::CutKind::Backdoor
        } else {
            crate::action::CutKind::Basket
        }),
        Some(cutter),
        state.hoop,
        Utility {
            epv,
            variance: variance_of(p_complete, crate::value::retained_value(world))
                * weights.w("variance_choice_weight"),
            clock_cost: clock_cost(world, state.shot_clock),
            foul_cost: f32::default(),
            tactical_bonus: f32::default(),
        },
    ))
}

fn post_candidate(world: &World, state: &DecisionState) -> Option<Candidate> {
    if world.possession.ball_phase == crate::state::BallPhase::Post || state.shot_clock < 3.0 {
        return None;
    }
    let weights = &world.params.policy;
    // 低位只有在真正的低位区域才存在：站得很远的人「打低位」要走很久，
    // 走不到位就变成在三分线外投一个勾手。这里只提供真正在低位范围内
    // 的选项。
    let distance = (state.actor_pos - state.hoop).length();
    if distance > weights.g("post_max_distance_ft") {
        return None;
    }
    let post_pos = state.hoop + (state.actor_pos - state.hoop).normalize_or_zero() * 7.0;
    // 低位候选用自己的出手类型与阶段估值，不借用通用两分估值。
    let net = crate::value::shot_value(
        world,
        state.actor,
        post_pos,
        crate::action::ShotKind::Hook,
        crate::state::BallPhase::Post,
    );
    let epv = EpvTerms {
        ability: weights.b("post_base") + net,
        ..Default::default()
    };
    Some((
        ActionKind::PostUp,
        None,
        post_pos,
        Utility {
            epv,
            variance: variance_of(0.5, net.max(0.1)) * weights.w("variance_choice_weight"),
            clock_cost: clock_cost(world, state.shot_clock) * weights.w("post_clock_weight"),
            foul_cost: weights.w("offensive_foul_rate_post") * crate::value::retained_value(world),
            tactical_bonus: f32::default(),
        },
    ))
}

/// 假动作候选：防守人贴得足够近时，骗一次比硬投或硬突更值。
///
/// 收益来自真实后果：被骗到的防守人在失衡窗口内干扰折减（投篮假动作）
/// 或抄截半径收窄（传球假动作）。没骗到时只剩时钟成本。
fn fake_candidate(world: &World, state: &DecisionState) -> Option<Candidate> {
    if world.possession.dribble_state != crate::state::DribbleState::Dribbling {
        return None;
    }
    // 时钟不够做完假动作再出手时，假动作只是白白消耗时间。
    if state.shot_clock < 3.5 {
        return None;
    }
    let defender = state.primary_defender?;
    let policy = &world.rules.resolve.technique;
    if defender.distance_ft > policy.fake_pressure_radius_ft {
        return None;
    }
    let read = policy.read_probability(
        world.players[defender.index].attribute("defense_perimeter"),
        world.players[state.actor].attribute("ball_handling"),
    );
    let bait = f32::from(1u8) - read;
    let quality = shot_quality(
        world,
        state.actor_pos,
        state.actor,
        crate::action::ShotKind::PullUp,
        crate::state::BallPhase::Dribble,
    );
    // 假动作的技术选择由处境决定：能投就骗投，传球线路被堵就骗传，
    // 其余情况用试探步换半步空间。
    // 干扰折减只改变出手的一部分价值：按出手价值的四分之一计入，
    // 否则假动作会被估成「一次没人防的出手」，在候选里压过所有真实动作。
    let fake_share = 0.25;
    let (kind, gain) = if state.shot_clock < 9.0 {
        (
            crate::action::FakeKind::Shot,
            quality.p_make * quality.points * policy.fake_bait_contest_relief * bait * fake_share,
        )
    } else if state.lane_clearance < 3.0 {
        // 传球假动作的价值是下一次传球：按打开宽度占抄截半径基准的比例折算。
        let opening = (policy.fake_pass_lane_ft
            / world
                .params
                .policy
                .g("pass_intercept_base_ft")
                .max(f32::EPSILON))
        .clamp(f32::default(), f32::from(1u8));
        (
            crate::action::FakeKind::Pass,
            quality.points * 0.25 * opening * bait,
        )
    } else {
        (
            crate::action::FakeKind::JabStep,
            quality.p_make * quality.points * fake_share * bait,
        )
    };
    Some((
        ActionKind::Fake(kind),
        Some(defender.index),
        state.actor_pos,
        Utility {
            epv: EpvTerms {
                // 假动作不冒丢球风险，球权价值与持球相同；骗到的空间是额外收益。
                ability: crate::value::dribble_value(world, state.shot_clock),
                context: gain,
                ..Default::default()
            },
            variance: f32::default(),
            clock_cost: clock_cost(world, state.shot_clock) * 0.5,
            foul_cost: f32::default(),
            tactical_bonus: f32::default(),
        },
    ))
}

/// 护球候选：贴身施压下丢球风险高时，护一下比硬着运更值。
///
/// 收益是被挡下的切球乘上回合价值；成本是这段时间占用的进攻时钟。
fn protect_candidate(world: &World, state: &DecisionState) -> Option<Candidate> {
    if world.possession.dribble_state != crate::state::DribbleState::Dribbling {
        return None;
    }
    if state.shot_clock < 2.0 {
        return None;
    }
    let defender = state.primary_defender?;
    let security = &world.rules.resolve.ball_security;
    if defender.distance_ft > security.poke_pressure_radius_ft {
        return None;
    }
    let policy = &world.rules.resolve.technique;
    // 每秒钟的切球危险：尝试速率乘上典型成功率，再乘护球窗口长度。
    let attempt_rate = f64::from(security.poke_attempt_rate_per_sec);
    let typical = f64::from((security.poke_success_floor + security.poke_success_ceiling) * 0.5);
    let hazard = (attempt_rate * typical * f64::from(policy.protection_seconds)) as f32;
    let avoided =
        hazard * policy.protection_poke_relief * world.params.policy.w("possession_value").max(0.0);
    // 护球技术选择：低位用背身，防守人在球与筐之间时用转身，其余面框。
    let kind = if state.distance_to_hoop() < 12.0 {
        crate::action::ProtectKind::Post
    } else if defender.between_ball_and_hoop {
        crate::action::ProtectKind::Spin
    } else {
        crate::action::ProtectKind::FaceUp
    };
    Some((
        ActionKind::Protect(kind),
        Some(defender.index),
        state.actor_pos,
        Utility {
            epv: EpvTerms {
                // 护球同样保住球权；被挡下的切球是额外收益。
                ability: crate::value::dribble_value(world, state.shot_clock),
                context: avoided,
                ..Default::default()
            },
            variance: f32::default(),
            clock_cost: clock_cost(world, state.shot_clock) * 0.5,
            foul_cost: f32::default(),
            tactical_bonus: f32::default(),
        },
    ))
}

fn hold_candidate(world: &World, state: &DecisionState) -> Option<Candidate> {
    // 已收球就不能再运球：二次运球是犯规，持球选项在物理上不存在。
    if world.possession.dribble_state != crate::state::DribbleState::Dribbling {
        return None;
    }
    // 时钟不足以完成一次运球调整时必须出手或处理球：时钟见底还运球会导致违例。
    if state.shot_clock < 2.5 {
        return None;
    }
    let weights = &world.params.policy;
    // 保留球权只计一次非负的保留价值；下一次决策重新估计，不能在同一
    // 条链上反复加上完整的回合价值。
    let epv = EpvTerms {
        ability: crate::value::dribble_value(world, state.shot_clock),
        context: weights.b("dribble_setup") * clock_slack(world, state)
            - weights.b("dribble_react_penalty")
                * f32::from(
                    state.primary_defender.and_then(|d| d.route) == Some(DefenderRoute::Under),
                ),
        ..Default::default()
    };
    // 技术细分由处境决定：贴得越紧越用短促的变向，空间大就用转身或推进。
    let gap = state
        .primary_defender
        .map(|defender| defender.distance_ft)
        .unwrap_or(f32::INFINITY);
    let move_kind = if gap <= 2.0 {
        nba_domain::action_window::DribbleMoveKind::Crossover
    } else if gap <= 3.2 {
        nba_domain::action_window::DribbleMoveKind::BetweenTheLegs
    } else if gap <= 4.5 {
        if state.shot_clock < 10.0 {
            nba_domain::action_window::DribbleMoveKind::Hesitation
        } else if state.lane_clearance < 3.0 {
            nba_domain::action_window::DribbleMoveKind::BehindTheBack
        } else {
            nba_domain::action_window::DribbleMoveKind::Retreat
        }
    } else if state.lane_clearance >= 3.0 {
        nba_domain::action_window::DribbleMoveKind::SpinMove
    } else {
        nba_domain::action_window::DribbleMoveKind::DirectDrive
    };
    Some((
        ActionKind::Dribble(move_kind),
        None,
        // 持球移动的落点是弧顶的持球位，不是篮筐：把「拿着球」指向篮筐会让
        // 每次运球都变成一次无意识的冲筐；停在原地又会让持球人停在还没
        // 走到的位置上（离筐三十多英尺处急停跳投）。
        crate::formation::top_of_key(world, world.players[state.actor].team),
        Utility {
            epv,
            variance: f32::default(),
            clock_cost: clock_cost(world, state.shot_clock),
            foul_cost: f32::default(),
            tactical_bonus: f32::default(),
        },
    ))
}

pub use crate::shot_quality::{putback_available, shot_choice};

pub use crate::formation::{move_toward_targets, spot_target, spot_target_for};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::RuntimeParams;
    use crate::state::Phase;
    use crate::world::{MatchSetup, World};
    use crate::TeamSide;

    #[test]
    fn catch_value_includes_the_next_action_not_only_the_immediate_shot() {
        let mut world = World::new(
            MatchSetup::builtin(nba_domain::GameRules::default()),
            RuntimeParams::embedded(),
            7,
        );
        for player in world.players.iter_mut() {
            player.on_court = false;
        }
        world.players[1].on_court = true;
        world.possession.team = TeamSide::Home;
        world.set_phase(Phase::Live);
        let pos = world.players[1].pos;
        let quality = shot_quality(
            &world,
            pos,
            1,
            crate::action::ShotKind::CatchShoot,
            crate::state::BallPhase::Catch,
        );
        let immediate = quality.points * quality.p_make;
        let valued = catch_value(&world, pos, 1, 0.0);
        assert!(valued > immediate);
    }
}
