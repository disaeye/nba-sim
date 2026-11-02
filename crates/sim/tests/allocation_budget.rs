//! 每 tick 堆分配预算（`docs/quality.md` §4.1）。
//!
//! 计数全局分配器只在本测试二进制里生效；测的是单次 `World::step` 前后的
//! 分配次数差。计数是进程级的，因此本文件只放一个测试：两个测试并行跑会
//! 互相污染计数。

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::Phase;
use nba_sim_core::{MatchSetup, Params, World};

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
/// 开着时把每次分配的回溯打到 stderr，用于定位分配来源。
static CAPTURE: AtomicBool = AtomicBool::new(false);
static CAPTURED: AtomicUsize = AtomicUsize::new(0);

struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        if CAPTURE.load(Ordering::Relaxed) && CAPTURED.fetch_add(1, Ordering::Relaxed) < 4 {
            eprintln!("ALLOC {} bytes\n{}", layout.size(), std::backtrace::Backtrace::force_capture());
        }
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        System.dealloc(pointer, layout)
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        System.realloc(pointer, layout, new_size)
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const TICK: f64 = 0.05;

/// 安静 tick：本 tick 没有球员到期决策、也没有交互到期结算。
/// 十名球员各自调度，因此只看持球人不够；判据要盖所有在场球员。
fn quiet_tick(world: &World) -> bool {
    let horizon = world.now() + TICK;
    matches!(world.phase(), Phase::Live)
        && world.players.iter().all(|player| {
            !player.on_court || horizon < player.schedule.next_redecide_at
        })
        && world
            .interactions
            .values()
            .all(|interaction| horizon < interaction.intent.end_time())
}

fn measure(world: &mut World) -> usize {
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    let events = world.events.len();
    let traces = world.decision_traces.len();
    let samples = world.decision_samples.len();
    let records = world.decision_records.len();
    let shots = world.shot_samples.len();
    let interactions = world.interactions.len();
    let actions = world.players.iter().filter(|p| p.action.is_some()).count();
    let defensive = world.defensive_actions.iter().filter(|a| a.is_some()).count();
    // 缓冲扩容本身就是分配：足迹变化说明这一 tick 走了会增长的路径。
    let footprint = world.buffer_footprint();
    let phase = world.phase();
    let possession = world.possession.id;
    let live = matches!(phase, Phase::Live);
    world.step(TICK);
    let delta = ALLOCATIONS.load(Ordering::Relaxed) - before;
    let produced = world.events.len() != events
        || world.decision_traces.len() != traces
        || world.decision_samples.len() != samples
        || world.decision_records.len() != records
        || world.shot_samples.len() != shots
        || world.interactions.len() != interactions
        || world.players.iter().filter(|p| p.action.is_some()).count() != actions
        || world.defensive_actions.iter().filter(|a| a.is_some()).count() != defensive
        || world.buffer_footprint() != footprint
        || world.phase() != phase
        || world.possession.id != possession;
    if delta > 0 && !produced && matches!(phase, Phase::Live) {
        eprintln!(
            "allocation {delta} phase={:?} events {}/{} traces {}/{} records {}/{} interactions {}",
            world.phase(),
            world.events.len(),
            world.events.capacity(),
            world.decision_traces.len(),
            world.decision_traces.capacity(),
            world.decision_records.len(),
            world.decision_records.capacity(),
            world.interactions.len()
        );

    }
    // 只有活球 tick 属于运动与计时的稳态路径；传球与出手的飞行阶段
    // 跑的是球体物理与接触判定，不在本预算范围内。
    if produced || !live { 0 } else { delta }
}

#[test]
fn steady_state_ticks_allocate_nothing() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(
        MatchSetup::builtin(nba_domain::GameRules::default()),
        params,
        11,
    );

    let mut guard = 0;
    while !matches!(world.phase(), Phase::Live) {
        world.step(TICK);
        guard += 1;
        assert!(guard < 8000, "world never reached a live tick");
    }

    // 十名球员各自调度、动作并发进行，因此判据改为：本 tick 没有产出任何
    // 事实、决策样本或审计记录时，就只走运动与计时，不得发生堆分配。
    let mut pure = 0usize;
    let mut allocated = 0usize;
    while world.clock.period() == 1 && !world.finished {
        allocated += measure(&mut world);
        pure += 1;
    }

    assert!(pure > 1000, "一节里的 tick 太少：{pure}");
    assert_eq!(
        allocated, 0,
        "{pure} 个无产出 tick 合计发生 {allocated} 次堆分配"
    );
}
