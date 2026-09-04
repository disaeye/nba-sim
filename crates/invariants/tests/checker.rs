use nba_invariants::InvariantChecker;
use nba_protocol::{RenderBall, RenderFrame, RenderPlayer, RenderScore, StreamTick};

fn base_player(id: &str, team: &str, x: f32, y: f32, on_court: bool) -> RenderPlayer {
    RenderPlayer {
        id: id.to_string(),
        jersey: "0".to_string(),
        team: team.to_string(),
        x,
        y,
        zone: "Arc".to_string(),
        has_ball: false,
        on_court,
        action: "Idle".to_string(),
        slot: "G".to_string(),
        morale: "Normal".to_string(),
        stm: 100.0,
        stm_max: 100.0,
        foul_count: 0,
    }
}

fn valid_5v5_players() -> Vec<RenderPlayer> {
    vec![
        base_player("H1", "home", 0.2, 0.5, true),
        base_player("H2", "home", 0.3, 0.2, true),
        base_player("H3", "home", 0.3, 0.8, true),
        base_player("H4", "home", 0.4, 0.3, true),
        base_player("H5", "home", 0.4, 0.7, true),
        base_player("H6", "home", 0.2, -0.1, false), // 替补
        base_player("A1", "away", 0.8, 0.5, true),
        base_player("A2", "away", 0.7, 0.2, true),
        base_player("A3", "away", 0.7, 0.8, true),
        base_player("A4", "away", 0.6, 0.3, true),
        base_player("A5", "away", 0.6, 0.7, true),
        base_player("A6", "away", 0.8, 1.1, false), // 替补
    ]
}

fn base_tick(players: Vec<RenderPlayer>, ball: RenderBall) -> StreamTick {
    StreamTick {
        frame: RenderFrame {
            t: 0.0,
            t_game: 720.0,
            shot_clock: 24.0,
            period: 1,
            phase: "Initiation".to_string(),
            possession_id: 1,
            possession_team: "home".to_string(),
            home_team: Default::default(),
            away_team: Default::default(),
            score: RenderScore { home: 0, away: 0 },
            players,
            ball,
            event_type: None,
            events: vec![],
            game_flow: "LiveBall".to_string(),
            team_fouls_home: 0,
            team_fouls_away: 0,
            free_throws_remaining: 0,
            defensive_tactic: None,
            event_sequence: 0,
            event_log: vec![],
            simulation_complete: false,
            completed_possessions: 0,
            target_possessions: 10,
            callout: None,
            intensity: None,
            debug: None,
            rules: nba_protocol::FrameRules::default(),
        },
        tactical_set: "Test".to_string(),
        game_clock: 720.0,
        keyframe_index: None,
    }
}

fn ball(x: f32, y: f32, holder: Option<&str>) -> RenderBall {
    RenderBall {
        x,
        y,
        z: 5.0,
        status: "HELD".to_string(),
        holder_id: holder.map(|s| s.to_string()),
    }
}

#[test]
fn clean_tick_produces_no_violations() {
    let mut checker = InvariantChecker::new();
    let tick = base_tick(valid_5v5_players(), ball(0.2, 0.5, Some("H1")));
    assert!(checker.check_tick(&tick).is_empty());
}

#[test]
fn eight_players_on_court_flagged() {
    let mut checker = InvariantChecker::new();
    let mut players = valid_5v5_players();
    // 把主队的 3 名替补全部放进场内（变成 8 名在场）
    players[5].on_court = true;
    players.push(base_player("H7", "home", 0.3, 0.4, true));
    players.push(base_player("H8", "home", 0.3, 0.6, true));
    let tick = base_tick(players, ball(0.2, 0.5, Some("H1")));
    let v = checker.check_tick(&tick);
    assert!(v.iter().any(|x| x.rule == "TEAM_ON_COURT_COUNT"), "{:?}", v);
}

#[test]
fn four_players_on_court_flagged() {
    let mut checker = InvariantChecker::new();
    let mut players = valid_5v5_players();
    // 移除 1 名在场球员
    players[4].on_court = false;
    let tick = base_tick(players, ball(0.2, 0.5, Some("H1")));
    let v = checker.check_tick(&tick);
    assert!(v.iter().any(|x| x.rule == "TEAM_ON_COURT_COUNT"), "{:?}", v);
}

#[test]
fn two_holders_flagged() {
    let mut checker = InvariantChecker::new();
    let mut players = valid_5v5_players();
    players[0].has_ball = true;
    players[6].has_ball = true;
    let tick = base_tick(players, ball(0.2, 0.5, Some("H1")));
    let v = checker.check_tick(&tick);
    assert!(v.iter().any(|x| x.rule == "BALL_SINGLE_HOLDER"), "{:?}", v);
}

#[test]
fn holder_off_court_flagged() {
    let mut checker = InvariantChecker::new();
    let mut players = valid_5v5_players();
    players[0].on_court = false;
    // 替补球员持球
    players[5].on_court = false;
    let tick = base_tick(players, ball(0.2, 0.5, Some("H6")));
    let v = checker.check_tick(&tick);
    assert!(v.iter().any(|x| x.rule == "BALL_HOLDER_ON_COURT" || x.rule == "TEAM_ON_COURT_COUNT"), "{:?}", v);
}

#[test]
fn holder_missing_flagged() {
    let mut checker = InvariantChecker::new();
    let players = valid_5v5_players();
    let tick = base_tick(players, ball(0.2, 0.5, Some("GHOST")));
    let v = checker.check_tick(&tick);
    assert!(v.iter().any(|x| x.rule == "BALL_HOLDER_EXISTS"), "{:?}", v);
}

#[test]
fn ball_far_from_holder_flagged() {
    let mut checker = InvariantChecker::new();
    let mut players = valid_5v5_players();
    players[0].has_ball = true;
    // 球距持球人 20 英尺
    let tick = base_tick(players, ball(0.2 + 20.0 / 94.0, 0.5, Some("H1")));
    let v = checker.check_tick(&tick);
    assert!(v.iter().any(|x| x.rule == "BALL_WITH_HOLDER"), "{:?}", v);
}

#[test]
fn player_out_of_bounds_flagged() {
    let mut checker = InvariantChecker::new();
    let mut players = valid_5v5_players();
    players[0].x = 1.05; // 在场球员越界
    let tick = base_tick(players, ball(0.2, 0.5, None));
    let v = checker.check_tick(&tick);
    assert!(v.iter().any(|x| x.rule == "PLAYER_IN_BOUNDS"), "{:?}", v);
}

#[test]
fn score_regression_flagged() {
    let mut checker = InvariantChecker::new();
    let p = valid_5v5_players();
    let mut t1 = base_tick(p.clone(), ball(0.2, 0.5, None));
    t1.frame.score.home = 10;
    checker.check_tick(&t1);
    let mut t2 = base_tick(p, ball(0.2, 0.5, None));
    t2.frame.score.home = 9;
    let v = checker.check_tick(&t2);
    assert!(v.iter().any(|x| x.rule == "SCORE_MONOTONIC"), "{:?}", v);
}

#[test]
fn ball_height_out_of_bounds_flagged() {
    let mut checker = InvariantChecker::new();
    let p = valid_5v5_players();
    let mut t = base_tick(p, ball(0.2, 0.5, None));
    t.frame.ball.z = -1.0; // 球掉入地下
    let v = checker.check_tick(&t);
    assert!(v.iter().any(|x| x.rule == "BALL_HEIGHT_BOUNDS"), "{:?}", v);
}

#[test]
fn shot_clock_negative_flagged() {
    let mut checker = InvariantChecker::new();
    let p = valid_5v5_players();
    let mut t = base_tick(p, ball(0.2, 0.5, None));
    t.frame.shot_clock = -0.5; // 24 秒倒计时小于 0
    let v = checker.check_tick(&t);
    assert!(v.iter().any(|x| x.rule == "SHOT_CLOCK_BOUNDS"), "{:?}", v);
}

#[test]
fn uncaused_score_increase_flagged() {
    let mut checker = InvariantChecker::new();
    let p = valid_5v5_players();
    let mut t1 = base_tick(p.clone(), ball(0.2, 0.5, None));
    t1.frame.score.home = 20;
    checker.check_tick(&t1);

    // 下一帧比分自增 2 分，但没有任何进球或得分事件
    let mut t2 = base_tick(p, ball(0.2, 0.5, None));
    t2.frame.score.home = 22;
    t2.frame.events = vec![]; // 空事件流
    let v = checker.check_tick(&t2);
    assert!(v.iter().any(|x| x.rule == "UNCAUSED_SCORE_DELTA"), "{:?}", v);
}

#[test]
fn inbounder_out_of_bounds_is_exempt_during_inbound_phase() {
    // quality.md §1.1 阶段语义：发球阶段持球发球者允许站界外，不得误报。
    let mut checker = InvariantChecker::new();
    let mut tick = base_tick(valid_5v5_players(), ball(-0.03, 0.5, Some("H1")));
    tick.frame.phase = "INBOUND".to_string();
    tick.frame.game_flow = "DeadBall".to_string();
    // 把持球发球者移到界外（发球站位）。
    for p in tick.frame.players.iter_mut() {
        if p.id == "H1" {
            p.x = -0.03;
        }
    }
    assert!(
        checker.check_tick(&tick).is_empty(),
        "inbounding holder standing out of bounds must be exempt during INBOUND phase"
    );

    // 非持球者在同一阶段越界仍必须被抓。
    let mut checker2 = InvariantChecker::new();
    let mut tick2 = base_tick(valid_5v5_players(), ball(0.2, 0.5, Some("H1")));
    tick2.frame.phase = "INBOUND".to_string();
    for p in tick2.frame.players.iter_mut() {
        if p.id == "H2" {
            p.x = 1.05;
        }
    }
    let v = checker2.check_tick(&tick2);
    assert!(
        v.iter().any(|x| x.rule == "PLAYER_IN_BOUNDS"),
        "non-holder out-of-bounds must still be flagged: {:?}",
        v
    );
}

#[test]
fn violations_carry_severity() {
    // design.md §3.1 M1：每条违反必须携带 severity。
    let mut checker = InvariantChecker::new();
    let mut tick = base_tick(valid_5v5_players(), ball(0.2, 0.5, Some("H1")));
    for p in tick.frame.players.iter_mut() {
        if p.id == "H1" {
            p.x = 1.2; // 出界 → Hard
        }
    }
    let v = checker.check_tick(&tick);
    assert!(v.iter().any(|x| x.rule == "PLAYER_IN_BOUNDS" && matches!(
        x.severity,
        nba_invariants::ViolationSeverity::Hard
    )));
    // PLAYER_SEPARATION 是唯一 Soft 规则。
    let mut checker2 = InvariantChecker::new();
    let mut tick2 = base_tick(valid_5v5_players(), ball(0.2, 0.5, Some("H1")));
    for (i, p) in tick2.frame.players.iter_mut().enumerate() {
        if i >= 2 && p.team == "home" {
            p.x = 0.5 + (i as f32) * 0.001;
            p.y = 0.5;
        }
    }
    let v2 = checker2.check_tick(&tick2);
    assert!(v2.iter().any(|x| x.rule == "PLAYER_SEPARATION" && matches!(
        x.severity,
        nba_invariants::ViolationSeverity::Soft
    )), "separation violations must be Soft: {:?}", v2);
}
#[test]
fn test_metamorphic_continuity_axiom_flags_teleportation() {
    let mut causal = nba_invariants::CausalEventGraph::new();
    let mut tick1 = base_tick(valid_5v5_players(), ball(0.2, 0.5, Some("H1")));
    tick1.frame.ball.status = "HELD".to_string();
    let violations1 = causal.validate_tick(&tick1, 1);
    assert!(violations1.is_empty());

    // 模拟恶意瞬移（瞬移了 0.6 归一化球场坐标，换算超过 50 英尺）
    let mut tick2 = base_tick(valid_5v5_players(), ball(0.8, 0.5, Some("H1")));
    tick2.frame.ball.status = "HELD".to_string();
    let violations2 = causal.validate_tick(&tick2, 2);
    assert!(
        violations2.iter().any(|v| v.rule == "BALL_POSITION_DISCONTINUITY"),
        "Teleporting ball must violate BALL_POSITION_DISCONTINUITY axiom: {:?}",
        violations2
    );
}

#[test]
fn test_impulse_origin_axiom_flags_ghost_shot() {
    let mut causal = nba_invariants::CausalEventGraph::new();
    let mut tick1 = base_tick(valid_5v5_players(), ball(0.5, 0.5, None));
    tick1.frame.ball.status = "DEAD".to_string();
    causal.validate_tick(&tick1, 1);

    // 模拟中圈无源投篮发射（死球直接变成投篮且周围无任何球员）
    let mut tick2 = base_tick(valid_5v5_players(), ball(0.5, 0.5, None));
    tick2.frame.ball.status = "SHOT".to_string();
    // 让所有球员远离中圈
    for p in &mut tick2.frame.players {
        p.x = 0.1;
        p.y = 0.1;
    }
    let violations = causal.validate_tick(&tick2, 2);
    assert!(
        violations.iter().any(|v| v.rule == "BALL_IMPULSE_WITHOUT_SOURCE"),
        "Spontaneous flight without holder must violate BALL_IMPULSE_WITHOUT_SOURCE: {:?}",
        violations
    );
}

#[test]
fn test_score_causal_precondition_flags_telepathic_points() {
    let mut causal = nba_invariants::CausalEventGraph::new();
    let tick1 = base_tick(valid_5v5_players(), ball(0.2, 0.5, Some("H1")));
    causal.validate_tick(&tick1, 1);

    // 比分无源增加（没有任何前置投篮或罚球动作）
    let mut tick2 = base_tick(valid_5v5_players(), ball(0.2, 0.5, Some("H1")));
    tick2.frame.score.home = 2;
    let violations = causal.validate_tick(&tick2, 2);
    assert!(
        violations.iter().any(|v| v.rule == "UNCAUSED_SCORE_DELTA"),
        "Score modification without prior score event must be flagged: {:?}",
        violations
    );
}
