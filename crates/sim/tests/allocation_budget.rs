//! 每 tick 堆分配预算（`docs/quality.md` §4.1）。
//!
//! 计数全局分配器只在本测试二进制里生效；测的是单次 `World::step` 前后的
//! 分配次数差。计数是进程级的，因此本文件只放一个测试：两个测试并行跑会
//! 互相污染计数。

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::Phase;
use nba_sim_core::{MatchSetup, Params, World};

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
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

/// 稳态 tick：活球、没有进行中的动作、下一步决策尚未到期、进攻时钟不临近
/// 到期。这类 tick 只做运动与计时，占全场绝大多数。
fn steady_tick(world: &World) -> bool {
    matches!(world.phase, Phase::Live)
        && world.active.is_none()
        && world.now() + TICK < world.next_decision_at
        && world.clock.shot_clock > 0.2
}

fn measure(world: &mut World) -> usize {
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    world.step(TICK);
    ALLOCATIONS.load(Ordering::Relaxed) - before
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
    while !steady_tick(&world) {
        world.step(TICK);
        guard += 1;
        assert!(guard < 8000, "world never reached a steady live tick");
    }

    let first = measure(&mut world);
    assert_eq!(first, 0, "首个稳态 tick 发生了 {first} 次堆分配");

    let mut steady = 1usize;
    let mut allocated = 0usize;
    while world.clock.period == 1 && !world.finished {
        if steady_tick(&world) {
            allocated += measure(&mut world);
            steady += 1;
        } else {
            world.step(TICK);
        }
    }

    assert!(steady > 1000, "一节里的稳态 tick 太少：{steady}");
    assert_eq!(
        allocated, 0,
        "{steady} 个稳态 tick 合计发生 {allocated} 次堆分配"
    );
}
