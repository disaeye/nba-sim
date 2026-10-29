use glam::Vec2;
use nba_sim_core::action::{ActionIntent, ActionKind, Fact, ShotKind};
use nba_sim_core::params::RuntimeParams;
use nba_sim_core::policy::{candidate_utilities, putback_available, shot_choice};
use nba_sim_core::resolve::drive;
use nba_sim_core::state::{BallPhase, DribbleState, Phase};
use nba_sim_core::world::{MatchSetup, World};
use nba_sim_core::TeamSide;

fn finishing_world(actor_pos: Vec2) -> World {
    let mut world = World::new(
        MatchSetup::builtin(nba_domain::GameRules::default()),
        RuntimeParams::embedded(),
        31,
    );
    for player in world.players.iter_mut() {
        player.on_court = false;
    }
    let defender = world
        .players
        .iter()
        .position(|player| player.team != TeamSide::Home)
        .expect("away defender");
    world.players[0].on_court = true;
    world.players[1].on_court = true;
    world.players[defender].on_court = true;
    world.players[0].pos = actor_pos;
    world.players[1].pos = actor_pos + Vec2::new(4.0, 4.0);
    world.players[defender].pos = actor_pos + Vec2::new(2.0, 0.0);
    world.possession.team = TeamSide::Home;
    world.phase = Phase::Live;
    world.ball.set_held(0);
    world
}

fn choice_for(world: &mut World, phase: BallPhase, pos: Vec2) -> (ShotKind, &'static str) {
    world.players[0].pos = pos;
    world.possession.ball_phase = phase;
    // 这些近期时间戳故意留空：出手类型只能由当前阶段与实际位置给出。
    world.possession.last_drive_at = None;
    world.possession.last_post_at = None;
    world.possession.last_cut_at = None;
    world.possession.last_pass_at = None;
    let state = world.decision_state();
    shot_choice(world, &state).expect("shot choice at this position")
}

#[test]
fn shot_type_is_read_from_current_phase_and_actual_position() {
    let mut world = finishing_world(Vec2::new(80.0, 25.0));
    let hoop = world.hoop_for(TeamSide::Home);
    let rim = hoop - Vec2::new(3.0, 0.0);
    let near = hoop - Vec2::new(8.0, 0.0);
    let mid = hoop - Vec2::new(18.0, 0.0);
    let three = hoop - Vec2::new(26.0, 0.0);

    assert!(matches!(
        choice_for(&mut world, BallPhase::Drive, rim).0,
        ShotKind::DrivingLayup | ShotKind::DrivingDunk
    ));
    assert_eq!(choice_for(&mut world, BallPhase::Post, rim).0, ShotKind::Hook);
    assert!(matches!(
        choice_for(&mut world, BallPhase::Cut, rim).0,
        ShotKind::CuttingLayup | ShotKind::CuttingDunk
    ));
    assert_eq!(
        choice_for(&mut world, BallPhase::Drive, near).0,
        ShotKind::Floater
    );
    assert_eq!(
        choice_for(&mut world, BallPhase::Dribble, mid).0,
        ShotKind::PullUp
    );
    assert_eq!(
        choice_for(&mut world, BallPhase::Catch, three).0,
        ShotKind::CatchShoot
    );
    assert_eq!(
        choice_for(&mut world, BallPhase::Dribble, three).0,
        ShotKind::PullUp
    );
    assert!(matches!(
        choice_for(&mut world, BallPhase::Gather, rim).0,
        ShotKind::Layup | ShotKind::Dunk
    ));
    assert!(matches!(
        choice_for(&mut world, BallPhase::Dribble, near).0,
        ShotKind::PullUp | ShotKind::Floater | ShotKind::Layup
    ));
}

#[test]
fn putback_is_bound_to_the_actual_rebounder_not_a_global_timer() {
    let mut world = finishing_world(Vec2::new(85.0, 25.0));
    world.possession.ball_phase = BallPhase::Putback;
    world.possession.last_offensive_rebound_actor = Some(0);
    assert!(putback_available(&world, 0));
    assert!(!putback_available(&world, 1));

    let state = world.decision_state();
    assert_eq!(shot_choice(&world, &state).expect("putback choice").0, ShotKind::Putback);

    world.possession.last_offensive_rebound_actor = Some(1);
    assert!(!putback_available(&world, 0));
    let state = world.decision_state();
    assert_ne!(
        shot_choice(&world, &state).expect("non-rebounder choice").0,
        ShotKind::Putback
    );
}

#[test]
fn rebound_audit_reports_opportunities_and_second_chance_boundaries() {
    let report = nba_sim_core::run_game_default(1);
    let mut opportunities = 0u32;
    let mut expected_second_chance = false;
    let mut second_chances = 0u32;
    for event in &report.events {
        match &event.fact {
            Fact::ReboundOpportunity {
                players,
                contestable,
            } => {
                opportunities += 1;
                assert!(*players > 0);
                assert!(*contestable <= *players);
            }
            Fact::Rebound { offensive, .. } if *offensive => expected_second_chance = true,
            Fact::SecondChanceStarted { .. } => second_chances += 1,
            _ => {}
        }
    }
    assert!(opportunities > 0, "a miss must expose a rebound opportunity");
    if expected_second_chance {
        assert!(second_chances > 0, "an offensive rebound must open a second-chance boundary");
    }
}

#[test]
fn ball_phase_changes_only_at_actual_resolution_points() {
    let mut world = finishing_world(Vec2::new(60.0, 25.0));
    let intent = ActionIntent {
        actor: 0,
        kind: ActionKind::Drive,
        target: None,
        from_ft: world.players[0].pos,
        to_ft: world.hoop_for(TeamSide::Home),
        start_time: world.now(),
        duration: 1.0,
    };
    drive::resolve(&mut world, &intent);
    assert_eq!(world.possession.ball_phase, BallPhase::Drive);

    world.players[0].pos = world.hoop_for(TeamSide::Home) - Vec2::new(4.0, 0.0);
    world.possession.gather_pos = Some(world.players[0].pos);
    let finish = ActionIntent {
        kind: ActionKind::Finish,
        ..intent.clone()
    };
    drive::resolve_finish(&mut world, &finish);
    assert_eq!(world.possession.ball_phase, BallPhase::Gather);
    assert_eq!(world.possession.dribble_state, DribbleState::Gathered);

    let post = ActionIntent {
        kind: ActionKind::PostUp,
        ..intent
    };
    drive::resolve_post(&mut world, &post);
    assert_eq!(world.possession.ball_phase, BallPhase::Post);

    world.start_second_chance(TeamSide::Home, 1, true);
    assert_eq!(world.possession.ball_phase, BallPhase::Putback);
    assert_eq!(world.possession.last_offensive_rebound_actor, Some(1));
}

#[test]
fn drive_and_finish_do_not_both_carry_the_full_possession_value() {
    let mut world = finishing_world(Vec2::new(70.0, 25.0));
    let state = world.decision_state();
    let drive = candidate_utilities(&world, &state)
        .into_iter()
        .find(|candidate| candidate.0 == ActionKind::Drive)
        .expect("drive candidate at 20 ft");
    let possession_value = world.params.policy.w("possession_value");
    let old_full_value = possession_value
        * (1.0 + world.params.policy.w("drive_paint_bonus"));
    assert!((drive.3.epv.ability - old_full_value).abs() > 1e-4);

    world.players[0].pos = world.hoop_for(TeamSide::Home) - Vec2::new(4.0, 0.0);
    world.possession.ball_phase = BallPhase::Drive;
    let state = world.decision_state();
    let finish = candidate_utilities(&world, &state)
        .into_iter()
        .find(|candidate| candidate.0 == ActionKind::Finish)
        .expect("finish candidate at 4 ft");
    assert!((finish.3.epv.ability - old_full_value).abs() > 1e-4);
}
