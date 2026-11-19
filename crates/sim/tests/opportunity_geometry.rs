use glam::Vec2;
use nba_sim_core::action::DefenderRoute;
use nba_sim_core::opportunity::{remember, shot_window};
use nba_sim_core::state::ScreenRead;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, RuntimeParams, World};

#[test]
fn the_same_screen_route_can_leave_or_close_the_release_window() {
    let mut baseline = World::new(MatchSetup::builtin(nba_domain::GameRules::default()), RuntimeParams::embedded(), 1);
    for _ in 0..60 {
        if baseline.live_play() && baseline.holder().is_some() {
            break;
        }
        baseline.step(TICK_SECONDS);
    }
    let actor = baseline.holder().expect("opening control");
    let team = baseline.players[actor].team;
    let defender = baseline.on_court(team.other()).as_slice()[0];
    let screener = *baseline.on_court(team).as_slice().iter().find(|index| **index != actor).unwrap();
    let hoop = baseline.hoop_for(team);
    let direction = (hoop - Vec2::new(baseline.rules.court.width_ft / 2.0, hoop.y)).normalize();
    let origin = hoop - direction * 24.0;
    baseline.players[actor].reads.clear();
    baseline.players[actor].pos = origin;
    baseline.players[actor].facing = direction;
    baseline.players[defender].vel = Vec2::ZERO;
    let reach = baseline.params.policy.g("contest_proximity_ft");
    let other: Vec<_> = baseline.players.iter().enumerate().filter(|(index, player)| player.on_court && *index != actor && *index != defender).map(|(index, _)| index).collect();
    for (slot, player) in other.into_iter().enumerate() {
        baseline.players[player].pos = origin - direction * (8.0 + slot as f32 * 2.0) + Vec2::Y * 10.0;
    }
    baseline.possession.last_screen = Some(ScreenRead { route: DefenderRoute::Under, at: baseline.now(), screener, separation_ft: reach });
    let mut far = baseline.clone();
    baseline.players[defender].pos = origin + direction * (reach - 1.0);
    far.players[defender].pos = origin + direction * (reach + 9.0);
    remember(&mut baseline, actor);
    remember(&mut far, actor);
    assert_eq!(nba_sim_core::views::screen_read_route(&baseline), nba_sim_core::views::screen_read_route(&far));
    let shot = |world: &World| nba_sim_core::policy::candidate_utilities(world, &world.decision_state())
        .into_iter().find(|(kind, _, _, _)| matches!(kind, nba_sim_core::ActionKind::Shoot(_)))
        .expect("shot is a candidate").3.epv_base();
    assert!(shot(&far) > shot(&baseline));
    let close = shot_window(&baseline, actor, origin, 0.5).margin().expect("close defender seen");
    let open = shot_window(&far, actor, origin, 0.5).margin().expect("distant defender seen");
    assert!(close < 0.0 && open > 0.0, "close={close}, open={open}");
}

#[test]
fn an_unseen_defender_movement_does_not_change_the_holders_candidates() {
    let mut world = World::new(MatchSetup::builtin(nba_domain::GameRules::default()), RuntimeParams::embedded(), 13);
    for _ in 0..100 {
        if world.live_play() && world.holder().is_some() {
            break;
        }
        world.step(TICK_SECONDS);
    }
    let actor = world.holder().expect("opening control");
    let team = world.players[actor].team;
    let defender = world.on_court(team.other()).as_slice()[0];
    let hoop = world.hoop_for(team);
    let direction = (hoop - Vec2::new(world.rules.court.width_ft / 2.0, hoop.y)).normalize();
    let origin = hoop - direction * 24.0;
    world.players[actor].pos = origin;
    world.players[actor].facing = direction;
    world.players[actor].reads.clear();
    let players: Vec<_> = world.players.iter().enumerate().filter(|(index, player)| player.on_court && *index != actor).map(|(index, _)| index).collect();
    for (slot, index) in players.into_iter().enumerate() {
        world.players[index].pos = origin - direction * (12.0 + slot as f32) + Vec2::Y * 8.0;
        world.players[index].vel = Vec2::ZERO;
    }
    world.players[defender].pos = origin - direction * 8.0;
    remember(&mut world, actor);
    assert!(nba_sim_core::opportunity::read(&world, actor, defender).is_none());
    let mut moved = world.clone();
    moved.players[defender].pos = origin - direction * 2.0;
    moved.players[defender].vel = direction;
    let candidates = |state: &World| nba_sim_core::policy::candidate_utilities(state, &state.decision_state());
    let before = candidates(&world);
    let after = candidates(&moved);
    assert_eq!(before.len(), after.len());
    for (left, right) in before.iter().zip(&after) {
        assert_eq!(left.0, right.0);
        assert_eq!(left.1, right.1);
        assert_eq!(left.2, right.2);
        assert_eq!(left.3.epv_base(), right.3.epv_base());
    }
    assert_eq!(nba_sim_core::views::observation_digest(&world, actor), nba_sim_core::views::observation_digest(&moved, actor));
}

#[test]
fn a_memory_expires_without_reading_the_hidden_position() {
    let mut world = World::new(MatchSetup::builtin(nba_domain::GameRules::default()), RuntimeParams::embedded(), 19);
    for _ in 0..100 {
        if world.live_play() && world.holder().is_some() {
            break;
        }
        world.step(TICK_SECONDS);
    }
    let actor = world.holder().expect("opening control");
    let defender = world.on_court(world.players[actor].team.other()).as_slice()[0];
    let origin = world.players[actor].pos;
    world.players[actor].facing = Vec2::X;
    world.players[actor].reads.clear();
    for index in 0..world.players.len() {
        if index != actor && world.players[index].on_court {
            world.players[index].pos = origin - Vec2::X * 15.0 + Vec2::Y * (index as f32 + 2.0);
        }
    }
    world.players[defender].pos = origin + Vec2::X * 8.0;
    world.players[defender].vel = Vec2::ZERO;
    remember(&mut world, actor);
    let last = nba_sim_core::opportunity::read(&world, actor, defender).expect("defender observed");
    world.players[defender].pos = origin - Vec2::X * 5.0;
    let predicted = nba_sim_core::opportunity::read(&world, actor, defender).expect("remembered defender");
    assert_eq!(predicted.position, last.position);
    let horizon = world.params.policy.g("perception_memory_seconds");
    for _ in 0..=((horizon / TICK_SECONDS as f32).ceil() as usize) {
        world.clock.advance_tick();
    }
    assert!(nba_sim_core::opportunity::read(&world, actor, defender).is_none());
}
