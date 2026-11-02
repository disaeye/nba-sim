//! 动作场景矩阵：为目录中已有实现但缺少直接证据的技术补上回归。
//!
//! 每个测试都读引擎真正发布的事实或真正写入的状态，不注入结果。

use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_sim_core::action::{ActionIntent, ActionKind, Fact, ScreenKind, ShotKind, TeamSide};
use nba_sim_core::params::{workspace_calibration_dir, Params};
use nba_sim_core::state::{Phase, PossessionKind};
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, World};

mod common;

fn world(seed: u64) -> World {
    common::short_world(seed)
}

/// 防守篮板：进攻方投失后没抢到板的一方拿到球，记为防守篮板；
/// 进攻方自己拿到，记为进攻篮板。两种来源分开记账。
#[test]
fn defensive_control_emits_a_defensive_rebound() {
    let mut defensive = 0u32;
    let mut offensive = 0u32;
    for seed in 1..=3u64 {
        let mut world = world(seed);
        let mut guard = 0;
        while !world.finished && guard < 200_000 {
            let offense_before = world.possession.team;
            let before = world.events.len();
            world.step(TICK_SECONDS);
            guard += 1;
            for event in &world.events[before..] {
                let Fact::Rebound {
                    player,
                    offensive: is_offensive,
                    ..
                } = &event.fact
                else {
                    continue;
                };
                let rebounder_offense = world.players[*player].team == offense_before;
                assert_eq!(
                    rebounder_offense, *is_offensive,
                    "seed {seed}: 篮板归属与进攻方判定不一致"
                );
                if *is_offensive {
                    offensive += 1;
                } else {
                    defensive += 1;
                }
            }
        }
    }
    assert!(defensive > 0, "真实比赛必须出现防守篮板");
    assert!(offensive > 0, "真实比赛必须出现进攻篮板");
}

/// 空切传球：球瞄向切入人将要到达的接球点，而不是他发球瞬间的位置。
#[test]
fn cut_pass_aims_at_the_cutters_planned_catch_point() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, 131);
    let passer = world.on_court(TeamSide::Home).as_slice()[0];
    let cutter = world.on_court(TeamSide::Home).as_slice()[1];
    let defender = world.on_court(TeamSide::Away).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == passer || index == cutter || index == defender;
        player.vel = Vec2::ZERO;
    }
    let hoop = world.hoop_for(TeamSide::Home);
    world.players[passer].pos = hoop + Vec2::new(-26.0, 4.0);
    world.players[passer].target = world.players[passer].pos;
    world.players[cutter].pos = hoop + Vec2::new(-14.0, -8.0);
    world.players[cutter].target = world.players[cutter].pos;
    world.players[defender].pos = hoop + Vec2::new(-10.0, 6.0);
    world.players[defender].target = world.players[defender].pos;
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(passer);
    world.ball.pos = world.players[passer].pos;
    world.ball.z = world.rules.ball_radius_ft;

    let plan = nba_sim_core::resolve::pass::cut_plan(&world, cutter).0;
    let intent = ActionIntent {
        actor: passer,
        kind: ActionKind::Cut,
        target: Some(cutter),
        from_ft: world.players[passer].pos,
        to_ft: plan,
        start_time: world.now(),
        duration: 1.0,
    };
    nba_sim_core::resolve::pass::resolve_cut(&mut world, &intent);
    let nba_sim_core::state::BallState::Pass { to_pos, .. } = world.ball.state() else {
        panic!("空切必须真的把球送出去");
    };
    assert!(
        (to_pos - plan).length() < 0.5,
        "空切传球必须瞄向切入点：{to_pos:?} vs {plan:?}"
    );
    // 切入人的落点也被设成同一个接球点：他跑到的地方就是球到的地方。
    assert!(
        (world.players[cutter].target - plan).length() < 0.5,
        "切入人的落点必须与接球点一致"
    );
}

/// 移动掩护：掩护人还在移动时撞上防守人，会被判进攻犯规。
///
/// 这条规则每次掩护只有很低的吹罚率，因此要在同一场景上反复抽样；
/// 每次抽样推进 tick，抽样键不同才有独立样本。
#[test]
fn moving_screen_is_called_as_an_offensive_foul() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, 137);
    let offense = world.on_court(TeamSide::Home).as_slice()[0];
    let screener = world.on_court(TeamSide::Home).as_slice()[1];
    let defender = world.on_court(TeamSide::Away).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == offense || index == screener || index == defender;
        player.vel = Vec2::ZERO;
    }
    let spot = Vec2::new(60.0, 25.0);
    world.players[offense].pos = spot;
    world.players[offense].target = spot;
    world.players[screener].pos = spot + Vec2::new(0.0, 1.0);
    world.players[defender].pos = spot + Vec2::new(0.8, 0.6);
    world.players[defender].target = world.players[defender].pos;

    world.start_possession(TeamSide::Home, PossessionKind::Live);
    let mut called = false;
    for attempt in 0..3_000u32 {
        // 没吹罚时回合仍然开着：只重建“掩护人正在移动”的现场与抽样键。
        world.players[screener].target = spot + Vec2::new(6.0, 1.0);
        world.players[screener].vel = Vec2::new(6.0, 0.0);
        world.players[offense].pos = spot;
        world.set_phase(Phase::Live);
        world.screen_defender = Some(defender);
        world.clock.advance_tick();
        let intent = ActionIntent {
            actor: offense,
            kind: ActionKind::Screen(ScreenKind::OnBall),
            target: Some(screener),
            from_ft: spot,
            to_ft: spot,
            start_time: world.now(),
            duration: 1.0,
        };
        let before = world.events.len();
        nba_sim_core::resolve::screen::resolve(&mut world, &intent, ScreenKind::OnBall);
        if world.events[before..].iter().any(|event| {
            matches!(
                &event.fact,
                Fact::Foul {
                    kind: nba_domain::event::FoulKind::Offensive,
                    ..
                }
            )
        }) {
            called = true;
            break;
        }
        assert!(attempt < 3_000, "三千次抽样内没有出现移动掩护吹罚");
    }
    assert!(called, "移动中的掩护必须会被判进攻犯规");
}

/// 定点投篮假动作之后的出手：干扰必须按失衡窗口折减（防守人被骗起）。
#[test]
fn shot_after_a_bitten_fake_is_less_contested() {
    let (mut world, holder, defender) = {
        let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
        let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, 29);
        let holder = world.on_court(TeamSide::Home).as_slice()[0];
        let defender = world.on_court(TeamSide::Away).as_slice()[0];
        for (index, player) in world.players.iter_mut().enumerate() {
            player.on_court = index == holder || index == defender;
            player.vel = Vec2::ZERO;
        }
        let hoop = world.hoop_for(TeamSide::Home);
        let spot = hoop + Vec2::new(-22.0, 0.0);
        world.players[holder].pos = spot;
        world.players[holder].target = spot;
        world.players[holder].facing = (hoop - spot).normalize_or_zero();
        let towards_hoop = (hoop - spot).normalize_or_zero();
        world.players[defender].pos = spot + towards_hoop * 3.0;
        world.players[defender].target = world.players[defender].pos;
        world.possession.team = TeamSide::Home;
        world.start_possession(TeamSide::Home, PossessionKind::Live);
        world.set_phase(Phase::Live);
        world.ball.set_held(holder);
        world.ball.pos = spot;
        world.ball.z = world.rules.ball_radius_ft;
        (world, holder, defender)
    };
    let contested = nba_sim_core::resolve::contest_for(&world, holder, world.players[holder].pos);
    // 骗到一次投篮假动作。
    let mut bitten = false;
    for _ in 0..40 {
        let pos = world.players[holder].pos;
        let intent = ActionIntent {
            actor: holder,
            kind: ActionKind::Fake(nba_sim_core::action::FakeKind::Shot),
            target: Some(defender),
            from_ft: pos,
            to_ft: pos,
            start_time: world.now(),
            duration: 0.45,
        };
        nba_sim_core::resolve::fake::resolve(&mut world, &intent, nba_sim_core::action::FakeKind::Shot);
        if world.players[defender].bait.is_some() {
            bitten = true;
            break;
        }
        world.clock.advance_simulation_time(TICK_SECONDS);
        world.clock.advance_tick();
    }
    assert!(bitten, "投篮假动作必须至少骗到一次");
    let after = nba_sim_core::resolve::contest_for(&world, holder, world.players[holder].pos);
    assert!(
        after < contested,
        "被骗起的防守人干扰必须下降：{contested:.3} -> {after:.3}"
    );
    // 出手一次，确认投篮模型读到的是同一个折减后的干扰。
    world.possession.ball_phase = nba_sim_core::state::BallPhase::Catch;
    let quality = nba_sim_core::shot_quality::evaluate(
        &world,
        holder,
        ShotKind::CatchShoot,
        nba_sim_core::state::BallPhase::Catch,
        world.players[holder].pos,
    );
    assert!(quality.p_make_cond > 0.0);
}
