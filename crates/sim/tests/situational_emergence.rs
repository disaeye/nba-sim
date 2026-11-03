//! 情境逻辑涌现验收：五类末段行为必须从统一效用函数里长出来。
//!
//! 这些测试注入比分与时间快照，跑真实模拟，再用统计量判断行为是否出现。
//! 它们不检查某个分支是否被走到——分支已经不存在了，检查的是「同样的处境
//! 反复出现时，选择分布是否落在真实比赛的方向与量级上」。

use nba_domain::rules::GameRules;
use nba_sim_core::action::Fact;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::Phase;

mod common;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, TeamSide, World};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

fn index(team: TeamSide) -> usize {
    World::team_index(team)
}

fn set_margin(world: &mut World, leader: TeamSide, margin: u32) {
    let lead = index(leader);
    let trail = index(leader.other());
    world.score[lead] = 80 + margin;
    world.score[trail] = 80;
}

/// 推进到首个活球回合后直接把两钟设定到指定节次与读数，避免为进入末段而模拟整场比赛。
fn advance_to(world: &mut World, period: u32, game_clock: f64) -> usize {
    for _ in 0..40_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::Live) && world.holder().is_some() {
            break;
        }
    }
    let holder = world
        .holder()
        .expect("the engine never reached a live ball state");
    world
        .clock
        .set_regulation_clock(period, game_clock, world.clock.shot_clock());
    holder
}

fn drain(world: &mut World, seen: &mut usize) -> Vec<nba_sim_core::world::TimedFact> {
    if world.events.len() == *seen {
        return Vec::new();
    }
    let fresh = world.events[*seen..].to_vec();
    *seen = world.events.len();
    fresh
}

/// 把每个种子的独立场景分到四个线程，结果按种子顺序汇合；
/// 种子间没有共享状态，聚合顺序与串行版本一致。
fn seeds_in_parallel<T, F>(seeds: std::ops::RangeInclusive<u64>, per_seed: F) -> Vec<T>
where
    F: Fn(u64) -> T + Sync,
    T: Send,
{
    let all: Vec<u64> = seeds.collect();
    let chunk_size = all.len().div_ceil(4);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .filter_map(|chunk| {
                let start = chunk * chunk_size;
                let end = (start + chunk_size).min(all.len());
                (start < end).then_some(&all[start..end])
            })
            .map(|chunk| {
                let per_seed = &per_seed;
                scope.spawn(move || chunk.iter().map(|seed| per_seed(*seed)).collect::<Vec<_>>())
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("scenario thread must not panic"))
            .flatten()
            .collect()
    })
}

#[test]
fn a_late_lead_milks_the_clock_and_a_late_deficit_hunts_threes() {
    // 每个种子返回（领先方出手时钟样本，落后方出手时钟样本，落后方三分，落后方出手）。
    let per_seed = |seed: u64| -> (Vec<f64>, Vec<f64>, u32, u32) {
        let mut leader_shot_clock = Vec::new();
        let mut trailer_shot_clock = Vec::new();
        let mut trailer_threes = 0u32;
        let mut trailer_shots = 0u32;
        let mut world = world(seed);
        advance_to(&mut world, 4, 90.0);
        set_margin(&mut world, TeamSide::Home, 6);
        let mut seen = world.events.len();
        while !world.finished {
            world.step(TICK_SECONDS);
            for event in drain(&mut world, &mut seen) {
                if let Fact::Shot { shooter, zone, .. } = &event.fact {
                    let team = world.players[*shooter].team;
                    if team == TeamSide::Home {
                        leader_shot_clock.push(world.clock.shot_clock());
                    } else {
                        trailer_shot_clock.push(world.clock.shot_clock());
                    }
                    if team == TeamSide::Away {
                        trailer_shots += 1;
                        if *zone == "three" {
                            trailer_threes += 1;
                        }
                    }
                }
            }
        }
        (
            leader_shot_clock,
            trailer_shot_clock,
            trailer_threes,
            trailer_shots,
        )
    };
    let mut leader_shot_clock = Vec::new();
    let mut trailer_shot_clock = Vec::new();
    let mut trailer_threes = 0u32;
    let mut trailer_shots = 0u32;
    for (leader, trailer, threes, shots) in seeds_in_parallel(1..=10u64, per_seed) {
        leader_shot_clock.extend(leader);
        trailer_shot_clock.extend(trailer);
        trailer_threes += threes;
        trailer_shots += shots;
    }
    let mean = |values: &[f64]| values.iter().sum::<f64>() / values.len().max(1) as f64;
    let leader = mean(&leader_shot_clock);
    let trailer = mean(&trailer_shot_clock);
    let three_share = trailer_threes as f64 / trailer_shots.max(1) as f64;
    println!(
        "leader shot clock {leader:.2}s over {} shots, trailer {trailer:.2}s over {}, trailer 3PA share {three_share:.3} ({} shots)",
        leader_shot_clock.len(),
        trailer_shot_clock.len(),
        trailer_shots
    );
    assert!(
        leader < trailer,
        "the leading team did not shoot later: {leader:.2}s vs {trailer:.2}s"
    );
    assert!(
        three_share > 0.4,
        "the trailing team did not hunt threes: {three_share:.3}"
    );
}

#[test]
fn a_late_deficit_produces_an_intentional_foul_within_seconds() {
    let trials = 16u64;
    // 每个种子做一次独立试验；返回 None 表示这次观察窗口里球权已经不在
    // 领先方手里，场景不再适用，不计入分母。
    let per_seed = |seed: u64| -> Option<bool> {
        let mut world = world(seed);
        advance_to(&mut world, 4, 180.0);
        // 等到一次「干净的活球回合」：没有正在执行的动作，客队持球，
        // 且下一个决策在下一秒内到来。推进途中的等待不属于防守方的反应时间。
        let mut ready = false;
        for _ in 0..20_000 {
            world.step(TICK_SECONDS);
            if world.clock.period() < 4 || world.clock.game_clock() < 120.0 {
                break;
            }
            if matches!(world.phase(), Phase::Live)
                && world.interactions.is_empty()
                && world.possession.team == TeamSide::Away
                && world.holder().is_some()
                && world.holder().is_some_and(|handle| {
                    world.players[handle].schedule.next_redecide_at - world.now() <= f64::from(1u8)
                })
            {
                ready = true;
                break;
            }
        }
        assert!(
            ready,
            "seed {seed}: no clean possession before a decision boundary"
        );
        set_margin(&mut world, TeamSide::Away, 2);
        let defense = index(TeamSide::Home);
        world.fouls[defense].period = world.rules.league.fouls.bonus_fouls_per_period as u8;
        // 比分是注入的，所以自然比赛里留下的一波得分已经无效。
        world.scoring_run = 0;
        world.last_scoring = None;
        world.clock.set_game_clock(18.0);
        // 场景是一次新回合：进攻方才拿到球，进攻时钟是满的。
        world
            .clock
            .set_shot_clock(world.rules.league.clocks.shot_clock_seconds as f64);
        let before = world.events.len();
        let deadline = world.now() + 5.0;
        let mut committed = false;
        let mut applicable = true;
        while world.now() < deadline && !world.finished {
            world.step(TICK_SECONDS);
            if world.events[before..].iter().any(|event| {
                matches!(
                    &event.fact,
                    Fact::Foul { fouler, .. } if world.players[*fouler].team == TeamSide::Home
                )
            }) {
                committed = true;
                break;
            }
            // 领先方交出球权后，落后方已经不需要再犯规了。
            if world.possession.team != TeamSide::Away {
                applicable = false;
                break;
            }
        }
        let stopped = world.events[before..]
            .iter()
            .any(|event| matches!(&event.fact, Fact::Timeout { .. }));
        if stopped {
            println!("seed {seed}: the defence stopped the clock with a timeout instead");
        }
        if committed {
            Some(true)
        } else if applicable {
            Some(false)
        } else {
            None
        }
    };
    let results: Vec<bool> = seeds_in_parallel(1..=trials, per_seed)
        .into_iter()
        .flatten()
        .collect();
    let fouled = results.iter().filter(|committed| **committed).count() as u64;
    let applicable = results.len() as u64;
    assert!(
        applicable >= 6,
        "only {applicable} of {trials} trials kept the leading side in possession"
    );
    let rate = fouled as f64 / applicable as f64;
    println!("intentional foul within five seconds: {rate:.2} over {applicable} applicable trials");
    assert!(
        rate >= 0.9,
        "the trailing defense only fouled in {fouled}/{applicable} applicable trials"
    );
}

#[test]
fn a_poor_free_throw_big_is_hacked_only_before_the_final_window() {
    // 每个种子做一次独立场景，返回（对大个子的犯规，对其他人的犯规）。
    let per_seed = |seed: u64| -> (u32, u32) {
        let mut against_big = 0;
        let mut against_others = 0;
        let mut world = world(seed);
        advance_to(&mut world, 3, 240.0);
        let offense = world.possession.team;
        let big = world
            .on_court(offense)
            .as_slice()
            .iter()
            .copied()
            .max_by_key(|handle| world.players[*handle].data.height_cm)
            .expect("an on-court big");
        world.players[big].data.attributes.free_throw = 0.38;
        set_margin(&mut world, offense, 8);
        let defense = index(offense.other());
        world.fouls[defense].period = world.rules.league.fouls.bonus_fouls_per_period as u8;
        world.clock.set_game_clock(240.0);
        let before = world.events.len();
        let deadline = world.now() + 40.0;
        while world.now() < deadline && !world.finished {
            world.step(TICK_SECONDS);
        }
        for event in &world.events[before..] {
            if let Fact::Foul { fouled, .. } = &event.fact {
                if world.players[*fouled].team != offense {
                    continue;
                }
                if *fouled == big {
                    against_big += 1;
                } else {
                    against_others += 1;
                }
            }
        }
        (against_big, against_others)
    };
    let mut against_big = 0;
    let mut against_others = 0;
    for (big, others) in seeds_in_parallel(1..=10u64, per_seed) {
        against_big += big;
        against_others += others;
    }
    println!("hack rate: big {against_big}, others {against_others}");
    assert!(
        against_big > against_others,
        "the poor free-throw big was not targeted more often than the rest: {against_big} vs {against_others}"
    );

    // 进入最后两分钟的窗口后，无球犯规是一罚一掷，引擎不应再对无球人下手。
    // 每个种子返回该场景下对大个子的无球犯规次数。
    let final_window_per_seed = |seed: u64| -> u32 {
        let mut late_off_ball = 0;
        let mut world = world(seed);
        advance_to(&mut world, 4, 110.0);
        let offense = world.possession.team;
        let big = world
            .on_court(offense)
            .as_slice()
            .iter()
            .copied()
            .max_by_key(|handle| world.players[*handle].data.height_cm)
            .expect("an on-court big");
        world.players[big].data.attributes.free_throw = 0.30;
        set_margin(&mut world, offense, 8);
        let defense = index(offense.other());
        world.fouls[defense].period = world.rules.league.fouls.bonus_fouls_per_period as u8;
        world.clock.set_game_clock(110.0);
        let mut seen = world.events.len();
        let deadline = world.now() + 40.0;
        while world.now() < deadline && !world.finished {
            let holder_before = world.holder();
            world.step(TICK_SECONDS);
            let holder_after = world.holder();
            for event in drain(&mut world, &mut seen) {
                if let Fact::Foul { fouled, .. } = &event.fact {
                    let on_ball = holder_before == Some(*fouled) || holder_after == Some(*fouled);
                    if *fouled == big && !on_ball {
                        late_off_ball += 1;
                    }
                }
            }
        }
        late_off_ball
    };
    let late_off_ball = seeds_in_parallel(1..=6u64, final_window_per_seed)
        .into_iter()
        .sum::<u32>();
    assert_eq!(
        late_off_ball, 0,
        "off-ball hacking continued inside the final two minutes"
    );
}

#[test]
fn a_blown_out_team_uses_its_bench_at_the_next_dead_ball() {
    let mut world = world(1);
    world
        .clock
        .set_regulation_clock(4, 240.0, world.rules.league.clocks.shot_clock_seconds as f64);
    set_margin(&mut world, TeamSide::Home, 24);
    let core: Vec<usize> = world
        .on_court(TeamSide::Home)
        .into_iter()
        .filter(|handle| world.players[*handle].data.starter)
        .take(3)
        .collect();
    assert_eq!(core.len(), 3, "the starting lineup must be on court");
    assert_eq!(
        (0..world.players.len())
            .filter(|handle| {
                let player = &world.players[*handle];
                player.team == TeamSide::Home
                    && !player.on_court
                    && !player.data.starter
                    && !player.disqualified
                    && player.fouls < world.rules.league.fouls.max_personal_fouls
            })
            .count(),
        3,
        "all three reserve players must be eligible at the start of the scenario"
    );
    world.set_phase(Phase::DeadBall {
        team: TeamSide::Home,
        ready_at: world.now(),
    });

    world.apply_dead_ball_window();

    for handle in core {
        assert!(
            !world.players[handle].on_court,
            "starter {handle} remained on court after the blowout substitution window"
        );
    }
    assert_eq!(world.on_court(TeamSide::Home).as_slice().len(), 5);
    assert_eq!(
        world
            .events
            .iter()
            .filter(|event| matches!(
                event.fact,
                Fact::Substitution {
                    team: TeamSide::Home,
                    ..
                }
            ))
            .count(),
        3,
        "the dead-ball window must replace all three selected starters"
    );
}

/// 两队名单属性完全对称时，出手结构必须对称。
///
/// 这条不是风格偏好：主客场名单的能力值逐个相同，如果三分占比与平均
/// 出手距离出现系统性差异，说明有某条路径读到了与攻防方向无关的球队
/// 标识，后面所有按球队切片的真实性统计都会被这个偏差污染。
#[test]
fn home_and_away_produce_comparable_offences() {
    // 每个种子返回（出手数，三分数，出手距离和）按主客两队切片。
    let per_seed = |seed: u64| -> ([u32; 2], [u32; 2], [f64; 2]) {
        let mut before = [0u32; 2];
        let mut before_threes = [0u32; 2];
        let mut before_distance = [0.0f64; 2];
        let mut world = common::short_world(seed);
        let mut seen = world.events.len();
        while !world.finished {
            world.step(TICK_SECONDS);
            for event in drain(&mut world, &mut seen) {
                let Fact::Shot {
                    shooter,
                    zone,
                    distance_ft,
                    ..
                } = &event.fact
                else {
                    continue;
                };
                let team = index(world.players[*shooter].team);
                before[team] += 1;
                before_threes[team] += u32::from(*zone == "three");
                before_distance[team] += f64::from(*distance_ft);
            }
        }
        println!(
            "seed {seed}: home {:.3}/{:.1}ft away {:.3}/{:.1}ft",
            f64::from(before_threes[0]) / f64::from(before[0].max(1)),
            before_distance[0] / f64::from(before[0].max(1)),
            f64::from(before_threes[1]) / f64::from(before[1].max(1)),
            before_distance[1] / f64::from(before[1].max(1)),
        );
        (before, before_threes, before_distance)
    };
    let mut threes = [0u32; 2];
    let mut shots = [0u32; 2];
    let mut distance = [0.0f64; 2];
    for (seed_shots, seed_threes, seed_distance) in seeds_in_parallel(1..=20u64, per_seed) {
        for team in 0..2 {
            shots[team] += seed_shots[team];
            threes[team] += seed_threes[team];
            distance[team] += seed_distance[team];
        }
    }
    let share = [
        threes[0] as f64 / shots[0].max(1) as f64,
        threes[1] as f64 / shots[1].max(1) as f64,
    ];
    let mean_distance = [
        distance[0] / shots[0].max(1) as f64,
        distance[1] / shots[1].max(1) as f64,
    ];
    println!("three-point share {share:?}, mean distance {mean_distance:?}");
    assert!(
        (share[0] - share[1]).abs() <= 0.12,
        "home and away three-point shares diverge: {share:?}"
    );
    assert!(
        (mean_distance[0] - mean_distance[1]).abs() <= 3.0,
        "home and away shot distances diverge: {mean_distance:?}"
    );
}
