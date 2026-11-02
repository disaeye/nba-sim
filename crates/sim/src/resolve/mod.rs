//! 结算层：动作 → 状态变化 + 事实。
//!
//! 每个结算函数只读世界状态与参数表，写出 [`StateDelta`]；裁定结果的事件
//! 进入事实流。任何结算都不读取动作历史，也不预采样未来动作的结果。

pub mod drive;
pub mod fake;
pub mod foul;
pub mod freethrow;
pub mod offball;
pub mod pass;
pub mod poke;
pub mod protect;
pub mod rebound;
pub mod screen;
pub mod shot;

use glam::Vec2;
use rand::Rng;

use crate::action::{
    ActionIntent, ActionKind, ActionOutcome, Completion, Fact, StateDelta, TurnoverKind,
};
use crate::state::{DribbleState, PossessionKind, TeamSide};
use crate::world::World;

pub fn resolve(world: &mut World, intent: ActionIntent) -> ActionOutcome {
    let outcome = match intent.kind {
        ActionKind::Pass(kind) => pass::resolve(world, &intent, kind),
        ActionKind::Screen(kind) => screen::resolve(world, &intent, kind),
        ActionKind::Drive => drive::resolve(world, &intent),
        ActionKind::Finish => drive::resolve_finish(world, &intent),
        ActionKind::Shoot(kind) => shot::resolve(world, &intent, kind),
        ActionKind::PostUp => drive::resolve_post(world, &intent),
        // 切入人自己发起的空切只跑路线；持球人发起的空切同时送出高吊球。
        ActionKind::Cut if intent.target.is_some() => pass::resolve_cut(world, &intent),
        ActionKind::Cut => offball::cut_route(world, &intent),
        ActionKind::Dribble(kind) => resolve_dribble(world, &intent, kind),
        ActionKind::Fake(kind) => fake::resolve(world, &intent, kind),
        ActionKind::Protect(kind) => protect::resolve(world, &intent, kind),
        ActionKind::Relocate => ActionOutcome {
            actor: intent.actor,
            kind: intent.kind,
            completion: Completion::Completed,
            delta: StateDelta::default(),
            facts: Vec::new(),
        },
    };
    world.screen_defender = None;
    outcome
}

/// 运球：正常运球结束即收球并记下位置；已收球再运球是二次运球。
///
/// 技术细分决定这次运球的分离距离与球的暴露程度：变向类换到横向空间，
/// 后撤步拉开距离，迟疑步冻结伸手时机，原地运球不改变位置。
fn resolve_dribble(
    world: &mut World,
    intent: &ActionIntent,
    kind: nba_domain::action_window::DribbleMoveKind,
) -> ActionOutcome {
    use nba_domain::action_window::DribbleMoveKind;
    let actor = intent.actor;
    let (separation, exposure) = match kind {
        DribbleMoveKind::DirectDrive => (0.0, 1.0),
        DribbleMoveKind::Crossover => (0.9, 0.45),
        DribbleMoveKind::BetweenTheLegs => (0.6, 0.40),
        DribbleMoveKind::BehindTheBack => (0.8, 0.35),
        DribbleMoveKind::SpinMove => (1.2, 0.60),
        DribbleMoveKind::Hesitation => (0.3, 0.30),
        DribbleMoveKind::Retreat => (1.0, 0.40),
    };
    if world.possession.dribble_state == DribbleState::Gathered {
        world.line_mut(actor).turnovers += 1;
        world.emit(Fact::Turnover {
            player: actor,
            kind: TurnoverKind::DoubleDribble,
        });
        let team = world.players[actor].team;
        world.end_possession(
            nba_domain::event::PossessionEndCause::TurnoverViolation,
            "double_dribble",
            0,
        );
        world.start_possession(team.other(), PossessionKind::Live);
        return ActionOutcome {
            actor,
            kind: ActionKind::Dribble(kind),
            completion: Completion::Failed,
            delta: StateDelta {
                turnover: Some(TurnoverKind::DoubleDribble),
                ..Default::default()
            },
            facts: Vec::new(),
        };
    }
    // 只有在能出手的位置上收球才成立：收球是「把球收起来投」的��备动作。
    // 远离篮筐的站定运球只是把球拿稳（调整步），此时球权仍然在手上，
    // 持球人还可以继续找机会；把它算成收球会让「二次运球」规则禁掉
    // 持球候选，于是球手在三分线外被逼着出手一个必失的球。
    let player = &world.players[actor];
    let stationary =
        (player.target - player.pos).length() < world.params.policy.g("gather_stationary_ft");
    let hoop = world.hoop_for(world.players[actor].team);
    let in_shooting_range = (player.pos - hoop).length() <= world.params.policy.g("finish_zone_ft");
    if stationary && in_shooting_range {
        world.possession.dribble_state = DribbleState::Gathered;
        world.possession.gather_pos = Some(player.pos);
        world.possession.ball_phase = crate::state::BallPhase::Gather;
    } else {
        world.possession.dribble_state = DribbleState::Dribbling;
        world.possession.gather_pos = None;
        world.possession.ball_phase = crate::state::BallPhase::Dribble;
    }
    // 球的暴露程度按技术设定，窗口覆盖这一次运球的持续时间。
    world.possession.dribble_exposure = exposure;
    world.possession.dribble_exposure_until = intent.end_time().max(world.now());
    // 变向与后撤步是被动的身体移动：把落点设到防守人的另一侧，
    // 由移动层按速度走完；迟疑步改为冻结对手的伸手时机。
    let defender = world
        .players
        .iter()
        .enumerate()
        .filter(|(handle, player)| {
            *handle != actor && player.on_court && player.team != world.players[actor].team
        })
        .map(|(handle, player)| (handle, (player.pos - world.players[actor].pos).length()))
        .min_by(|left, right| left.1.partial_cmp(&right.1).expect("finite distance"));
    if let Some((defender, _)) = defender {
        match kind {
            DribbleMoveKind::Crossover
            | DribbleMoveKind::BetweenTheLegs
            | DribbleMoveKind::BehindTheBack
            | DribbleMoveKind::SpinMove
            | DribbleMoveKind::Retreat => {
                let away =
                    (world.players[actor].pos - world.players[defender].pos).normalize_or_zero();
                let lateral = Vec2::new(-away.y, away.x);
                let step = match kind {
                    DribbleMoveKind::Crossover | DribbleMoveKind::BetweenTheLegs => lateral,
                    DribbleMoveKind::BehindTheBack | DribbleMoveKind::SpinMove => -lateral,
                    _ => away,
                };
                let target = world.players[actor].pos + step * separation;
                crate::assignment::force_target_as(
                    world,
                    actor,
                    target,
                    crate::state::TargetKind::Window,
                    actor,
                );
            }
            DribbleMoveKind::Hesitation => {
                // 迟疑步冻结防守人的伸手时机：留下一个短失衡窗口。
                let until =
                    world.now() + f64::from(world.rules.resolve.technique.fake_bait_seconds) * 0.5;
                world.players[defender].bait = Some(crate::state::Bait {
                    kind: crate::action::FakeKind::Hesitation,
                    until,
                    displacement_ft: 0.0,
                });
            }
            DribbleMoveKind::DirectDrive => {}
        }
    }
    world.emit(Fact::DribbleMove {
        player: actor,
        kind,
        separation_ft: separation,
        exposure,
    });
    ActionOutcome {
        actor,
        kind: ActionKind::Dribble(kind),
        completion: Completion::Completed,
        delta: StateDelta::default(),
        facts: Vec::new(),
    }
}

pub fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

pub fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

pub fn shooter_zone(world: &World, pos: Vec2, team: TeamSide) -> &'static str {
    let attacking_right = world.attacking_right(team);
    match world.rules.court.shot_zone(
        pos,
        attacking_right,
        world.rules.league.three_point_distance_ft,
        world.rules.league.corner_three_distance_ft,
    ) {
        nba_domain::ShotZone::Rim => "rim",
        nba_domain::ShotZone::Near => "near",
        nba_domain::ShotZone::Mid => "mid",
        nba_domain::ShotZone::Three => "three",
    }
}

pub fn contest_for(world: &World, shooter: usize, pos: Vec2) -> f32 {
    let team = world.players[shooter].team;
    let hoop = world.hoop_for(team);
    let mut best = 0.0_f32;
    for (defender_index, defender) in world
        .players
        .iter()
        .enumerate()
        .filter(|(_, p)| p.on_court && p.team != team)
    {
        let distance = (defender.pos - pos).length();
        if distance > 9.0 {
            continue;
        }
        let between = crate::state::is_between(defender.pos, pos, hoop);
        let reach = crate::physical::Physical::of(defender, &world.params.policy)
            .reach_excess_ft(&world.params.policy);
        let contest = crate::state::contest_from_defender(distance, between, reach);
        // 被投篮假动作骗起的防守人不在封盖位置上：失衡窗口内的干扰折减。
        let bait_relief = match defender.bait {
            Some(bait)
                if bait.until > world.now()
                    && matches!(bait.kind, crate::action::FakeKind::Shot) =>
            {
                f32::from(1u8) - world.rules.resolve.technique.fake_bait_contest_relief
            }
            _ => f32::from(1u8),
        };
        let contest = contest * bait_relief;
        // 伸手切球的收势窗口里，防守人重心已经前送：干扰折减。
        let steal_penalty = match world.defensive_actions[defender_index].as_ref() {
            Some(action)
                if action.kind == crate::action::DefensiveActionKind::StealAttempt
                    && action.phase == nba_domain::action_window::ActionPhase::FollowThrough =>
            {
                f32::from(1u8) - world.rules.resolve.technique.steal_attempt_follow_penalty
            }
            _ => f32::from(1u8),
        };
        let contest = contest * steal_penalty;
        // 协防倾向高的防守人更早收缩，位置不变时干扰更强。
        let help = defender.data.tendencies.help_aggressiveness;
        let contest = contest
            * (world.params.policy.w("help_contest_base")
                + world.params.policy.w("help_contest_slope") * help);
        if contest > best {
            best = contest;
        }
    }
    // 防守人钻过掩护意味着持球人拿到了这段分离距离对应的出手空间。
    let relief = match world.possession.last_screen {
        Some(read) if matches!(read.route, crate::action::DefenderRoute::Under) => {
            let span = world.params.policy.g("screen_under_relief_ft");
            let ceiling = world.params.policy.g("screen_under_relief_max");
            let floor = world.params.policy.g("screen_under_relief_floor");
            (read.separation_ft / span).clamp(floor, ceiling)
        }
        _ => f32::default(),
    };
    // 转换期防守尚未落位：折减随回合推进线性收回。
    let transition = if world.possession.kind == crate::state::PossessionKind::Live {
        let window = world.params.policy.g("transition_window_seconds");
        let elapsed = (world.now() - world.possession.started_at) as f32;
        (f32::from(1u8) - elapsed / window).clamp(f32::default(), f32::from(1u8))
            * world.params.policy.g("transition_unset_relief")
    } else {
        f32::default()
    };
    clamp01(best - relief - transition)
}

pub fn weighted_pick<R: Rng>(rng: &mut R, weights: &[f32]) -> usize {
    weighted_pick_at(rng.gen::<f32>(), weights)
}

pub fn weighted_pick_at(unit: f32, weights: &[f32]) -> usize {
    let total: f32 = weights.iter().sum();
    assert!(total > 0.0, "action weights must not all be zero");
    let mut threshold = unit * total;
    for (index, weight) in weights.iter().enumerate() {
        threshold -= weight;
        if threshold <= 0.0 {
            return index;
        }
    }
    weights.len() - 1
}
