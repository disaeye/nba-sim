use nba_domain::rules::GameRules;
use nba_sim_core::action::{ActionIntent, ActionKind, Fact, JumpBallKind, ShotKind};
use nba_sim_core::params::RuntimeParams;
use nba_sim_core::state::{MatchProgram, Phase};
use nba_sim_core::world::{MatchSetup, World, TICK_SECONDS};

fn world(rules: GameRules) -> World {
    World::new(MatchSetup::builtin(rules), RuntimeParams::embedded(), 17)
}

fn advance_to_first_possession(world: &mut World) {
    for _ in 0..500 {
        world.step(nba_sim_core::world::TICK_SECONDS);
        if matches!(world.phase(), Phase::Live) && world.holder().is_some() {
            return;
        }
    }
    panic!("jump-ball procedure did not produce a controlled possession");
}

#[test]
fn nba_opening_possession_is_established_by_a_resolved_center_jump_ball() {
    let mut world = world(GameRules::default());
    assert!(matches!(
        world.phase(),
        Phase::JumpBall {
            kind: JumpBallKind::Opening,
            ..
        }
    ));
    assert_eq!(world.match_program(), MatchProgram::JumpBall);
    assert_eq!(world.possession_count, 0);
    assert!(world.holder().is_none());

    advance_to_first_possession(&mut world);

    let jump = world
        .events
        .iter()
        .find_map(|event| match &event.fact {
            Fact::JumpBall {
                kind,
                home_jumper,
                away_jumper,
                tapper,
                tapped_team,
            } => Some((*kind, *home_jumper, *away_jumper, *tapper, *tapped_team)),
            _ => None,
        })
        .expect("opening jump-ball result is published");
    assert_eq!(jump.0, JumpBallKind::Opening);
    assert_ne!(jump.1, jump.2);
    assert!(jump.3 == jump.1 || jump.3 == jump.2);
    assert_eq!(world.first_possession, Some(jump.4));
    assert_eq!(world.possession.team, jump.4);
    assert_eq!(world.possession_count, 1);
    assert_eq!(
        world.players[world.holder().expect("possession controller")].team,
        jump.4
    );
    assert_eq!(
        world.hoop_for(jump.4),
        world.rules.court.hoop_pos(world.attacking_right(jump.4)),
        "first-possession direction must determine the attacking hoop"
    );
}

#[test]
fn unresolved_live_shot_is_settled_before_period_break() {
    let mut world = world(GameRules::default());
    advance_to_first_possession(&mut world);
    let shooter = world.holder().expect("live possession has a shooter");
    let intent = ActionIntent {
        actor: shooter,
        kind: ActionKind::Shoot(ShotKind::CatchShoot),
        target: None,
        from_ft: world.players[shooter].pos,
        to_ft: world.players[shooter].pos,
        start_time: world.now(),
        duration: 0.0,
    };
    nba_sim_core::resolve::shot::resolve(&mut world, &intent, ShotKind::CatchShoot);
    world
        .clock
        .set_regulation_clock(1, 0.0, world.clock.shot_clock());

    for _ in 0..200 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::PeriodBreak { .. } | Phase::Finished) {
            break;
        }
    }

    assert!(matches!(world.phase(), Phase::PeriodBreak { .. }));
    assert!(world.pending_shot.is_none());
    assert!(world.pending_rebound.is_none());
    assert!(world.events.iter().any(|event| matches!(event.fact, Fact::Shot { .. })));
    assert!(world.events.iter().any(|event| matches!(event.fact, Fact::PeriodEnd { period: 1 })));
    assert_eq!(world.possession.id, 0);
}

#[test]
fn nba_non_overtime_periods_alternate_the_opening_throw_in() {
    let mut world = world(GameRules::default());
    advance_to_first_possession(&mut world);
    let first_period_team = world.first_possession.expect("opening jump-ball team");
    world
        .clock
        .set_regulation_clock(1, 0.0, world.clock.shot_clock());
    world.step(nba_sim_core::world::TICK_SECONDS);
    assert!(matches!(world.phase(), Phase::PeriodBreak { .. }));

    for _ in 0..100 {
        world.step(nba_sim_core::world::TICK_SECONDS);
        if matches!(world.phase(), Phase::Inbound { .. }) {
            break;
        }
    }
    assert_eq!(world.clock.period(), 2);
    assert!(
        matches!(world.phase(), Phase::Inbound { team, .. } if team == first_period_team.other())
    );
    assert_eq!(world.period_started_with, first_period_team.other());
    assert_eq!(world.alternate_possession_next, first_period_team.other());
    assert!(!world.attacking_right(nba_sim_core::TeamSide::Home));
    assert!(world.attacking_right(nba_sim_core::TeamSide::Away));
    for _ in 0..100 {
        world.step(nba_sim_core::world::TICK_SECONDS);
        if world.possession_count == 2 {
            break;
        }
    }
    assert_eq!(world.possession.team, first_period_team.other());
    assert_eq!(world.alternate_possession_next, first_period_team);

    world
        .clock
        .set_regulation_clock(2, 0.0, world.clock.shot_clock());
    world.step(nba_sim_core::world::TICK_SECONDS);
    assert!(matches!(world.phase(), Phase::PeriodBreak { .. }));
    for _ in 0..100 {
        world.step(nba_sim_core::world::TICK_SECONDS);
        if matches!(world.phase(), Phase::Inbound { .. }) {
            break;
        }
    }
    assert_eq!(world.clock.period(), 3);
    assert!(matches!(world.phase(), Phase::Inbound { team, .. } if team == first_period_team));
    assert_eq!(world.period_started_with, first_period_team);
    assert_eq!(world.alternate_possession_next, first_period_team);
    assert!(world.attacking_right(nba_sim_core::TeamSide::Home));
    assert!(!world.attacking_right(nba_sim_core::TeamSide::Away));
}

#[test]
fn nba_overtime_starts_with_a_fresh_jump_ball() {
    let mut world = world(GameRules::default());
    advance_to_first_possession(&mut world);
    world
        .clock
        .set_regulation_clock(4, 0.0, world.clock.shot_clock());
    world.score = [88, 88];
    world.step(nba_sim_core::world::TICK_SECONDS);
    assert!(matches!(world.phase(), Phase::PeriodBreak { .. }));

    for _ in 0..100 {
        world.step(nba_sim_core::world::TICK_SECONDS);
        if matches!(
            world.phase(),
            Phase::JumpBall {
                kind: JumpBallKind::Overtime,
                ..
            }
        ) {
            break;
        }
    }
    assert_eq!(world.clock.period(), 5);
    assert_eq!(world.clock.game_clock(), 300.0);
    assert!(matches!(
        world.phase(),
        Phase::JumpBall {
            kind: JumpBallKind::Overtime,
            ..
        }
    ));
    let prior_arrow = world.alternate_possession_next;
    let previous_possessions = world.possession_count;
    advance_to_first_possession(&mut world);
    assert!(world.events.iter().any(|event| matches!(
        event.fact,
        Fact::JumpBall {
            kind: JumpBallKind::Overtime,
            ..
        }
    )));
    assert!(matches!(world.phase(), Phase::Live));
    assert_eq!(world.possession_count, previous_possessions + 1);
    assert_eq!(world.alternate_possession_next, prior_arrow);
    assert!(
        matches!(world.events.iter().rev().find_map(|event| match &event.fact {
        Fact::JumpBall { kind, tapped_team, .. } if *kind == JumpBallKind::Overtime => Some((*kind, *tapped_team)),
        _ => None,
    }), Some((JumpBallKind::Overtime, team)) if team == world.possession.team)
    );
}

#[test]
fn fiba_period_starts_follow_the_alternating_possession_arrow() {
    let mut rules = GameRules::default();
    rules.league = nba_domain::league::LeagueProfile::fiba();
    let mut world = world(rules);
    assert!(matches!(
        world.phase(),
        Phase::JumpBall {
            kind: JumpBallKind::Opening,
            ..
        }
    ));
    advance_to_first_possession(&mut world);
    let opening_team = world.first_possession.expect("FIBA opening jump-ball team");
    assert_eq!(world.alternate_possession_next, opening_team.other());

    world
        .clock
        .set_regulation_clock(1, 0.0, world.clock.shot_clock());
    world.step(nba_sim_core::world::TICK_SECONDS);
    assert!(matches!(world.phase(), Phase::PeriodBreak { .. }));
    for _ in 0..100 {
        world.step(nba_sim_core::world::TICK_SECONDS);
        if matches!(world.phase(), Phase::Inbound { .. }) {
            break;
        }
    }
    assert_eq!(world.clock.period(), 2);
    let second_period_team = opening_team.other();
    assert!(matches!(world.phase(), Phase::Inbound { team, .. } if team == second_period_team));
    assert_eq!(world.period_started_with, second_period_team);
    assert!(!world.attacking_right(nba_sim_core::TeamSide::Home));
    assert!(world.attacking_right(nba_sim_core::TeamSide::Away));
    for _ in 0..100 {
        world.step(nba_sim_core::world::TICK_SECONDS);
        if world.possession_count == 2 {
            break;
        }
    }
    assert_eq!(world.possession.team, second_period_team);
    assert_eq!(world.alternate_possession_next, opening_team);

    world
        .clock
        .set_regulation_clock(2, 0.0, world.clock.shot_clock());
    world.step(nba_sim_core::world::TICK_SECONDS);
    assert!(matches!(world.phase(), Phase::PeriodBreak { .. }));
    for _ in 0..100 {
        world.step(nba_sim_core::world::TICK_SECONDS);
        if matches!(world.phase(), Phase::Inbound { .. }) {
            break;
        }
    }
    assert_eq!(world.clock.period(), 3);
    assert!(matches!(world.phase(), Phase::Inbound { team, .. } if team == opening_team));
    assert_eq!(world.period_started_with, opening_team);
    assert!(world.attacking_right(nba_sim_core::TeamSide::Home));
    assert!(!world.attacking_right(nba_sim_core::TeamSide::Away));
}
