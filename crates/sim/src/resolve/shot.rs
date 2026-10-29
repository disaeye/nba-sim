//! 出手结算：干扰、技巧、体能进 logit，命中率来自校准表。

use glam::Vec2;

use crate::action::{ActionIntent, ActionKind, ActionOutcome, Completion, ShotKind, StateDelta};
use crate::draw::{purpose, Stream};
use crate::resolve::{clamp01, shooter_zone};
use crate::world::{Phase, World};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotResult {
    Made,
    Missed { rim_contact: bool },
    Blocked { by: usize },
}

#[derive(Debug, Clone)]
pub struct PendingShot {
    pub shooter: usize,
    pub kind: ShotKind,
    pub zone: &'static str,
    pub distance_ft: f32,
    pub contest: f32,
    pub fouled: bool,
    pub and_one: bool,
    pub free_throws: u8,
    pub points: u8,
    pub is_three: bool,
    pub assisted_by: Option<usize>,
    pub shot_id: u64,
    pub free_throw: Option<crate::resolve::freethrow::FreeThrowMeta>,
    /// 球穿过筐口、触筐、触地或触手后才有值；出手瞬间保持未定。
    pub result: Option<ShotResult>,
}

pub fn resolve(world: &mut World, intent: &ActionIntent, kind: ShotKind) -> ActionOutcome {
    let shooter = intent.actor;
    let team = world.players[shooter].team;
    let hoop = world.hoop_for(team);
    let pos = world.players[shooter].pos;
    let zone = shooter_zone(world, pos, team);
    let is_three = zone == "three";
    let distance = (pos - hoop).length();
    let phase = world.possession.ball_phase;
    let model = crate::shot_quality::evaluate(world, shooter, kind, phase, pos);
    let contest = model.contest;
    let ball_phase = phase.as_str();
    // 投篮终结了持球状态：收球标志在出手时清掉。
    world.possession.dribble_state = crate::state::DribbleState::Dribbling;
    world.possession.gather_pos = None;
    world.possession.ball_phase = crate::state::BallPhase::Dribble;
    let p_make = model.p_make;

    let foul_rate = world.params.policy.w("shooting_foul_rate");
    let p_foul = clamp01(foul_rate * (0.6 + 0.9 * contest));
    // 封盖是球飞到手上才发生的事件，不在出手时抽结果。这里只保留封盖
    // 尝试带来的犯规风险；封盖本身由手部接触事实决定。
    let (block_candidate, _physical_block) = block_probability(world, shooter, pos, contest, is_three);
    let block_foul = match block_candidate {
        Some(candidate) => {
            world.draw(Stream::Foul, candidate, purpose::SHOT_BLOCK_FOUL, 0)
                < block_foul_probability(world, candidate)
        }
        None => false,
    };
    let fouled = block_foul
        || world.draw(Stream::Foul, shooter, purpose::SHOOTING_FOUL, 0) < p_foul;
    let and_one = fouled
        && world.draw(Stream::Foul, shooter, purpose::AND_ONE, 0)
            < world.params.policy.w("and_one_rate");
    let free_throws = if fouled {
        if is_three {
            3
        } else {
            2
        }
    } else {
        0
    };
    let points = if is_three { 3 } else { 2 };
    let shot_id = world.new_shot_id();
    // 瞄准误差是出手时唯一的随机结果：命中由球是否穿过筐口决定。
    let u1 = world
        .draw(Stream::Aim, shooter, purpose::SHOT_MAKE, 0)
        .max(1e-6);
    let u2 = world.draw(Stream::Aim, shooter, purpose::SHOT_MAKE, 1);
    let sigma = model.sigma_ft;
    let spread = sigma * (-2.0 * u1.ln()).sqrt();
    let angle = u2 * std::f32::consts::TAU;
    // 瞄准误差是球心在筐平面上的落点偏移：抛体与释放点仍由出手计划决定，
    // 因此模型预测的入射角就是实际飞行的入射角。
    let aim_offset = Vec2::new(angle.cos(), angle.sin()) * spread;
    // 封盖风险用这一手真实的落点轨迹来算：球从释放点飞向落点，防守人
    // 的手在这条轨迹附近就可能碰到球。执行层用同一条轨迹判定接触，
    // 因此模型预测的封盖率与实际封盖率描述同一件事。
    let block_risk = crate::shot_quality::block_risk_for_path(
        world,
        shooter,
        model.origin,
        hoop + aim_offset,
        &model.plan,
        model.release_z_ft,
    );
    let p_block = block_risk;

    let assisted_by = world
        .possession
        .assist_candidate
        .filter(|(passer, time)| *passer != shooter && world.now() - *time < 4.0)
        .map(|(passer, _)| passer);

    world.possession.had_shot = true;
    // 判据要和决策时刻一致：出手动作在决策之后才结算，因此这里按这次
    // 出手动作的起始时刻判断「是不是刚接到球」，而不是按结算时刻。
    let just_received = world
        .possession
        .last_pass_at
        .map(|time| intent.start_time - time < 1.2)
        .unwrap_or(false);
    // 判据按「这次出手实际属于哪种进攻」来判，而不是按出手类型的粗分类：
    // 只有真正的接球就投（收到球之后立刻出手）才需要前面有一次传球；
    // 收球步之后的篮下完成不需要。
    let catch_and_shoot_without_pass =
        matches!(kind, crate::action::ShotKind::CatchShoot) && !just_received;
    // 判据按「这次出手实际属于哪种进攻」来判，而不是按出手类型的粗分类：
    // 只有真正的接球就投（收到球之后立刻出手）才需要前面有一次传球。
    let recent = |at: Option<f64>| at.is_some_and(|time| world.now() - time < 6.0);
    let drive_finish = matches!(
        kind,
        crate::action::ShotKind::DrivingLayup
            | crate::action::ShotKind::DrivingDunk
            | crate::action::ShotKind::ReverseLayup
    ) && !recent(world.possession.last_drive_at);
    let cut_finish = matches!(
        kind,
        crate::action::ShotKind::CuttingLayup | crate::action::ShotKind::CuttingDunk
    ) && !recent(world.possession.last_cut_at);
    let post_finish = matches!(
        kind,
        crate::action::ShotKind::Hook | crate::action::ShotKind::Turnaround
    ) && !recent(world.possession.last_post_at);
    if catch_and_shoot_without_pass {
        world
            .violations
            .push("context: catch-and-shoot without a preceding pass".to_string());
    }
    if drive_finish {
        world
            .violations
            .push("context: drive finish without a drive action".to_string());
    }
    if cut_finish {
        world
            .violations
            .push("context: cut finish without a cut action".to_string());
    }
    if post_finish {
        world
            .violations
            .push("context: post finish without a post action".to_string());
    }
    if matches!(kind, crate::action::ShotKind::Putback) && !world.possession.had_offensive_rebound {
        world
            .violations
            .push("context: putback without an offensive rebound".to_string());
    }
    world.shot_clock_at_release.push(world.clock.shot_clock);
    world.contest_samples.push(contest);
    let zone_index = match zone {
        "rim" => 0,
        "near" => 1,
        "mid" => 2,
        _ => 3,
    };
    let defender_ft = crate::state::nearest_opponent(&world.players, shooter)
        .map(|defender| (defender.pos - world.players[shooter].pos).length())
        .unwrap_or(f32::INFINITY);
    world.shot_samples.push(crate::state::ShotSample {
        shot_clock: world.clock.shot_clock,
        game_clock: world.clock.game_clock,
        period: world.clock.period,
        contest,
        defender_ft,
        zone: zone_index as u8,
        phase: ball_phase,
        distance_ft: distance,
        target_radius_ft: spread,
        aperture_ft: model.effective_aperture_ft,
        sigma_ft: model.sigma_ft,
        p_make,
        fouled,
        blocked: false,
        p_foul,
        p_block,
        p_make_cond: p_make,
        result: "pending",
        made: false,
        points,
    });
    world.shot_zone_counts[crate::world::World::team_index(team)][zone_index] += 1;
    let creation_index = match kind.creation() {
        "drive" => 0,
        "cut" => 1,
        "post" => 2,
        "pull_up" => 3,
        "putback" => 4,
        "transition" => 5,
        _ => 6,
    };
    world.creation_counts[creation_index] += 1;
    world.pending_shot = Some(PendingShot {
        shooter,
        kind,
        zone,
        distance_ft: distance,
        contest,
        fouled,
        and_one,
        free_throws,
        points,
        is_three,
        assisted_by,
        shot_id,
        free_throw: None,
        result: None,
    });
    let now = world.now();
    world.players[shooter].facing = (hoop - pos).normalize_or_zero();
    let rules = world.rules.clone();
    world.ball.release_shot(crate::ball::ShotRelease {
        rules: &rules,
        now,
        shot_id,
        shooter,
        hoop,
        release_z: model.release_z_ft,
        aperture_ft: model.effective_aperture_ft,
        origin: model.origin,
        plan: model.plan,
        aim_offset,
    });
    world.phase = Phase::ShotFlight {
        resolve_at: now + f64::from(rules.max_shot_duration_seconds) + 2.0,
    };

    ActionOutcome {
        actor: shooter,
        kind: ActionKind::Shoot(kind),
        completion: Completion::Completed,
        delta: StateDelta {
            balance: Some(1.0 - contest),
            ..Default::default()
        },
        facts: Vec::new(),
    }
}

/// 球到筐：应用得分、犯规、罚球与打铁后的篮板。
pub fn finish(world: &mut World) {
    let pending = world
        .pending_shot
        .take()
        .expect("shot flight must carry a pending shot");
    if let Some(meta) = pending.free_throw {
        crate::resolve::freethrow::finish(world, pending, meta);
        return;
    }
    let shooter = pending.shooter;
    let team = world.players[shooter].team;
    let result = pending
        .result
        .expect("shot flight must resolve before settlement");
    let made_ball = result == ShotResult::Made;
    let waived = pending.fouled && !pending.and_one;
    let made = made_ball && !waived;
    let blocked_by = match result {
        ShotResult::Blocked { by } => Some(by),
        _ => None,
    };
    let rim_contact = matches!(result, ShotResult::Missed { rim_contact: true });
    let points = if made { pending.points } else { 0 };
    let free_throws = if pending.fouled {
        if pending.and_one && made_ball {
            1
        } else if pending.is_three {
            3
        } else {
            2
        }
    } else {
        0
    };
    let line = world.line_mut(shooter);
    // 被犯规的出手仍然是一次出手：NBA 统计里它计入 FGA，只是得分作废。
    // 把犯规出手排除在 FGA 之外会让出手数与命中率同时失真。
    line.fga += 1;
    if pending.is_three {
        line.tpa += 1;
    }
    if made {
        line.fgm += 1;
        if pending.is_three {
            line.tpm += 1;
        }
        line.points += points as u32;
        let made_zone = match pending.zone {
            "rim" => 0,
            "near" => 1,
            "mid" => 2,
            _ => 3,
        };
        world.shot_zone_makes[crate::world::World::team_index(team)][made_zone] += 1;
        world.add_points(team, points as u32);
        if let Some(passer) = pending.assisted_by {
            world.line_mut(passer).assists += 1;
        }
    }
    if let Some(blocker) = blocked_by {
        world.line_mut(blocker).blocks += 1;
    }
    if let Some(sample) = world.shot_samples.last_mut() {
        sample.made = made;
        sample.blocked = blocked_by.is_some();
        sample.result = match result {
            ShotResult::Made => "made",
            ShotResult::Missed { rim_contact: true } => "missed_rim",
            ShotResult::Missed { rim_contact: false } => "missed",
            ShotResult::Blocked { .. } => "blocked",
        };
    }
    world.emit(crate::action::Fact::Shot {
        shot_id: pending.shot_id,
        shooter,
        kind: pending.kind,
        zone: pending.zone,
        distance_ft: pending.distance_ft,
        contest: pending.contest,
        made,
        points,
        assisted_by: if made { pending.assisted_by } else { None },
    });
    if pending.fouled {
        let fouler = world
            .players
            .iter()
            .enumerate()
            .filter(|(_, p)| p.on_court && p.team != team)
            .min_by(|a, b| {
                (a.1.pos - world.players[shooter].pos)
                    .length_squared()
                    .partial_cmp(&(b.1.pos - world.players[shooter].pos).length_squared())
                    .expect("finite distances")
            })
            .map(|(index, _)| index)
            .expect("a shooting foul requires a defender");
        crate::resolve::foul::apply_shooting_foul(
            world,
            fouler,
            shooter,
            free_throws,
            made,
        );
        return;
    }
    if made {
        world.end_possession("score", points as u32);
        world.phase = Phase::DeadBall {
            team: team.other(),
            ready_at: world.now() + 1.4,
        };
    } else {
        crate::resolve::rebound::resolve(
            world,
            pending.zone,
            pending.distance_ft,
            rim_contact,
        );
    }
}

/// 封盖：出手点附近、封盖与弹跳更强的防守人更可能起跳。返回被选中的
/// 防守人与这次出手被封盖的概率。
fn block_probability(
    world: &World,
    shooter: usize,
    pos: Vec2,
    contest: f32,
    is_three: bool,
) -> (Option<usize>, f32) {
    let team = world.players[shooter].team;
    let candidate = world
        .players
        .iter()
        .enumerate()
        .filter(|(_, p)| p.on_court && p.team != team)
        .filter(|(_, p)| (p.pos - pos).length() < 5.5)
        .max_by(|a, b| {
            let weights = &world.params.policy;
            let reference = weights.g("vertical_reference_ft");
            let score = |p: &crate::state::PlayerState| {
                let vertical = crate::physical::Physical::of(p, weights).vertical_ft / reference;
                p.data.attributes.block * weights.w("block_select_skill")
                    + vertical * weights.w("block_select_height")
            };
            score(a.1)
                .partial_cmp(&score(b.1))
                .expect("finite block attributes")
        })
        .map(|(index, _)| index);
    let Some(candidate) = candidate else {
        return (None, 0.0);
    };
    let defender = &world.players[candidate];
    let skill = defender.data.attributes.block;
    let aggression = defender.data.tendencies.block_aggressiveness;
    let weights = &world.params.policy;
    // 弹跳通过物理映射换算成英尺，再除以参照起跳高度：模型里没有
    // 「属性」这个东西，只有能摸到多高。
    let vertical = crate::physical::Physical::of(defender, weights).vertical_ft
        / weights.g("vertical_reference_ft");
    let base = weights.w("block_base");
    let range = if is_three {
        weights.w("block_range_factor_three")
    } else {
        1.0
    };
    let p_block = (base
        * (weights.w("block_skill_base") + weights.w("block_skill_slope") * skill)
        * (weights.w("block_height_base") + weights.w("block_height_slope") * vertical)
        * (weights.w("block_contest_base") + contest)
        * (weights.w("block_aggression_base") + weights.w("block_aggression_slope") * aggression)
        * range)
        .clamp(f32::MIN_POSITIVE, weights.w("block_cap"));
    (Some(candidate), p_block)
}

/// 起跳犯规：对抗强度高的防守人在封盖尝试里更容易撞到投篮人。
fn block_foul_probability(world: &World, blocker: usize) -> f32 {
    let physicality = world.players[blocker].data.tendencies.physicality;
    let base = world.params.policy.w("block_foul_physicality_base");
    clamp01(world.params.policy.w("block_foul_rate") * (base + physicality))
}

pub fn shot_origin(shooter_pos: Vec2, hoop: Vec2) -> Vec2 {
    let direction = (hoop - shooter_pos).normalize_or_zero();
    shooter_pos + direction * 0.4
}
