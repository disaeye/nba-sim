//! 假动作：投篮假动作、传球假动作、试探步。
//!
//! 假动作不产生得分，它的产出是防守人的失衡窗口。窗口写在
//! `PlayerState::bait` 上，由干扰计算、抄截半径与切球判定分别读取，
//! 因此一次假动作的后果可以在后续几个真实结算里被观察到。

use crate::action::{ActionIntent, ActionKind, ActionOutcome, Completion, Fact, FakeKind, StateDelta};
use crate::state::{Bait, Phase};
use crate::world::World;

/// 最近的防守人：距离上限来自策略，太远的人没有被骗的意义。
fn nearest_defender(world: &World, actor: usize, radius: f32) -> Option<usize> {
    let team = world.players[actor].team;
    let pos = world.players[actor].pos;
    let mut best: Option<(usize, f32)> = None;
    for (handle, player) in world.players.iter().enumerate() {
        if !player.on_court || player.team == team {
            continue;
        }
        let gap = (player.pos - pos).length();
        if gap > radius {
            continue;
        }
        if best.is_none_or(|(_, nearest)| gap < nearest) {
            best = Some((handle, gap));
        }
    }
    best.map(|(handle, _)| handle)
}

pub fn resolve(world: &mut World, intent: &ActionIntent, kind: FakeKind) -> ActionOutcome {
    let actor = intent.actor;
    let now = world.now();
    let policy = world.rules.resolve.technique.clone();
    let defender = nearest_defender(world, actor, policy.fake_pressure_radius_ft);
    let delta = StateDelta::default();
    let mut displacement = f32::default();
    let mut bitten = false;
    if let Some(defender) = defender {
        let defense_skill = world.players[defender].attribute("defense_perimeter");
        let handler_skill = world.players[actor].attribute("ball_handling");
        let read = policy.read_probability(defense_skill, handler_skill);
        let roll = world.draw(crate::draw::Stream::Decision, actor, crate::draw::purpose::FAKE_READ, 0);
        bitten = roll >= read;
        if bitten {
            let until = now + f64::from(policy.fake_bait_seconds);
            match kind {
                FakeKind::Shot => {
                    // 骗起的防守人向持球人扑出去：这是一次真实的移动承诺，
                    // 由移动层按速度走完，不是瞬移；失衡窗口内他的干扰与
                    // 抄截半径同时折减。
                    let towards = (world.players[actor].pos - world.players[defender].pos)
                        .normalize_or_zero();
                    let step = policy.fake_shot_displacement_ft;
                    let target = world.players[defender].pos + towards * step;
                    crate::assignment::force_target_as(
                        world,
                        defender,
                        target,
                        crate::state::TargetKind::Window,
                        actor,
                    );
                    displacement = step;
                }
                FakeKind::Pass => {
                    // 传球假动作把防守人的重心引向假目标：他转向，
                    // 真实传球线路上的手位因此离开线路。
                    let fake = intent
                        .target
                        .filter(|target| *target < world.players.len())
                        .unwrap_or(actor);
                    let aim = world.players[fake].pos - world.players[defender].pos;
                    world.players[defender].facing = aim.normalize_or_zero();
                }
                FakeKind::Hesitation => {
                    // 迟疑步是运球技术，不走这条假动作路径。
                }
                FakeKind::JabStep => {
                    // 试探步向篮筐方向跨出：持球人与防守人都走真实脚步，
                    // 位移由移动层按速度完成。
                    let hoop = world.hoop_for(world.players[actor].team);
                    let towards = (hoop - world.players[actor].pos).normalize_or_zero();
                    let step = policy.jab_step_ft;
                    let holder_target = world.players[actor].pos + towards * step;
                    crate::assignment::force_target_as(
                        world,
                        actor,
                        holder_target,
                        crate::state::TargetKind::Window,
                        actor,
                    );
                    let defender_target = world.players[defender].pos + towards * step;
                    crate::assignment::force_target_as(
                        world,
                        defender,
                        defender_target,
                        crate::state::TargetKind::Window,
                        actor,
                    );
                    displacement = step;
                }
            }
            world.players[defender].bait = Some(Bait {
                kind,
                until,
                displacement_ft: displacement,
            });
            // 失衡的人无法继续贴身执行原来的动作：取消他的防守动作。
            world.cancel_player_action(defender, "faked_out");
        }
    }
    world.emit(Fact::FakeAction {
        player: actor,
        defender,
        kind,
        bitten,
        displacement_ft: displacement,
    });
    ActionOutcome {
        actor,
        kind: ActionKind::Fake(kind),
        completion: Completion::Completed,
        delta,
        facts: Vec::new(),
    }
}

/// 假动作只在活球且持球人未被收球时成立。
pub fn available(world: &World, actor: usize) -> bool {
    matches!(world.phase(), Phase::Live)
        && world.holder() == Some(actor)
        && world.possession.dribble_state == crate::state::DribbleState::Dribbling
}
