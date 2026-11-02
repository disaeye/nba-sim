use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use nba_domain::rules::GameRules;
use nba_sim_core::defense::{refresh_matchup, solve_defense_targets};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::Phase;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};

thread_local! {
    /// 只统计当前线程的分配：测试框架或其他线程的分配不应混入稳态计量。
    static LOCAL_ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LOCAL_ALLOCATIONS.with(|count| count.set(count.get() + 1));
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LOCAL_ALLOCATIONS.with(|count| count.set(count.get() + 1));
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn allocations() -> usize {
    LOCAL_ALLOCATIONS.with(|count| count.get())
}

#[test]
fn solve_defense_targets_allocates_nothing() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, 13);
    for _ in 0..40_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::Live) && world.holder().is_some() {
            break;
        }
    }
    refresh_matchup(&mut world);
    // 先跑一次让任何一次性初始化发生，再计量稳态调用。
    let first = solve_defense_targets(&world);
    let before = allocations();
    for _ in 0..500 {
        let targets = solve_defense_targets(&world);
        assert_eq!(targets, first);
    }
    let allocated = allocations() - before;
    assert_eq!(
        allocated, 0,
        "solve_defense_targets allocated {allocated} times over 500 calls"
    );
}
