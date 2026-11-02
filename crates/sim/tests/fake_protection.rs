//! 假动作与护球：两个动作族的候选、执行与后果消费者。
//!
//! 这些测试检验的不是「事实里出现了名字」，而是动作真的改写了后续结算读到的量：
//! 被骗到的防守人在失衡窗口内干扰折减、抄截半径收窄；护球窗口内切球成功率折减。

use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_sim_core::action::{ActionIntent, ActionKind, Fact, FakeKind, ProtectKind};
use nba_sim_core::params::{workspace_calibration_dir, Params};
use nba_sim_core::state::{DribbleState, Phase};
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, TeamSide, World};

mod common;

fn fixture(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

/// 摆出一次持球面对贴防：只有持球人与一名防守人在场上。
fn setup(seed: u64, gap_ft: f32) -> (World, usize, usize) {
    let mut world = fixture(seed);
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
    world.ball.pos = spot;
    world.ball.z = world.rules.ball_radius_ft;
    world.set_phase(Phase::Live);
    world.pending_shot = None;
    (world, holder, defender)
}

fn fake_intent(world: &World, actor: usize, target: Option<usize>, kind: FakeKind) -> ActionIntent {
    let pos = world.players[actor].pos;
    ActionIntent {
        actor,
        kind: ActionKind::Fake(kind),
        target,
        from_ft: pos,
        to_ft: pos,
        start_time: world.now(),
        duration: 0.45,
    }
}

fn protect_intent(
    world: &World,
    actor: usize,
    target: Option<usize>,
    kind: ProtectKind,
) -> ActionIntent {
    let pos = world.players[actor].pos;
    ActionIntent {
        actor,
        kind: ActionKind::Protect(kind),
        target,
        from_ft: pos,
        to_ft: pos,
        start_time: world.now(),
        duration: 0.35,
    }
}

/// 反复做同一种假动作直到防守人被骗到，返回被骗那一次的事实。
///
/// 每次尝试推进时钟，抽样键因此不同；假动作的识破概率由技能决定，
/// 四十次内必然出现一次被骗到，否则策略参数与技能映射不一致。
fn bite_once(world: &mut World, actor: usize, defender: usize, kind: FakeKind) -> Fact {
    for _ in 0..40 {
        let before = world.events.len();
        let intent = fake_intent(world, actor, Some(defender), kind);
        nba_sim_core::resolve::fake::resolve(world, &intent, kind);
        for event in &world.events[before..] {
            if let Fact::FakeAction { bitten, .. } = &event.fact {
                if *bitten {
                    return event.fact.clone();
                }
            }
        }
        world.clock.advance_simulation_time(TICK_SECONDS);
        world.clock.advance_tick();
    }
    panic!("防守人从未被骗到：识破概率与技能映射不一致");
}

#[test]
fn shot_fake_baits_the_defender_and_reduces_the_next_contest() {
    let (mut world, holder, defender) = setup(11, 3.0);
    let before = nba_sim_core::resolve::contest_for(&world, holder, world.players[holder].pos);
    assert!(before > 0.0, "贴防下的干扰必须为正，否则测试场景无效");
    let defender_before = world.players[defender].pos;
    let fact = bite_once(&mut world, holder, defender, FakeKind::Shot);
    let Fact::FakeAction {
        bitten,
        displacement_ft,
        kind,
        ..
    } = fact
    else {
        panic!("假动作必须发布 FakeAction 事实");
    };
    assert!(bitten);
    assert_eq!(kind, FakeKind::Shot);
    assert!(displacement_ft > 0.0, "骗起的防守人必须收到移动承诺");
    // 位移由移动层按速度完成，不是瞬移：承诺的落点必须已经离开原位。
    assert!(
        world.players[defender].target != defender_before,
        "被骗到的防守人必须被派向新的落点"
    );
    assert!(
        (world.players[defender].target - defender_before).length() >= displacement_ft - 0.01,
        "承诺的位移必须达到策略值"
    );
    let bait = world.players[defender]
        .bait
        .expect("被骗到之后必须留下失衡窗口");
    assert_eq!(bait.kind, FakeKind::Shot);
    assert!(bait.until > world.now());
    let after = nba_sim_core::resolve::contest_for(&world, holder, world.players[holder].pos);
    assert!(
        after < before,
        "失衡窗口内干扰必须折减：{before:.3} -> {after:.3}"
    );
}

#[test]
fn pass_fake_narrows_the_defender_intercept_radius() {
    let (mut world, holder, defender) = setup(13, 3.0);
    let before = nba_sim_core::ball::pass_intercept_radius(&world, &world.players[defender]);
    let fact = bite_once(&mut world, holder, defender, FakeKind::Pass);
    assert!(matches!(
        fact,
        Fact::FakeAction {
            kind: FakeKind::Pass,
            bitten: true,
            ..
        }
    ));
    let after = nba_sim_core::ball::pass_intercept_radius(&world, &world.players[defender]);
    assert!(
        after < before,
        "传球假动作必须收窄真实线路上的抄截半径：{before:.3} -> {after:.3}"
    );
}

#[test]
fn jab_step_moves_the_holder_toward_the_hoop() {
    let (mut world, holder, defender) = setup(17, 3.0);
    let hoop = world.hoop_for(TeamSide::Home);
    let before = world.players[holder].pos;
    let gap_before = (before - hoop).length();
    let fact = bite_once(&mut world, holder, defender, FakeKind::JabStep);
    assert!(matches!(
        fact,
        Fact::FakeAction {
            kind: FakeKind::JabStep,
            bitten: true,
            ..
        }
    ));
    // 试探步先给落点，再由移动层按速度走完：推进若干 tick 后必须真的靠近篮筐。
    for _ in 0..20 {
        nba_sim_core::motion::advance_movement(&mut world, TICK_SECONDS);
        world.clock.advance_simulation_time(TICK_SECONDS);
        world.clock.advance_tick();
    }
    let gap_after = (world.players[holder].pos - hoop).length();
    assert!(
        gap_after < gap_before,
        "试探步必须真的向篮筐方向跨出：{gap_before:.2} -> {gap_after:.2}"
    );
}

#[test]
fn ball_protection_reduces_poke_success() {
    // 同一场景、同一位置，唯一差别是护球窗口是否生效；概率由结算层同一个
    // 函数给出，因此比较的是真实执行用的那一个数。
    let (exposed, holder, defender) = setup(23, 2.0);
    let (mut guarded, _, _) = setup(23, 2.0);
    let intent = protect_intent(&guarded, holder, Some(defender), ProtectKind::FaceUp);
    nba_sim_core::resolve::protect::resolve(&mut guarded, &intent, ProtectKind::FaceUp);
    let protection = guarded.protection.expect("护球动作必须建立护球窗口");
    assert_eq!(protection.kind, ProtectKind::FaceUp);
    assert_eq!(protection.holder, holder);

    let nearest = 2.0;
    let now = guarded.now();
    let before =
        nba_sim_core::resolve::poke::poke_success(&exposed, defender, holder, nearest, now);
    let after = nba_sim_core::resolve::poke::poke_success(&guarded, defender, holder, nearest, now);
    let relief = guarded.rules.resolve.technique.protection_poke_relief;
    assert!(before > 0.0, "场景无效：基础成功率必须为正");
    assert!(
        after < before,
        "护球必须降低切球成功率：{before:.4} -> {after:.4}"
    );
    assert!(
        (after - before * (1.0 - relief)).abs() < 1e-6,
        "折减必须正好等于策略值：{after:.4} vs {:.4}",
        before * (1.0 - relief)
    );
}

#[test]
fn protection_window_produces_prevented_pokes() {
    // 抬高切球速率后，护球窗口内失败的伸手必须发布 PokePrevented 事实。
    let (mut world, holder, defender) = setup(31, 2.0);
    world.rules.resolve.ball_security.poke_attempt_rate_per_sec = 20.0;
    world.rules.resolve.ball_security.poke_success_ceiling = 0.5;
    let intent = protect_intent(&world, holder, Some(defender), ProtectKind::Post);
    nba_sim_core::resolve::protect::resolve(&mut world, &intent, ProtectKind::Post);
    let mut prevented = 0u32;
    for _ in 0..20 {
        let before = world.events.len();
        world.clock.advance_simulation_time(TICK_SECONDS);
        world.clock.advance_tick();
        nba_sim_core::resolve::poke::step(&mut world, TICK_SECONDS);
        prevented += world.events[before..]
            .iter()
            .filter(|event| matches!(event.fact, Fact::PokePrevented { .. }))
            .count() as u32;
    }
    assert!(prevented > 0, "护球窗口内被挡下的切球必须发布事实");
}

#[test]
fn starting_another_action_ends_the_protection_window() {
    let (mut world, holder, defender) = setup(29, 2.0);
    let intent = protect_intent(&world, holder, Some(defender), ProtectKind::Post);
    nba_sim_core::resolve::protect::resolve(&mut world, &intent, ProtectKind::Post);
    assert!(world.protection.is_some());
    let dribble = ActionIntent {
        actor: holder,
        kind: ActionKind::Dribble(nba_domain::action_window::DribbleMoveKind::Crossover),
        target: None,
        from_ft: world.players[holder].pos,
        to_ft: world.players[holder].pos,
        start_time: world.now(),
        duration: 1.4,
    };
    let participants = [(holder, nba_sim_core::action::ActionIntentRole::Primary)];
    assert!(world.start_action(dribble, &participants));
    assert!(world.protection.is_none(), "开始其他动作必须退出护球姿态");
}

#[test]
fn both_families_appear_in_real_games() {
    let mut fakes = 0u32;
    let mut bitten = 0u32;
    let mut protections = 0u32;
    let mut completed_fakes = 0u32;
    let mut completed_protections = 0u32;
    for seed in 1..=6u64 {
        let report = common::short_world(seed).run();
        for event in &report.events {
            match &event.fact {
                Fact::FakeAction {
                    bitten: was_bitten, ..
                } => {
                    fakes += 1;
                    bitten += u32::from(*was_bitten);
                }
                Fact::BallProtection { .. } => protections += 1,
                Fact::ActionCompleted { kind, .. } => match *kind {
                    "Fake" => completed_fakes += 1,
                    "Protect" => completed_protections += 1,
                    _ => {}
                },
                _ => {}
            }
        }
    }
    assert!(fakes > 0, "真实比赛里必须出现假动作");
    assert!(bitten > 0, "假动作必须至少骗到过一次防守人");
    assert!(protections > 0, "真实比赛里必须出现护球");
    assert_eq!(
        fakes, completed_fakes,
        "每个假动作都必须走完整生命周期并结算"
    );
    assert_eq!(
        protections, completed_protections,
        "每个护球都必须走完整生命周期并结算"
    );
}
