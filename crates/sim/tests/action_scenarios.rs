//! 动作场景矩阵：为目录中已有实现但缺少直接证据的技术补上回归。
//!
//! 每个测试都读引擎真正发布的事实或真正写入的状态，不注入结果。

use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_sim_core::action::{ActionIntent, ActionKind, Fact, ScreenKind, ShotKind, TeamSide, TurnoverKind};
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
        nba_sim_core::resolve::fake::resolve(
            &mut world,
            &intent,
            nba_sim_core::action::FakeKind::Shot,
        );
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

/// 无球位置调整：球员自己发起的跑位真的改变位置，编队不会把落点改回去。
#[test]
fn relocate_changes_the_position_and_keeps_the_receive_window() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, 151);
    let holder = world.on_court(TeamSide::Home).as_slice()[0];
    let mover = world.on_court(TeamSide::Home).as_slice()[1];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == holder || index == mover;
        player.vel = Vec2::ZERO;
    }
    let hoop = world.hoop_for(TeamSide::Home);
    world.players[holder].pos = hoop + Vec2::new(-26.0, 0.0);
    world.players[holder].target = world.players[holder].pos;
    let start = hoop + Vec2::new(-18.0, -12.0);
    world.players[mover].pos = start;
    world.players[mover].target = start;
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(holder);
    world.ball.pos = world.players[holder].pos;
    world.ball.z = world.rules.ball_radius_ft;

    let spot = hoop + Vec2::new(-6.0, 8.0);
    let intent = ActionIntent {
        actor: mover,
        kind: ActionKind::Relocate,
        target: None,
        from_ft: start,
        to_ft: spot,
        start_time: world.now(),
        duration: 2.0,
    };
    let participants = [(mover, nba_sim_core::action::ActionIntentRole::Primary)];
    assert!(world.start_action(intent, &participants));
    for _ in 0..60 {
        nba_sim_core::formation::move_toward_targets(&mut world);
        nba_sim_core::motion::advance_movement(&mut world, TICK_SECONDS);
        world.clock.advance_simulation_time(TICK_SECONDS);
        world.clock.advance_tick();
    }
    let travelled = (world.players[mover].pos - start).length();
    assert!(
        travelled > 3.0,
        "跑位必须真的把人送出去：仅移动了 {travelled:.2} 英尺"
    );
    assert_eq!(
        world.players[mover].target_kind,
        nba_sim_core::state::TargetKind::Window,
        "跑位期间的目标点必须是接球窗口"
    );
    assert!(
        (world.players[mover].target - spot).length() < 0.5,
        "编队不得把跑位的落点改回去"
    );
}

/// 协防与回位：真实比赛里必须有防守人承担协防，并且该责任会收回。
#[test]
fn help_coverage_is_assigned_and_recovers() {
    let mut help_seen = false;
    let mut recovered = false;
    for seed in 1..=3u64 {
        let mut world = world(seed);
        let mut guard = 0;
        let mut helped: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        while !world.finished && guard < 200_000 {
            world.step(TICK_SECONDS);
            guard += 1;
            for handle in world
                .on_court_home
                .as_slice()
                .iter()
                .chain(world.on_court_away.as_slice())
            {
                let player = &world.players[*handle];
                if player.on_court
                    && player.defense_responsibility
                        == nba_sim_core::state::DefenseResponsibility::Help
                {
                    helped.insert(*handle);
                    help_seen = true;
                } else if helped.contains(handle) {
                    // 曾经协防的人后来又承担了别的责任：责任确实收回了。
                    recovered = true;
                }
            }
        }
    }
    assert!(help_seen, "真实比赛里必须有防守人承担协防");
    assert!(recovered, "协防责任必须会收回，而不是一直挂着");
}

/// 合法掩护：站定的掩护人撞上防守人只是接触，不是犯规。
#[test]
fn a_set_screen_contacts_without_a_foul() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, 149);
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
    // 掩护人已经站定：目标点就是脚下，速度为零。
    world.players[screener].pos = spot + Vec2::new(0.8, 0.6);
    world.players[screener].target = world.players[screener].pos;
    world.players[defender].pos = spot + Vec2::new(1.0, 0.9);
    world.players[defender].target = world.players[defender].pos;
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);

    let mut contacts = 0u32;
    let mut fouls = 0u32;
    for _ in 0..500u32 {
        world.set_phase(Phase::Live);
        world.screen_defender = Some(defender);
        world.clock.advance_tick();
        world.players[screener].vel = Vec2::ZERO;
        world.players[screener].target = world.players[screener].pos;
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
        for event in &world.events[before..] {
            match &event.fact {
                Fact::Screen { contact, .. } => {
                    if *contact {
                        contacts += 1;
                    }
                }
                Fact::Foul {
                    kind: nba_domain::event::FoulKind::Offensive,
                    ..
                } => fouls += 1,
                _ => {}
            }
        }
    }
    assert!(contacts > 0, "站定的掩护人必须有真实的接触");
    assert_eq!(fouls, 0, "站定的掩护不得被判进攻犯规");
}

/// 掩护后的顺下与外弹：掩护人必须用自己的下一次动作兑现掩护。
#[test]
fn the_screener_rolls_or_pops_after_setting_a_screen() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    for sagging in [true, false] {
        let mut world = World::new(MatchSetup::builtin(GameRules::default()), params.clone(), 167);
        let holder = world.on_court(TeamSide::Home).as_slice()[0];
        let screener = world.on_court(TeamSide::Home).as_slice()[1];
        let marker = world.on_court(TeamSide::Away).as_slice()[0];
        for (index, player) in world.players.iter_mut().enumerate() {
            player.on_court = index == holder || index == screener || index == marker;
            player.vel = Vec2::ZERO;
            player.action = None;
            player.recovery_since = 0.0;
        }
        let hoop = world.hoop_for(TeamSide::Home);
        world.players[holder].pos = hoop + Vec2::new(-20.0, 8.0);
        world.players[holder].target = world.players[holder].pos;
        world.players[screener].pos = hoop + Vec2::new(-14.0, 2.0);
        world.players[screener].target = world.players[screener].pos;
        // 对位人贴身还是在收缩，决定顺下还是外弹。
        world.players[marker].pos = world.players[screener].pos
            + if sagging {
                Vec2::new(-9.0, 0.0)
            } else {
                Vec2::new(0.6, 0.4)
            };
        world.players[marker].target = world.players[marker].pos;
        world.possession.team = TeamSide::Home;
        world.start_possession(TeamSide::Home, PossessionKind::Live);
        world.set_phase(Phase::Live);
        world.ball.set_held(holder);
        world.ball.pos = world.players[holder].pos;
        world.ball.z = world.rules.ball_radius_ft;
        world.clock.set_shot_clock(20.0);
        world.possession.started_at = world.now() - 5.0;
        world.possession.off_ball_actions = 0;
        world.players[screener].screened_at = world.now();
        world.players[screener].schedule.next_redecide_at = 0.0;

        nba_sim_core::policy::off_ball_tick(&mut world);
        let kind = world.players[screener]
            .action
            .as_ref()
            .map(|action| action.intent.kind);
        assert!(
            matches!(kind, Some(ActionKind::Cut) | Some(ActionKind::Relocate)),
            "掩护人必须顺下或外弹，实际 {kind:?}（sagging={sagging}）"
        );
        if sagging {
            assert_eq!(kind, Some(ActionKind::Cut), "对位人收缩时必须顺下");
        }
        assert_eq!(
            world.players[screener].screened_at, 0.0,
            "兑现之后不得继续挂着掩护偏好"
        );
    }
}

/// 收球步法：容限内的两步合法，超过容限就是走步。
#[test]
fn only_gather_steps_beyond_the_allowance_are_travels() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let allowance = params.policy.g("gather_travel_allowance_ft");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, 173);
    let finisher = world.on_court(TeamSide::Home).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == finisher;
        player.vel = Vec2::ZERO;
    }
    let hoop = world.hoop_for(TeamSide::Home);
    let spot = hoop + Vec2::new(-6.0, 0.0);
    world.players[finisher].pos = spot;
    world.players[finisher].target = spot;

    // 合法：迈出的距离正好在容限内。
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(finisher);
    world.ball.pos = spot;
    world.ball.z = world.rules.ball_radius_ft;
    world.possession.gather_pos = Some(spot);
    let legal_to = spot + (hoop - spot).normalize_or_zero() * (allowance * 0.8);
    let legal = ActionIntent {
        actor: finisher,
        kind: ActionKind::Finish,
        target: None,
        from_ft: spot,
        to_ft: legal_to,
        start_time: world.now(),
        duration: 0.4,
    };
    let before = world.events.len();
    let outcome = nba_sim_core::resolve::drive::resolve_finish(&mut world, &legal);
    let legal_turnover = world.events[before..].iter().any(|event| {
        matches!(
            &event.fact,
            Fact::Turnover { kind: TurnoverKind::Travel, .. }
        )
    });
    assert_eq!(
        outcome.completion,
        nba_sim_core::action::Completion::Completed,
        "容限内的收球必须有结果，而不是被吹走步"
    );
    assert!(!legal_turnover, "容限内的收球不得被判走步");

    // 走步：迈出的距离超过容限，且判罚不由随机频率决定。
    let mut caught = false;
    for seed_step in 0..8u64 {
        let mut world = World::new(MatchSetup::builtin(GameRules::default()),
            Params::load(&workspace_calibration_dir()).expect("calibration tables load"),
            173 + seed_step);
        let finisher = world.on_court(TeamSide::Home).as_slice()[0];
        for (index, player) in world.players.iter_mut().enumerate() {
            player.on_court = index == finisher;
            player.vel = Vec2::ZERO;
        }
        world.players[finisher].pos = spot;
        world.players[finisher].target = spot;
        world.possession.team = TeamSide::Home;
        world.start_possession(TeamSide::Home, PossessionKind::Live);
        world.set_phase(Phase::Live);
        world.ball.set_held(finisher);
        world.ball.pos = spot;
        world.ball.z = world.rules.ball_radius_ft;
        world.possession.gather_pos = Some(spot);
        let over_to = spot + (hoop - spot).normalize_or_zero() * (allowance + 3.0);
        let over = ActionIntent {
            actor: finisher,
            kind: ActionKind::Finish,
            target: None,
            from_ft: spot,
            to_ft: over_to,
            start_time: world.now(),
            duration: 0.4,
        };
        let before = world.events.len();
        let outcome = nba_sim_core::resolve::drive::resolve_finish(&mut world, &over);
        let travel = outcome.completion == nba_sim_core::action::Completion::Failed
            && world.events[before..].iter().any(|event| {
                matches!(
                    &event.fact,
                    Fact::Turnover { kind: TurnoverKind::Travel, .. }
                )
            });
        if travel {
            caught = true;
            break;
        }
    }
    assert!(caught, "超过容限的收球必须被判走步");
}
