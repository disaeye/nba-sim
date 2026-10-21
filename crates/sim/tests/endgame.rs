use nba_domain::rules::GameRules;
use nba_sim_core::action::ActionKind;
use nba_sim_core::endgame::{intentional_foul, shot_constraint, EndgameShot};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::resolve::shooter_zone;
use nba_sim_core::state::{Phase, PossessionKind};
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, TeamSide, World};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

/// 推进到末节的一次活球进攻。
fn fourth_quarter_live(world: &mut World) -> usize {
    for _ in 0..400_000 {
        world.step(TICK_SECONDS);
        if world.clock.period >= 4
            && matches!(world.phase, Phase::Live)
            && world.possession.kind == PossessionKind::Live
        {
            if let Some(handle) = world.holder() {
                return handle;
            }
        }
    }
    panic!("no fourth-quarter live possession");
}

fn set_score(world: &mut World, team: TeamSide, diff: i32) {
    let base = 100;
    let mine = World::team_index(team);
    let theirs = World::team_index(team.other());
    if diff >= 0 {
        world.score[mine] = base + diff as u32;
        world.score[theirs] = base;
    } else {
        world.score[mine] = base;
        world.score[theirs] = base + (-diff) as u32;
    }
}

#[test]
fn trailing_by_three_late_takes_only_threes() {
    let mut world = world(13);
    let carrier = fourth_quarter_live(&mut world);
    let team = world.players[carrier].team;
    set_score(&mut world, team, -3);
    world.clock.game_clock = 7.0;
    // 把持球人放到弧顶，三分才是可选项。
    let hoop = world.hoop_for(team);
    let spot = hoop
        + (hoop - world.players[carrier].pos).normalize_or_zero() * 25.0;
    world.players[carrier].pos = spot;
    world.players[carrier].target = spot;
    assert_eq!(shot_constraint(&world), EndgameShot::ThreeOnly);
    let mut threes = 0;
    for _ in 0..300 {
        let intent = nba_sim_core::policy::choose_action(&mut world);
        if let ActionKind::Shoot(_) = intent.kind {
            let actor = intent.actor;
            let zone = shooter_zone(&world, world.players[actor].pos, world.players[actor].team);
            assert_eq!(zone, "three", "a two-point shot was taken while down three late");
            threes += 1;
        }
    }
    assert!(threes > 0, "the trailing team never attempted a three");
}

#[test]
fn leading_by_four_late_holds_until_the_shot_clock_runs_down() {
    let mut world = world(13);
    let carrier = fourth_quarter_live(&mut world);
    let team = world.players[carrier].team;
    set_score(&mut world, team, 4);
    world.clock.game_clock = 26.0;
    world.clock.shot_clock = 20.0;
    assert_eq!(shot_constraint(&world), EndgameShot::Hold);
    for _ in 0..200 {
        let intent = nba_sim_core::policy::choose_action(&mut world);
        assert!(
            !matches!(intent.kind, ActionKind::Shoot(_)),
            "the leading team shot with {:.1}s left on the shot clock",
            world.clock.shot_clock
        );
    }
    // 进攻时钟走到阈值以下就恢复出手自由。
    world.clock.shot_clock = 4.0;
    assert_eq!(shot_constraint(&world), EndgameShot::Free);
}

#[test]
fn trailing_team_commits_an_intentional_foul_when_down_four_late() {
    let mut world = world(13);
    let carrier = fourth_quarter_live(&mut world);
    let team = world.players[carrier].team;
    set_score(&mut world, team, 4);
    world.clock.game_clock = 26.0;
    let fouler = intentional_foul(&world);
    assert!(fouler.is_some(), "the trailing defense did not commit an intentional foul");
    let fouler = fouler.expect("fouler");
    assert_eq!(world.players[fouler].team, team.other());
}

#[test]
fn hack_a_shaq_targets_the_poor_free_throw_shooter_before_the_last_two_minutes() {
    let mut world = world(13);
    let carrier = fourth_quarter_live(&mut world);
    let team = world.players[carrier].team;
    set_score(&mut world, team, 2);
    world.clock.game_clock = 205.0;
    world.players[carrier].data.attributes.free_throw = 0.36;
    let fouler = intentional_foul(&world);
    assert!(
        fouler.is_some(),
        "the poor free-throw shooter was not intentionally fouled at 3:30"
    );
    assert_eq!(world.players[fouler.expect("fouler")].team, team.other());
}

#[test]
fn no_off_ball_intentional_foul_in_the_last_two_minutes() {
    let mut world = world(13);
    let carrier = fourth_quarter_live(&mut world);
    let team = world.players[carrier].team;
    set_score(&mut world, team, 2);
    world.clock.game_clock = 90.0;
    // 无球人罚球再差也不能砍：最后两分钟无球故意犯规是一罚一掷。
    for handle in 0..world.players.len() {
        if world.players[handle].team == team && handle != carrier {
            world.players[handle].data.attributes.free_throw = 0.20;
        }
    }
    let fouler = intentional_foul(&world);
    if let Some(fouler) = fouler {
        let distance = (world.players[fouler].pos - world.players[carrier].pos).length();
        assert!(
            distance < world.params.policy.w("screen_clearance_ft"),
            "an off-ball defender was sent to commit an intentional foul at 1:30"
        );
    }
}
