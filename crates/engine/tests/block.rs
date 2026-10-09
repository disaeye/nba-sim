//! 封盖行为：封盖作为独立比赛事实，且只在合法的动作阶段窗口内成立。
//!
//! 守卫对象：
//! - 封盖与「投失 + 篮板」是不同事实（前者之后是松球争夺，责任主体是防守人）；
//! - 封盖只在出手者的 `ActionPhase::Execution` 内发生，合球阶段归切球（Strip）；
//! - 封盖事实必须携带出手阶段与命中裁定，使评判能判断「封盖是否改变了结果」。
//!
//! 对应 `docs/architecture.md` §6、`docs/quality.md` §2.1（回合因果）与
//! `attributes.md` §2.4（`block` / `vertical` 的消费链）。

mod support;

use nba_engine::MatchEngine;
use rayon::prelude::*;

/// 封盖事实必须真实出现在比赛里，且比率处于真实区间。
///
/// 实测（seed 42/1/7/100/999，全场）：合计 33/1023 = 3.2% 的出手被封，
/// 单 seed 2.4%–4.9%；真实 NBA 约 5.5%。这里只断言一个宽松区间——精确标定属于 `stats_baseline`。
#[test]
fn blocked_shots_occur_at_a_plausible_rate() {
    // 种子间并行：两场完整比赛互不共享状态，并行只改变 wall time。
    let (blocks, shots) = [42u64, 100]
        .par_iter()
        .copied()
        .map(|seed| {
            let mut engine = MatchEngine::new(seed);
            engine.set_scope("full").expect("full scope is valid");
            let mut blocks = 0u32;
            let mut shots = 0u32;
            while !engine.is_finished() {
                let tick = engine.step();
                for ev in &tick.frame.event_log {
                    match ev.kind.as_str() {
                        "BLOCKED_SHOT" => blocks += 1,
                        "SHOT_RELEASE" => shots += 1,
                        _ => {}
                    }
                }
            }
            (blocks, shots)
        })
        .reduce(|| (0u32, 0u32), |a, b| (a.0 + b.0, a.1 + b.1));
    assert!(shots > 0, "a full game must contain shot attempts");
    let rate = f64::from(blocks) / f64::from(shots);
    assert!(
        (0.005..=0.12).contains(&rate),
        "blocked-shot rate {:.3} ({blocks}/{shots}) is outside the plausible band; \
         a rate near 0 means the block path never fires, a rate above 0.12 means it \
         fires more than once per attempt or on every tick",
        rate
    );
}

/// 封盖事实必须与「投失」区分：`BLOCKED_SHOT` 出现时，后续是松球而非篮板。
#[test]
fn blocked_shot_is_distinct_from_a_miss() {
    let mut engine = MatchEngine::new(100);
    engine.set_scope("full").expect("full scope is valid");
    let mut saw_block = false;
    while !engine.is_finished() && !saw_block {
        let tick = engine.step();
        for ev in &tick.frame.event_log {
            if ev.kind != "BLOCKED_SHOT" {
                continue;
            }
            saw_block = true;
            let payload = ev
                .data
                .as_ref()
                .and_then(|d| d.get("BlockedShot"))
                .expect("BLOCKED_SHOT must carry its own payload");
            // 责任主体必须是防守人（与投失不同：投失没有防守责任方）。
            assert!(
                payload.get("blocker_id").and_then(|v| v.as_str()).is_some(),
                "a block must name the defender who stopped the shot"
            );
            // 出手阶段必须是可封盖窗口，使评判能验证门控真的生效。
            assert_eq!(
                payload.get("phase").and_then(|v| v.as_str()),
                Some("Execution"),
                "blocks may only be recorded during the Execution phase \
                 (Preparation belongs to strip attempts, FollowThrough means the ball is gone)"
            );
        }
    }
    assert!(
        saw_block,
        "a full game must contain at least one blocked shot (seed 100 measures ~11)"
    );
}

/// 同一出手轨迹在封盖与未封盖分支必须产生不同的终态。
#[test]
fn identical_shot_parameters_diverge_when_a_block_is_applied() {
    use glam::Vec2;
    use nba_domain::{action_window::ActionTimeWindow, GameRules};
    use nba_engine::{MatchEngine, MatchSetup};
    use nba_physics::ballistics::BallTrajectoryKind;

    let mut setup = MatchSetup::builtin(GameRules::default());
    setup.rules.resolve.block.base_probability = 1.0;
    setup.rules.resolve.block.probability_ceiling = 1.0;
    let hoop = setup.rules.court.hoop_pos(true);
    let origin = hoop - Vec2::new(3.0, 0.0);
    let baseline = BallTrajectoryKind::Shot {
        shooter_id: "H_01".to_string(),
        from_pos: origin,
        hoop_pos: hoop,
        aim_pos: hoop,
        start_time: 0.0,
        duration: 0.8,
        collision_checked_until: 0.0,
        is_three: false,
        peak_z: 13.0,
        make_probability: 0.7,
        contest_intensity: 0.0,
    };
    let mut blocked = MatchEngine::with_setup(setup.clone(), 811);
    let control_time = 0.3;
    blocked.set_game_flow_for_test(nba_domain::GameFlowState::LiveBall);
    blocked.force_possession_for_test(nba_domain::Possession::Home);
    blocked.set_ball_state_for_test(baseline.clone());
    blocked.set_ball_pos_for_test(origin, blocked.rules().chest_height_ft);
    blocked.set_current_time_for_test(0.2);
    let mut clear = MatchEngine::with_setup(setup, 811);
    clear.set_game_flow_for_test(nba_domain::GameFlowState::LiveBall);
    clear.force_possession_for_test(nba_domain::Possession::Home);
    clear.set_ball_state_for_test(baseline.clone());
    clear.set_ball_pos_for_test(origin, clear.rules().chest_height_ft);
    clear.set_current_time_for_test(0.2);

    let mut window = ActionTimeWindow::new_jump_shot("H_01", 0.0, blocked.rules());
    window.prep_duration = 0.0;
    window.exec_duration = 1.0;
    window.interference_start = 0.0;
    window.interference_end = 1.0;
    blocked.start_action_window_for_test("H_01", window, None, None);
    blocked.advance_action_windows_for_test(control_time);
    blocked.set_current_time_for_test(control_time);
    blocked.set_player_position_for_test("H_01", origin);
    blocked.set_player_position_for_test("A_05", origin + Vec2::new(-0.2, 0.0));
    clear.set_player_position_for_test("H_01", origin);
    blocked.set_player_block_attributes_for_test("A_05", 220, 1.0, 1.0, 1.0);
    assert_eq!(
        blocked.window_phase("H_01").as_deref(),
        Some("Execution"),
        "block probe must be in the shooter execution window"
    );
    blocked.set_ball_pos_for_test(origin, 8.5);
    assert!(
        blocked.try_resolve_shot_block_for_test(),
        "same parameters must generate a BLOCKED_SHOT fact when the block intervention succeeds"
    );
    let block_reached = !matches!(blocked.ball_state(), BallTrajectoryKind::Shot { .. });
    assert!(block_reached, "blocked trajectory must leave Shot state");
    let clear_reached = clear.resolve_shot_flight_for_test();
    assert!(
        !clear_reached,
        "unblocked trajectory before first contact must remain Shot"
    );
    assert!(
        matches!(clear.ball_state(), BallTrajectoryKind::Shot { .. })
            && !matches!(blocked.ball_state(), BallTrajectoryKind::Shot { .. }),
        "block and clear controls must have different ball-state outcomes"
    );
}

/// 封盖率必须响应 `block` 能力：把全队封盖能力推到极端高，被封数必须上升。
#[test]
fn block_rate_responds_to_the_block_attribute() {
    use nba_domain::GameRules;
    use nba_engine::MatchSetup;

    let run = |block_value: f32| -> u32 {
        [42u64, 100]
            .par_iter()
            .map(|&seed| {
                let mut setup = MatchSetup::builtin(GameRules::default());
                for team in [&mut setup.home_team, &mut setup.away_team] {
                    for p in team.players.iter_mut() {
                        p.attributes.block = block_value;
                        p.attributes.vertical = block_value;
                    }
                }
                let mut engine = MatchEngine::with_setup(setup, seed);
                engine.set_scope("full").expect("full scope is valid");
                let mut blocks = 0u32;
                while !engine.is_finished() {
                    let tick = engine.step();
                    for ev in &tick.frame.event_log {
                        if ev.kind == "BLOCKED_SHOT" {
                            blocks += 1;
                        }
                    }
                }
                blocks
            })
            .sum()
    };

    let low = run(0.05);
    let high = run(0.95);
    assert!(
        high > low,
        "raising `block` + `vertical` on every player must raise the blocked-shot count \
         (low={low}, high={high}); equal counts mean the capability is not consumed"
    );
}
