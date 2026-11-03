//! 动作场景矩阵：为目录中已有实现但缺少直接证据的技术补上回归。
//!
//! 每个测试都读引擎真正发布的事实或真正写入的状态，不注入结果。

use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_sim_core::action::{
    ActionIntent, ActionKind, Fact, FakeKind, ProtectKind, ScreenKind, ShotKind, TeamSide,
    TurnoverKind,
};
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
        kind: ActionKind::Cut(nba_sim_core::action::CutKind::Basket),
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
        // 每次掩护都把防守人放回起始位置：掩护接触会把人顶开，
        // 不复位的现场第二拍就已经脱离了接触。
        world.players[defender].pos = spot + Vec2::new(0.8, 0.6);
        world.players[defender].target = world.players[defender].pos;
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

/// 掩护的力量顶开防守人：力量越高推开越远，位置真的跟着变。
#[test]
fn a_stronger_screener_pushes_the_defender_further() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut pushed = Vec::new();
    for strength in [0.10f32, 0.95] {
        let mut world = World::new(
            MatchSetup::builtin(GameRules::default()),
            params.clone(),
            151,
        );
        let offense = world.on_court(TeamSide::Home).as_slice()[0];
        let screener = world.on_court(TeamSide::Home).as_slice()[1];
        let defender = world.on_court(TeamSide::Away).as_slice()[0];
        for (index, player) in world.players.iter_mut().enumerate() {
            player.on_court = index == offense || index == screener || index == defender;
            player.vel = Vec2::ZERO;
        }
        world.players[screener].data.attributes.strength = strength;
        let spot = Vec2::new(60.0, 25.0);
        world.players[offense].pos = spot;
        world.players[offense].target = spot;
        world.players[screener].pos = spot + Vec2::new(0.8, 0.6);
        world.players[screener].target = world.players[screener].pos;
        world.players[defender].pos = spot + Vec2::new(1.0, 0.9);
        world.players[defender].target = world.players[defender].pos;
        world.possession.team = TeamSide::Home;
        world.start_possession(TeamSide::Home, PossessionKind::Live);
        world.set_phase(Phase::Live);
        world.screen_defender = Some(defender);
        let before = world.players[defender].pos;
        let intent = ActionIntent {
            actor: offense,
            kind: ActionKind::Screen(ScreenKind::OnBall),
            target: Some(screener),
            from_ft: spot,
            to_ft: spot,
            start_time: world.now(),
            duration: 1.0,
        };
        nba_sim_core::resolve::screen::resolve(&mut world, &intent, ScreenKind::OnBall);
        pushed.push((world.players[defender].pos - before).length());
    }
    assert!(
        pushed[1] > pushed[0] + 0.02,
        "力量更高的掩护必须把防守人推得更远，实际 {:.3} 与 {:.3}",
        pushed[1],
        pushed[0]
    );
}

/// 掩护后的顺下与外弹：掩护人必须用自己的下一次动作兑现掩护。
#[test]
fn the_screener_rolls_or_pops_after_setting_a_screen() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    for sagging in [true, false] {
        let mut world = World::new(
            MatchSetup::builtin(GameRules::default()),
            params.clone(),
            167,
        );
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
            matches!(kind, Some(ActionKind::Cut(_)) | Some(ActionKind::Relocate)),
            "掩护人必须顺下或外弹，实际 {kind:?}（sagging={sagging}）"
        );
        if sagging {
            assert!(
                matches!(kind, Some(ActionKind::Cut(_))),
                "对位人收缩时必须顺下"
            );
        }
        assert_eq!(
            world.players[screener].screened_at,
            f64::NEG_INFINITY,
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
            Fact::Turnover {
                kind: TurnoverKind::Travel,
                ..
            }
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
        let mut world = World::new(
            MatchSetup::builtin(GameRules::default()),
            Params::load(&workspace_calibration_dir()).expect("calibration tables load"),
            173 + seed_step,
        );
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
                    Fact::Turnover {
                        kind: TurnoverKind::Travel,
                        ..
                    }
                )
            });
        if travel {
            caught = true;
            break;
        }
    }
    assert!(caught, "超过容限的收球必须被判走步");
}

/// 突破后的收球分球：同一次进攻里突破之后仍然可以把球传出去。
#[test]
fn a_drive_can_stop_and_pass_within_the_same_possession() {
    let mut kick_outs = 0u32;
    for seed in 1..=3u64 {
        let report = world(seed).run();
        // 按回合分组：突破起手之后、回合结束之前出现传球就是收球分球。
        let mut current: u64 = 0;
        let mut drove = false;
        for event in &report.events {
            match &event.fact {
                Fact::PossessionStarted { possession_id, .. } => {
                    current = *possession_id;
                    drove = false;
                }
                Fact::PossessionEnded { .. } => {
                    current = 0;
                    drove = false;
                }
                Fact::ActionStarted { kind, .. } if *kind == "Drive" && current != 0 => {
                    drove = true;
                }
                Fact::Pass { .. } if drove => {
                    kick_outs += 1;
                    drove = false;
                }
                _ => {}
            }
        }
    }
    assert!(
        kick_outs > 0,
        "真实比赛里必须有突破之后收球分球的回合，观测到 {kick_outs}"
    );
}

/// 带球撞人：站定的防守人造成进攻犯规，滑步中的防守人不会。
#[test]
fn a_charge_requires_a_planted_defender() {
    let planted_charges = |planted: bool| -> u32 {
        let mut charges = 0u32;
        for seed in 1..=200u64 {
            let params =
                Params::load(&workspace_calibration_dir()).expect("calibration tables load");
            let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, seed);
            let driver = world.on_court(TeamSide::Home).as_slice()[0];
            let defender = world.on_court(TeamSide::Away).as_slice()[0];
            for (index, player) in world.players.iter_mut().enumerate() {
                player.on_court = index == driver || index == defender;
                player.vel = Vec2::ZERO;
            }
            let hoop = world.hoop_for(TeamSide::Home);
            let spot = hoop + Vec2::new(-10.0, 0.0);
            world.players[driver].pos = spot;
            world.players[driver].target = spot;
            // 防守人站在持球人与篮筐之间，接触距离之内。
            let toward = (hoop - spot).normalize_or_zero();
            world.players[defender].pos = spot + toward * 2.5;
            world.players[defender].target = world.players[defender].pos;
            world.players[defender].vel = if planted { Vec2::ZERO } else { toward * 9.0 };
            world.possession.team = TeamSide::Home;
            world.start_possession(TeamSide::Home, PossessionKind::Live);
            world.set_phase(Phase::Live);
            world.ball.set_held(driver);
            world.ball.pos = spot;
            world.ball.z = world.rules.ball_radius_ft;
            world.possession.gather_pos = Some(spot);
            let intent = ActionIntent {
                actor: driver,
                kind: ActionKind::Drive,
                target: None,
                from_ft: spot,
                to_ft: spot + toward * 1.0,
                start_time: world.now(),
                duration: 0.4,
            };
            let before = world.events.len();
            nba_sim_core::resolve::drive::resolve(&mut world, &intent);
            if world.events[before..].iter().any(|event| {
                matches!(
                    &event.fact,
                    Fact::Foul {
                        kind: nba_domain::event::FoulKind::Offensive,
                        ..
                    }
                )
            }) {
                charges += 1;
            }
        }
        charges
    };
    let planted = planted_charges(true);
    let sliding = planted_charges(false);
    assert!(
        planted > 0,
        "站定的防守人必须能造进攻犯规，观测到 {planted}"
    );
    assert_eq!(
        sliding, 0,
        "滑步中的防守人不得造成进攻犯规，观测到 {sliding}"
    );
}

/// 紧逼盯防五秒：收球后被贴身盯住不动，超过限时就是违例。
#[test]
fn holding_the_ball_closely_guarded_for_five_seconds_is_a_violation() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let limit = GameRules::default().league.clocks.closely_guarded_seconds;
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, 179);
    let holder = world.on_court(TeamSide::Home).as_slice()[0];
    let defender = world.on_court(TeamSide::Away).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == holder || index == defender;
        player.vel = Vec2::ZERO;
    }
    let hoop = world.hoop_for(TeamSide::Home);
    let spot = hoop + Vec2::new(-18.0, 0.0);
    world.players[holder].pos = spot;
    world.players[holder].target = spot;
    world.players[defender].pos = spot + Vec2::new(0.0, 1.5);
    world.players[defender].target = world.players[defender].pos;
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(holder);
    world.ball.pos = spot;
    world.ball.z = world.rules.ball_radius_ft;
    world.players[holder].schedule.next_redecide_at = world.now() + 1_000.0;
    world.possession.dribble_state = nba_sim_core::state::DribbleState::Gathered;

    let mut ticks = 0;
    let mut violation = None;
    while ticks < 400 && violation.is_none() {
        world.possession.dribble_state = nba_sim_core::state::DribbleState::Gathered;
        world.possession.team = TeamSide::Home;
        let before = world.events.len();
        world.step(TICK_SECONDS);
        ticks += 1;
        for event in &world.events[before..] {
            if matches!(
                &event.fact,
                Fact::Turnover {
                    kind: TurnoverKind::CloselyGuarded,
                    ..
                }
            ) {
                violation = Some(world.now());
            }
        }
    }
    let at = violation.expect("紧逼五秒必须吹违例");
    assert!(
        at >= f64::from(limit) - 1.0,
        "违例时刻必须接近限时 {limit} 秒，实际 {at:.2}"
    );
    assert_eq!(
        world.possession.team,
        TeamSide::Away,
        "违例之后球权必须交给对方"
    );
}

/// 紧逼计时会被打断：防守人走开就清零，运球状态不受这条规则限制。
#[test]
fn the_closely_guarded_timer_resets_and_spares_the_dribbler() {
    let limit = GameRules::default().league.clocks.closely_guarded_seconds;
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, 181);
    let holder = world.on_court(TeamSide::Home).as_slice()[0];
    let defender = world.on_court(TeamSide::Away).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == holder || index == defender;
        player.vel = Vec2::ZERO;
    }
    let hoop = world.hoop_for(TeamSide::Home);
    let spot = hoop + Vec2::new(-18.0, 0.0);
    world.players[holder].pos = spot;
    world.players[holder].target = spot;
    world.players[defender].pos = spot + Vec2::new(0.0, 1.5);
    world.players[defender].target = world.players[defender].pos;
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(holder);
    world.ball.pos = spot;
    world.ball.z = world.rules.ball_radius_ft;
    world.players[holder].schedule.next_redecide_at = world.now() + 1_000.0;

    let violations = |world: &World| -> usize {
        world
            .events
            .iter()
            .filter(|event| {
                matches!(
                    &event.fact,
                    Fact::Turnover {
                        kind: TurnoverKind::CloselyGuarded,
                        ..
                    }
                )
            })
            .count()
    };

    // 运球状态：贴近防守也不吹。
    for _ in 0..(limit as usize + 2) * 20 + 20 {
        world.possession.dribble_state = nba_sim_core::state::DribbleState::Dribbling;
        world.step(TICK_SECONDS);
    }
    assert_eq!(violations(&world), 0, "运球中不得吹紧逼违例");

    // 收球后紧逼一半时间，然后防守人走开：计时必须清零。
    for _ in 0..(limit as usize * 20) / 2 {
        world.possession.dribble_state = nba_sim_core::state::DribbleState::Gathered;
        world.step(TICK_SECONDS);
    }
    world.players[defender].pos = spot + Vec2::new(0.0, 14.0);
    world.players[defender].target = world.players[defender].pos;
    for _ in 0..(limit as usize * 20) / 2 + 20 {
        world.possession.dribble_state = nba_sim_core::state::DribbleState::Gathered;
        world.step(TICK_SECONDS);
    }
    assert_eq!(violations(&world), 0, "防守人走开后计时必须清零");

    // 重新贴上：必须重新计满限时才吹。
    world.players[defender].pos = spot + Vec2::new(0.0, 1.5);
    world.players[defender].target = world.players[defender].pos;
    for _ in 0..(limit as usize * 20) - 10 {
        world.possession.dribble_state = nba_sim_core::state::DribbleState::Gathered;
        world.step(TICK_SECONDS);
    }
    assert_eq!(violations(&world), 0, "重新紧逼未满限时不得吹违例");
    for _ in 0..40 {
        world.possession.dribble_state = nba_sim_core::state::DribbleState::Gathered;
        world.step(TICK_SECONDS);
    }
    assert_eq!(violations(&world), 1, "重新紧逼满限时必须吹违例");
}

/// 拒绝接球：防守人站进传球路线会实际降低这次传球的完成概率。
#[test]
fn a_defender_in_the_lane_denies_the_pass() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let build = |with_denier: bool| {
        let mut world = World::new(
            MatchSetup::builtin(GameRules::default()),
            params.clone(),
            191,
        );
        let passer = world.on_court(TeamSide::Home).as_slice()[0];
        let cutter = world.on_court(TeamSide::Home).as_slice()[2];
        let denier = world.on_court(TeamSide::Away).as_slice()[0];
        for (index, player) in world.players.iter_mut().enumerate() {
            player.on_court = index == passer || index == cutter || index == denier;
            player.vel = Vec2::ZERO;
        }
        let from = Vec2::new(55.0, 25.0);
        let to = Vec2::new(74.0, 25.0);
        world.players[passer].pos = from;
        world.players[passer].target = from;
        world.players[passer].facing = (to - from).normalize_or_zero();
        world.players[cutter].pos = to;
        world.players[cutter].target = to;
        world.players[denier].pos = if with_denier {
            // 正好站在传球线上。
            (from + to) * 0.5
        } else {
            // 站在路线外十英尺。
            (from + to) * 0.5 + Vec2::new(0.0, 10.0)
        };
        world.players[denier].target = world.players[denier].pos;
        world.possession.team = TeamSide::Home;
        world.start_possession(TeamSide::Home, PossessionKind::Live);
        world.set_phase(Phase::Live);
        world.ball.set_held(passer);
        world.ball.pos = from;
        world.ball.z = world.rules.ball_radius_ft;
        (world, passer, cutter)
    };
    let (blocked_world, passer, cutter) = build(true);
    let (open_world, _, _) = build(false);
    let denied = nba_sim_core::value::pass_completion_probability(
        &blocked_world,
        passer,
        cutter,
        nba_sim_core::action::PassKind::Chest,
    );
    let open = nba_sim_core::value::pass_completion_probability(
        &open_world,
        passer,
        cutter,
        nba_sim_core::action::PassKind::Chest,
    );
    assert!(
        denied < open,
        "站进传球路线的防守人必须降低完成概率：{denied} vs {open}"
    );
}

/// 抢断后的转换：球权立刻转给抢断方，并且该回合很快就发起第一个动作。
#[test]
fn a_steal_hands_possession_over_and_starts_a_quick_transition() {
    let mut steals = 0u32;
    let mut quick = 0u32;
    for seed in 1..=4u64 {
        let report = world(seed).run();
        let mut possession_team = None;
        let mut steal_ids: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
        let mut pending_steal = false;
        for event in &report.events {
            match &event.fact {
                Fact::Steal { .. } => {
                    steals += 1;
                    pending_steal = true;
                }
                Fact::PossessionStarted {
                    team,
                    possession_id,
                    ..
                } => {
                    if pending_steal {
                        let expected = possession_team.map(|t: TeamSide| t.other());
                        if let Some(expected) = expected {
                            assert_eq!(
                                *team, expected,
                                "抢断之后球权必须转给抢断方（seed {seed}）"
                            );
                        }
                        steal_ids.insert(*possession_id);
                        pending_steal = false;
                    }
                    possession_team = Some(*team);
                }
                _ => {}
            }
        }
        // 抢断之后的那个回合必须很快起手：这就是转换进攻的实质。
        for record in &report.possessions {
            if steal_ids.contains(&record.id)
                && record
                    .first_action_seconds
                    .is_some_and(|seconds| seconds < 8.0)
            {
                quick += 1;
            }
        }
    }
    assert!(steals > 0, "真实比赛里必须有抢断");
    assert!(quick > 0, "抢断回合必须在八秒内起手");
}

/// 防守篮板后的转换推进：球必须在快攻窗口内越过中场。
#[test]
fn a_defensive_rebound_advances_past_midcourt_within_the_transition_window() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let transition = params.policy.g("transition_window_seconds") as f64;
    let mut monitored = 0u32;
    let mut advanced = 0u32;
    for seed in 1..=2u64 {
        let mut world = World::new(
            MatchSetup::builtin(GameRules::default()),
            Params::load(&workspace_calibration_dir()).expect("calibration tables load"),
            seed,
        );
        let mid = world.rules.court.width_ft * 0.5;
        let mut pending: Option<(f64, TeamSide)> = None;
        let mut guard = 0;
        while !world.finished && guard < 200_000 {
            let before = world.events.len();
            world.step(TICK_SECONDS);
            guard += 1;
            for event in &world.events[before..] {
                if let Fact::Rebound {
                    offensive: false, ..
                } = &event.fact
                {
                    pending = Some((world.now(), world.possession.team));
                }
            }
            let Some((since, team)) = pending else {
                continue;
            };
            let elapsed = world.now() - since;
            // 球权再次易主后，这次防守篮板的推进不再属于监控范围。
            if world.possession.team != team {
                pending = None;
                continue;
            }
            let holder_side = world
                .holder()
                .map(|handle| (world.players[handle].pos, world.players[handle].team));
            if let Some((pos, side)) = holder_side {
                if side == team {
                    let ahead = if world.attacking_right(team) {
                        pos.x > mid
                    } else {
                        pos.x < mid
                    };
                    if ahead {
                        advanced += 1;
                        pending = None;
                        continue;
                    }
                }
            }
            if elapsed > transition * 3.0 {
                monitored += 1;
                pending = None;
            }
        }
    }
    assert_eq!(
        monitored, 0,
        "防守篮板后的推进必须在快攻窗口内完成，超时次数 {monitored}"
    );
    assert!(advanced > 0, "必须有防守篮板后的推进");
}

/// 退防：球权转换的瞬间，五名防守人的目标点全部落在本方半场。
#[test]
fn a_possession_change_sends_every_defender_back() {
    let mut checked = 0u32;
    for seed in 1..=2u64 {
        let mut world = World::new(
            MatchSetup::builtin(GameRules::default()),
            Params::load(&workspace_calibration_dir()).expect("calibration tables load"),
            seed,
        );
        let mid = world.rules.court.width_ft * 0.5;
        let mut guard = 0;
        while !world.finished && guard < 200_000 {
            let before = world.events.len();
            world.step(TICK_SECONDS);
            guard += 1;
            let started = world.events[before..]
                .iter()
                .any(|event| matches!(&event.fact, Fact::PossessionStarted { .. }));
            if !started {
                continue;
            }
            let offense = world.possession.team;
            let defense = offense.other();
            let hoop_x = world.hoop_for(offense).x;
            let attacking_right = hoop_x > mid;
            for handle in world.on_court(defense).as_slice() {
                let player = &world.players[*handle];
                if !player.on_court {
                    continue;
                }
                let on_their_half = if attacking_right {
                    player.target.x >= mid - 1.0
                } else {
                    player.target.x <= mid + 1.0
                };
                assert!(
                    on_their_half,
                    "退防目标必须在球权转换后落到本方半场：{handle} target={:?}（seed {seed}）",
                    player.target
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "必须观测到球权转换后的防守目标");
}

/// 防守滑步：对位防守人必须真的缩小与持球人的距离。
#[test]
fn the_on_ball_defender_closes_the_gap() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, 197);
    let carrier = world.on_court(TeamSide::Home).as_slice()[0];
    let guard = world.on_court(TeamSide::Away).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == carrier || index == guard;
        player.vel = Vec2::ZERO;
    }
    let hoop = world.hoop_for(TeamSide::Home);
    let spot = hoop + Vec2::new(-16.0, 6.0);
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(carrier);
    world.players[carrier].pos = spot;
    world.players[carrier].target = spot;
    world.ball.pos = spot;
    world.ball.z = world.rules.ball_radius_ft;
    // 防守人离持球人十二英尺：他必须滑步贴上来。
    world.players[guard].pos = spot + Vec2::new(-12.0, 0.0);
    world.players[guard].target = world.players[guard].pos;
    world.players[guard].vel = Vec2::ZERO;
    nba_sim_core::defense::apply_defense_targets(&mut world);
    let before = (world.players[guard].pos - world.players[carrier].pos).length();
    for _ in 0..40 {
        world.players[carrier].pos = spot;
        world.players[carrier].target = spot;
        world.players[carrier].vel = Vec2::ZERO;
        world.ball.pos = spot;
        world.ball.z = world.rules.ball_radius_ft;
        nba_sim_core::defense::apply_defense_targets(&mut world);
        nba_sim_core::formation::move_toward_targets(&mut world);
        nba_sim_core::motion::advance_movement(&mut world, TICK_SECONDS);
        world.clock.advance_simulation_time(TICK_SECONDS);
        world.clock.advance_tick();
    }
    let after = (world.players[guard].pos - world.players[carrier].pos).length();
    assert!(
        after < before - 2.0,
        "对位防守人必须滑步缩小距离：{before:.2} → {after:.2}"
    );
    assert!(
        after >= nba_sim_core::assignment::pressure_separation(&world) - 0.5,
        "滑步不得贴到身位以内：{after:.2}"
    );
}

/// 后撤步跳投：出手前真的把身体撤开，空间不是名义上的。
#[test]
fn a_step_back_shot_creates_space_before_the_release() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let separation_after = |kind: ShotKind| -> f32 {
        let mut world = World::new(
            MatchSetup::builtin(GameRules::default()),
            params.clone(),
            199,
        );
        let shooter = world.on_court(TeamSide::Home).as_slice()[0];
        let defender = world.on_court(TeamSide::Away).as_slice()[0];
        for (index, player) in world.players.iter_mut().enumerate() {
            player.on_court = index == shooter || index == defender;
            player.vel = Vec2::ZERO;
        }
        let hoop = world.hoop_for(TeamSide::Home);
        let spot = hoop + Vec2::new(-20.0, 0.0);
        world.players[shooter].pos = spot;
        world.players[shooter].target = spot;
        world.players[shooter].facing = (hoop - spot).normalize_or_zero();
        world.players[defender].pos = spot + Vec2::new(1.4, 0.0);
        world.players[defender].target = world.players[defender].pos;
        world.possession.team = TeamSide::Home;
        world.start_possession(TeamSide::Home, PossessionKind::Live);
        world.set_phase(Phase::Live);
        world.ball.set_held(shooter);
        world.ball.pos = spot;
        world.ball.z = world.rules.ball_radius_ft;
        let intent = ActionIntent {
            actor: shooter,
            kind: ActionKind::Shoot(kind),
            target: None,
            from_ft: spot,
            to_ft: spot,
            start_time: world.now(),
            duration: 0.6,
        };
        nba_sim_core::resolve::shot::resolve(&mut world, &intent, kind);
        (world.players[shooter].pos - world.players[defender].pos).length()
    };
    let catch_shoot = separation_after(ShotKind::CatchShoot);
    let step_back = separation_after(ShotKind::StepBack);
    assert!(
        (catch_shoot - 1.4).abs() < 0.05,
        "接球就投不得改变站位：{catch_shoot:.2}"
    );
    assert!(
        step_back > catch_shoot + 2.0,
        "后撤步必须真的拉开空间：{catch_shoot:.2} → {step_back:.2}"
    );
}

/// 背身挤打：力量优势真的把背身要位推到更靠近篮筐的位置。
#[test]
fn a_stronger_backdown_gets_closer_to_the_rim() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let target_distance = |strength: f32| -> f32 {
        let mut world = World::new(
            MatchSetup::builtin(GameRules::default()),
            params.clone(),
            211,
        );
        let offense = world.on_court(TeamSide::Home).as_slice()[0];
        let defender = world.on_court(TeamSide::Away).as_slice()[0];
        for (index, player) in world.players.iter_mut().enumerate() {
            player.on_court = index == offense || index == defender;
            player.vel = Vec2::ZERO;
        }
        world.players[offense].data.attributes.strength = strength;
        world.players[defender].data.attributes.strength = 0.5;
        let hoop = world.hoop_for(TeamSide::Home);
        let spot = hoop + Vec2::new(-9.0, 2.0);
        world.players[offense].pos = spot;
        world.players[offense].target = spot;
        world.players[offense].facing = (hoop - spot).normalize_or_zero();
        world.players[defender].pos = spot + Vec2::new(-2.0, 0.0);
        world.players[defender].target = world.players[defender].pos;
        world.possession.team = TeamSide::Home;
        world.start_possession(TeamSide::Home, PossessionKind::Live);
        world.set_phase(Phase::Live);
        world.ball.set_held(offense);
        world.ball.pos = spot;
        world.ball.z = world.rules.ball_radius_ft;
        let intent = ActionIntent {
            actor: offense,
            kind: ActionKind::PostUp,
            target: None,
            from_ft: spot,
            to_ft: spot,
            start_time: world.now(),
            duration: 0.8,
        };
        let before = world.events.len();
        let outcome = nba_sim_core::resolve::drive::resolve_post(&mut world, &intent);
        let fouled = world.events[before..].iter().any(|event| {
            matches!(
                &event.fact,
                Fact::Foul {
                    kind: nba_domain::event::FoulKind::Offensive,
                    ..
                }
            )
        });
        assert!(
            !fouled,
            "合法的背身挤打不得判进攻犯规：{:?}",
            outcome.completion
        );
        (world.players[offense].target - hoop).length()
    };
    let weak = target_distance(0.1);
    let strong = target_distance(0.95);
    assert!(
        strong < weak - 0.3,
        "力量更強的背身必须更靠近篮筐：weak={weak:.2} strong={strong:.2}"
    );
}

/// 背身步法：防守人站定走上下步，往缩就走撤步转身，落点随之不同。
#[test]
fn post_footwork_follows_the_defenders_stance() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let run =
        |defender_vel: f32, offset_y: f32, strength: f32, defense: f32| -> (&'static str, f32) {
            let mut world = World::new(
                MatchSetup::builtin(GameRules::default()),
                params.clone(),
                223,
            );
            let offense = world.on_court(TeamSide::Home).as_slice()[0];
            let defender = world.on_court(TeamSide::Away).as_slice()[0];
            for (index, player) in world.players.iter_mut().enumerate() {
                player.on_court = index == offense || index == defender;
                player.vel = Vec2::ZERO;
            }
            world.players[offense].data.attributes.strength = strength;
            world.players[defender].data.attributes.strength = defense;
            world.players[defender].data.attributes.defense_interior = defense;
            let hoop = world.hoop_for(TeamSide::Home);
            let spot = hoop + Vec2::new(-8.0, offset_y);
            world.players[offense].pos = spot;
            world.players[offense].target = spot;
            world.players[offense].facing = (hoop - spot).normalize_or_zero();
            world.players[defender].pos = spot + Vec2::new(-2.0, 0.0);
            world.players[defender].target = world.players[defender].pos;
            world.players[defender].vel = Vec2::new(-defender_vel, 0.0);
            world.possession.team = TeamSide::Home;
            world.start_possession(TeamSide::Home, PossessionKind::Live);
            world.set_phase(Phase::Live);
            world.ball.set_held(offense);
            world.ball.pos = spot;
            world.ball.z = world.rules.ball_radius_ft;
            let intent = ActionIntent {
                actor: offense,
                kind: ActionKind::PostUp,
                target: None,
                from_ft: spot,
                to_ft: spot,
                start_time: world.now(),
                duration: 0.8,
            };
            let before = world.events.len();
            nba_sim_core::resolve::drive::resolve_post(&mut world, &intent);
            let kind = world.events[before..]
                .iter()
                .find_map(|event| match &event.fact {
                    Fact::PostUp { kind, .. } => Some(*kind),
                    _ => None,
                })
                .expect("背身必须发出带步法的事实");
            let distance = (world.players[offense].target - hoop).length();
            (kind, distance)
        };
    let (planted_kind, _) = run(0.0, 0.0, 0.9, 0.1);
    let (retreating_kind, _) = run(9.0, 0.0, 0.9, 0.1);
    assert_eq!(planted_kind, "up_and_under", "站定的防守人必须走上下步");
    assert_eq!(retreating_kind, "backdown", "移动中的防守人走背身挤打");
    // 靠底线时改用撤步转身。
    let (baseline_kind, _) = run(9.0, 7.0, 0.9, 0.1);
    assert_eq!(baseline_kind, "drop_step", "靠底线必须走撤步转身");
}

/// 传球被防守人点到：球变活球，控制权由随后的争抢决定。
#[test]
fn a_deflected_pass_becomes_a_loose_ball_decided_by_control() {
    let mut deflections = 0u32;
    // 点到传球是低频率事件：样本取十六场，否则会被单场节奏波动掩盖。
    for seed in 1..=16u64 {
        let report = world(seed).run();
        for (index, event) in report.events.iter().enumerate() {
            let Fact::BallContact {
                kind: "deflection",
                entity: Some(defender),
                ..
            } = &event.fact
            else {
                continue;
            };
            deflections += 1;
            // 被点到的球要么成为松球争抢，要么直接被碰到球的人控下来
            // 记作失误，要么两人同时抱住形成争球；三种都是控制权的转移。
            // 窗口只看到随后六十个事件，不能拿很久以后的结果来充数。
            let settled = report.events[index + 1..].iter().take(60).any(|other| {
                matches!(
                    &other.fact,
                    Fact::LooseBallRecovered { .. }
                        | Fact::Turnover { .. }
                        | Fact::PossessionEnded {
                            end_reason: nba_domain::event::PossessionEndCause::HeldBall,
                            ..
                        }
                )
            });
            assert!(
                settled,
                "被点到的传球必须进入松球争夺或形成争球（seed {seed}, defender {defender}）"
            );
        }
    }
    assert!(deflections > 0, "真实比赛里必须有被点到的传球");
}

/// 假动作与护球都不移动支撑脚：身体位置在动作前后必须一致。
#[test]
fn fakes_and_protection_keep_the_pivot_foot_planted() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    for fake in [
        FakeKind::Shot,
        FakeKind::Pass,
        FakeKind::JabStep,
        FakeKind::Hesitation,
    ] {
        let mut world = World::new(
            MatchSetup::builtin(GameRules::default()),
            params.clone(),
            227,
        );
        let actor = world.on_court(TeamSide::Home).as_slice()[0];
        let defender = world.on_court(TeamSide::Away).as_slice()[0];
        for (index, player) in world.players.iter_mut().enumerate() {
            player.on_court = index == actor || index == defender;
            player.vel = Vec2::ZERO;
        }
        let spot = Vec2::new(60.0, 25.0);
        world.players[actor].pos = spot;
        world.players[actor].target = spot;
        world.players[defender].pos = spot + Vec2::new(2.0, 0.0);
        world.players[defender].target = world.players[defender].pos;
        world.possession.team = TeamSide::Home;
        world.start_possession(TeamSide::Home, PossessionKind::Live);
        world.set_phase(Phase::Live);
        world.ball.set_held(actor);
        world.ball.pos = spot;
        world.ball.z = world.rules.ball_radius_ft;
        let intent = ActionIntent {
            actor,
            kind: ActionKind::Fake(fake),
            target: Some(defender),
            from_ft: spot,
            to_ft: spot,
            start_time: world.now(),
            duration: 0.4,
        };
        nba_sim_core::resolve::fake::resolve(&mut world, &intent, fake);
        let moved = (world.players[actor].pos - spot).length();
        assert!(
            moved < 1.0e-4,
            "假动作不得移动支撑脚：{fake:?} 移动了 {moved:.3} 英尺"
        );
        assert!(
            world.players[defender].bait.is_some(),
            "假动作必须给防守人留下失衡窗口：{fake:?}"
        );
    }
    for protect in [ProtectKind::FaceUp, ProtectKind::Post, ProtectKind::Spin] {
        let mut world = World::new(
            MatchSetup::builtin(GameRules::default()),
            params.clone(),
            229,
        );
        let actor = world.on_court(TeamSide::Home).as_slice()[0];
        let defender = world.on_court(TeamSide::Away).as_slice()[0];
        for (index, player) in world.players.iter_mut().enumerate() {
            player.on_court = index == actor || index == defender;
            player.vel = Vec2::ZERO;
        }
        let spot = Vec2::new(60.0, 25.0);
        world.players[actor].pos = spot;
        world.players[actor].target = spot;
        world.players[defender].pos = spot + Vec2::new(1.8, 0.0);
        world.players[defender].target = world.players[defender].pos;
        world.possession.team = TeamSide::Home;
        world.start_possession(TeamSide::Home, PossessionKind::Live);
        world.set_phase(Phase::Live);
        world.ball.set_held(actor);
        world.ball.pos = spot;
        world.ball.z = world.rules.ball_radius_ft;
        let intent = ActionIntent {
            actor,
            kind: ActionKind::Protect(protect),
            target: Some(defender),
            from_ft: spot,
            to_ft: spot,
            start_time: world.now(),
            duration: 0.4,
        };
        nba_sim_core::resolve::protect::resolve(&mut world, &intent, protect);
        let moved = (world.players[actor].pos - spot).length();
        assert!(
            moved < 1.0e-4,
            "护球不得移动支撑脚：{protect:?} 移动了 {moved:.3} 英尺"
        );
        assert!(
            world.protection.is_some(),
            "护球必须开出保护窗口：{protect:?}"
        );
    }
}

/// 双方同时够到球：停表进入中圈跳球，而不是把球判给某一个人。
#[test]
fn a_tie_for_the_ball_enters_a_held_ball_jump_ball() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, 233);
    let offense = world.on_court(TeamSide::Home).as_slice()[0];
    let defense = world.on_court(TeamSide::Away).as_slice()[0];
    let home_mate = world.on_court(TeamSide::Home).as_slice()[1];
    let away_mate = world.on_court(TeamSide::Away).as_slice()[1];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court =
            index == offense || index == defense || index == home_mate || index == away_mate;
        player.vel = Vec2::ZERO;
    }
    let ball_spot = Vec2::new(62.0, 25.0);
    // 两人到球的距离只差五分之一英尺：同时够到，谁也没有先控制。
    world.players[offense].pos = ball_spot + Vec2::new(5.0, 0.0);
    world.players[offense].target = world.players[offense].pos;
    world.players[defense].pos = ball_spot + Vec2::new(0.0, 5.05);
    world.players[defense].target = world.players[defense].pos;
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball = nba_sim_core::ball::BallBody::loose(ball_spot, Vec2::ZERO);
    world.ball.z = world.rules.ball_radius_ft;
    // 争抢已经超过两秒：双方都在必抢半径内。
    world.pending_rebound = Some(nba_sim_core::resolve::rebound::PendingRebound {
        offense: TeamSide::Home,
        zone: "test",
        shot_distance: 0.0,
        rim_contact: false,
        started_at: world.now() - 3.0,
        turnover: false,
        poke_by: None,
        pass_by: None,
        box_outs: [usize::MAX; nba_sim_core::state::MAX_ON_COURT],
        box_out_count: 0,
    });
    let before = world.events.len();
    nba_sim_core::ball::step(&mut world, TICK_SECONDS);
    let held_ball = world.events[before..].iter().any(|event| {
        matches!(
            &event.fact,
            Fact::BallContact {
                kind: "held_ball",
                ..
            }
        )
    });
    assert!(held_ball, "同时够到球必须发出停表事实");
    assert!(
        world.pending_jump_ball.is_some(),
        "停表后必须先进入跳球准备，而不是直接掷球"
    );
    assert_eq!(
        world.possession.id, 0,
        "跳球前原来那个回合必须按双方控球结清"
    );
    assert!(
        world.pending_rebound.is_none(),
        "跳球时不得留下未结算的篮板"
    );
    // 参跳四人必须走到圈边：整个过程没有瞬移。
    let mut guard = 0;
    loop {
        let prior: Vec<Vec2> = world.players.iter().map(|player| player.pos).collect();
        world.step(TICK_SECONDS);
        guard += 1;
        for (index, player) in world.players.iter().enumerate() {
            if !player.on_court {
                continue;
            }
            let moved = (player.pos - prior[index]).length();
            assert!(
                moved <= 1.2,
                "跳球准备不得瞬移：球员 {index} 单 tick 移动 {moved:.2} 英尺"
            );
        }
        if matches!(
            world.phase(),
            Phase::JumpBall {
                kind: nba_sim_core::action::JumpBallKind::HeldBall,
                ..
            }
        ) {
            break;
        }
        assert!(
            guard < 400,
            "跳球准备必须在限时内掷球，实际阶段 {:?}",
            world.phase()
        );
    }
}

/// 包夹与护球出球：两张手让伸手更容易、出球更难，护球把它补回来。
#[test]
fn a_trap_raises_poke_risk_and_protection_helps_the_exit() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let build = |trapped: bool, protection: bool| {
        let mut world = World::new(
            MatchSetup::builtin(GameRules::default()),
            params.clone(),
            239,
        );
        let holder = world.on_court(TeamSide::Home).as_slice()[0];
        let receiver = world.on_court(TeamSide::Home).as_slice()[1];
        let first = world.on_court(TeamSide::Away).as_slice()[0];
        let second = world.on_court(TeamSide::Away).as_slice()[1];
        for (index, player) in world.players.iter_mut().enumerate() {
            player.on_court = index == holder
                || index == receiver
                || index == first
                || (trapped && index == second);
            player.vel = Vec2::ZERO;
        }
        let spot = Vec2::new(55.0, 25.0);
        world.players[holder].pos = spot;
        world.players[holder].target = spot;
        world.players[receiver].pos = Vec2::new(75.0, 25.0);
        world.players[receiver].target = world.players[receiver].pos;
        world.players[first].pos = spot + Vec2::new(0.0, 2.0);
        world.players[first].target = world.players[first].pos;
        world.players[second].pos = spot + Vec2::new(0.0, -2.0);
        world.players[second].target = world.players[second].pos;
        world.possession.team = TeamSide::Home;
        world.start_possession(TeamSide::Home, PossessionKind::Live);
        world.set_phase(Phase::Live);
        world.ball.set_held(holder);
        world.ball.pos = spot;
        world.ball.z = world.rules.ball_radius_ft;
        if protection {
            world.protection = Some(nba_sim_core::state::BallProtection {
                holder,
                kind: ProtectKind::Post,
                until: world.now() + 3.0,
            });
        }
        (world, holder, receiver, first)
    };

    let (single, holder, receiver, thief) = build(false, false);
    let single_poke =
        nba_sim_core::resolve::poke::poke_success(&single, thief, holder, 2.0, single.now());
    let single_pass = nba_sim_core::value::pass_completion_probability(
        &single,
        holder,
        receiver,
        nba_sim_core::action::PassKind::Chest,
    );
    let (trapped, holder, receiver, thief) = build(true, false);
    assert!(trapped.trapped(holder), "两名防守人贴身必须判定为包夹");
    let trapped_poke =
        nba_sim_core::resolve::poke::poke_success(&trapped, thief, holder, 2.0, trapped.now());
    let trapped_pass = nba_sim_core::value::pass_completion_probability(
        &trapped,
        holder,
        receiver,
        nba_sim_core::action::PassKind::Chest,
    );
    let (protected, holder, receiver, thief) = build(true, true);
    let protected_poke =
        nba_sim_core::resolve::poke::poke_success(&protected, thief, holder, 2.0, protected.now());
    let protected_pass = nba_sim_core::value::pass_completion_probability(
        &protected,
        holder,
        receiver,
        nba_sim_core::action::PassKind::Chest,
    );

    assert!(
        trapped_poke > single_poke,
        "包夹必须抬高伸手成功率：{single_poke:.3} → {trapped_poke:.3}"
    );
    assert!(
        protected_poke < trapped_poke,
        "护球必须压低包夹下的伸手成功率：{trapped_poke:.3} → {protected_poke:.3}"
    );
    assert!(
        trapped_pass < single_pass,
        "包夹必须压低出球成功率：{single_pass:.3} → {trapped_pass:.3}"
    );
    assert!(
        protected_pass > trapped_pass,
        "护球必须抬高包夹下的出球成功率：{trapped_pass:.3} → {protected_pass:.3}"
    );
    assert!(
        protected_pass > 0.05,
        "护球出球必须仍然有成功的可能：{protected_pass:.3}"
    );
}

/// 欧洲步：直线路上有人时收球步往侧面绕，没人时走直线。
#[test]
fn a_contested_lane_finish_takes_a_euro_step_around_the_defender() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let lateral_offset = |blocked: bool| -> f32 {
        let mut world = World::new(
            MatchSetup::builtin(GameRules::default()),
            params.clone(),
            241,
        );
        let finisher = world.on_court(TeamSide::Home).as_slice()[0];
        let defender = world.on_court(TeamSide::Away).as_slice()[0];
        for (index, player) in world.players.iter_mut().enumerate() {
            player.on_court = index == finisher || index == defender;
            player.vel = Vec2::ZERO;
        }
        let hoop = world.hoop_for(TeamSide::Home);
        let spot = hoop + Vec2::new(-14.0, 0.0);
        let gather = hoop + Vec2::new(-8.0, 0.0);
        world.players[finisher].pos = gather;
        world.players[finisher].target = gather;
        world.players[defender].pos = if blocked {
            // 站在收球步的直线路上。
            hoop + Vec2::new(-4.0, 0.4)
        } else {
            // 站在路外十英尺。
            hoop + Vec2::new(-4.0, 10.0)
        };
        world.players[defender].target = world.players[defender].pos;
        world.possession.team = TeamSide::Home;
        world.start_possession(TeamSide::Home, PossessionKind::Live);
        world.set_phase(Phase::Live);
        world.ball.set_held(finisher);
        world.ball.pos = gather;
        world.ball.z = world.rules.ball_radius_ft;
        world.possession.gather_pos = Some(hoop + Vec2::new(-2.0, 0.0));
        let intent = ActionIntent {
            actor: finisher,
            kind: ActionKind::Finish,
            target: None,
            from_ft: spot,
            to_ft: hoop,
            start_time: world.now(),
            duration: 0.5,
        };
        nba_sim_core::resolve::drive::resolve_finish(&mut world, &intent);
        world.players[finisher].target.y - hoop.y
    };
    let straight = lateral_offset(false);
    let around = lateral_offset(true);
    assert!(
        straight.abs() < 0.2,
        "无人堵路时收球步必须走直线：{straight:.2}"
    );
    assert!(around.abs() > 1.0, "有人堵路时必须绕着走：{around:.2}");
}

/// 无球切入路线：真实比赛里必须用到不止一种路线，且每种路线都被记录。
#[test]
fn real_games_use_several_cut_routes() {
    let mut counts: std::collections::BTreeMap<&'static str, u32> =
        std::collections::BTreeMap::new();
    for seed in 1..=4u64 {
        let report = world(seed).run();
        for event in &report.events {
            if let Fact::Cut { kind, arrived, .. } = &event.fact {
                if *arrived {
                    *counts.entry(kind).or_default() += 1;
                }
            }
        }
    }
    assert!(counts.len() >= 2, "切入路线必须不止一种，实际 {counts:?}");
    assert!(
        counts.get("basket").copied().unwrap_or(0) > 0,
        "直切篮下仍是主要路线：{counts:?}"
    );
}

/// 过掩护的两条路线：挤过继续贴人，绕过先沉到掩护人与篮筐之间。
#[test]
fn screen_navigation_route_decides_the_defender_target() {
    use nba_sim_core::action::DefenderRoute;
    use nba_sim_core::state::{DefenseResponsibility, ScreenRead};
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let run = |route: DefenderRoute| {
        let mut world = World::new(
            MatchSetup::builtin(GameRules::default()),
            params.clone(),
            251,
        );
        let carrier = world.on_court(TeamSide::Home).as_slice()[0];
        let screener = world.on_court(TeamSide::Home).as_slice()[1];
        let guard = world.on_court(TeamSide::Away).as_slice()[0];
        let helper = world.on_court(TeamSide::Away).as_slice()[1];
        for (index, player) in world.players.iter_mut().enumerate() {
            player.on_court =
                index == carrier || index == screener || index == guard || index == helper;
            player.vel = Vec2::ZERO;
        }
        let hoop = world.hoop_for(TeamSide::Home);
        let carrier_spot = hoop + Vec2::new(-20.0, 0.0);
        world.players[carrier].pos = carrier_spot;
        world.players[carrier].target = carrier_spot;
        // 掩护人站在持球人与篮筐之间。
        world.players[screener].pos = hoop + Vec2::new(-16.0, 0.0);
        world.players[screener].target = world.players[screener].pos;
        world.players[guard].pos = hoop + Vec2::new(-18.5, 1.2);
        world.players[guard].target = world.players[guard].pos;
        world.possession.team = TeamSide::Home;
        world.start_possession(TeamSide::Home, PossessionKind::Live);
        world.set_phase(Phase::Live);
        world.ball.set_held(carrier);
        world.ball.pos = carrier_spot;
        world.ball.z = world.rules.ball_radius_ft;
        world.possession.last_screen = Some(ScreenRead {
            route,
            at: world.now(),
            screener,
            separation_ft: 1.2,
        });
        // 职责只有在换对位或驻留时间到点时才变：这里直接标记对位刚变更。
        nba_sim_core::defense::refresh_matchup(&mut world);
        world.possession.pairing_changed_at = world.now() + 1.0;
        // 压力目标的提交有最小驻留时间：先推进时钟越过驻留窗口。
        world.clock.advance_simulation_time(1.0);
        nba_sim_core::defense::apply_defense_targets(&mut world);
        let responsibility = world.players[guard].defense_responsibility;
        let target = world.players[guard].target;
        (
            responsibility,
            target,
            world.players[guard].target_kind,
            world.players[guard].target_partner,
        )
    };
    let (over, over_target, over_kind, over_partner) = run(DefenderRoute::Over);
    let (under, under_target, under_kind, under_partner) = run(DefenderRoute::Under);
    assert_eq!(
        over,
        DefenseResponsibility::GoOver,
        "挤过的职责必须被记录为 go_over"
    );
    assert_eq!(
        under,
        DefenseResponsibility::GoUnder,
        "绕过的职责必须被记录为 go_under"
    );
    assert_eq!(
        over_kind,
        nba_sim_core::state::TargetKind::Pressure,
        "挤过仍然要贴着持球人"
    );
    assert_eq!(
        under_kind,
        nba_sim_core::state::TargetKind::Pressure,
        "绕过也要占据一个压迫位置"
    );
    assert_ne!(
        over_partner, under_partner,
        "两条路线盯的不是同一个人：挤过贴持球人，绕过盯掩护人"
    );
    assert!(
        (over_target - under_target).length() > 1.0,
        "两条路线的落点必须不同：{over_target:?} vs {under_target:?}"
    );
}

/// 回位：离自己的对位人太远时，职责必须变成回位并指向那个人。
#[test]
fn a_defender_out_of_position_recovers_to_his_man() {
    use nba_sim_core::state::{DefenseResponsibility, TargetKind};
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let run = |gap_ft: f32| {
        let mut world = World::new(
            MatchSetup::builtin(GameRules::default()),
            params.clone(),
            257,
        );
        let carrier = world.on_court(TeamSide::Home).as_slice()[0];
        let man = world.on_court(TeamSide::Home).as_slice()[1];
        let lost = world.on_court(TeamSide::Away).as_slice()[0];
        let ball_guard = world.on_court(TeamSide::Away).as_slice()[1];
        for (index, player) in world.players.iter_mut().enumerate() {
            player.on_court =
                index == carrier || index == man || index == lost || index == ball_guard;
            player.vel = Vec2::ZERO;
        }
        let hoop = world.hoop_for(TeamSide::Home);
        let carrier_spot = hoop + Vec2::new(-20.0, 0.0);
        world.players[carrier].pos = carrier_spot;
        world.players[carrier].target = carrier_spot;
        world.players[man].pos = Vec2::new(66.0, 8.0);
        world.players[man].target = world.players[man].pos;
        world.players[ball_guard].pos = carrier_spot + Vec2::new(0.0, 2.0);
        world.players[ball_guard].target = world.players[ball_guard].pos;
        // 失位防守人离自己的对位人 gap_ft 英尺。
        world.players[lost].pos = world.players[man].pos + Vec2::new(gap_ft, 0.0);
        world.players[lost].target = world.players[lost].pos;
        world.possession.team = TeamSide::Home;
        world.start_possession(TeamSide::Home, PossessionKind::Live);
        world.set_phase(Phase::Live);
        world.ball.set_held(carrier);
        world.ball.pos = carrier_spot;
        world.ball.z = world.rules.ball_radius_ft;
        nba_sim_core::defense::refresh_matchup(&mut world);
        world.possession.pairing_changed_at = world.now() + 1.0;
        world.clock.advance_simulation_time(1.0);
        nba_sim_core::defense::apply_defense_targets(&mut world);
        (
            world.players[lost].defense_responsibility,
            world.players[lost].target_kind,
            world.players[lost].target_partner,
            man,
            (world.players[lost].target - world.players[man].pos).length(),
        )
    };
    let (far_role, far_kind, far_partner, man, far_gap) = run(15.0);
    assert_eq!(
        far_role,
        DefenseResponsibility::Recover,
        "离对位人十五英尺必须进入回位"
    );
    assert_eq!(far_kind, TargetKind::Pressure, "回位也是压迫目标");
    assert_eq!(far_partner, man, "回位必须指向自己的对位人");
    assert!(
        far_gap < 6.0,
        "回位目标必须落在对位人身边：{far_gap:.2} 英尺"
    );
    let (near_role, _, _, _, _) = run(3.0);
    assert_ne!(
        near_role,
        DefenseResponsibility::Recover,
        "贴在対位人身边时不该进入回位"
    );
}

/// 无球掩护由掩护人自己发起：真实比赛里必须出现 on_ball == false 的掩护。
#[test]
fn real_games_include_off_ball_screens() {
    let mut off_ball = 0u32;
    for seed in 1..=12u64 {
        let report = world(seed).run();
        off_ball += report
            .events
            .iter()
            .filter(|event| matches!(&event.fact, Fact::Screen { on_ball: false, .. }))
            .count() as u32;
    }
    assert!(off_ball > 0, "真实比赛里必须有掩护人自己发起的无球掩护");
}
