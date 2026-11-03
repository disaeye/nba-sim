use nba_domain::rules::GameRules;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::resolve::foul::book;
use nba_sim_core::rotation::{apply_dead_ball_window, note_intents, record_foul, update_stamina};
use nba_sim_core::state::Phase;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, TeamSide, World};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
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

fn home_lineup(world: &World) -> Vec<usize> {
    world.on_court(TeamSide::Home).as_slice().to_vec()
}

#[test]
fn a_player_who_sits_four_minutes_regains_stamina() {
    let mut world = world(13);
    world.set_phase(Phase::Live);
    let bench = (0..world.players.len())
        .find(|handle| !world.players[*handle].on_court)
        .expect("a bench player exists");
    world.players[bench].stamina = 0.60;
    let before = world.players[bench].stamina;
    // 4 分钟比赛时间，每 tick 0.05 秒。
    let ticks = (240.0 / TICK_SECONDS) as usize;
    for _ in 0..ticks {
        update_stamina(&mut world, TICK_SECONDS);
    }
    let after = world.players[bench].stamina;
    assert!(
        after > before,
        "bench stamina did not rise: {before:.4} -> {after:.4}"
    );
}

#[test]
fn live_play_never_changes_the_lineup() {
    let mut world = live_world(13);
    let exhausted = world.on_court(TeamSide::Home).as_slice()[0];
    world.players[exhausted].stamina = 0.0;
    let before = home_lineup(&world);
    // 活球期间登记意图，然后尝试结算：阵容必须一个都不动。
    note_intents(&mut world);
    apply_dead_ball_window(&mut world);
    assert_eq!(
        home_lineup(&world),
        before,
        "a substitution happened while the ball was live"
    );
    assert!(
        world.rotation.pending[0],
        "the exhausted player's substitution intent was not queued"
    );

    // 同一批意图在死球窗口内才被执行。
    world.set_phase(Phase::DeadBall {
        team: TeamSide::Home,
        ready_at: f64::MAX,
    });
    apply_dead_ball_window(&mut world);
    assert_ne!(
        home_lineup(&world),
        before,
        "the queued substitution never executed at the dead ball window"
    );
    assert!(
        !world.players[exhausted].on_court,
        "the exhausted player stayed on the court after the dead ball window"
    );
}

#[test]
fn nba_team_keeps_five_players_when_all_substitutes_are_disqualified() {
    let mut world = live_world(13);
    let team = TeamSide::Home;
    let on_court = world.on_court(team).as_slice().to_vec();
    let bench: Vec<usize> = (0..world.players.len())
        .filter(|handle| world.players[*handle].team == team && !world.players[*handle].on_court)
        .collect();
    for handle in bench {
        world.players[handle].fouls = world.rules.league.fouls.max_personal_fouls;
        world.players[handle].disqualified = true;
    }
    let fouled_out = on_court[0];
    world.players[fouled_out].fouls = world.rules.league.fouls.max_personal_fouls - 1;
    world.set_phase(Phase::DeadBall {
        team,
        ready_at: f64::MAX,
    });

    let fouled = world.on_court(TeamSide::Away).as_slice()[0];
    let booking = book(
        &mut world,
        fouled_out,
        fouled,
        nba_sim_core::action::FoulKind::Personal,
        0,
        false,
    );
    assert!(booking.team_technical_id.is_some());
    apply_dead_ball_window(&mut world);
    world.set_phase(Phase::Live);
    note_intents(&mut world);

    assert_eq!(world.on_court(team).as_slice().len(), 5);
    assert!(world.players[fouled_out].on_court);
    assert!(world.players[fouled_out].disqualified);
    assert_eq!(world.on_court(team).as_slice().len(), 5);
    assert!(world.players[fouled_out].disqualified);
    assert!(world.players[fouled_out].retained_after_foul_limit);
}

#[test]
fn nba_foul_out_with_empty_bench_assesses_team_technical_and_resumes_foul_procedure() {
    let mut world = live_world(93);
    while world.possession.team != TeamSide::Away {
        world.step(TICK_SECONDS);
    }
    let team = world.possession.team.other();
    let on_court = world.on_court(team).as_slice().to_vec();
    let fouler = on_court[0];
    let fouled = world.holder().expect("live offense has a ball holder");
    let possession_id = world.possession.id;
    let possession_count = world.possession_count;
    let possession_start_ids = world
        .events
        .iter()
        .filter_map(|event| match &event.fact {
            nba_sim_core::action::Fact::PossessionStarted { possession_id, .. } => {
                Some(*possession_id)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    for handle in 0..world.players.len() {
        if world.players[handle].team == team && !world.players[handle].on_court {
            world.players[handle].fouls = world.rules.league.fouls.max_personal_fouls;
            world.players[handle].disqualified = true;
        }
    }
    world.players[fouler].fouls = world.rules.league.fouls.max_personal_fouls - 1;
    world.ball.set_held(fouled);

    nba_sim_core::resolve::foul::apply_non_shooting_foul(&mut world, fouler, fouled);
    let technical_id = world
        .events
        .iter()
        .find_map(|event| match event.fact {
            nba_sim_core::action::Fact::TeamTechnicalFoul { foul_id, .. } => Some(foul_id),
            _ => None,
        })
        .expect("foul-out technical is recorded");
    assert!(matches!(world.phase(), Phase::FreeThrowFlight { .. }));
    assert!(world.players[fouler].disqualified);
    assert!(world.players[fouler].retained_after_foul_limit);
    assert_eq!(world.fouls[World::team_index(team)].technical, 1);
    assert!(world.events.iter().any(|event| matches!(
        event.fact,
        nba_sim_core::action::Fact::TeamTechnicalFoul { team: foul_team, foul_id, .. }
            if foul_team == team && foul_id == technical_id
    )));
    let shooter = world
        .pending_shot
        .as_ref()
        .expect("technical free throw is in flight")
        .shooter;
    assert_eq!(world.players[shooter].team, TeamSide::Away);
    let mut saw_free_throw = false;
    for _ in 0..800 {
        world.step(TICK_SECONDS);
        saw_free_throw |= world.events.iter().any(|event| matches!(
            event.fact,
            nba_sim_core::action::Fact::FreeThrow { foul_id, .. } if foul_id == technical_id
        ));
        if saw_free_throw && matches!(world.phase(), Phase::DeadBall { team: TeamSide::Away, .. }) {
            break;
        }
    }
    assert!(saw_free_throw, "team technical free throw did not resolve");
    assert!(matches!(world.phase(), Phase::DeadBall { team: TeamSide::Away, .. }));
    assert_eq!(world.possession.team, TeamSide::Away);
    assert_eq!(world.possession.id, possession_id);
    assert_eq!(world.possession_count, possession_count);
    assert!(world.possession_log.iter().all(|record| record.id != possession_id));
    assert_eq!(
        world
            .events
            .iter()
            .filter_map(|event| match &event.fact {
                nba_sim_core::action::Fact::PossessionStarted { possession_id, .. } => {
                    Some(*possession_id)
                }
                _ => None,
            })
            .collect::<Vec<_>>(),
        possession_start_ids,
        "administering the technical foul must not create a new offensive possession"
    );
}

#[test]
fn a_six_foul_player_never_returns_to_the_court() {
    let mut world = live_world(13);
    let rules = world.rules.clone();
    let limit = rules.league.fouls.max_personal_fouls;
    let victim = world.on_court(TeamSide::Home).as_slice()[0];
    for _ in 0..limit - 1 {
        record_foul(&mut world, victim);
    }
    assert!(!world.players[victim].disqualified);
    assert!(
        record_foul(&mut world, victim),
        "the sixth foul did not report disqualification"
    );
    assert!(
        world.players[victim].disqualified,
        "the sixth foul did not disqualify the player"
    );

    // 犯满离场发生在停表程序里；之后任何时刻都不再上场。
    let mut left = false;
    for _ in 0..4000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::Live) {
            assert!(
                !world.players[victim].on_court,
                "a disqualified player was on the court while the ball was live"
            );
        }
        if !world.players[victim].on_court {
            left = true;
        }
        if left {
            assert!(
                !world.players[victim].on_court,
                "the disqualified player returned to the court"
            );
        }
    }
    assert!(left, "the disqualified player never left the court");
}
