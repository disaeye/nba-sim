//! 批4物理层守卫：朝向转动、加速度包络与垂直自由度。
//!
//! 从真实比赛的逐帧投影测量，不靠注入：单帧朝向翻转来自朝向直写绕过
//! 限速，加速度峰值来自位置被非运动路径瞬移。守卫把这些量钉在物理
//! 包络内，并确认球员的垂直坐标成为协议事实。

mod common;

use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(
        MatchSetup::builtin(GameRules::default().shortened(120.0, 60.0)),
        params,
        seed,
    )
}

#[derive(Default)]
struct PhysicsStats {
    flips: u64,
    max_accel: f32,
    frames: u64,
    with_z: u64,
    jumping_frames: u64,
}

fn measure(seed: u64) -> PhysicsStats {
    let mut world = world(seed);
    let mut stats = PhysicsStats::default();
    let mut prev_facing: Vec<Option<Vec2>> = vec![None; world.players.len()];
    let mut prev_vel: Vec<Option<Vec2>> = vec![None; world.players.len()];
    world.run_with(|w| {
        stats.frames += 1;
        let tick = w.stream_tick();
        if !tick.frame.players.is_empty()
            && tick.frame.players.iter().all(|p| p.z >= 0.0)
        {
            stats.with_z += 1;
        }
        stats.jumping_frames += tick
            .frame
            .players
            .iter()
            .filter(|p| p.z > 0.1)
            .count() as u64;
        for (handle, player) in w.players.iter().enumerate() {
            if !player.on_court {
                prev_facing[handle] = None;
                prev_vel[handle] = None;
                continue;
            }
            if let Some(prev) = prev_facing[handle] {
                let dot = prev.dot(player.facing).clamp(-1.0, 1.0);
                if dot < -0.5 {
                    stats.flips += 1;
                    eprintln!("FLIP seed={} t={:.2} tick={} player={} phase={:?} dot={:.2} prev={:?} now={:?}", seed, w.now(), w.clock.tick(), player.id, w.phase(), dot, prev, player.facing);
                }
            }
            if let Some(prev) = prev_vel[handle] {
                // 程序性列位（跳球、罚球、替补）会直接重置位置与速度，
                // 那是离散的程序迁移，不是连续运动，不进加速度包络。
                let program_placement = matches!(
                    w.phase(),
                    nba_sim_core::state::Phase::JumpBall { .. }
                );
                if !program_placement {
                    let accel = (player.vel - prev).length() / TICK_SECONDS as f32;
                    stats.max_accel = stats.max_accel.max(accel);
                }
            }
            prev_facing[handle] = Some(player.facing);
            prev_vel[handle] = Some(player.vel);
        }
    });
    stats
}

#[test]
fn physics_envelopes_hold_in_complete_matches() {
    let mut flips = 0u64;
    let mut max_accel = 0.0f32;
    for seed in 1..=4u64 {
        let stats = measure(seed);
        flips += stats.flips;
        max_accel = max_accel.max(stats.max_accel);
        eprintln!(
            "seed {seed}: frames={} flips={} max_accel={:.1}",
            stats.frames, stats.flips, stats.max_accel
        );
    }
    assert_eq!(flips, 0, "单帧朝向翻转次数={flips}");
    assert!(max_accel < 60.0, "最大加速度={max_accel:.1} ft/s²");
}

#[test]
fn players_carry_a_vertical_coordinate() {
    let stats = measure(1);
    assert_eq!(stats.with_z, stats.frames, "每帧球员都应带 z 坐标");
    assert!(stats.jumping_frames > 0, "完整比赛必须有起跳帧");
}
