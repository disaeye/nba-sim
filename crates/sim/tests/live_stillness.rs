//! 活球静止守卫：比赛钟走字阶段不允许出现全员静止的长窗口。
//!
//! 真实篮球的活球阶段不存在十人完全静止：持球人运球试探、无球人跑位、
//! 防守人滑步。停表期间的列位静止（发球、罚球、暂停）不在此列——本守卫
//! 只统计比赛钟与进攻钟都在走、且阶段为 Live 的帧。

use nba_domain::rules::GameRules;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};

const COURT_WIDTH_FT: f32 = 94.0;
const COURT_HEIGHT_FT: f32 = 50.0;
/// 全员速度都低于该值视为静止（ft/s）。
const STILL_SPEED_FTPS: f32 = 1.0;
/// 单个静止窗口的时长上限（秒）。
///
/// 上限的依据是最长的合法近静止复合段：收球观察、出手蓄力与出手收势
/// 叠加约 2.1 秒，运球试探周期（准备、横移、收势与下一个决策间隙）
/// 实测最长约 2.8 秒，均低于 3 秒。多秒雕像（修复前实测 4.6~7.3 秒，
/// 无人移动、无人出手、事件流静默）在任何比赛语义下都不成立。
const MAX_STILL_SECONDS: f64 = 3.0;

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

/// 一场比赛里最长的活球全员静止窗口（秒）。
fn longest_live_stillness(world: &mut World) -> (f64, f64) {
    let mut worst = 0.0f64;
    let mut worst_at = 0.0f64;
    let mut still_since: Option<f64> = None;
    let mut prev: Option<(f64, Vec<(f32, f32)>)> = None;
    world.run_with(|w| {
        let tick = w.stream_tick();
        let frame = &tick.frame;
        let t_game = f64::from(frame.t_game);
        let live = matches!(w.phase(), nba_sim_core::state::Phase::Live)
            && prev.as_ref().is_some_and(|(t, _)| t_game > *t);
        if live {
            let positions: Vec<(f32, f32)> = frame
                .players
                .iter()
                .map(|p| (p.x * COURT_WIDTH_FT, p.y * COURT_HEIGHT_FT))
                .collect();
            if let Some((_, before)) = &prev {
                if before.len() == positions.len() {
                    let fastest = before
                        .iter()
                        .zip(&positions)
                        .map(|(a, b)| {
                            ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
                                / (TICK_SECONDS as f32)
                        })
                        .fold(0.0f32, f32::max);
                    if fastest < STILL_SPEED_FTPS {
                        let start = still_since.unwrap_or(t_game - TICK_SECONDS);
                        still_since = Some(start);
                        let duration = t_game - start;
                        if duration > worst {
                            worst = duration;
                            worst_at = start;
                        }
                    } else {
                        still_since = None;
                    }
                }
            }
        } else {
            still_since = None;
        }
        prev = Some((t_game, {
            let mut v = Vec::with_capacity(frame.players.len());
            for p in &frame.players {
                v.push((p.x * COURT_WIDTH_FT, p.y * COURT_HEIGHT_FT));
            }
            v
        }));
    });
    (worst, worst_at)
}

#[test]
fn live_play_never_freezes_everyone() {
    let mut worst_all = 0.0f64;
    let mut worst_seed = 0u64;
    let mut worst_at = 0.0f64;
    for seed in 1..=4u64 {
        let mut w = world(seed);
        let (worst, at) = longest_live_stillness(&mut w);
        if worst > worst_all {
            worst_all = worst;
            worst_seed = seed;
            worst_at = at;
        }
    }
    assert!(
        worst_all < MAX_STILL_SECONDS,
        "活球全员静止窗口 {worst_all:.2}s（seed {worst_seed}，t_game {worst_at:.2}），\
         超过上限 {MAX_STILL_SECONDS}s"
    );
}
