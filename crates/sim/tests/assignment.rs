use std::collections::BTreeMap;

use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_sim_core::action::Fact;
use nba_sim_core::params::{workspace_calibration_dir, AcceptanceTable, ValidationParams};
use nba_sim_core::state::{DefenseResponsibility, OffenseJob, TargetKind};
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};

/// 恢复段允许的额外路径（ft）：与验收表 `recovery_path_extra_ft` 一致。
const RECOVERY_PATH_EXTRA_FT: f64 = 22.62;

mod common;

fn world(seed: u64) -> World {
    common::short_world(seed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Offense(OffenseJob),
    Defense(DefenseResponsibility),
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Self::Offense(job) => job.as_str(),
            Self::Defense(responsibility) => responsibility.as_str(),
        }
    }

    fn is_on_ball(self) -> bool {
        self == Self::Defense(DefenseResponsibility::OnBall)
    }
}

#[derive(Clone, Debug)]
struct Segment {
    seed: u64,
    handle: usize,
    role: Role,
    man: Option<usize>,
    kind: TargetKind,
    partner: usize,
    recovery: bool,
    recovery_d0: f64,
    recovery_marker: f64,
    start_pos: Vec2,
    initial_target_distance: f64,
    path: f64,
    path_motion: f64,
    path_separation: f64,
    target_path: f64,
    paired_path: f64,
    paired_valid: bool,
    net: f64,
    progress: f64,
    nonprogress_max: f64,
    yielding_seconds: f64,
    yielding_current: f64,
    yielding_continuous_max: f64,
    invalid_commits: u32,
    invalid_by_kind: [u32; 4],
    side_switches: u32,
    side: i8,
    reachable: bool,
    ticks: u32,
    changes: u32,
}

impl Segment {
    fn new(seed: u64, handle: usize, role: Role, man: Option<usize>, world: &World) -> Self {
        Self {
            seed,
            handle,
            role,
            man,
            kind: world.players[handle].target_kind,
            partner: world.players[handle].target_partner,
            recovery: world.players[handle].yielding_since > 0.0
                || world.players[handle].recovery_since > 0.0
                || (role.is_on_ball()
                    && (world.players[handle].target - world.players[handle].pos).length()
                        > world.rules.min_player_separation_ft),
            recovery_d0: (world.players[handle].target - world.players[handle].pos).length() as f64,
            recovery_marker: world.players[handle].recovery_since,
            start_pos: world.players[handle].pos,
            initial_target_distance: (world.players[handle].target - world.players[handle].pos)
                .length() as f64,
            path: 0.0,
            path_motion: 0.0,
            path_separation: 0.0,
            target_path: 0.0,
            paired_path: 0.0,
            paired_valid: role.is_on_ball() && man.is_some(),
            net: 0.0,
            progress: 0.0,
            nonprogress_max: 0.0,
            yielding_seconds: 0.0,
            yielding_current: 0.0,
            yielding_continuous_max: 0.0,
            invalid_commits: 0,
            invalid_by_kind: [0; 4],
            side_switches: 0,
            side: 0,
            reachable: true,
            ticks: 0,
            changes: 0,
        }
    }

    fn finish(mut self, world: &World) -> Self {
        self.net = (world.players[self.handle].pos - self.start_pos).length() as f64;
        let remaining =
            (world.players[self.handle].target - world.players[self.handle].pos).length() as f64;
        self.progress = self.initial_target_distance - remaining;
        self
    }
}

fn role_for(world: &World, handle: usize) -> Role {
    if world.players[handle].team == world.possession.team {
        Role::Offense(world.players[handle].offense_job)
    } else {
        Role::Defense(world.players[handle].defense_responsibility)
    }
}

fn matchup_man(world: &World, handle: usize) -> Option<usize> {
    let defense = world.possession.team.other();
    let defenders = world.on_court(defense);
    let slot = defenders
        .as_slice()
        .iter()
        .position(|candidate| *candidate == handle)?;
    let man = world.possession.matchup[slot];
    if man == usize::MAX || !world.players[man].on_court {
        return None;
    }
    Some(man)
}

fn start_segment(seed: u64, handle: usize, world: &World, active: &mut [Option<Segment>]) {
    let role = role_for(world, handle);
    let man = if role.is_on_ball() {
        matchup_man(world, handle)
    } else {
        None
    };
    active[handle] = Some(Segment::new(seed, handle, role, man, world));
}

fn finish_segment(
    handle: usize,
    world: &World,
    active: &mut [Option<Segment>],
    segments: &mut Vec<Segment>,
) {
    if let Some(segment) = active[handle].take() {
        segments.push(segment.finish(world));
    }
}

fn collect_segments(seed: u64) -> Vec<Segment> {
    let mut world = world(seed);
    let mut active: Vec<Option<Segment>> = (0..world.players.len()).map(|_| None).collect();
    let mut segments = Vec::new();
    for handle in 0..world.players.len() {
        if world.players[handle].on_court {
            start_segment(seed, handle, &world, &mut active);
        }
    }
    let mut possession = world.possession.id;
    let mut period = world.clock.period();
    let mut tick = 0u32;
    while !world.finished && tick < 100_000 {
        let before: Vec<Vec2> = world.players.iter().map(|player| player.pos).collect();
        let targets: Vec<Vec2> = world.players.iter().map(|player| player.target).collect();
        let invalid_before: Vec<[u32; 4]> = world
            .players
            .iter()
            .map(|player| player.invalid_commits_by_kind)
            .collect();
        world.step(TICK_SECONDS);
        tick += 1;
        if world.clock.period() != period {
            for handle in 0..world.players.len() {
                finish_segment(handle, &world, &mut active, &mut segments);
            }
            period = world.clock.period();
            possession = world.possession.id;
            continue;
        }
        for handle in 0..world.players.len() {
            if !world.players[handle].on_court {
                finish_segment(handle, &world, &mut active, &mut segments);
                continue;
            }
            let Some(segment) = active[handle].as_mut() else {
                start_segment(seed, handle, &world, &mut active);
                continue;
            };
            segment.ticks += 1;
            for (index, before) in invalid_before[handle].iter().enumerate() {
                let delta = world.players[handle].invalid_commits_by_kind[index] - before;
                segment.invalid_commits += delta;
                segment.invalid_by_kind[index] += delta;
            }
            segment.path += (world.players[handle].pos - before[handle]).length() as f64;
            segment.path_motion += f64::from(world.players[handle].last_motion_ft);
            segment.path_separation += f64::from(world.players[handle].last_separation_ft);
            segment.nonprogress_max = segment
                .nonprogress_max
                .max(world.players[handle].nonprogress_seconds);
            let target_changed = targets[handle] != world.players[handle].target;
            if target_changed {
                segment.yielding_current = 0.0;
            }
            if world.players[handle].yielding_since > 0.0 {
                segment.yielding_seconds += TICK_SECONDS;
                segment.yielding_current += TICK_SECONDS;
                segment.yielding_continuous_max = segment
                    .yielding_continuous_max
                    .max(segment.yielding_current);
            } else {
                segment.yielding_current = 0.0;
            }
            let side = world.players[handle].avoidance_side;
            if side != 0 {
                if segment.side != 0 && segment.side != side {
                    segment.side_switches += 1;
                }
                segment.side = side;
            } else {
                segment.side = 0;
            }
            segment.target_path += (world.players[handle].target - targets[handle]).length() as f64;
            if target_changed {
                segment.changes += 1;
            }
            if segment.role.is_on_ball() {
                match segment.man {
                    Some(man) if world.players[man].on_court && segment.paired_valid => {
                        segment.paired_path +=
                            (world.players[man].pos - before[man]).length() as f64;
                    }
                    _ => segment.paired_valid = false,
                }
            }
            let role = role_for(&world, handle);
            let man = if role.is_on_ball() {
                matchup_man(&world, handle)
            } else {
                None
            };
            let recovery_now = world.players[handle].yielding_since > 0.0
                || world.players[handle].recovery_since > 0.0
                || (role.is_on_ball()
                    && (world.players[handle].target - world.players[handle].pos).length()
                        > world.rules.min_player_separation_ft);
            let recovery_changed = segment.recovery_marker != world.players[handle].recovery_since;
            let kind_changed = segment.kind != world.players[handle].target_kind
                || segment.partner != world.players[handle].target_partner;
            if segment.role != role
                || (role.is_on_ball() && segment.man != man)
                || segment.recovery != recovery_now
                || recovery_changed
                || target_changed
                || kind_changed
            {
                finish_segment(handle, &world, &mut active, &mut segments);
                start_segment(seed, handle, &world, &mut active);
            }
        }
        if world.possession.id != possession || world.clock.period() != period {
            for handle in 0..world.players.len() {
                finish_segment(handle, &world, &mut active, &mut segments);
            }
            possession = world.possession.id;
            period = world.clock.period();
        }
    }
    for handle in 0..world.players.len() {
        finish_segment(handle, &world, &mut active, &mut segments);
    }
    assert!(
        world.finished,
        "seed {seed} did not finish within {tick} ticks"
    );
    segments
}

fn percentile(values: &mut [f64], fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let index = ((values.len() - 1) as f64 * fraction).round() as usize;
    values[index]
}

fn print_distribution(segments: &[Segment]) {
    let min_separation = f64::from(GameRules::default().min_player_separation_ft);
    let mut by_role: BTreeMap<&'static str, Vec<&Segment>> = BTreeMap::new();
    for segment in segments {
        by_role
            .entry(segment.role.as_str())
            .or_default()
            .push(segment);
    }
    println!(
        "S3 segment distribution over 32 seeds: {} segments",
        segments.len()
    );
    for (role, list) in &by_role {
        let mut ratios: Vec<f64> = list
            .iter()
            .filter(|segment| segment.net > 5.0)
            .map(|segment| segment.path / segment.net)
            .collect();
        let measured = ratios.len();
        let p50 = percentile(&mut ratios, 0.50);
        let p95 = percentile(&mut ratios, 0.95);
        let max = ratios.last().copied().unwrap_or(0.0);
        let mut changes: Vec<f64> = list
            .iter()
            .map(|segment| f64::from(segment.changes))
            .collect();
        let changes_p95 = percentile(&mut changes, 0.95);
        let changes_max = changes.last().copied().unwrap_or(0.0);
        let mut durations: Vec<f64> = list
            .iter()
            .map(|segment| f64::from(segment.ticks) * TICK_SECONDS)
            .collect();
        let duration_p50 = percentile(&mut durations, 0.50);
        let yielding_max = list
            .iter()
            .map(|segment| segment.yielding_seconds)
            .fold(0.0_f64, f64::max);
        let nonprogress_max = list
            .iter()
            .map(|segment| segment.nonprogress_max)
            .fold(0.0_f64, f64::max);
        let side_switches_max = list
            .iter()
            .map(|segment| segment.side_switches)
            .max()
            .unwrap_or(0);
        let unreachable = list.iter().filter(|segment| !segment.reachable).count();
        let motion: f64 = list.iter().map(|segment| segment.path_motion).sum();
        let separation: f64 = list.iter().map(|segment| segment.path_separation).sum();
        println!(
            "{role}: n={} measured={} ratio p50={p50:.2} p95={p95:.2} max={max:.2}; changes p95={changes_p95:.0} max={changes_max:.0}; duration_p50={duration_p50:.2}s; yield_max={yielding_max:.2}s nonprogress_max={nonprogress_max:.2}s side_switches_max={side_switches_max} unreachable={unreachable} motion={motion:.0} separation={separation:.0}",
            list.len(),
            measured
        );
        if *role == "on_ball" {
            let mut follows: Vec<f64> = list
                .iter()
                .filter(|segment| segment.paired_valid && segment.paired_path > 5.0)
                .map(|segment| segment.path / segment.paired_path)
                .collect();
            let follow_p50 = percentile(&mut follows, 0.50);
            let follow_p95 = percentile(&mut follows, 0.95);
            let follow_max = follows.last().copied().unwrap_or(0.0);
            let follow_min = follows.first().copied().unwrap_or(0.0);
            println!(
                "  follow: n={} p50={follow_p50:.2} p95={follow_p95:.2} min={follow_min:.2} max={follow_max:.2}",
                follows.len()
            );
        }
    }
    println!("S3 stable/recovery envelope:");
    for role in by_role.keys() {
        let role = *role;
        let stable: Vec<&Segment> = segments
            .iter()
            .filter(|segment| segment.role.as_str() == role && !segment.recovery)
            .collect();
        let recovery: Vec<&Segment> = segments
            .iter()
            .filter(|segment| segment.role.as_str() == role && segment.recovery)
            .collect();
        let over = recovery
            .iter()
            .filter(|segment| {
                segment.path > segment.recovery_d0 + 2.0 * std::f64::consts::PI * min_separation
            })
            .count();
        let yield_cont = recovery
            .iter()
            .map(|segment| segment.yielding_continuous_max)
            .fold(0.0_f64, f64::max);
        let invalid: u32 = segments
            .iter()
            .filter(|segment| segment.role.as_str() == role)
            .map(|segment| segment.invalid_commits)
            .sum();
        let side = segments
            .iter()
            .filter(|segment| segment.role.as_str() == role)
            .map(|segment| segment.side_switches)
            .max()
            .unwrap_or(0);
        if role == "on_ball" {
            let mut follows: Vec<f64> = stable
                .iter()
                .filter(|segment| segment.paired_valid && segment.paired_path > 5.0)
                .map(|segment| segment.path / segment.paired_path)
                .collect();
            let p95 = percentile(&mut follows, 0.95);
            let max = follows.last().copied().unwrap_or(0.0);
            println!(
                "{role}: stable={} follow_p95={p95:.2} follow_max={max:.2}; recovery={} over={over} yield_cont={yield_cont:.2}s invalid={invalid} side_switches={side}",
                stable.len(),
                recovery.len()
            );
        } else {
            let mut ratios: Vec<f64> = stable
                .iter()
                .filter(|segment| segment.net > 5.0)
                .map(|segment| segment.path / segment.net)
                .collect();
            let p95 = percentile(&mut ratios, 0.95);
            let max = ratios.last().copied().unwrap_or(0.0);
            println!(
                "{role}: stable={} ratio_p95={p95:.2} ratio_max={max:.2}; recovery={} over={over} yield_cont={yield_cont:.2}s invalid={invalid} side_switches={side}",
                stable.len(),
                recovery.len()
            );
        }
    }
    let mut worst: Vec<(&Segment, f64)> = segments
        .iter()
        .filter(|segment| segment.net > 5.0)
        .map(|segment| (segment, segment.path / segment.net))
        .collect();
    worst.sort_by(|left, right| right.1.total_cmp(&left.1));
    println!("worst path/net segments:");
    for (segment, ratio) in worst.iter().take(10) {
        println!(
            "  seed={} handle={} role={} ratio={ratio:.2} path={:.2} motion={:.2} separation={:.2} target_path={:.2} net={:.2} ticks={} changes={} yield={:.2} nonprogress={:.2} side_switches={} reachable={} recovery={} d0={:.2}",
            segment.seed,
            segment.handle,
            segment.role.as_str(),
            segment.path,
            segment.path_motion,
            segment.path_separation,
            segment.target_path,
            segment.net,
            segment.ticks,
            segment.changes,
            segment.yielding_seconds,
            segment.nonprogress_max,
            segment.side_switches,
            segment.reachable,
            segment.recovery,
            segment.recovery_d0
        );
    }
    // 恢复段超出上限的样本：路径长度必须由这一段起点的目标距离计量。
    println!("recovery segments over the allowance:");
    for segment in segments
        .iter()
        .filter(|segment| segment.recovery && segment.role.as_str() != "handler")
        .filter(|segment| segment.path > segment.recovery_d0 + RECOVERY_PATH_EXTRA_FT)
        .take(10)
    {
        println!(
            "  seed={} handle={} role={} path={:.2} d0={:.2} limit={:.2} ticks={} changes={} yield={:.2} nonprogress={:.2} target_path={:.2} net={:.2} motion={:.2} separation={:.2}",
            segment.seed,
            segment.handle,
            segment.role.as_str(),
            segment.path,
            segment.recovery_d0,
            segment.recovery_d0 + RECOVERY_PATH_EXTRA_FT,
            segment.ticks,
            segment.changes,
            segment.yielding_seconds,
            segment.nonprogress_max,
            segment.target_path,
            segment.net,
            segment.path_motion,
            segment.path_separation
        );
    }
    let mut worst_follow: Vec<(&Segment, f64)> = segments
        .iter()
        .filter(|segment| segment.paired_valid && segment.paired_path > 5.0)
        .map(|segment| (segment, segment.path / segment.paired_path))
        .collect();
    worst_follow.sort_by(|left, right| right.1.total_cmp(&left.1));
    println!("worst on-ball follow segments:");
    for (segment, ratio) in worst_follow.iter().take(10) {
        println!(
            "  seed={} handle={} follow={ratio:.2} path={:.2} target_path={:.2} paired={:.2} net={:.2} ticks={} changes={}",
            segment.seed,
            segment.handle,
            segment.path,
            segment.target_path,
            segment.paired_path,
            segment.net,
            segment.ticks,
            segment.changes
        );
    }
}

fn assert_envelope(segments: &[Segment], acceptance: &AcceptanceTable) {
    let s3 = &acceptance.s3;
    for segment in segments {
        assert_eq!(
            segment.side_switches,
            0,
            "seed {} handle {} role {} switched avoidance side",
            segment.seed,
            segment.handle,
            segment.role.as_str()
        );
        assert!(
            segment.yielding_continuous_max <= s3.max_continuous_yielding_seconds,
            "seed {} handle {} role {} continuous yielding {:.2}s above {:.2}s",
            segment.seed,
            segment.handle,
            segment.role.as_str(),
            segment.yielding_continuous_max,
            s3.max_continuous_yielding_seconds
        );
        if segment.recovery {
            if segment.role.as_str() != "handler" {
                let limit = segment.recovery_d0 + s3.recovery_path_extra_ft;
                assert!(
                    segment.path <= limit + 0.01,
                    "seed {} handle {} role {} recovery path {:.2} above limit {:.2}",
                    segment.seed,
                    segment.handle,
                    segment.role.as_str(),
                    segment.path,
                    limit
                );
            }
        } else if segment.role.is_on_ball() {
            if segment.paired_valid && segment.paired_path > s3.on_ball_paired_min_ft {
                let ratio = segment.path / segment.paired_path;
                assert!(
                    ratio <= s3.on_ball_follow_ratio,
                    "seed {} handle {} stable follow {:.2} above {:.2}",
                    segment.seed,
                    segment.handle,
                    ratio,
                    s3.on_ball_follow_ratio
                );
            }
        } else if segment.net > 5.0 {
            let cap = s3
                .stable_path_ratio
                .get(segment.role.as_str())
                .copied()
                .unwrap_or(f64::MAX);
            let ratio = segment.path / segment.net;
            assert!(
                ratio <= cap + 0.01,
                "seed {} handle {} role {} stable ratio {:.2} above cap {:.2}",
                segment.seed,
                segment.handle,
                segment.role.as_str(),
                ratio,
                cap
            );
        }
        assert_eq!(
            segment.invalid_by_kind,
            [0; 4],
            "seed {} handle {} role {} had invalid commits {:?}",
            segment.seed,
            segment.handle,
            segment.role.as_str(),
            segment.invalid_by_kind
        );
    }
}

#[test]
fn role_segments_are_measured_by_responsibility() {
    // 三十二个种子各自独立采段，四个线程分摊后按种子顺序汇合。
    let seeds: Vec<u64> = (1..=32u64).collect();
    let chunk_size = seeds.len().div_ceil(4);
    let all: Vec<Segment> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .filter_map(|chunk| {
                let start = chunk * chunk_size;
                let end = (start + chunk_size).min(seeds.len());
                (start < end).then(|| &seeds[start..end])
            })
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .flat_map(|seed| collect_segments(*seed))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("segment thread must not panic"))
            .flatten()
            .collect::<Vec<_>>()
    });
    print_distribution(&all);
    let validation =
        ValidationParams::load(&workspace_calibration_dir()).expect("validation tables load");
    assert_envelope(&all, &validation.acceptance);
    assert!(!all.is_empty(), "no responsibility segment was measured");
}

#[test]
fn off_ball_facts_are_emitted() {
    let mut world = world(73);
    let report = world.run();
    let mut screen = false;
    let mut cut = false;
    let mut box_out = false;
    for event in &report.events {
        match event.fact {
            Fact::Screen { .. } => screen = true,
            Fact::Cut { .. } => cut = true,
            Fact::BoxOut { .. } => box_out = true,
            _ => {}
        }
    }
    assert!(screen, "no screen fact");
    assert!(cut, "no cut fact");
    assert!(box_out, "no box-out fact");
}
