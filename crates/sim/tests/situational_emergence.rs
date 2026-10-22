//! 情境逻辑涌现验收：五类末段行为必须从统一效用函数里长出来。
//!
//! 这些测试注入比分与时间快照，跑真实模拟，再用统计量判断行为是否出现。
//! 它们不检查某个分支是否被走到——分支已经不存在了，检查的是「同样的处境
//! 反复出现时，选择分布是否落在真实比赛的方向与量级上」。

use nba_domain::rules::GameRules;
use nba_sim_core::action::Fact;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::Phase;
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

fn is_live(phase: &Phase) -> bool {
    matches!(
        phase,
        Phase::Live | Phase::PassFlight { .. } | Phase::ShotFlight { .. }
    )
}

/// 推进到指定节次、指定比赛钟读数（本节剩余秒数）附近的活球回合。
fn advance_to(world: &mut World, period: u32, game_clock: f64) -> usize {
    for _ in 0..900_000 {
        world.step(TICK_SECONDS);
        if world.clock.period >= period
            && world.clock.game_clock <= game_clock
            && matches!(world.phase, Phase::Live)
        {
            if let Some(handle) = world.holder() {
                return handle;
            }
        }
    }
    panic!(
        "never reached period {period} with {game_clock}s left and a live possession (ended at period {} clock {:.2} phase {:?} finished {})",
        world.clock.period,
        world.clock.game_clock,
        world.phase,
        world.finished
    );
}

fn drain(world: &mut World, seen: &mut usize) -> Vec<nba_sim_core::world::TimedFact> {
    if world.events.len() == *seen {
        return Vec::new();
    }
    let fresh = world.events[*seen..].to_vec();
    *seen = world.events.len();
    fresh
}

#[test]
fn a_late_lead_milks_the_clock_and_a_late_deficit_hunts_threes() {
    let mut leader_seconds = Vec::new();
    let mut trailer_seconds = Vec::new();
    let mut trailer_threes = 0u32;
    let mut trailer_shots = 0u32;
    for seed in 1..=10u64 {
        let mut world = world(seed);
        advance_to(&mut world, 4, 90.0);
        set_margin(&mut world, TeamSide::Home, 6);
        let start = world.possession_log.len();
        let mut seen = world.events.len();
        while !world.finished {
            world.step(TICK_SECONDS);
            for event in drain(&mut world, &mut seen) {
                if let Fact::Shot { shooter, zone, .. } = &event.fact {
                    if world.players[*shooter].team == TeamSide::Away {
                        trailer_shots += 1;
                        if *zone == "three" {
                            trailer_threes += 1;
                        }
                    }
                }
            }
        }
        for record in &world.possession_log[start.min(world.possession_log.len())..] {
            // 每次进攻都算：死球发球开始的回合同样是回合，把它们排除掉
            // 会把样本切成「转换进攻」，测出来的不是「一回合用多久」。
            let elapsed = (record.start_game_clock - record.end_game_clock).max(0.0);
            if record.team == TeamSide::Home {
                leader_seconds.push(elapsed);
            } else {
                trailer_seconds.push(elapsed);
            }
        }
    }
    let mean = |values: &[f64]| values.iter().sum::<f64>() / values.len().max(1) as f64;
    let leader = mean(&leader_seconds);
    let trailer = mean(&trailer_seconds);
    let three_share = trailer_threes as f64 / trailer_shots.max(1) as f64;
    println!(
        "leader {leader:.2}s over {} possessions, trailer {trailer:.2}s over {}, trailer 3PA share {three_share:.3} ({} shots)",
        leader_seconds.len(),
        trailer_seconds.len(),
        trailer_shots
    );
    assert!(
        leader > trailer,
        "the leading team did not consume more clock: {leader:.2}s vs {trailer:.2}s"
    );
    assert!(
        three_share > 0.4,
        "the trailing team did not hunt threes: {three_share:.3}"
    );
}

#[test]
fn a_late_deficit_produces_an_intentional_foul_within_seconds() {
    let trials = 12u64;
    let mut fouled = 0;
    for seed in 1..=trials {
        let mut world = world(seed);
        advance_to(&mut world, 4, 180.0);
        // 等到一次「干净的活球回合」：没有正在执行的动作，客队持球，
        // 且下一个决策在下一秒内到来。推进途中的等待不属于防守方的反应时间。
        let mut ready = false;
        for _ in 0..20_000 {
            world.step(TICK_SECONDS);
            if world.clock.period < 4 || world.clock.game_clock < 120.0 {
                break;
            }
            if matches!(world.phase, Phase::Live)
                && world.active.is_none()
                && world.possession.team == TeamSide::Away
                && world.holder().is_some()
                && world.next_decision_at - world.now() <= f64::from(1u8)
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
        world.fouls[defense].period = world.rules.league.bonus_fouls_per_period as u8;
        // 比分是注入的，所以自然比赛里留下的一波得分已经无效。
        world.scoring_run = 0;
        world.last_scoring = None;
        world.clock.game_clock = 18.0;
        // 场景是一次新回合：进攻方才拿到球，进攻时钟是满的。
        world.clock.shot_clock = world.rules.league.shot_clock_seconds as f64;
        let before = world.events.len();
        let deadline = world.now() + 3.0;
        while world.now() < deadline && !world.finished {
            world.step(TICK_SECONDS);
        }
        let committed = world.events[before..].iter().any(|event| {
            matches!(
                &event.fact,
                Fact::Foul { fouler, .. } if world.players[*fouler].team == TeamSide::Home
            )
        });
        let stopped = world.events[before..]
            .iter()
            .any(|event| matches!(&event.fact, Fact::Timeout { .. }));
        if stopped {
            println!("seed {seed}: the defence stopped the clock with a timeout instead");
        }
        if committed {
            fouled += 1;
        }
    }
    let rate = fouled as f64 / trials as f64;
    println!("intentional foul within three seconds: {rate:.2}");
    assert!(
        rate >= 0.9,
        "the trailing defense only fouled in {fouled}/{trials} trials"
    );
}

#[test]
fn a_poor_free_throw_big_is_hacked_only_before_the_final_window() {
    let mut against_big = 0;
    let mut against_others = 0;
    for seed in 1..=10u64 {
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
        world.fouls[defense].period = world.rules.league.bonus_fouls_per_period as u8;
        world.clock.game_clock = 240.0;
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
    }
    println!("hack rate: big {against_big}, others {against_others}");
    assert!(
        against_big > against_others,
        "the poor free-throw big was not targeted more often than the rest: {against_big} vs {against_others}"
    );

    // 进入最后两分钟的窗口后，无球犯规是一罚一掷，引擎不应再对无球人下手。
    let mut late_off_ball = 0;
    for seed in 1..=6u64 {
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
        world.fouls[defense].period = world.rules.league.bonus_fouls_per_period as u8;
        world.clock.game_clock = 110.0;
        let mut seen = world.events.len();
        let deadline = world.now() + 40.0;
        while world.now() < deadline && !world.finished {
            // 犯规那一刻谁持球，只能在本 tick 开始时取：犯规结算之后
            // 球权可能已经变了。
            let holder = world.holder();
            world.step(TICK_SECONDS);
            for event in drain(&mut world, &mut seen) {
                if let Fact::Foul { fouled, .. } = &event.fact {
                    if world.players[*fouled].team == offense && Some(*fouled) != holder {
                        late_off_ball += 1;
                    }
                }
            }
        }
    }
    assert_eq!(
        late_off_ball, 0,
        "off-ball hacking continued inside the final two minutes"
    );
}

#[test]
fn a_blown_out_game_empties_the_bench_within_two_stops() {
    let trials = 60u64;
    let mut cleared = 0;
    let mut judged = 0;
    for seed in 1..=trials {
        let mut world = world(seed);
        advance_to(&mut world, 4, 240.0);
        // 场景前提是「就在此刻变成垃圾时间」：自然比分已经拉开时，
        // 替补席早就上过了，不是这个测试要衡量的局面。
        let natural = (world.score[0] as i32 - world.score[1] as i32).abs();
        if natural > 8 {
            println!("seed {seed}: skipped, natural margin {natural}");
            continue;
        }
        set_margin(&mut world, TeamSide::Home, 24);
        // 核心首发按注入时刻的出场时间排：这是「前三节半谁在扛」的度量，
        // 不受垃圾时间换人之后累计分钟变化的影响。
        let mut core: Vec<usize> = (0..world.players.len())
            .filter(|handle| {
                world.players[*handle].team == TeamSide::Home && world.players[*handle].data.starter
            })
            .collect();
        core.sort_by(|left, right| {
            world
                .line(*right)
                .seconds
                .total_cmp(&world.line(*left).seconds)
        });
        core.truncate(3);
        // 名单只有八个球员：替补席上不足三人时，场上必然留人，
        // 这个局面无法满足要求，不计入统计。
        let bench = (0..world.players.len())
            .filter(|handle| {
                let player = &world.players[*handle];
                player.team == TeamSide::Home
                    && !player.on_court
                    && !player.data.starter
                    && !player.disqualified
                    && player.fouls < world.rules.league.max_personal_fouls
            })
            .count();
        if bench < core.len() {
            continue;
        }

        let mut stops = 0;
        let mut seen = world.events.len();
        let mut previous_live = is_live(&world.phase);
        let mut after_two: Option<Vec<bool>> = None;
        while !world.finished {
            world.step(TICK_SECONDS);
            drain(&mut world, &mut seen);
            let live = is_live(&world.phase);
            if previous_live && !live {
                stops += 1;
            }
            // 换人是在死球窗口结束时执行的，所以第二个窗口的换人要等
            // 比赛重新进入活球后才生效。
            if stops >= 2 && live && after_two.is_none() {
                after_two = Some(world.players.iter().map(|player| player.on_court).collect());
            }
            previous_live = live;
            if stops >= 4 {
                break;
            }
        }
        let after_two = after_two.expect("two dead-ball windows in four minutes of garbage time");
        let out = core.iter().all(|handle| !after_two[*handle]);
        judged += 1;
        if out {
            cleared += 1;
        } else {
            println!("seed {seed}: core still on the floor after two windows");
        }
    }
    let rate = cleared as f64 / judged.max(1) as f64;
    println!("bench cleared within two stops: {cleared}/{judged} = {rate:.2}");
    assert!(
        rate >= 0.95,
        "the bench only cleared in {cleared}/{judged} blowouts"
    );
}

/// 两队名单属性完全对称时，出手结构必须对称。
///
/// 这条不是风格偏好：主客场名单的能力值逐个相同，如果三分占比与平均
/// 出手距离出现系统性差异，说明有某条路径读到了与攻防方向无关的球队
/// 标识，后面所有按球队切片的真实性统计都会被这个偏差污染。
#[test]
fn home_and_away_produce_comparable_offences() {
    let mut threes = [0u32; 2];
    let mut shots = [0u32; 2];
    let mut distance = [0.0f64; 2];
    for seed in 1..=12u64 {
        let mut before = [0u32; 2];
        let mut before_threes = [0u32; 2];
        let mut before_distance = [0.0f64; 2];
        let mut world = world(seed);
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
                shots[team] += 1;
                threes[team] += u32::from(*zone == "three");
                distance[team] += f64::from(*distance_ft);
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
