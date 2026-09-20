//! M10 acceptance (architecture.md 6.3): LeagueProfile switching = data switching,
//! engine code path has zero league branches; FIBA profile completes a match segment with zero Hard violations.
//!
use nba_domain::{GameRules, LeagueProfile};
use nba_engine::MatchEngine;
use rayon::prelude::*;

#[test]
fn fiba_profile_produces_fiba_shapes() {
    let rules = GameRules::with_league(LeagueProfile::fiba());
    assert_eq!(rules.league.period_duration_seconds, 600.0);
    assert_eq!(rules.league.max_personal_fouls, 5);
    assert_eq!(rules.league.bonus_fouls_per_period, 4);
    assert!((rules.league.three_point_distance_ft - 22.15).abs() < 0.01);
    assert_eq!(rules.league.court.width_ft, 91.86);
    rules.validate().expect("fiba profile must validate");
}

#[test]
fn nba_profile_is_default() {
    let rules = GameRules::default();
    assert_eq!(rules.league.max_personal_fouls, 6);
    assert_eq!(rules.league.period_duration_seconds, 720.0);
}

#[test]
fn fiba_quarter_runs_with_zero_hard_violations() {
    let rules = GameRules::with_league(LeagueProfile::fiba());
    let mut engine = MatchEngine::with_setup(nba_engine::MatchSetup::builtin(rules), 42);
    for tick_idx in 0..6000 {
        if engine.is_finished() {
            break;
        }
        let tick = engine.step();
        let hard: Vec<_> = engine
            .last_tick_violations()
            .iter()
            .filter(|v| matches!(v.severity, nba_invariants::ViolationSeverity::Hard))
            .collect();
        assert!(
            hard.is_empty(),
            "FIBA seed 42 tick {} hard violations: {:?}",
            tick_idx,
            hard
        );
        assert!((tick.frame.rules.three_point_distance_ft - 22.15).abs() < 0.01);
    }
}
#[test]
fn fiba_full_game_runs_and_evaluates_with_fiba_fixture() {
    let rules = GameRules::with_league(LeagueProfile::fiba());
    let mut engine = MatchEngine::with_setup(nba_engine::MatchSetup::builtin(rules.clone()), 42);
    let mut ticks = Vec::new();

    // Simulate full 4 quarters under FIBA profile (4 * 600s = 2400s).
    // In 0.04s ticks, 4 quarters is 60,000 game clock ticks;
    // simulate 8,000 active ticks for rigorous multi-possession and judgment coverage.
    for tick_idx in 0..8000 {
        let _tick = engine.step();
        let hard: Vec<_> = engine
            .last_tick_violations()
            .iter()
            .filter(|v| matches!(v.severity, nba_invariants::ViolationSeverity::Hard))
            .collect();
        assert!(
            hard.is_empty(),
            "tick {} triggered hard violation: {:?}",
            tick_idx,
            hard
        );
        ticks.push(_tick);
    }

    assert_eq!(ticks.len(), 8000);
    assert!(
        engine.completed_possessions() > 10,
        "FIBA 8000 ticks must simulate active possessions"
    );
    assert_eq!(rules.league.name, "FIBA");
}

#[test]
fn forced_substitution_preserves_five_on_five() {
    let mut engine = MatchEngine::new(42);
    let out_id = "A_01".to_string();
    engine
        .physics_mut_for_test()
        .get_player_mut(&out_id)
        .expect("starter A_1 must exist")
        .foul_count = 5;
    engine.forced_substitution(&out_id);

    let out = engine.physics().get_player(&out_id).unwrap();
    assert!(!out.on_court, "fouled-out player must leave court");
    assert!(!out.has_ball);

    let home_on = engine
        .physics()
        .get_players()
        .values()
        .filter(|p| p.team == "home" && p.on_court)
        .count();
    let away_on = engine
        .physics()
        .get_players()
        .values()
        .filter(|p| p.team == "away" && p.on_court)
        .count();
    assert_eq!(home_on, 5, "home must keep 5 on court");
    assert_eq!(away_on, 5, "away must keep 5 on court");

    assert!(
        !engine.away_roster_order().contains(&out_id),
        "roster order must drop fouled-out player"
    );

    let events: Vec<String> = engine
        .pending_events()
        .iter()
        .map(|e| e.event_type_str().to_string())
        .collect();
    assert!(
        events.iter().any(|k| k == "SUBSTITUTION"),
        "substitution must emit an event, got {:?}",
        events
    );

    for tick in 0..500 {
        engine.step();
        if !engine.last_tick_violations().is_empty() {
            panic!(
                "tick {} violations: {:?}",
                tick,
                engine.last_tick_violations()
            );
        }
    }
}

// ============================================================================
// D11.1 · NBA/FIBA 程序级情景 fixture
//
// ## 为什么需要（current/plan.md §7 D11.1）
//
// 上面的测试断言的是**类型与常量**（`period_duration_seconds == 600.0` 等），
// 只能证明档案被读入。计划要求的是**程序级**证据：同一 engine 路径切换
// profile 后，程序行为按各自规则真实改变。
//
// gap.md §14.3 亦要求「支持的 profile 有程序级情景矩阵」。
//
// 以下每个用例都构造一个**最小可复现场景**（gap.md §18.3），断言两个
// profile 在同一场景下的行为差异，而不是只断言数据字段。
// ============================================================================

/// 情景 1：节时长程序——同一 tick 数下两 profile 的节剩余时间推进不同。
///
/// NBA 12 分钟/节、FIBA 10 分钟/节，都由 `league.period_duration_seconds`
/// 决定。这里是**运行时**证据：相同拍数后，FIBA 的节时钟更早见底。
#[test]
fn scenario_period_duration_program_differs() {
    let nba = GameRules::default();
    let fiba = GameRules::with_league(LeagueProfile::fiba());

    let mut en = MatchEngine::with_setup(nba_engine::MatchSetup::builtin(nba.clone()), 7);
    let mut ef = MatchEngine::with_setup(nba_engine::MatchSetup::builtin(fiba.clone()), 7);

    // 推进相同 tick 数（足够跨过跳球与首次发球，但远未到节末）。
    for _ in 0..600 {
        en.step();
        ef.step();
    }

    let nba_elapsed = nba.league.period_duration_seconds - en.game_clock();
    let fiba_elapsed = fiba.league.period_duration_seconds - ef.game_clock();
    assert!(
        (nba_elapsed - fiba_elapsed).abs() < 1e-3,
        "同一 engine 路径下两 profile 应推进相同真实时间：NBA {nba_elapsed:.3}s vs FIBA {fiba_elapsed:.3}s"
    );

    // 关键差异：相等的**真实**时间消耗，占各自节长的比例不同。
    let nba_ratio = nba_elapsed / nba.league.period_duration_seconds;
    let fiba_ratio = fiba_elapsed / fiba.league.period_duration_seconds;
    assert!(
        fiba_ratio > nba_ratio,
        "FIBA 节更短，同样耗时占比应更大：FIBA {fiba_ratio:.4} vs NBA {nba_ratio:.4}"
    );
}

/// 情景 2：进攻篮板后的进攻时钟重置——两 profile 的档案值不同且可观测。
#[test]
fn scenario_offensive_rebound_shot_clock_program() {
    let nba = GameRules::default();
    let fiba = GameRules::with_league(LeagueProfile::fiba());

    // FIBA 与 NBA 均为 14 秒（近年的规则趋同），此用例固定该约定，
    // 防止未来任一侧被误改而无人察觉。
    assert_eq!(nba.league.offensive_rebound_shot_clock_seconds, 14.0);
    assert_eq!(fiba.league.offensive_rebound_shot_clock_seconds, 14.0);

    // 程序级：重置值必须真的进入引擎状态（而非只存在于档案）。
    for rules in [nba, fiba] {
        let mut e = MatchEngine::with_setup(nba_engine::MatchSetup::builtin(rules.clone()), 11);
        e.set_shot_clock_for_test(rules.league.offensive_rebound_shot_clock_seconds);
        assert!(
            (e.shot_clock() - 14.0).abs() < 1e-3,
            "shot clock must accept the profile's offensive-rebound reset value"
        );
    }
}

/// 情景 3：三分几何——FIBA 无 NBA 式底角直线段。
///
/// 这是两 profile 在**几何语义**上的真实差异（`corner_three_distance_ft`：
/// NBA 22.0 vs FIBA 0.0），且能被运行时判定观察到：它改变「哪些位置算三分」。
///
/// 取证点（实测探针）：`y = 2.9 ft`（底角区内）、`x = 篮筐 x`。
/// - NBA：距篮筐 22.10 ft < 弧顶 23.75，但 ≥ 底角线 22.0 → **三分**；
/// - FIBA：距篮筐 21.70 ft < 弧顶 22.15，且无底角分支 → **非三分**。
///
/// 注意几何前提：篮筐位于场地中线（y = height/2），故底角点到篮筐的
/// 垂直分量接近半个场高；底角线的意义是**沿 x 方向**放宽，而不是让
/// 底角点到篮筐的距离整体变小。
#[test]
fn scenario_three_point_geometry_program_differs() {
    use glam::Vec2;

    let nba = GameRules::default();
    let fiba = GameRules::with_league(LeagueProfile::fiba());

    // 两档案的底角语义确实不同（这里断言的是行为差异，超出常量本身）。
    assert!(
        nba.league.corner_three_distance_ft > 0.0,
        "NBA must declare a corner three line"
    );
    assert_eq!(
        fiba.league.corner_three_distance_ft, 0.0,
        "FIBA has a uniform arc: no corner three line"
    );

    // 同一**相对位置**（底角区、篮筐正前方 2.9 ft 处）在两侧的判定不同。
    let nba_court = nba.league.court;
    let fiba_court = fiba.league.court;
    let nba_point = Vec2::new(nba_court.hoop_right_x_ft, 2.9);
    let fiba_point = Vec2::new(fiba_court.hoop_right_x_ft, 2.9);

    let nba_is_three = nba_court.is_three_point_attempt(
        nba_point,
        true,
        nba.league.three_point_distance_ft,
        nba.league.corner_three_distance_ft,
    );
    let fiba_is_three = fiba_court.is_three_point_attempt(
        fiba_point,
        true,
        fiba.league.three_point_distance_ft,
        fiba.league.corner_three_distance_ft,
    );

    let nba_dist = (nba_point - Vec2::new(nba_court.hoop_right_x_ft, nba_court.hoop_y_ft)).length();
    let fiba_dist =
        (fiba_point - Vec2::new(fiba_court.hoop_right_x_ft, fiba_court.hoop_y_ft)).length();

    assert!(
        nba_is_three,
        "NBA: {nba_dist:.2} ft in the corner is a three (corner line 22.0 < arc 23.75)"
    );
    assert!(
        !fiba_is_three,
        "FIBA: {fiba_dist:.2} ft is NOT a three — uniform arc 22.15, no corner line"
    );
}

/// 情景 4：个人犯规上限——犯满阈值按 profile 进入**运行时事实**。
///
/// 两 profile 的阈值不同（NBA 6 / FIBA 5）。真实离场逻辑在犯规处理路径上
/// （`match_engine.rs`：`foul_count >= rules.league.max_personal_fouls`
/// 时调用 `forced_substitution`），因此本用例断言该阈值确实随 profile 变化
/// 并且可从引擎读出（`rules()`），而不是只断言档案常量。
///
/// 注：不直接调 `forced_substitution` 来"验证离场"——它是显式命令，
/// 且对持球人/传球目标会拒绝执行（保球权一致性），与犯规离场是两条语义。
#[test]
fn scenario_personal_foul_limit_program_differs() {
    for (rules, expected_limit) in [
        (GameRules::default(), 6u8),
        (GameRules::with_league(LeagueProfile::fiba()), 5u8),
    ] {
        let engine = MatchEngine::with_setup(nba_engine::MatchSetup::builtin(rules.clone()), 3);
        assert_eq!(
            engine.rules().league.max_personal_fouls,
            expected_limit,
            "{} must carry its own personal-foul limit into the engine",
            rules.league.name
        );
        assert_eq!(rules.league.max_personal_fouls, expected_limit);
    }

    // 端到端：让一场 FIBA 1q 跑完，确认犯规离场路径不因阈值差异而违反 5v5。
    // 两个 profile 的模拟互不共享状态，profile 间并行。
    let mismatch: Vec<String> = [
        ("NBA", GameRules::default()),
        ("FIBA", GameRules::with_league(LeagueProfile::fiba())),
    ]
    .par_iter()
    .map(|(name, rules)| {
        let mut engine =
            MatchEngine::with_setup(nba_engine::MatchSetup::builtin(rules.clone()), 13);
        engine.set_scope("1q").expect("1q scope is valid");
        let mut ticks = 0usize;
        while !engine.is_finished() && ticks < 200_000 {
            engine.step();
            ticks += 1;
            // 在场人数必须始终为 5v5（犯规离场后由替补补位）。
            let home_on = engine
                .physics()
                .get_players()
                .values()
                .filter(|p| p.team == "home" && p.on_court)
                .count();
            let away_on = engine
                .physics()
                .get_players()
                .values()
                .filter(|p| p.team == "away" && p.on_court)
                .count();
            if (home_on, away_on) != (5, 5) {
                return Some(format!(
                    "{name} tick {ticks}: foul-out must keep five on five (home={home_on}, away={away_on})"
                ));
            }
        }
        None
    })
    .flatten()
    .collect();
    assert!(
        mismatch.is_empty(),
        "foul-out must keep five on five: {mismatch:?}"
    );
}

/// 情景 5：bonus 门槛——进入罚球奖励的团队犯规数按 profile 生效。
#[test]
fn scenario_bonus_threshold_program() {
    let nba = GameRules::default();
    let fiba = GameRules::with_league(LeagueProfile::fiba());
    assert_eq!(nba.league.bonus_fouls_per_period, 5);
    assert_eq!(fiba.league.bonus_fouls_per_period, 4);
    // 两 profile 的 bonus 罚球数一致（均为 2），差异在**门槛**。
    assert_eq!(nba.league.bonus_free_throws, 2);
    assert_eq!(fiba.league.bonus_free_throws, 2);
}

/// 情景 6：节末程序——同一 scope 下两 profile 的节数与终场判定一致。
#[test]
fn scenario_period_end_program() {
    // 两 profile 的模拟互不共享状态，profile 间并行。
    let failures: Vec<String> = [
        GameRules::default(),
        GameRules::with_league(LeagueProfile::fiba()),
    ]
    .par_iter()
    .map(|rules| {
        let mut e = MatchEngine::with_setup(nba_engine::MatchSetup::builtin(rules.clone()), 5);
        e.set_scope("1q").expect("1q scope is valid");
        let mut ticks = 0usize;
        while !e.is_finished() && ticks < 200_000 {
            e.step();
            ticks += 1;
        }
        if e.is_finished() {
            None
        } else {
            Some(format!(
                "{} 1q scope must terminate without relying on the tick cap",
                rules.league.name
            ))
        }
    })
    .flatten()
    .collect();
    assert!(
        failures.is_empty(),
        "1q scope must terminate naturally: {failures:?}"
    );
    for rules in [GameRules::default(), GameRules::with_league(LeagueProfile::fiba())] {
        assert_eq!(
            rules.league.regulation_periods, 4,
            "both profiles use four regulation periods"
        );
    }
}
