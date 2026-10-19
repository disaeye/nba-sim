//! 结算层：动作 → 状态变化 + 事实。
//!
//! 每个结算函数只读世界状态与参数表，写出 [`StateDelta`]；裁定结果的事件
//! 进入事实流。任何结算都不读取动作历史，也不预采样未来动作的结果。

pub mod drive;
pub mod foul;
pub mod freethrow;
pub mod pass;
pub mod rebound;
pub mod screen;
pub mod shot;

use glam::Vec2;
use rand::Rng;

use crate::action::{
    ActionIntent, ActionKind, ActionOutcome, Completion, Fact, StateDelta, TurnoverKind,
};
use crate::state::{DribbleState, PlayerState, PossessionKind, TeamSide};
use crate::world::World;

pub fn resolve(world: &mut World) -> ActionOutcome {
    let intent = world
        .active
        .take()
        .expect("resolve requires an active action");
    match intent.kind {
        ActionKind::Pass(kind) => pass::resolve(world, &intent, kind),
        ActionKind::Screen(kind) => screen::resolve(world, &intent, kind),
        ActionKind::Drive => drive::resolve(world, &intent),
        ActionKind::Shoot(kind) => shot::resolve(world, &intent, kind),
        ActionKind::PostUp => drive::resolve_post(world, &intent),
        ActionKind::Cut => pass::resolve_cut(world, &intent),
        ActionKind::Dribble => resolve_dribble(world, &intent),
        ActionKind::Relocate => ActionOutcome {
            actor: intent.actor,
            kind: intent.kind,
            completion: Completion::Completed,
            delta: StateDelta::default(),
            facts: Vec::new(),
        },
    }
}

/// 运球：正常运球结束即收球并记下位置；已收球再运球是二次运球。
fn resolve_dribble(world: &mut World, intent: &ActionIntent) -> ActionOutcome {
    let actor = intent.actor;
    if world.possession.dribble_state == DribbleState::Gathered {
        world.line_mut(actor).turnovers += 1;
        world.emit(Fact::Turnover {
            player: actor,
            kind: TurnoverKind::DoubleDribble,
        });
        let team = world.players[actor].team;
        world.end_possession("turnover", 0);
        world.start_possession(team.other(), PossessionKind::Live);
        return ActionOutcome {
            actor,
            kind: ActionKind::Dribble,
            completion: Completion::Failed,
            delta: StateDelta {
                turnover: Some(TurnoverKind::DoubleDribble),
                ..Default::default()
            },
            facts: Vec::new(),
        };
    }
    // 只有站定的调整运球才以收球结束；移动中的运球是持球推进，
    // 收球留到真正的终结动作。
    let player = &world.players[actor];
    let stationary =
        (player.target - player.pos).length() < world.params.policy.g("gather_stationary_ft");
    if stationary {
        world.possession.dribble_state = DribbleState::Gathered;
        world.possession.gather_pos = Some(player.pos);
    } else {
        world.possession.dribble_state = DribbleState::Dribbling;
        world.possession.gather_pos = None;
    }
    ActionOutcome {
        actor,
        kind: ActionKind::Dribble,
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
    let attacking_right = team == TeamSide::Home;
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
    for defender in world
        .players
        .iter()
        .filter(|p| p.on_court && p.team != team)
    {
        let distance = (defender.pos - pos).length();
        if distance > 9.0 {
            continue;
        }
        let between = crate::state::is_between(defender.pos, pos, hoop);
        let contest = crate::state::contest_from_defender(distance, between);
        // 协防倾向高的防守人更早收缩，位置不变时干扰更强。
        let help = defender.data.tendencies.help_aggressiveness;
        let contest = contest
            * (world.params.policy.w("help_contest_base")
                + world.params.policy.w("help_contest_slope") * help);
        if contest > best {
            best = contest;
        }
    }
    best
}

pub fn weighted_pick<R: Rng>(rng: &mut R, weights: &[f32]) -> usize {
    let total: f32 = weights.iter().sum();
    assert!(total > 0.0, "action weights must not all be zero");
    let mut threshold = rng.gen::<f32>() * total;
    for (index, weight) in weights.iter().enumerate() {
        threshold -= weight;
        if threshold <= 0.0 {
            return index;
        }
    }
    weights.len() - 1
}

pub fn move_player(player: &mut PlayerState, target: Vec2, dt: f32) {
    let delta = target - player.pos;
    let distance = delta.length();
    let speed = player.effective_speed();
    if distance < 0.05 {
        player.vel = Vec2::ZERO;
        return;
    }
    let desired = delta / distance * speed;
    let accel = player.effective_acceleration();
    let max_change = accel * dt;
    let change = desired - player.vel;
    let change_length = change.length();
    if change_length > max_change {
        player.vel += change / change_length * max_change;
    } else {
        player.vel = desired;
    }
    player.pos += player.vel * dt;
    player.facing = delta.normalize_or_zero();
}
