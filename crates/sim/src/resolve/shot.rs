//! 出手结算：干扰、技巧、体能进 logit，命中率来自校准表。

use glam::Vec2;

use crate::action::{
    ActionIntent, ActionKind, ActionOutcome, Completion, Fact, ShotKind, StateDelta, TurnoverKind,
};
use crate::draw::{purpose, Stream};
use crate::resolve::{clamp01, shooter_zone};
use crate::world::World;

const MIN_BLOCKED_SHOT_CONTEST: f32 = 0.06;

/// 干扰球：球在下落、水平位置还在篮筐上方的筐柱范围内时被防守人碰到，
/// 出手直接记中。球越过最高点之前可以合法封盖，越过后就是干扰。
pub fn check_goaltending(world: &mut World) -> bool {
    if !world
        .ball_flight
        .is_some_and(|flight| flight.kind == crate::state::FlightKind::Shot)
    {
        return false;
    }
    let Some(pending) = world.pending_shot.as_ref() else {
        return false;
    };
    // 这次出手已经有结果：不再重复判定，也不重复发布事实。
    if pending.result.is_some() || pending.free_throw.is_some() {
        return false;
    }
    let (shooter, shot_id, touched_backboard) =
        (pending.shooter, pending.shot_id, pending.touched_backboard);
    let team = world.players[shooter].team;
    let hoop = world.hoop_for(team);
    let policy = &world.params.policy;
    let rim_height = world.rules.rim_height_ft;
    let reach = policy.g("goaltending_reach_ft");
    let ball_pos = world.ball.pos;
    let ball_z = world.ball.z;
    let descending = world.ball.vz < 0.0;
    // 球碰到了篮板：碰过篮板之后如果球还有得分机会，防守人再碰它就是
    // 干扰（NBA Rule 11 §I.3：碰过篮板之上之后，无论球在上升还是
    // 下落，碰到都算干扰）。
    //
    // 这一条读的是本次出手自己的记录，不是扫事件日志：上一次出手碰过
    // 篮板不该让这一次出手的球也变成干扰。
    // 球是否还在筐上（坐在筐沿上或者滚在筐沿上）：这种球被碰到就是干扰，
    // 无论防守人有没有把它按进（NBA Rule 11 §I.1）。
    let on_ring = (ball_pos - hoop).length()
        <= world.rules.rim_radius_ft + world.rules.ball_radius_ft
        && (ball_z - rim_height).abs() <= world.rules.ball_radius_ft;

    // 三种成立的情形（NBA Rule 11 §I）：
    //
    // 1. 球在筐上方到筐下的圆柱段里下落：这一段里碰球就是干扰，
    //    无论它原本会不会进。
    // 2. 球碰过篮板而且还有得分机会：碰球就是干扰，不管球在上升还是
    //    下落。
    // 3. 球坐在或者滚在筐沿上：碰球就是干扰。
    let inside_cylinder = (ball_pos - hoop).length() <= policy.g("goaltending_cylinder_radius_ft")
        && ball_z <= rim_height + 0.75
        && ball_z >= rim_height * 0.5;
    let descending_in_cylinder = descending && inside_cylinder;
    let after_backboard = touched_backboard && inside_cylinder;
    let touching_ring = on_ring && descending;
    if !(descending_in_cylinder || after_backboard || touching_ring) {
        return false;
    }
    let offender = world
        .players
        .iter()
        .enumerate()
        .find_map(|(index, player)| {
            if !player.on_court || player.team == team {
                return None;
            }
            let physical = crate::physical::Physical::of(player, policy);
            let hand = physical.contest_reach_ft(policy);
            let flat = (player.pos - ball_pos).length();
            (flat <= reach && ball_z <= hand + 0.5).then_some(index)
        });
    let Some(defender) = offender else {
        return false;
    };
    world.emit(Fact::Goaltending {
        defender,
        shooter,
        shot_id,
        time: world.now(),
    });
    // 干扰球与篮筐干扰是两个复核问题：球坐在筐沿上被碰到是篮筐干扰，
    // 球在筐柱内被碰到是干扰球。裁判当时的认定因此可复核。
    let call = if touching_ring {
        "basket_interference"
    } else {
        "goaltending"
    };
    let address = world.last_event_id;
    let misjudged = world.judge_with_error(defender, address);
    world.record_ruling_with_observation(
        team,
        call,
        Some(defender),
        crate::review_program::Observation::Contact,
        crate::review_program::ReviewCorrection::Possession,
        crate::review_program::ReviewBasis::Judgement { misjudged },
        crate::review_program::ReviewTrigger::Immediate,
        vec![address],
    );
    // 干扰球在球还在飞的时候就能看清：裁判当场复核，结论立刻落地。
    let scope = crate::review_program::ReviewScope::from_call(call)
        .expect("the goaltending call maps to a review scope");
    world
        .conclude_immediate_review(scope)
        .expect("the goaltending ruling was just registered");
    if let Some(pending) = world.pending_shot.as_mut() {
        pending.result = Some(ShotResult::Made);
    }
    true
}

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
    /// 本次出手被判定会封盖的那只手：出手时按 `block_risk` 抽一次确定，
    /// 球飞行中只等它真的进到这双手的球体里。
    pub block_by: Option<usize>,
    /// 球穿过筐口、触筐、触地或触手后才有值；出手瞬间保持未定。
    pub result: Option<ShotResult>,
    /// 这一次出手有没有碰过篮板：碰过之后防守人再碰球就是干扰
    /// （NBA Rule 11 §I.3）。
    pub touched_backboard: bool,
    /// 球离手那一刻的本节剩余比赛时间：节末即时回放在结算时用它与
    /// 当前钟值比对，判定这次出手是否在钟响之前离手。
    pub released_game_clock: f64,
    /// 出手那一刻的比赛钟检查点：节末即时回放改判时，比赛钟从它恢复。
    ///
    /// 复核纠正的是「钟响前还是钟响后离手」，纠正之后比赛钟必须回到
    /// 出手那一刻的读数，否则这一节剩下的时间凭空少了。
    pub released_clock_checkpoint: Option<nba_domain::CheckpointId>,
    /// 出手那一刻的事件身份：待决出手与信号前接触都引用它。
    pub released_event_id: u64,
    /// 出手那一刻的进攻钟读数：节末进攻钟复核用它判断出手是否在钟响前。
    pub released_shot_clock: f64,
}

/// 后撤步的落点：远离最近的防守人。
///
/// 落点是位移的目的地，不是位移本身：突破那一步把落点写进目标，位移由
/// 运动层逐 tick 完成，因此每 tick 的位移仍在速度上限内。
pub fn step_back_target(world: &World, shooter: usize) -> glam::Vec2 {
    let team = world.players[shooter].team;
    let pos = world.players[shooter].pos;
    let nearest = world
        .players
        .iter()
        .filter(|player| player.on_court && player.team != team)
        .min_by(|left, right| {
            (left.pos - pos)
                .length_squared()
                .total_cmp(&(right.pos - pos).length_squared())
        });
    let away = nearest
        .map(|player| (pos - player.pos).normalize_or_zero())
        .unwrap_or(glam::Vec2::ZERO);
    let radius = world.rules.player_radius_ft;
    (pos + away * world.params.policy.g("step_back_distance_ft")).clamp(
        glam::Vec2::new(radius, radius),
        glam::Vec2::new(
            world.rules.court.width_ft - radius,
            world.rules.court.height_ft - radius,
        ),
    )
}

pub fn resolve(world: &mut World, intent: &ActionIntent, kind: ShotKind) -> ActionOutcome {
    let shooter = intent.actor;
    let team = world.players[shooter].team;
    // 释放时的走步判定：收球确立的轴心脚被移动、或者脚步用尽。
    // 判定在出手之前，出手被取消，球权转给对手。
    let release = crate::footwork::judge_release(world, shooter);
    if release != crate::footwork::FootworkVerdict::Legal {
        // 跳步与走步是两条不同的违例（NBA Rule 10 §XIII）。
        if release == crate::footwork::FootworkVerdict::Hop {
            crate::footwork::book_hop(world, shooter, team);
        } else {
            crate::footwork::book_travel(world, shooter, team);
        }
        return ActionOutcome {
            actor: shooter,
            kind: ActionKind::Shoot(kind),
            completion: Completion::Failed,
            delta: StateDelta {
                turnover: Some(TurnoverKind::Travel),
                ..Default::default()
            },
            facts: Vec::new(),
        };
    }
    let hoop = world.hoop_for(team);
    // 后撤步跳投：撤步是突破那一步的事，随后的出手站在撤开的那个点上。
    // 出手本身不再改位置：位置由运动层按突破时写下的目标走完。
    let pos = world.players[shooter].pos;
    let zone = shooter_zone(world, pos, team);
    let is_three = zone == "three";
    // 二分与三分的判定落在线上时，裁判的观察误差足以改变这次判罚：这个
    // 联赛承认这类复核问题时，出手点登记一条可复核的裁决。
    if world.rules.league.reviews("two_versus_three_points") {
        let margin = world.rules.court.three_point_margin_ft(
            pos,
            world.attacking_right(team),
            world.rules.league.three_point_distance_ft,
            world.rules.league.corner_three_distance_ft,
        );
        if margin.abs() <= world.params.policy.g("three_point_call_tolerance_ft") {
            // 判定带观察误差：三分线附近的出手可能被裁判量错。复核读到
            // 冻结的几何时把它纠正，分值随之改判。
            let judged_three = is_three;
            let misjudged = world.judge_with_error(shooter, world.last_event_id);
            let corrected_three = if misjudged {
                !judged_three
            } else {
                judged_three
            };
            let delta = if corrected_three && !judged_three {
                1
            } else if judged_three && !corrected_three {
                -1
            } else {
                0
            };
            world.record_ruling_with_observation(
                team,
                "two_versus_three_points",
                Some(shooter),
                crate::review_program::Observation::Zone(if judged_three {
                    "three"
                } else {
                    "two"
                }),
                crate::review_program::ReviewCorrection::Points { delta },
                crate::review_program::ReviewBasis::Judgement { misjudged },
                crate::review_program::ReviewTrigger::AtShotSettlement,
                vec![world.last_event_id],
            );
        }
    }
    let distance = (pos - hoop).length();
    let phase = world.possession.ball_phase;
    let model = crate::shot_quality::evaluate(world, shooter, kind, phase, pos);
    let contest = model.contest;
    let ball_phase = phase.as_str();
    // 投篮终结了持球状态：收球标志在出手时清掉。
    world.possession.dribble_state = crate::state::DribbleState::Dribbling;
    world.possession.gather_pos = None;
    crate::footwork::clear_pivot(world);
    world.possession.ball_phase = crate::state::BallPhase::Dribble;
    let p_make = model.p_make;

    // 防守人已经起跳：滞空的人不能改向，进攻方因此多出两个选择。靠上去
    // 把身体造进对方身上要犯规，或者侧移把出手点从他身侧让开。一旦选
    // 定其中一种，这次出手就不再可能被盖：起跳封盖的手臂已经越过了球。
    //
    // 这个决定在防守人起跳的那一刻就做出了（见 `advance_block_jumps`），
    // 因此这里读的是已经落定的应答，而不是再抽一次。
    let response = world.players[shooter]
        .closeout_response
        .map(|response| match response {
            "draw_contact" => CloseoutResponse::DrawContact,
            _ => CloseoutResponse::Sidestep,
        })
        .unwrap_or(CloseoutResponse::None);
    let draw_contact = response == CloseoutResponse::DrawContact;
    let sidestep = response == CloseoutResponse::Sidestep;
    let mut p_make = p_make;
    if sidestep {
        // 侧移把出手点让开了封盖，代价是出手姿势被拉歪。
        p_make = clamp01(p_make * world.params.policy.w("closeout_sidestep_make_scale"));
    }

    // 犯规概率按出手价值分开标定：三分出手的投篮犯规比两分低一个数量级，
    // 同一个概率给两档会把罚球数推到实测的一点五倍。
    let base_foul_rate = world.params.policy.w(if is_three {
        "shooting_foul_rate_three_point"
    } else {
        "shooting_foul_rate_two_point"
    });
    // 造犯规的那一步是主动把身体送进滞空防守人的落地路线：接触成立，
    // 犯规因此几乎必然发生。这个概率不乘在底频上，而是直接成为判罚概率。
    let initiated_contact = draw_contact
        && world.draw(Stream::Foul, shooter, purpose::CLOSEOUT_DRAW_CONTACT, 1)
            < world.params.policy.w("closeout_draw_contact_foul_chance");
    // 封盖是球飞到手上才发生的事件，不在出手时抽结果。这里只保留封盖
    // 尝试带来的犯规风险；封盖本身由手部接触事实决定。防守人已经起跳时
    // 他不能改向，这次出手不可能被盖。
    let airborne = airborne_block_jumper(world, team, pos);
    let (block_candidate, _physical_block) = if airborne.is_some() {
        (None, 0.0)
    } else {
        block_probability(world, shooter, pos, contest, is_three)
    };
    let block_foul = match block_candidate {
        Some(candidate) => {
            world.draw(Stream::Foul, candidate, purpose::SHOT_BLOCK_FOUL, 0)
                < block_foul_probability(world, candidate)
        }
        None => false,
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
    // 封盖概率由模型给出，这里只抽一次并定下封盖的人；执行层再用同一个
    // 手球几何把这次封盖演成手真的碰到球，因此概率与实际判定是同一件事。
    let p_block = model.block_risk;
    let block_draw = world.draw(Stream::Aim, shooter, purpose::SHOT_BLOCK, 0);
    let block_by = model.block_hand.filter(|_| block_draw < p_block);
    let contest = if block_by.is_some() {
        contest.max(MIN_BLOCKED_SHOT_CONTEST)
    } else {
        contest
    };
    let p_foul = clamp01(base_foul_rate * (0.6 + 0.9 * contest));
    let fouled = initiated_contact
        || block_foul
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
        .map(|time| intent.start_time - time < 2.5)
        .unwrap_or(false);
    // 判据按「这次出手实际属于哪种进攻」来判，而不是按出手类型的粗分类：
    // 只有真正的接球就投（收到球之后立刻出手）才需要前面有一次传球；
    // 收球步之后的篮下完成不需要。
    let catch_and_shoot_without_pass =
        matches!(kind, crate::action::ShotKind::CatchShoot) && !just_received;
    // 判据按「这次出手实际属于哪种进攻」来判，而不是按出手类型的粗分类：
    // 只有真正的接球就投（收到球之后立刻出手）才需要前面有一次传球。
    // 出手类型必须与本次回合里投篮者本人的动作对应：一次上篮后面可以是
    // 任意长的收球、假动作与调整步，时间窗口判不准这件事，只有「这个回合
    // 里他做过这个动作」是确定成立的。
    let drive_finish = matches!(
        kind,
        crate::action::ShotKind::DrivingLayup
            | crate::action::ShotKind::DrivingDunk
            | crate::action::ShotKind::ReverseLayup
    ) && world.possession.last_drive_actor != Some(shooter);
    let cut_finish = matches!(
        kind,
        crate::action::ShotKind::CuttingLayup | crate::action::ShotKind::CuttingDunk
    ) && world.possession.last_cut_actor != Some(shooter);
    let post_finish = matches!(
        kind,
        crate::action::ShotKind::Hook | crate::action::ShotKind::Turnaround
    ) && world.possession.last_post_actor != Some(shooter);
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
    world.shot_clock_at_release.push(world.clock.shot_clock());
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
        shot_clock: world.clock.shot_clock(),
        game_clock: world.clock.game_clock(),
        period: world.clock.period(),
        contest,
        movement: model.movement,
        skill: model.skill,
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
        p_make_cond: model.p_make_cond,
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
    // 出手那一刻记下比赛钟检查点：节末即时回放改判时，比赛钟从它恢复。
    let released_clock_checkpoint = world.snapshot_clock_checkpoint();
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
        block_by,
        // 模型判定的封盖必须算作封盖：球在出手点就被拍掉，而不是先飞出去再赌接触。
        result: block_by.map(|by| ShotResult::Blocked { by }),
        touched_backboard: false,
        released_game_clock: world.clock.game_clock(),
        released_clock_checkpoint,
        released_event_id: world.last_event_id,
        released_shot_clock: world.clock.shot_clock(),
    });
    let now = world.now();
    world.players[shooter].facing = (hoop - pos).normalize_or_zero();
    let rules = world.rules.clone();
    if let Some(by) = block_by {
        // 手在出手点碰到球：球从投篮人手上直接脱落。
        let hand = model.origin;
        world.emit(Fact::BallContact {
            kind: "hand",
            entity: Some(by),
            time: now,
            x: hand.x,
            y: hand.y,
            z: model.release_z_ft,
            sub_tick: 0,
        });
        world.ball.pos = hand;
        world.ball.z = model.release_z_ft;
        world.release_ball(crate::state::BallState::Loose {
            pos: hand,
            vel: Vec2::ZERO,
        });
    } else {
        world.ball.release_shot(crate::ball::ShotRelease {
            rules: &rules,
            now,
            shot_id,
            shooter,
            hoop,
            release_z: model.release_z_ft,
            aperture_ft: model.effective_aperture_ft,
            rim_in_probability: model.rim_in_probability,
            origin: model.origin,
            plan: model.plan,
            aim_offset,
            context: crate::state::ShotContext::FieldGoal,
        });
    }
    world.set_flight(
        crate::state::FlightKind::Shot,
        now + f64::from(rules.max_shot_duration_seconds) + 2.0,
    );

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
    world.clear_flight();
    let pending = world
        .pending_shot
        .take()
        .expect("shot flight must carry a pending shot");
    // 这次出手落地：扑防的应对到此结束。
    world.players[pending.shooter].closeout_response = None;
    world.end_posture(pending.shooter, crate::state::PostureSlot::Closeout);
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
    // 节末压哨：裁判按联赛档案的即时回放范围自动复核两个问题——出手在
    // 比赛钟响前还是响后，出手在进攻钟走完前还是走完后。复核有实质后果：
    // 钟响后离手或者进攻钟已走完的出手，得分被取消。
    let game_clock_scope = crate::review_program::ReviewScope::GameClockAtPeriodEnd;
    let shot_clock_scope = crate::review_program::ReviewScope::ShotClockAtPeriodEnd;
    let clock_malfunction_scope = crate::review_program::ReviewScope::ClockMalfunction;
    let at_horn = world.clock.game_clock() <= 0.0 && made_ball && !waived;
    let mut waved_off = false;
    if at_horn && crate::review_program::is_reviewable(world, game_clock_scope) {
        let released_before_horn = pending.released_game_clock > 0.0;
        waved_off = !released_before_horn;
        // 复核按程序发布：它登记待复核的裁决，读权威记录，然后发布结论。
        // 结论与原判都留在事件流里，统计引用更正关系。
        world.record_record_ruling(
            team,
            "game_clock_at_period_end",
            Some(shooter),
            crate::review_program::Observation::ReleaseClock {
                game_clock: pending.released_game_clock,
                shot_clock: pending.released_shot_clock,
            },
            released_before_horn,
            crate::review_program::ReviewCorrection::Points { delta: 0 },
            crate::review_program::ReviewTrigger::Immediate,
            vec![pending.released_event_id],
        );
        let conclusion = if waved_off {
            crate::review_program::ReviewConclusion::Overturned
        } else {
            crate::review_program::ReviewConclusion::Upheld
        };
        world
            .conclude_review(game_clock_scope, conclusion)
            .expect("the period-end ruling was just registered");
    }
    if at_horn && crate::review_program::is_reviewable(world, shot_clock_scope) {
        let shot_clock_violation = pending.released_shot_clock <= 0.0;
        world.record_record_ruling(
            team,
            "shot_clock_at_period_end",
            Some(shooter),
            crate::review_program::Observation::ReleaseClock {
                game_clock: pending.released_game_clock,
                shot_clock: pending.released_shot_clock,
            },
            !shot_clock_violation,
            crate::review_program::ReviewCorrection::Points { delta: 0 },
            crate::review_program::ReviewTrigger::Immediate,
            vec![pending.released_event_id],
        );
        let conclusion = if shot_clock_violation {
            crate::review_program::ReviewConclusion::Overturned
        } else {
            crate::review_program::ReviewConclusion::Upheld
        };
        world
            .conclude_review(shot_clock_scope, conclusion)
            .expect("the shot-clock ruling was just registered");
        waved_off = waved_off || shot_clock_violation;
    }
    // 改判成立（球在钟响前离手，得分保留）时，比赛钟回到出手那一刻的
    // 读数：复核否定了「钟响后离手」这个认定，本节剩余时间因此要恢复。
    //
    // 恢复只在节末即时回放成立时发生。普通进球不恢复比赛钟：出手到
    // 结算之间消耗的比赛时间是真实消耗，恢复它会让比赛钟在每一次进球
    // 后倒退。
    if at_horn
        && !waved_off
        && pending.released_clock_checkpoint.is_some()
        && crate::review_program::is_reviewable(world, game_clock_scope)
    {
        world.restore_clock_checkpoint(
            pending
                .released_clock_checkpoint
                .expect("the checkpoint identity is present"),
        );
        // 时钟读数被更正：这条复核问题读的是时钟读数，只能确认读数，
        // 不能改判。
        if crate::review_program::is_reviewable(world, clock_malfunction_scope) {
            world.record_record_ruling(
                team,
                "clock_malfunction",
                Some(shooter),
                crate::review_program::Observation::Clock(pending.released_game_clock),
                true,
                crate::review_program::ReviewCorrection::Points { delta: 0 },
                crate::review_program::ReviewTrigger::Immediate,
                vec![pending.released_event_id],
            );
            world
                .conclude_review(
                    clock_malfunction_scope,
                    crate::review_program::ReviewConclusion::Upheld,
                )
                .expect("the clock ruling was just registered");
        }
    }
    let made = made_ball && !waived && !waved_off;
    let blocked_by = match result {
        ShotResult::Blocked { by } => Some(by),
        _ => None,
    };
    // 被盖是一次真实的手球接触：无论出手时模型抽到了谁，结算确认被盖
    // 之后干扰度必须为正，账本与事实流都读同一个值。
    let contest = if blocked_by.is_some() {
        pending.contest.max(MIN_BLOCKED_SHOT_CONTEST)
    } else {
        pending.contest
    };
    let rim_contact = matches!(result, ShotResult::Missed { rim_contact: true });
    // 二分与三分的复核在这次出手结算时落地：分值据此确定。
    let mut points = if made { pending.points } else { 0 };
    let mut is_three = pending.is_three;
    let mut tpa = pending.is_three;
    if world.review_state().pending_all().iter().any(|ruling| {
        ruling.scope == Some(crate::review_program::ReviewScope::TwoVersusThreePoints)
    }) {
        let scope = crate::review_program::ReviewScope::TwoVersusThreePoints;
        world
            .conclude_immediate_review(scope)
            .expect("the two-versus-three ruling was registered at the release");
        if let Some((_, conclusion)) = world.review_state().concluded().last() {
            if matches!(
                conclusion,
                crate::review_program::ReviewConclusion::Overturned
            ) {
                // 复核把这一球改判成另一档：分值与三分统计一起改。
                if let Some((ruling, _)) = world
                    .review_state()
                    .concluded()
                    .iter()
                    .rev()
                    .find(|(ruling, _)| ruling.scope == Some(scope))
                {
                    let delta = match ruling.correction {
                        crate::review_program::ReviewCorrection::Points { delta } => delta,
                        _ => 0,
                    };
                    if delta > 0 {
                        points += 1;
                        is_three = true;
                        tpa = true;
                    } else if delta < 0 {
                        points = points.saturating_sub(1);
                        is_three = false;
                        tpa = false;
                    }
                }
            }
        }
    }
    let free_throws = if pending.fouled {
        if pending.and_one && made {
            1
        } else if is_three {
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
    if tpa {
        line.tpa += 1;
    }
    if made {
        line.fgm += 1;
        if is_three {
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
        // 封盖有自己的事件身份：回放与账本都能单独引用这次防守动作，
        // 并沿 parent_event_id 指回它改变的这次出手。
        world.emit(crate::action::Fact::Block {
            defender: blocker,
            shooter,
            shot_id: pending.shot_id,
        });
    }
    if let Some(sample) = world.shot_samples.last_mut() {
        sample.contest = contest;
        sample.made = made;
        sample.blocked = blocked_by.is_some();
        sample.result = match result {
            ShotResult::Made => "made",
            ShotResult::Missed { rim_contact: true } => "missed_rim",
            ShotResult::Missed { rim_contact: false } => "missed",
            ShotResult::Blocked { .. } => "blocked",
        };
    }
    if let Some(sample) = world.contest_samples.last_mut() {
        *sample = contest;
    }
    world.emit(crate::action::Fact::Shot {
        shot_id: pending.shot_id,
        shooter,
        kind: pending.kind,
        zone: pending.zone,
        distance_ft: pending.distance_ft,
        contest,
        is_three,
        blocked_by,
        made,
        points,
        assisted_by: if made { pending.assisted_by } else { None },
    });
    if pending.fouled {
        if let Some(pending) = world.pending_shot.as_mut() {
            pending.fouled = true;
            pending.and_one = made;
            pending.free_throws = free_throws;
        }
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
        let incident =
            crate::detect::detect_shooting_foul(world, fouler, shooter, free_throws, made);
        crate::adjudicate_incidents::adjudicate_one(world, Some(incident));
        return;
    }
    if made {
        world.adjudicate(
            crate::adjudicate::Adjudication::new(
                crate::adjudicate::StoppageCause::MadeBasket,
                crate::adjudicate::RestartPlan::Inbound {
                    team: team.other(),
                    location: crate::adjudicate::InboundLocation::OwnBaseline,
                    ready_after: Some(1.4),
                },
            )
            .ending(nba_domain::event::PossessionEndCause::Score, "score")
            .scoring(points as u32),
        );
    } else {
        crate::resolve::rebound::resolve(world, pending.zone, pending.distance_ft, rim_contact);
    }
}

/// 面对滞空防守人时进攻方的两个应对。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseoutResponse {
    /// 没有人在滞空，这次出手没有第三个选择。
    None,
    /// 主动把身体送进防守人的落地路线，靠接触要犯规。
    DrawContact,
    /// 侧移把出手点让开封盖。
    Sidestep,
}

/// 出手点附近是否有防守人正在封盖跳跃的执行阶段。
///
/// 只有执行阶段算滞空：准备阶段他还在屈膝，跟进阶段他已经落地。
fn airborne_block_jumper(world: &World, team: crate::action::TeamSide, pos: Vec2) -> Option<usize> {
    world
        .players
        .iter()
        .enumerate()
        .filter(|(_, player)| player.on_court && player.team != team)
        .filter(|(_, player)| {
            (player.pos - pos).length() < world.rules.resolve.technique.block_jump_radius_ft
        })
        .find(|(index, _)| {
            world.defensive_actions[*index]
                .as_ref()
                .is_some_and(|action| {
                    action.kind == crate::action::DefensiveActionKind::BlockJump
                        && action.phase == nba_domain::action_window::ActionPhase::Execution
                })
        })
        .map(|(index, _)| index)
}

/// 进攻方对滞空防守人的选择：先看敢不敢迎上去制造接触，不敢再试图侧移。
/// 两个都不成立时按原姿势出手。
pub fn closeout_response(world: &mut World, shooter: usize) -> CloseoutResponse {
    let lean = world.players[shooter].data.tendencies.risk_tolerance;
    if world.draw(Stream::Foul, shooter, purpose::CLOSEOUT_DRAW_CONTACT, 0)
        < lean * world.params.policy.w("closeout_draw_contact_rate")
    {
        return CloseoutResponse::DrawContact;
    }
    let agility = world.players[shooter].data.attributes.agility;
    if world.draw(Stream::Decision, shooter, purpose::CLOSEOUT_SIDESTEP, 0)
        < agility * world.params.policy.w("closeout_sidestep_rate")
    {
        return CloseoutResponse::Sidestep;
    }
    CloseoutResponse::None
}

impl CloseoutResponse {
    /// 这个应答在协议里的动作变体。
    pub fn action_kind(self) -> Option<&'static str> {
        match self {
            Self::DrawContact => Some("draw_contact"),
            Self::Sidestep => Some("sidestep"),
            Self::None => None,
        }
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
