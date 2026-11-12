//! 资源测量：走完整比赛的生产路径，输出单场的事件、动作与分配统计。
//!
//! 计数分配器只统计 `World::run()` 推进期间的堆分配；参数表加载等启动
//! 分配不计入。每 tick 分配是这段计数除以推进的 tick 数。
//!
//! 用法：
//!   measure --scan 1..64 --league nba      每行输出一场的 JSON
//!   measure --seed 7 --league fiba         输出单场的 `MEASURE {...}`

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use nba_sim_core::action::Fact;
use nba_sim_core::params::workspace_calibration_dir;
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

#[derive(serde::Serialize)]
struct Measure {
    seed: u64,
    league: String,
    scenario: String,
    ticks: u64,
    events: usize,
    actions: usize,
    overtimes: u32,
    fouls: usize,
    passes: usize,
    timeouts: usize,
    reviews: usize,
    finished: bool,
    allocations: usize,
    allocations_per_tick: f64,
}

fn parse_league(value: &str) -> nba_domain::league::LeagueId {
    value
        .parse()
        .unwrap_or_else(|_| panic!("unknown league `{value}`"))
}

fn measure(
    seed: u64,
    league: nba_domain::league::LeagueId,
    label: &str,
    scenario: &str,
) -> Measure {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut rules = nba_domain::GameRules::for_league(league);
    let forced_overtimes = if scenario == "overtime" {
        // 短节次让加时程序在远少的 tick 里重复执行；节时长仍必须不小于
        // 一个进攻钟，因此取 30 秒。
        rules = rules.shortened(30.0, 30.0);
        5u32
    } else {
        0
    };
    let setup = MatchSetup::builtin(rules);
    let mut world = World::new(setup, params, seed);
    ALLOCATIONS.store(0, Ordering::Relaxed);
    let report = if forced_overtimes > 0 {
        // 平局是权威状态里的合法局面：每次节次开始时把比分设为相等，
        // 之后由引擎自己的时钟、得分与节末结算决定是否再进一个加时。
        // 达到测量需要的次数后停止干预，比赛按正常规则终场。
        let regulation = world.rules.league.periods.regulation_periods;
        let mut overtimes_seen = 0u32;
        world.score = [world.score[1], world.score[1]];
        while !world.finished && world.clock.tick() < 400_000 {
            // 在所需的加时数之内保持比分相等：节末结算读到平局就按
            // 档案进入下一个加时。比分是权威状态，不注入动作结果。
            if overtimes_seen < forced_overtimes {
                world.score = [world.score[1], world.score[1]];
            }
            world.step(nba_sim_core::world::TICK_SECONDS);
            if world.clock.period() > regulation {
                overtimes_seen = world.clock.period() - regulation;
            }
        }
        world.report()
    } else {
        world.run()
    };
    let allocations = ALLOCATIONS.load(Ordering::Relaxed);

    let mut actions = 0usize;
    let mut fouls = 0usize;
    let mut passes = 0usize;
    let mut timeouts = 0usize;
    let mut reviews = 0usize;
    for event in &report.events {
        match &event.fact {
            Fact::ActionStarted { .. }
            | Fact::DefensiveActionStarted { .. }
            | Fact::PostureStarted { .. } => actions += 1,
            Fact::Foul { .. } | Fact::TeamTechnicalFoul { .. } => fouls += 1,
            Fact::Pass { .. } => passes += 1,
            Fact::Timeout { .. } => timeouts += 1,
            Fact::Review { .. } => reviews += 1,
            _ => {}
        }
    }
    let finished = report
        .events
        .iter()
        .any(|event| matches!(event.fact, Fact::GameEnd));
    let ticks = report.ticks.max(1);
    Measure {
        seed,
        league: label.to_string(),
        scenario: scenario.to_string(),
        ticks: report.ticks,
        events: report.events.len(),
        actions,
        overtimes: report
            .periods
            .saturating_sub(report.regulation_periods),
        fouls,
        passes,
        timeouts,
        reviews,
        finished,
        allocations,
        allocations_per_tick: allocations as f64 / ticks as f64,
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut scan: Option<(u64, u64)> = None;
    let mut seed: Option<u64> = None;
    let mut league = nba_domain::league::LeagueId::Nba;
    let mut scenario = "regular".to_string();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--scan" => {
                let value = args.next().expect("--scan requires start..end");
                let (start, end) = value
                    .split_once("..")
                    .expect("scan range must be `start..end`");
                scan = Some((start.parse().expect("scan start"), end.parse().expect("scan end")));
            }
            "--seed" => {
                seed = Some(
                    args.next()
                        .expect("--seed requires a number")
                        .parse()
                        .expect("seed must be a number"),
                );
            }
            "--league" => {
                league = parse_league(&args.next().expect("--league requires nba|fiba"));
            }
            "--scenario" => {
                scenario = args.next().expect("--scenario requires a scenario name");
            }
            other => panic!("unknown option `{other}`"),
        }
    }
    let label = match league {
        nba_domain::league::LeagueId::Nba => "nba",
        nba_domain::league::LeagueId::Fiba => "fiba",
    };
    if let Some((start, end)) = scan {
        for value in start..end {
            let result = measure(value, league, label, &scenario);
            println!(
                "{}",
                serde_json::to_string(&result).expect("measure serializes")
            );
        }
    } else {
        let result = measure(seed.unwrap_or(1), league, label, &scenario);
        println!(
            "MEASURE {}",
            serde_json::to_string(&result).expect("measure serializes")
        );
    }
}
