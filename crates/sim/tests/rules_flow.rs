//! 规则状态机的世界级守卫：进攻时钟重置、犯规罚则矩阵、死球换人门控、暂停配额。
//!
//! 纯函数层面的阈值矩阵在 `nba-domain` 的 `game_flow_transitions` 里卡死；
//! 这里卡的是「引擎真的按那份阈值执行」：球权、阶段、配额三件事必须对得上。

use nba_domain::rules::GameRules;
use nba_sim_core::action::Fact;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::resolve::foul::{apply_non_shooting_foul, apply_offensive_foul};
use nba_sim_core::state::Phase;
use nba_sim_core::timeouts::{can_request, request, TimeoutRejection};
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

fn live_world(seed: u64) -> World {
    let mut world = world(seed);
    for _ in 0..40_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase, Phase::Live) && world.holder().is_some() {
            return world;
        }
    }
    panic!("the engine never reached a live ball state");
}

fn is_dead_ball(phase: &Phase) -> bool {
    !matches!(
        phase,
        Phase::Live | Phase::PassFlight { .. } | Phase::ShotFlight { .. }
    )
}

fn last_foul(world: &World) -> (u8, u8) {
    for event in world.events.iter().rev() {
        if let Fact::Foul {
            free_throws,
            kind,
            ..
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

    world.clock.shot_clock = 8.0;
    world.start_second_chance(offense, rebounder, true);
    assert_eq!(
        world.clock.shot_clock, 14.0,
        "a rim-touching offensive rebound must refill the clock to 14"
    );

    world.clock.shot_clock = 16.0;
    world.start_second_chance(offense, rebounder, true);
    assert_eq!(
        world.clock.shot_clock, 16.0,
        "a rim-touching offensive rebound must not shorten a longer clock"
    );

    world.clock.shot_clock = 8.0;
    world.start_second_chance(offense, rebounder, false);
    assert_eq!(
        world.clock.shot_clock, 8.0,
        "an offensive rebound off a shot that never touched the rim must not reset the clock"
    );
}

#[test]
fn the_fifth_team_foul_awards_free_throws_while_the_fourth_only_restarts_play() {
    let mut world = live_world(31);
    world.clock.period = 2;
    world.clock.game_clock = 400.0;
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
                matches!(world.phase, Phase::DeadBall { team, .. } if team == offense),
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
    world.clock.period = 4;
    world.clock.game_clock = 90.0;
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
    assert_eq!(last_foul(&world).0, 0, "the first foul must not award free throws");
    assert!(matches!(world.phase, Phase::DeadBall { .. }));

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
    world.clock.period = 2;
    world.clock.game_clock = 400.0;
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
    assert_eq!(last_foul(&world).0, 0, "an offensive foul awards no free throws");
    assert!(
        matches!(world.phase, Phase::DeadBall { team, .. } if team == offense.other()),
        "an offensive foul must hand the ball to the other team after a stoppage"
    );
}

#[test]
fn only_the_team_in_control_may_stop_the_clock_and_the_budget_is_capped() {
    let mut world = live_world(31);
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
    assert!(matches!(world.phase, Phase::Timeout { .. }));
    assert!(
        can_request(&world, offense.other()).is_ok(),
        "during a dead ball either team may request a timeout"
    );

    // 第四节：最多四次。
    world.clock.period = 4;
    world.clock.game_clock = 400.0;
    world.timeouts[World::team_index(offense)] = nba_sim_core::timeouts::TimeoutBudget {
        remaining: 7,
        used_in_fourth: 1,
        used_in_final_window: 0,
        period: 4,
    };
    for _ in 0..3 {
        assert!(request(&mut world, offense));
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
    world.clock.game_clock = 120.0;
    for _ in 0..2 {
        assert!(request(&mut world, offense));
    }
    assert_eq!(
        can_request(&world, offense),
        Err(TimeoutRejection::FinalWindowExhausted),
        "a third timeout inside the final window must be refused"
    );
}

#[test]
fn one_hundred_full_games_never_change_the_lineup_while_the_ball_is_live() {
    let mut violations = 0;
    let mut substitutions = 0;
    for seed in 1..=100u64 {
        let mut world = world(seed);
        while !world.finished && world.clock.tick < 2_000_000 {
            let before = world.phase.clone();
            let lineup_before: Vec<usize> = world
                .players
                .iter()
                .enumerate()
                .filter(|(_, p)| p.on_court)
                .map(|(handle, _)| handle)
                .collect();
            world.step(TICK_SECONDS);
            let lineup_after: Vec<usize> = world
                .players
                .iter()
                .enumerate()
                .filter(|(_, p)| p.on_court)
                .map(|(handle, _)| handle)
                .collect();
            if lineup_before != lineup_after {
                substitutions += 1;
                if !is_dead_ball(&before) && !is_dead_ball(&world.phase) {
                    violations += 1;
                }
            }
        }
        assert!(world.finished, "seed {seed}: the game did not finish");
    }
    assert!(
        substitutions > 0,
        "no substitution happened across one hundred games"
    );
    assert_eq!(
        violations, 0,
        "{violations} substitutions happened with the ball live across one hundred games"
    );
}

#[test]
fn an_away_from_play_foul_in_the_last_two_minutes_is_one_free_throw_and_the_ball() {
    let mut world = live_world(31);
    world.clock.period = 4;
    world.clock.game_clock = 90.0;
    let offense = world.possession.team;
    let fouler = world
        .on_court(offense.other())
        .as_slice()
        .iter()
        .copied()
        .next()
        .expect("a defender is on the court");
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
    match world.phase {
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
        if !matches!(world.phase, Phase::FreeThrows { .. }) {
            break;
        }
    }
    assert_eq!(
        world.possession.team, offense,
        "the away from play foul handed the ball to the defence"
    );
}

#[test]
fn play_resumes_with_the_ball_after_a_timeout() {
    let mut world = live_world(31);
    let offense = world.possession.team;
    assert!(request(&mut world, offense));
    for _ in 0..4000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase, Phase::Live) {
            break;
        }
    }
    assert!(
        matches!(world.phase, Phase::Live),
        "play never resumed after the timeout"
    );
    assert_eq!(
        world.possession.team, offense,
        "the team that called the timeout lost the ball"
    );
}
