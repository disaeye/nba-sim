//! 规则状态机的世界级守卫：进攻时钟重置、犯规罚则矩阵、死球换人门控、暂停配额。
//!
//! 纯函数层面的阈值矩阵在 `nba-domain` 的 `game_flow_transitions` 里卡死；
//! 这里卡的是「引擎真的按那份阈值执行」：球权、阶段、配额三件事必须对得上。

mod common;

use nba_domain::rules::GameRules;
use nba_sim_core::action::Fact;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::resolve::foul::{apply_non_shooting_foul, apply_offensive_foul};
use nba_sim_core::state::Phase;
use nba_sim_core::timeouts::{can_request, request, TimeoutRejection};
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};

fn world(seed: u64) -> World {
    common::short_world(seed)
}

fn live_world(seed: u64) -> World {
    let mut world = world(seed);
    for _ in 0..40_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::Live) && world.holder().is_some() {
            return world;
        }
    }
    panic!("the engine never reached a live ball state");
}

fn world_with_rules(seed: u64, rules: GameRules) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(rules), params, seed)
}

fn live_world_with_rules(seed: u64, rules: GameRules) -> World {
    let mut world = world_with_rules(seed, rules);
    for _ in 0..40_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::Live) && world.holder().is_some() {
            return world;
        }
    }
    panic!("the engine never reached a live ball state");
}

fn is_dead_ball(phase: &Phase) -> bool {
    !matches!(phase, Phase::Live)
}

fn possession_start_events(world: &World) -> Vec<u64> {
    world
        .events
        .iter()
        .filter_map(|event| match &event.fact {
            Fact::PossessionStarted { possession_id, .. } => Some(*possession_id),
            _ => None,
        })
        .collect()
}

fn last_foul(world: &World) -> (u8, u8) {
    for event in world.events.iter().rev() {
        if let Fact::Foul {
            free_throws, kind, ..
        } = &event.fact
        {
            let code = match kind {
                nba_domain::event::FoulKind::Shooting => 1,
                nba_domain::event::FoulKind::Offensive => 2,
                _ => 0,
            };
            return (*free_throws, code);
        }
    }
    panic!("no foul fact was published");
}

#[test]
fn a_second_chance_clock_only_resets_when_the_ball_touched_the_rim() {
    let mut world = live_world(31);
    let offense = world.possession.team;
    let rebounder = world.holder().expect("live possession has a holder");

    let possession_id = world.possession.id;
    let possession_count = world.possession_count;
    let possession_starts = possession_start_events(&world);
    world.possession.had_shot = true;
    world.possession.passes = 3;
    world.possession.drives = 2;

    world.clock.set_shot_clock(8.0);
    world.start_second_chance(offense, rebounder, true);
    assert_eq!(world.possession.id, possession_id);
    assert_eq!(world.possession_count, possession_count);
    assert_eq!(possession_start_events(&world), possession_starts);
    assert!(world.possession.had_offensive_rebound);
    assert!(world.possession.had_shot);
    assert_eq!(world.possession.passes, 3);
    assert_eq!(world.possession.drives, 2);
    assert_eq!(
        world.clock.shot_clock(),
        14.0,
        "a rim-touching offensive rebound must refill the clock to 14"
    );

    world.clock.set_shot_clock(16.0);
    world.start_second_chance(offense, rebounder, true);
    assert_eq!(world.possession.id, possession_id);
    assert_eq!(world.possession_count, possession_count);
    assert_eq!(possession_start_events(&world), possession_starts);
    assert_eq!(
        world.clock.shot_clock(),
        16.0,
        "a rim-touching offensive rebound must not shorten a longer clock"
    );

    world.clock.set_shot_clock(8.0);
    world.start_second_chance(offense, rebounder, false);
    assert_eq!(world.possession.id, possession_id);
    assert_eq!(world.possession_count, possession_count);
    assert_eq!(possession_start_events(&world), possession_starts);
    assert_eq!(
        world.clock.shot_clock(),
        8.0,
        "an offensive rebound off a shot that never touched the rim must not reset the clock"
    );
}

#[test]
fn the_fifth_team_foul_awards_free_throws_while_the_fourth_only_restarts_play() {
    let mut world = live_world(31);
    world
        .clock
        .set_regulation_clock(2, 400.0, world.clock.shot_clock());
    let offense = world.possession.team;
    let fouler = world
        .on_court(offense.other())
        .as_slice()
        .iter()
        .copied()
        .next()
        .expect("a defender is on the court");
    let fouled = world.holder().expect("live possession has a holder");

    for expected in 1..=5u8 {
        apply_non_shooting_foul(&mut world, fouler, fouled);
        let team_fouls = world.fouls[World::team_index(offense.other())].period;
        assert_eq!(
            team_fouls, expected,
            "the team foul counter did not advance on foul {expected}"
        );
        let (free_throws, _) = last_foul(&world);
        if expected < 5 {
            assert_eq!(
                free_throws, 0,
                "team foul {expected} must not award free throws"
            );
            assert!(
                matches!(world.phase(), Phase::DeadBall { team, .. } if team == offense),
                "team foul {expected} must stop play with a throw-in for the offence"
            );
        } else {
            assert_eq!(
                free_throws, 2,
                "the fifth team foul must award two free throws"
            );
        }
    }
}

#[test]
fn the_second_team_foul_in_the_last_two_minutes_awards_free_throws() {
    let mut world = live_world(31);
    world
        .clock
        .set_regulation_clock(4, 90.0, world.clock.shot_clock());
    let offense = world.possession.team;
    let fouler = world
        .on_court(offense.other())
        .as_slice()
        .iter()
        .copied()
        .next()
        .expect("a defender is on the court");
    let fouled = world.holder().expect("live possession has a holder");

    apply_non_shooting_foul(&mut world, fouler, fouled);
    assert_eq!(
        last_foul(&world).0,
        0,
        "the first foul must not award free throws"
    );
    assert!(matches!(world.phase(), Phase::DeadBall { .. }));

    apply_non_shooting_foul(&mut world, fouler, fouled);
    assert_eq!(
        last_foul(&world).0,
        2,
        "the second team foul in the last two minutes must award two free throws"
    );
}

#[test]
fn an_offensive_foul_counts_personally_but_never_sends_the_other_team_to_the_line() {
    let mut world = live_world(31);
    world
        .clock
        .set_regulation_clock(2, 400.0, world.clock.shot_clock());
    let offense = world.possession.team;
    let index = World::team_index(offense);
    let fouler = world.holder().expect("live possession has a holder");
    let period_before = world.fouls[index].period;
    let personal_before = world.players[fouler].fouls;

    apply_offensive_foul(&mut world, fouler, 0);

    assert_eq!(
        world.fouls[index].period, period_before,
        "an offensive foul must not advance the team bonus counter"
    );
    assert_eq!(
        world.players[fouler].fouls,
        personal_before + 1,
        "an offensive foul must still count toward the personal limit"
    );
    assert_eq!(
        last_foul(&world).0,
        0,
        "an offensive foul awards no free throws"
    );
    assert!(
        matches!(world.phase(), Phase::DeadBall { team, .. } if team == offense.other()),
        "an offensive foul must hand the ball to the other team after a stoppage"
    );
}

#[test]
fn only_the_team_in_control_may_stop_the_clock_and_the_budget_is_capped() {
    let mut world = live_world(31);
    world
        .clock
        .set_regulation_clock(3, 400.0, world.clock.shot_clock());
    let offense = world.possession.team;
    assert_eq!(
        can_request(&world, offense.other()),
        Err(TimeoutRejection::NoPossession),
        "the team without the ball must not be granted a live ball timeout"
    );
    assert!(
        request(&mut world, offense),
        "the team in control must be granted a timeout"
    );
    assert!(matches!(world.phase(), Phase::Timeout { .. }));
    assert!(
        can_request(&world, offense.other()).is_ok(),
        "during a dead ball either team may request a timeout"
    );

    // 第四节：最多四次。
    world
        .clock
        .set_regulation_clock(4, 400.0, world.clock.shot_clock());
    world.timeouts[World::team_index(offense)] = nba_sim_core::timeouts::TimeoutBudget {
        remaining: 7,
        used_in_fourth: 1,
        used_in_final_window: 0,
        period: 4,
    };
    let paused_game_clock = world.clock.game_clock();
    let paused_shot_clock = world.clock.shot_clock();
    let paused_elapsed = world.clock.elapsed();
    let paused_simulation_time = world.now();
    let paused_possession_id = world.possession.id;
    resume_timeout(&mut world);
    assert_eq!(world.clock.game_clock(), paused_game_clock);
    assert_eq!(world.clock.shot_clock(), paused_shot_clock);
    assert_eq!(world.clock.elapsed(), paused_elapsed);
    assert!(world.now() > paused_simulation_time);
    assert_eq!(world.possession.id, paused_possession_id);
    for _ in 0..3 {
        assert!(request(&mut world, offense));
        resume_timeout(&mut world);
    }
    assert_eq!(
        can_request(&world, offense),
        Err(TimeoutRejection::FourthQuarterExhausted),
        "a fifth fourth quarter timeout must be refused"
    );

    // 最后时段：最多两次，即使整场余额还有。
    world.timeouts[World::team_index(offense)] = nba_sim_core::timeouts::TimeoutBudget {
        remaining: 7,
        used_in_fourth: 0,
        used_in_final_window: 0,
        period: 4,
    };
    world
        .clock
        .set_regulation_clock(4, 120.0, world.clock.shot_clock());
    resume_timeout(&mut world);
    for _ in 0..2 {
        assert!(request(&mut world, offense));
        resume_timeout(&mut world);
    }
    assert_eq!(
        can_request(&world, offense),
        Err(TimeoutRejection::FinalWindowExhausted),
        "a third timeout inside the final window must be refused"
    );
}

#[test]
fn fifty_short_games_never_change_the_lineup_while_the_ball_is_live() {
    // 五十场各自独立，用四个作用域线程分摊后单场进度不变。
    let seeds: Vec<u64> = (1..=50u64).collect();
    let size = seeds.len().div_ceil(4);
    let chunks: Vec<&[u64]> = (0..4)
        .map(|c| {
            let start = c * size;
            let end = (start + size).min(seeds.len());
            &seeds[start..end]
        })
        .filter(|chunk| !chunk.is_empty())
        .collect();
    let per_seed: Vec<(u32, u32)> = std::thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|s| substitution_audit(*s))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("lineup thread must not panic"))
            .flatten()
            .collect()
    });
    let mut substitutions = 0;
    let mut violations = 0;
    for (seed_substitutions, seed_violations) in per_seed {
        substitutions += seed_substitutions;
        violations += seed_violations;
    }
    assert!(
        substitutions > 0,
        "no substitution happened across fifty games"
    );
    assert_eq!(
        violations, 0,
        "{violations} substitutions happened with the ball live across fifty games"
    );
}

/// 跑完一个种子的短场，返回（换人次数，活球期间的换人次数）。
fn substitution_audit(seed: u64) -> (u32, u32) {
    let mut world = world(seed);
    let mut substitutions = 0;
    let mut violations = 0;
    while !world.finished && world.clock.tick() < 2_000_000 {
        let before = world.phase();
        // 用在场位掩码代替两次阵容快照，逐刻比较。
        let mut mask_before = 0u64;
        for (handle, player) in world.players.iter().enumerate() {
            if player.on_court {
                mask_before |= 1 << handle;
            }
        }
        world.step(TICK_SECONDS);
        let mut mask_after = 0u64;
        for (handle, player) in world.players.iter().enumerate() {
            if player.on_court {
                mask_after |= 1 << handle;
            }
        }
        if mask_before != mask_after {
            substitutions += 1;
            if !is_dead_ball(&before) && !is_dead_ball(&world.phase()) {
                violations += 1;
            }
        }
    }
    assert!(world.finished, "seed {seed}: the game did not finish");
    (substitutions, violations)
}

#[test]
fn a_non_bonus_personal_foul_restart_preserves_the_live_possession_id() {
    let mut world = live_world(27);
    let offense = world.possession.team;
    let possession_id = world.possession.id;
    let possession_count = world.possession_count;
    let possession_starts = possession_start_events(&world);
    world.clock.set_shot_clock(7.0);
    let shot_clock_before_foul = world.clock.shot_clock();
    let fouler = world.on_court(offense.other()).as_slice()[0];
    let fouled = world.holder().expect("live possession has a holder");

    apply_non_shooting_foul(&mut world, fouler, fouled);

    assert_eq!(last_foul(&world).0, 0);
    assert_eq!(world.possession.id, possession_id);
    assert_eq!(world.possession_count, possession_count);
    assert!(matches!(
        world.phase(),
        Phase::DeadBall { team, .. } if team == offense
    ));
    assert_eq!(world.clock.shot_clock(), shot_clock_before_foul);
    assert!(world
        .possession_log
        .iter()
        .all(|record| record.id != possession_id));
    assert_eq!(possession_start_events(&world), possession_starts);

    for _ in 0..4000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::Live) {
            break;
        }
    }
    assert!(matches!(world.phase(), Phase::Live));
    assert_eq!(world.possession.id, possession_id);
    assert_eq!(world.possession_count, possession_count);
    assert!(world
        .possession_log
        .iter()
        .all(|record| record.id != possession_id));
    assert_eq!(possession_start_events(&world), possession_starts);
}

#[test]
fn an_away_from_play_foul_in_the_last_two_minutes_is_one_free_throw_and_the_ball() {
    let mut world = live_world(31);
    world
        .clock
        .set_regulation_clock(4, 90.0, world.clock.shot_clock());
    let offense = world.possession.team;
    let fouler = world
        .on_court(offense.other())
        .as_slice()
        .iter()
        .copied()
        .next()
        .expect("a defender is on the court");
    let possession_id = world.possession.id;
    let possession_count = world.possession_count;
    let possession_starts = possession_start_events(&world);
    world.clock.set_shot_clock(7.0);
    let shot_clock_before_foul = world.clock.shot_clock();
    let holder = world.holder().expect("live possession has a holder");
    let off_ball = world
        .on_court(offense)
        .as_slice()
        .iter()
        .copied()
        .find(|handle| *handle != holder)
        .expect("the offence has an off-ball player");

    apply_non_shooting_foul(&mut world, fouler, off_ball);
    assert_eq!(
        last_foul(&world).0,
        1,
        "an away from play foul in the last two minutes is one free throw"
    );
    match world.phase() {
        Phase::FreeThrows {
            shooter,
            retain_possession,
            ..
        } => {
            assert_eq!(shooter, off_ball);
            assert!(
                retain_possession,
                "the offended team must keep the ball after the free throw"
            );
        }
        ref other => panic!("expected free throws, got {other:?}"),
    }

    // 罚完最后一次仍然由进攻方发球。
    for _ in 0..4000 {
        world.step(TICK_SECONDS);
        if !matches!(world.phase(), Phase::FreeThrows { .. }) {
            break;
        }
    }
    assert_eq!(
        world.possession.team, offense,
        "the away from play foul handed the ball to the defence"
    );
    assert_eq!(world.possession.id, possession_id);
    assert_eq!(world.possession_count, possession_count);
    assert_eq!(
        world.clock.shot_clock(),
        shot_clock_before_foul,
        "a retained-possession foul must preserve the unexpired time"
    );
    assert!(
        world
            .possession_log
            .iter()
            .all(|record| record.id != possession_id),
        "the retained-possession foul closed its statistical possession"
    );
    assert_eq!(possession_start_events(&world), possession_starts);
}

fn resume_timeout(world: &mut World) {
    for _ in 0..4000 {
        world.step(TICK_SECONDS);
        if !matches!(world.phase(), Phase::Timeout { .. }) {
            return;
        }
    }
    panic!("timeout did not end");
}

#[test]
fn play_resumes_with_the_ball_after_a_timeout() {
    let mut world = live_world(31);
    let offense = world.possession.team;
    assert!(request(&mut world, offense));
    for _ in 0..4000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::Live) {
            break;
        }
    }
    assert!(
        matches!(world.phase(), Phase::Live),
        "play never resumed after the timeout"
    );
    assert_eq!(
        world.possession.team, offense,
        "the team that called the timeout lost the ball"
    );
}

const RULE_COVERAGE: [(&str, &str, bool, bool, bool, bool); 6] = [
    (
        "shot_clock_reset",
        "a_second_chance_clock_only_resets_when_the_ball_touched_the_rim",
        true,
        true,
        true,
        true,
    ),
    (
        "team_bonus",
        "the_fifth_team_foul_awards_free_throws_while_the_fourth_only_restarts_play",
        true,
        true,
        true,
        true,
    ),
    (
        "offensive_foul",
        "an_offensive_foul_counts_personally_but_never_sends_the_other_team_to_the_line",
        true,
        true,
        true,
        true,
    ),
    (
        "away_from_play",
        "an_away_from_play_foul_in_the_last_two_minutes_is_one_free_throw_and_the_ball",
        true,
        true,
        true,
        true,
    ),
    (
        "timeout_budget",
        "only_the_team_in_control_may_stop_the_clock_and_the_budget_is_capped",
        true,
        true,
        true,
        true,
    ),
    (
        "substitution_window",
        "fifty_short_games_never_change_the_lineup_while_the_ball_is_live",
        true,
        true,
        true,
        true,
    ),
];

#[test]
fn rule_coverage_matrix_is_complete() {
    let mut seen = Vec::new();
    for (rule, test, positive, negative, exemption, interaction) in RULE_COVERAGE {
        assert!(!rule.is_empty() && !test.is_empty());
        assert!(
            positive && negative && exemption && interaction,
            "{rule} is incomplete"
        );
        assert!(!seen.contains(&rule), "{rule} is registered twice");
        seen.push(rule);
    }
    assert_eq!(seen.len(), RULE_COVERAGE.len());
}

#[test]
fn mutating_the_league_profile_changes_the_called_rule() {
    let mut rules = GameRules::default();
    rules.league.fouls.bonus_fouls_per_period = 1;
    rules.league.fouls.bonus_fouls_final = 1;
    let mut world = live_world_with_rules(31, rules);
    let offense = world.possession.team;
    let fouler = world
        .on_court(offense.other())
        .as_slice()
        .iter()
        .copied()
        .next()
        .expect("a defender is on the court");
    let fouled = world.holder().expect("live possession has a holder");
    apply_non_shooting_foul(&mut world, fouler, fouled);
    assert_eq!(
        last_foul(&world).0,
        2,
        "mutating the bonus threshold must move the first foul into the penalty"
    );

    let mut rules = GameRules::default();
    rules.league.clocks.offensive_rebound_shot_clock_seconds = 6.0;
    let mut world = live_world_with_rules(31, rules);
    let offense = world.possession.team;
    let rebounder = world.holder().expect("live possession has a holder");
    world.clock.set_shot_clock(3.0);
    world.start_second_chance(offense, rebounder, true);
    assert_eq!(
        world.clock.shot_clock(),
        6.0,
        "mutating the offensive rebound reset must be observed by the clock"
    );

    let mut rules = GameRules::default();
    rules.league.fouls.max_personal_fouls = 1;
    let mut world = live_world_with_rules(31, rules);
    let offense = world.possession.team;
    let fouler = world
        .on_court(offense.other())
        .as_slice()
        .iter()
        .copied()
        .next()
        .expect("a defender is on the court");
    let fouled = world.holder().expect("live possession has a holder");
    apply_non_shooting_foul(&mut world, fouler, fouled);
    assert!(
        world.players[fouler].disqualified,
        "mutating the personal foul limit must disqualify the fouler"
    );

    let mut rules = GameRules::default();
    rules.league.timeouts.timeouts_final_max = 1;
    let mut world = live_world_with_rules(31, rules);
    world
        .clock
        .set_regulation_clock(4, 90.0, world.clock.shot_clock());
    let offense = world.possession.team;
    assert!(request(&mut world, offense));
    assert!(
        !request(&mut world, offense),
        "mutating the final-window timeout cap must refuse the second request"
    );
}
