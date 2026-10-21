//! 按回合回放：只输出指定回合的帧流。
//!
//! 整场逐帧序列化需要两秒以上，按回合取帧只需要一次推演。回合身份由
//! `World::start_possession` 分配的单调 id 给出，因此同一 `(seed, possession_id)`
//! 的帧流是确定的。

use std::path::Path;

use nba_protocol::StreamTick;

use crate::params::Params;
use crate::world::{MatchSetup, World, TICK_SECONDS};

/// 单个回合的帧流。
#[derive(Debug)]
pub struct PossessionFrames {
    pub possession_id: u64,
    pub frames: Vec<StreamTick>,
}

impl PossessionFrames {
    /// 帧流的 JSON 行，供逐字节比较与按需下发。
    pub fn ndjson(&self) -> Vec<String> {
        self.frames
            .iter()
            .map(|frame| serde_json::to_string(frame).expect("stream tick serializes"))
            .collect()
    }
}

/// 重放指定回合的帧流。`possession_id` 取自 `GameReport::possessions[].id`。
pub fn replay_possession(
    seed: u64,
    calibration_dir: &Path,
    possession_id: u64,
) -> Result<PossessionFrames, String> {
    let params = Params::load(calibration_dir)?;
    let setup = MatchSetup::builtin(nba_domain::GameRules::default());
    let mut world = World::new(setup, params, seed);
    let mut frames = Vec::new();
    let mut entered = false;
    while !world.finished {
        world.step(TICK_SECONDS);
        if world.possession.id == possession_id {
            entered = true;
            frames.push(world.stream_tick());
        } else if entered {
            break;
        }
    }
    if !entered {
        return Err(format!(
            "seed {seed} produced no possession {possession_id}"
        ));
    }
    Ok(PossessionFrames {
        possession_id,
        frames,
    })
}
