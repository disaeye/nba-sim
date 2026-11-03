//! 护球姿态：面框、背身、转身三种护球技术。
//!
//! 护球不改位置也不改球权，它把持球人放进一个窗口：窗口内防守人的
//! 切球成功率按策略折减。窗口由 `World::protection` 保存，切球判定读取它，
//! 开始任何其他动作时取消。

use crate::action::{
    ActionIntent, ActionKind, ActionOutcome, Completion, Fact, ProtectKind, StateDelta,
};
use crate::state::BallProtection;
use crate::world::World;

pub fn resolve(world: &mut World, intent: &ActionIntent, kind: ProtectKind) -> ActionOutcome {
    let actor = intent.actor;
    let seconds = world.rules.resolve.technique.protection_seconds;
    let until = world.now() + f64::from(seconds);
    world.protection = Some(BallProtection {
        holder: actor,
        kind,
        until,
    });
    // 背身与转身护球把身体转向防守人：球被挡在远离防守的一侧。
    if let Some(defender) = intent.target.filter(|target| *target < world.players.len()) {
        let away = world.players[defender].pos - world.players[actor].pos;
        let facing = match kind {
            ProtectKind::FaceUp => away.normalize_or_zero(),
            ProtectKind::Post => -away.normalize_or_zero(),
            ProtectKind::Spin => {
                let axis = away.normalize_or_zero();
                glam::Vec2::new(-axis.y, axis.x)
            }
        };
        world.players[actor].facing = facing;
    }
    world.emit(Fact::BallProtection {
        player: actor,
        kind,
        seconds,
    });
    ActionOutcome {
        actor,
        kind: ActionKind::Protect(kind),
        completion: Completion::Completed,
        delta: StateDelta::default(),
        facts: Vec::new(),
    }
}

/// 护球只在活球、持球人未被收球时成立。
pub fn available(world: &World, actor: usize) -> bool {
    world.live_play()
        && world.holder() == Some(actor)
        && world.possession.dribble_state == crate::state::DribbleState::Dribbling
}
