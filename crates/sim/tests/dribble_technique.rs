//! 运球技术细分：变向、后撤步、迟疑步与原地推进各自改变真实状态。
//!
//! 技术细分不是标签：变向换到横向空间并把球收在体侧，迟疑步冻结防守人的
//! 伸手时机，原地推进保持几何暴露。测试读的是结算真正用到的那些量。

use glam::Vec2;
use nba_domain::action_window::DribbleMoveKind;
use nba_domain::rules::GameRules;
use nba_sim_core::action::{ActionIntent, ActionKind, Fact};
use nba_sim_core::params::{workspace_calibration_dir, Params};
use nba_sim_core::state::{DribbleState, Phase};
use nba_sim_core::{MatchSetup, TeamSide, World};

mod common;

/// 摆出一次持球面对贴防：只有持球人与一名防守人在场上。
fn setup(seed: u64, gap_ft: f32) -> (World, usize, usize) {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, seed);
    let holder = world.on_court(TeamSide::Home).as_slice()[0];
    let defender = world.on_court(TeamSide::Away).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == holder || index == defender;
        player.vel = Vec2::ZERO;
    }
    let hoop = world.hoop_for(TeamSide::Home);
    let spot = hoop + Vec2::new(-24.0, 0.0);
    world.players[holder].pos = spot;
    world.players[holder].target = spot;
    world.players[holder].facing = (hoop - spot).normalize_or_zero();
    let towards_hoop = (hoop - spot).normalize_or_zero();
    let defender_spot = spot + towards_hoop * gap_ft;
    world.players[defender].pos = defender_spot;
    world.players[defender].target = defender_spot;
    world.players[defender].facing = (spot - defender_spot).normalize_or_zero();
    world.possession.team = TeamSide::Home;
    world.possession.dribble_state = DribbleState::Dribbling;
    world.possession.ball_phase = nba_sim_core::state::BallPhase::Dribble;
    world.ball.set_held(holder);
    // 真实的运球点不在身体正中：球在手侧一个身位，几何暴露因此不为零。
    world.ball.pos = spot + Vec2::new(0.0, 1.5);
    world.ball.z = world.rules.ball_radius_ft;
    world.set_phase(Phase::Live);
    world.pending_shot = None;
    (world, holder, defender)
}

fn dribble(world: &mut World, actor: usize, kind: DribbleMoveKind) -> Fact {
    let pos = world.players[actor].pos;
    let intent = ActionIntent {
        actor,
        kind: ActionKind::Dribble(kind),
        target: None,
        from_ft: pos,
        to_ft: pos,
        start_time: world.now(),
        duration: 1.4,
    };
    let before = world.events.len();
    nba_sim_core::resolve::resolve(world, intent);
    world.events[before..]
        .iter()
        .find(|event| matches!(event.fact, Fact::DribbleMove { .. }))
        .map(|event| event.fact.clone())
        .expect("运球必须发布 DribbleMove 事实")
}

#[test]
fn crossover_moves_the_holder_and_reduces_the_poke_exposure() {
    let (mut world, holder, defender) = setup(61, 2.0);
    let nearest = 2.0;
    let now = world.now();
    let before = nba_sim_core::resolve::poke::poke_success(&world, defender, holder, nearest, now);
    let start = world.players[holder].pos;
    let fact = dribble(&mut world, holder, DribbleMoveKind::Crossover);
    let Fact::DribbleMove {
        kind,
        separation_ft,
        exposure,
        ..
    } = fact
    else {
        panic!("运球必须发布 DribbleMove 事实");
    };
    assert_eq!(kind, DribbleMoveKind::Crossover);
    assert!(separation_ft > 0.0, "变向必须换到空间");
    assert!(exposure < 1.0, "变向期间球必须收在体侧");
    // 落点已经改到防守人的另一侧，由移动层按速度走过去。
    assert!(
        (world.players[holder].target - start).length() >= separation_ft - 0.01,
        "变向必须给出真实的落点"
    );
    let after = nba_sim_core::resolve::poke::poke_success(&world, defender, holder, nearest, now);
    assert!(
        after < before,
        "变向期间切球成功率必须下降：{before:.4} -> {after:.4}"
    );
}

#[test]
fn hesitation_freezes_the_defender_and_lowers_the_poke_success() {
    let (mut world, holder, defender) = setup(67, 2.0);
    let nearest = 2.0;
    let now = world.now();
    let before = nba_sim_core::resolve::poke::poke_success(&world, defender, holder, nearest, now);
    let fact = dribble(&mut world, holder, DribbleMoveKind::Hesitation);
    assert!(matches!(
        fact,
        Fact::DribbleMove {
            kind: DribbleMoveKind::Hesitation,
            ..
        }
    ));
    let bait = world.players[defender]
        .bait
        .expect("迟疑步必须留下失衡窗口");
    assert_eq!(bait.kind, nba_sim_core::action::FakeKind::Hesitation);
    assert!(bait.until > world.now());
    let after = nba_sim_core::resolve::poke::poke_success(&world, defender, holder, nearest, now);
    assert!(
        after < before,
        "迟疑步必须冻结伸手时机：{before:.4} -> {after:.4}"
    );
}

#[test]
fn direct_drive_keeps_the_geometric_exposure() {
    let (mut world, holder, defender) = setup(71, 2.0);
    let start = world.players[holder].pos;
    let fact = dribble(&mut world, holder, DribbleMoveKind::DirectDrive);
    let Fact::DribbleMove {
        separation_ft,
        exposure,
        ..
    } = fact
    else {
        panic!("运球必须发布 DribbleMove 事实");
    };
    assert_eq!(separation_ft, 0.0, "原地推进不改变相对位置");
    assert_eq!(exposure, 1.0, "原地推进保持几何暴露");
    assert_eq!(
        world.players[holder].target, start,
        "原地推进不该给出新落点"
    );
    assert!(
        world.players[defender].bait.is_none(),
        "原地推进不冻结防守人"
    );
}

#[test]
fn real_games_select_several_dribble_techniques() {
    let mut kinds: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for seed in 1..=3u64 {
        let report = common::short_world(seed).run();
        for event in &report.events {
            if let Fact::DribbleMove { kind, .. } = &event.fact {
                kinds.insert(format!("{kind:?}"));
            }
        }
    }
    assert!(
        kinds.len() >= 2,
        "真实比赛必须用出不止一种运球技术：{kinds:?}"
    );
}
