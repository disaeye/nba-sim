//! 出手结算：干扰、技巧、体能进 logit，命中率来自校准表。

use glam::Vec2;
use rand::Rng;

use crate::action::{ActionIntent, ActionKind, ActionOutcome, Completion, ShotKind, StateDelta};
use crate::resolve::{clamp01, contest_for, shooter_zone, sigmoid};
use crate::state::BallState;
use crate::world::{Phase, World};

#[derive(Debug, Clone)]
pub struct PendingShot {
    pub shooter: usize,
    pub kind: ShotKind,
    pub zone: &'static str,
    pub distance_ft: f32,
    pub contest: f32,
    pub made: bool,
    pub fouled: bool,
    pub free_throws: u8,
    pub points: u8,
    pub is_three: bool,
    pub assisted_by: Option<usize>,
    pub blocked_by: Option<usize>,
    pub shot_id: u64,
}

pub fn resolve(world: &mut World, intent: &ActionIntent, kind: ShotKind) -> ActionOutcome {
    let shooter = intent.actor;
    let team = world.players[shooter].team;
    let hoop = world.hoop_for(team);
    let pos = world.players[shooter].pos;
    let zone = shooter_zone(world, pos, team);
    let is_three = zone == "three";
    let distance = (pos - hoop).length();
    let contest = contest_for(world, shooter, pos);
    let table = &world.params.shot_make;
    let base = table.base_rate(kind.as_str(), distance, is_three);
    let skill = world.players[shooter].shooting_attribute(zone);
    let spread = table.skill_spread(zone);
    let stamina = world.players[shooter].stamina;
    let p_skill = (base + (skill - 0.5) * spread).clamp(0.02, 0.98);
    let logit = logit(p_skill)
        + table.contest_sensitivity * (table.contest_center - contest)
        + table.stamina_sensitivity * (stamina - 1.0);
    let p_make = sigmoid(logit).clamp(0.02, 0.98);

    let foul_rate = world.params.policy.w("shooting_foul_rate");
    let p_foul = clamp01(foul_rate * (0.6 + 0.9 * contest));
    let fouled = world.rng.gen::<f32>() < p_foul;
    let mut made = world.rng.gen::<f32>() < p_make;
    let shot_id = world.new_shot_id();
    let blocker = if made {
        None
    } else {
        nearest_blocker(world, shooter, pos, contest)
    };
    if blocker.is_some() {
        made = false;
    }
    let (free_throws, points) = if made && fouled {
        (1, if is_three { 3 } else { 2 })
    } else if fouled {
        (if is_three { 3 } else { 2 }, 0)
    } else {
        (
            0,
            if made {
                if is_three {
                    3
                } else {
                    2
                }
            } else {
                0
            },
        )
    };

    let assisted_by = if made {
        world
            .possession
            .assist_candidate
            .filter(|(passer, time)| *passer != shooter && world.now() - *time < 4.0)
            .map(|(passer, _)| passer)
    } else {
        None
    };

    world.possession.had_shot = true;
    let just_received = world
        .possession
        .last_pass_at
        .map(|time| world.now() - time < 1.2)
        .unwrap_or(false);
    match kind.creation() {
        "set" if !just_received => world
            .violations
            .push("context: catch-and-shoot without a preceding pass".to_string()),
        "putback" if !world.possession.had_offensive_rebound => world
            .violations
            .push("context: putback without an offensive rebound".to_string()),
        "cut" if world.possession.last_cut_at.is_none() => world
            .violations
            .push("context: cut finish without a cut action".to_string()),
        "post" if world.possession.last_post_at.is_none() => world
            .violations
            .push("context: post finish without a post action".to_string()),
        "drive" if world.possession.last_drive_at.is_none() => world
            .violations
            .push("context: drive finish without a drive action".to_string()),
        _ => {}
    }
    world.shot_clock_at_release.push(world.clock.shot_clock);
    world.contest_samples.push(contest);
    let zone_index = match zone {
        "rim" => 0,
        "near" => 1,
        "mid" => 2,
        _ => 3,
    };
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
        made,
        fouled,
        free_throws,
        points,
        is_three,
        assisted_by,
        blocked_by: blocker,
        shot_id,
    });
    world.players[shooter].facing = (hoop - pos).normalize_or_zero();
    world.ball = BallState::Shot {
        shooter,
        from_pos: pos,
        to_pos: hoop,
        start: world.now(),
        duration: 1.1,
        made,
    };
    world.phase = Phase::ShotFlight {
        resolve_at: world.now() + 1.1,
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
    let shooter = pending.shooter;
    let team = world.players[shooter].team;
    let line = world.line_mut(shooter);
    line.fga += 1;
    if pending.is_three {
        line.tpa += 1;
    }
    if pending.made {
        line.fgm += 1;
        if pending.is_three {
            line.tpm += 1;
        }
        line.points += pending.points as u32;
        let made_zone = match pending.zone {
            "rim" => 0,
            "near" => 1,
            "mid" => 2,
            _ => 3,
        };
        world.shot_zone_makes[crate::world::World::team_index(team)][made_zone] += 1;
        world.add_points(team, pending.points as u32);
        if let Some(passer) = pending.assisted_by {
            world.line_mut(passer).assists += 1;
        }
    }
    if let Some(blocker) = pending.blocked_by {
        world.line_mut(blocker).blocks += 1;
    }
    world.emit(crate::action::Fact::Shot {
        shot_id: pending.shot_id,
        shooter,
        kind: pending.kind,
        zone: pending.zone,
        distance_ft: pending.distance_ft,
        contest: pending.contest,
        made: pending.made,
        points: pending.points,
        assisted_by: pending.assisted_by,
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
            pending.free_throws,
            pending.made,
        );
        return;
    }
    if pending.made {
        world.end_possession("score", pending.points as u32);
        world.phase = Phase::DeadBall {
            team: team.other(),
            ready_at: world.now() + 1.4,
        };
    } else {
        crate::resolve::rebound::resolve(world, pending.zone, pending.distance_ft);
    }
}

fn logit(p: f32) -> f32 {
    let p = p.clamp(1e-4, 1.0 - 1e-4);
    (p / (1.0 - p)).ln()
}

/// 封盖：出手点附近、封盖能力强的防守人才能碰到球。
fn nearest_blocker(world: &mut World, shooter: usize, pos: Vec2, contest: f32) -> Option<usize> {
    let team = world.players[shooter].team;
    let candidate = world
        .players
        .iter()
        .enumerate()
        .filter(|(_, p)| p.on_court && p.team != team)
        .filter(|(_, p)| (p.pos - pos).length() < 4.5)
        .max_by(|a, b| {
            a.1.data
                .attributes
                .block
                .partial_cmp(&b.1.data.attributes.block)
                .expect("finite block attributes")
        })
        .map(|(index, _)| index)?;
    let skill = world.players[candidate].data.attributes.block;
    let vertical = world.players[candidate].data.attributes.vertical;
    let base = world.params.policy.w("block_base");
    let p_block =
        (base * (0.4 + 1.2 * skill) * (0.5 + 0.5 * vertical) * (0.4 + contest)).clamp(0.0, 0.30);
    if world.rng.gen::<f32>() < p_block {
        Some(candidate)
    } else {
        None
    }
}

pub fn shot_origin(shooter_pos: Vec2, hoop: Vec2) -> Vec2 {
    let direction = (hoop - shooter_pos).normalize_or_zero();
    shooter_pos + direction * 0.4
}
