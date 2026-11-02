//! 时间类违例：后场八秒。
//!
//! 规则读 `GameRules::backcourt_seconds`：控球方在后场停满秒数即违例，
//! 停表并把球权交给对方。

use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_sim_core::action::{Fact, TurnoverKind};
use nba_sim_core::params::{workspace_calibration_dir, Params};
use nba_sim_core::state::{Phase, PossessionKind};
use nba_sim_core::world::{World, TICK_SECONDS};
use nba_sim_core::{MatchSetup, TeamSide};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

#[test]
fn a_backcourt_possession_that_never_advances_is_an_eight_second_violation() {
    let mut world = world(307);
    let holder = world.on_court(TeamSide::Home).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == holder;
        player.vel = Vec2::ZERO;
    }
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(holder);
    // Home 向右进攻：x < 中线就是后场。
    world.players[holder].pos = Vec2::new(30.0, 25.0);
    world.players[holder].target = world.players[holder].pos;
    world.ball.pos = world.players[holder].pos;
    world.ball.z = world.rules.ball_radius_ft;
    // 持球人自己不动：不给决策层把它推进前场的机会。
    world.players[holder].schedule.next_redecide_at = world.now() + 1_000.0;

    let mut violation = None;
    for _ in 0..(f64::from(world.rules.backcourt_seconds) / TICK_SECONDS) as usize + 40 {
        let before = world.events.len();
        world.step(TICK_SECONDS);
        for event in &world.events[before..] {
            if matches!(
                &event.fact,
                Fact::Turnover {
                    kind: TurnoverKind::EightSecond,
                    ..
                }
            ) {
                violation = Some(world.now());
            }
        }
        if violation.is_some() {
            break;
        }
    }
    assert!(violation.is_some(), "后场停满八秒必须判违例");
    match world.phase() {
        Phase::DeadBall { team, .. } => assert_eq!(team, TeamSide::Away, "八秒违例后由对方发球"),
        other => panic!("八秒违例后必须停表换边，实际 {other:?}"),
    }
    assert_eq!(
        world.possession.id, 0,
        "违例之后旧回合必须结清"
    );
}

#[test]
fn a_loose_ball_over_the_sideline_hands_possession_to_the_opponent() {
    let mut world = world(317);
    let handler = world.on_court(TeamSide::Home).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == handler;
        player.vel = Vec2::ZERO;
    }
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.players[handler].pos = Vec2::new(10.0, 25.0);
    world.players[handler].target = world.players[handler].pos;
    // 进攻人把球碰出边线：最后触球是进攻方，按违例交出球权。
    world.last_touch = Some(handler);
    world.ball =
        nba_sim_core::ball::BallBody::loose(Vec2::new(2.0, 25.0), Vec2::new(-30.0, 0.0));

    let mut contact = false;
    for _ in 0..40 {
        let before = world.events.len();
        world.step(TICK_SECONDS);
        for event in &world.events[before..] {
            if matches!(
                &event.fact,
                Fact::BallContact {
                    kind: "out_of_bounds",
                    ..
                }
            ) {
                contact = true;
            }
        }
        if contact {
            break;
        }
    }
    assert!(contact, "球越过边线必须停表");
    assert!(
        world.events.iter().any(|event| matches!(
            &event.fact,
            Fact::Turnover {
                kind: TurnoverKind::OutOfBounds,
                ..
            }
        )),
        "进攻方碰出界必须记违例"
    );
    assert_eq!(world.possession.id, 0, "出界之后旧回合必须结清");
    match world.phase() {
        Phase::DeadBall { team, .. } => assert_eq!(team, TeamSide::Away, "出界后由对方发球"),
        other => panic!("出界后必须停表换边，实际 {other:?}"),
    }
    // 发球地点是出界点：等停表结束，发球人的目标点必须落在边线那一侧。
    for _ in 0..200 {
        if matches!(world.phase(), Phase::Inbound { .. }) {
            break;
        }
        world.step(TICK_SECONDS);
    }
    let passer = world.inbound_passer.expect("发球人必须已确定");
    let target = world.players[passer].target;
    assert!(
        target.x <= 4.0,
        "出界后的发球点必须在边线一侧：{target:?}"
    );
    assert!(
        (target.y - 25.0).abs() <= 3.0,
        "发球点必须留在出界处的纵向位置：{target:?}"
    );
}

#[test]
fn carrying_the_ball_back_over_half_court_is_a_backcourt_violation() {
    let mut world = world(319);
    let holder = world.on_court(TeamSide::Home).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == holder;
        player.vel = Vec2::ZERO;
    }
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(holder);
    world.players[holder].schedule.next_redecide_at = world.now() + 1_000.0;
    let mid = world.rules.court.width_ft * 0.5;
    // 先在前场建立控制。
    world.players[holder].pos = Vec2::new(mid + 12.0, 25.0);
    world.players[holder].target = world.players[holder].pos;
    world.ball.pos = world.players[holder].pos;
    world.ball.z = world.rules.ball_radius_ft;
    for _ in 0..10 {
        world.players[holder].pos = Vec2::new(mid + 12.0, 25.0);
        world.players[holder].vel = Vec2::ZERO;
        world.step(TICK_SECONDS);
    }
    assert!(
        world.possession.frontcourt_established,
        "过中线之后必须记下前场控制"
    );
    // 再把球带回后场。
    let before = world.events.len();
    world.players[holder].pos = Vec2::new(mid - 5.0, 25.0);
    world.players[holder].target = world.players[holder].pos;
    world.ball.pos = world.players[holder].pos;
    world.step(TICK_SECONDS);
    let violation = world.events[before..].iter().any(|event| {
        matches!(
            &event.fact,
            Fact::Turnover {
                kind: TurnoverKind::Backcourt,
                ..
            }
        )
    });
    assert!(violation, "把球带回后场必须判回场违例");
    assert_eq!(world.possession.id, 0, "回场之后旧回合必须结清");
    match world.phase() {
        Phase::DeadBall { team, .. } => assert_eq!(team, TeamSide::Away, "回场后由对方发球"),
        other => panic!("回场后必须停表换边，实际 {other:?}"),
    }
}

#[test]
fn a_defender_inside_the_lane_before_the_release_is_recorded() {
    let mut world = world(331);
    let shooter = world.on_court(TeamSide::Home).as_slice()[0];
    let defender = world.on_court(TeamSide::Away).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == shooter || index == defender;
        player.vel = Vec2::ZERO;
    }
    let hoop = world.hoop_for(TeamSide::Home);
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.ball.set_held(shooter);
    world.players[shooter].pos = hoop + Vec2::new(-18.0, 0.0);
    world.players[defender].pos = hoop + Vec2::new(-6.0, 0.0);
    world.free_throw_lane = Some(nba_sim_core::world::FreeThrowLane::default());
    world.set_phase(Phase::FreeThrows {
        shooter,
        total: 1,
        remaining: 1,
        made: 0,
        next_at: world.now() + 100.0,
        and_one: false,
        retain_possession: false,
        foul_id: 1,
        technical_retained_team: None,
        after_technical: None,
    });
    world.step(TICK_SECONDS);
    let lane = world.free_throw_lane.expect("罚球期间必须有禁区记录");
    assert!(lane.defense_early, "防守人站在禁区里必须被记为提前进区");
    assert!(!lane.offense_early, "进攻方没有进区");
}

#[test]
fn an_early_defensive_entry_awards_another_free_throw() {
    let mut world = world(337);
    let shooter = world.on_court(TeamSide::Home).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == shooter;
        player.vel = Vec2::ZERO;
    }
    let hoop = world.hoop_for(TeamSide::Home);
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.ball.set_held(shooter);
    world.players[shooter].pos = hoop + Vec2::new(-18.0, 0.0);
    world.free_throw_lane = Some(nba_sim_core::world::FreeThrowLane {
        offense_early: false,
        defense_early: true,
    });
    world.set_phase(Phase::FreeThrows {
        shooter,
        total: 1,
        remaining: 1,
        made: 0,
        next_at: world.now(),
        and_one: false,
        retain_possession: false,
        foul_id: 1,
        technical_retained_team: None,
        after_technical: None,
    });
    // 第一步发射罚球，第二步把结果定为碰筐不中并结算。
    world.step(TICK_SECONDS);
    if let Some(pending) = world.pending_shot.as_mut() {
        pending.result = Some(nba_sim_core::resolve::shot::ShotResult::Missed {
            rim_contact: true,
        });
    }
    world.step(TICK_SECONDS);
    assert!(
        world.events.iter().any(|event| matches!(
            &event.fact,
            Fact::FreeThrowLaneViolation { side: "defense" }
        )),
        "防守方提前进区必须被记录"
    );
    assert!(
        matches!(world.phase(), Phase::FreeThrows { remaining: 1, .. }),
        "防守方提前进区后必须重罚一次，实际 {:?}",
        world.phase()
    );
}

#[test]
fn an_early_offensive_entry_gives_the_ball_to_the_defence() {
    let mut world = world(341);
    let shooter = world.on_court(TeamSide::Home).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == shooter;
        player.vel = Vec2::ZERO;
    }
    let hoop = world.hoop_for(TeamSide::Home);
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.ball.set_held(shooter);
    world.players[shooter].pos = hoop + Vec2::new(-18.0, 0.0);
    world.free_throw_lane = Some(nba_sim_core::world::FreeThrowLane {
        offense_early: true,
        defense_early: false,
    });
    world.set_phase(Phase::FreeThrows {
        shooter,
        total: 1,
        remaining: 1,
        made: 0,
        next_at: world.now(),
        and_one: false,
        retain_possession: false,
        foul_id: 1,
        technical_retained_team: None,
        after_technical: None,
    });
    world.step(TICK_SECONDS);
    if let Some(pending) = world.pending_shot.as_mut() {
        pending.result = Some(nba_sim_core::resolve::shot::ShotResult::Missed {
            rim_contact: true,
        });
    }
    world.step(TICK_SECONDS);
    assert!(
        world.events.iter().any(|event| matches!(
            &event.fact,
            Fact::FreeThrowLaneViolation { side: "offense" }
        )),
        "进攻方提前进区必须被记录"
    );
    match world.phase() {
        Phase::DeadBall { team, .. } => assert_eq!(team, TeamSide::Away, "球权交给对方"),
        other => panic!("进攻方提前进区后必须停表换边，实际 {other:?}"),
    }
}

#[test]
fn a_frontcourt_foul_inbounds_from_the_nearest_sideline() {
    let mut world = world(347);
    let carrier = world.on_court(TeamSide::Home).as_slice()[0];
    let defender = world.on_court(TeamSide::Away).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == carrier || index == defender;
        player.vel = Vec2::ZERO;
    }
    let hoop = world.hoop_for(TeamSide::Home);
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(carrier);
    // 前场上方：中断点在中线以上、靠近上边线。
    let spot = hoop + Vec2::new(-14.0, 9.0);
    world.players[carrier].pos = spot;
    world.players[carrier].target = spot;
    world.ball.pos = spot;
    world.ball.z = world.rules.ball_radius_ft;
    world.players[defender].pos = spot + Vec2::new(1.5, 0.0);
    world.players[defender].target = world.players[defender].pos;
    // 未进入罚则的普通犯规：停表、保留球权、在中断点就近边线发球。
    world.fouls[1].period = 0;
    let ball_before = world.ball.pos;
    let radius = world.rules.player_radius_ft;
    let height = world.rules.court.height_ft;
    nba_sim_core::resolve::foul::apply_non_shooting_foul(&mut world, defender, carrier);
    let inbound = world.inbound_spot.expect("犯规后必须记录发球位置");
    assert!(
        (inbound.y - radius).abs() <= 0.5 || (inbound.y - (height - radius)).abs() <= 0.5,
        "犯规后的发球点必须落在边线上：{inbound:?}"
    );
    assert!(
        (inbound.x - ball_before.x).abs() <= 0.5,
        "发球点必须保留中断处的纵向位置：{inbound:?} 对 {ball_before:?}"
    );
}

#[test]
fn a_made_basket_inbounds_from_the_conceding_baseline() {
    let mut world = world(349);
    let shooter = world.on_court(TeamSide::Home).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == shooter;
        player.vel = Vec2::ZERO;
    }
    let hoop = world.hoop_for(TeamSide::Home);
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(shooter);
    world.players[shooter].pos = hoop + Vec2::new(-4.0, 0.0);
    world.players[shooter].target = world.players[shooter].pos;
    world.ball.pos = world.players[shooter].pos;
    world.ball.z = world.rules.ball_radius_ft;
    let intent = nba_sim_core::action::ActionIntent {
        actor: shooter,
        kind: nba_sim_core::action::ActionKind::Shoot(nba_sim_core::action::ShotKind::Layup),
        target: None,
        from_ft: world.players[shooter].pos,
        to_ft: world.players[shooter].pos,
        start_time: world.now(),
        duration: 0.4,
    };
    nba_sim_core::resolve::shot::resolve(
        &mut world,
        &intent,
        nba_sim_core::action::ShotKind::Layup,
    );
    if let Some(pending) = world.pending_shot.as_mut() {
        pending.result = Some(nba_sim_core::resolve::shot::ShotResult::Made);
    }
    // 命中→死球→对方底线发球。
    for _ in 0..200 {
        if matches!(world.phase(), Phase::Inbound { .. }) {
            break;
        }
        world.step(TICK_SECONDS);
    }
    assert!(
        matches!(world.phase(), Phase::Inbound { .. }),
        "命中后必须进入发球阶段，实际 {:?}",
        world.phase()
    );
    let passer = world.inbound_passer.expect("发球人必须已确定");
    let target = world.players[passer].target;
    let width = world.rules.court.width_ft;
    let height = world.rules.court.height_ft;
    assert!(
        target.x <= 5.0 || target.x >= width - 5.0,
        "命中后的发球点必须在底线上：{target:?}"
    );
    assert!(
        (target.y - height * 0.5).abs() <= 2.0,
        "底线发球点在中点附近：{target:?}"
    );
}

#[test]
fn a_late_timeout_may_inbound_in_the_frontcourt() {
    // 短节次：三步之内就能打到第四节。
    let mut rules = GameRules::default();
    rules.league.period_duration_seconds = 8.0;
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(rules), params, 353);
    let mut guard = 0;
    while guard < 200_000 {
        world.step(TICK_SECONDS);
        guard += 1;
        if world.clock.period() >= 4
            && matches!(world.phase(), Phase::Live)
            && world.holder().is_some()
        {
            break;
        }
    }
    assert!(
        world.clock.period() >= 4,
        "必须能推进到第四节，实际第 {} 节",
        world.clock.period()
    );
    // 进入末段窗口。
    world.clock.set_game_clock(60.0);
    let team = world.possession.team;
    assert!(
        nba_sim_core::timeouts::request(&mut world, team),
        "末段持球方必须可以请求暂停"
    );
    let spot = world.inbound_spot.expect("末段暂停必须记录前场发球点");
    let mid = world.rules.court.width_ft * 0.5;
    let attacking_right = world.attacking_right(team);
    let in_frontcourt = if attacking_right {
        spot.x > mid
    } else {
        spot.x < mid
    };
    assert!(in_frontcourt, "末段暂停的发球点必须在前场：{spot:?}");
}

#[test]
fn a_foul_before_the_horn_still_produces_free_throws() {
    let mut world = world(359);
    let shooter = world.on_court(TeamSide::Home).as_slice()[0];
    let fouler = world.on_court(TeamSide::Away).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == shooter || index == fouler;
        player.vel = Vec2::ZERO;
    }
    let hoop = world.hoop_for(TeamSide::Home);
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(shooter);
    world.players[shooter].pos = hoop + Vec2::new(-16.0, 0.0);
    world.players[shooter].target = world.players[shooter].pos;
    world.ball.pos = world.players[shooter].pos;
    world.ball.z = world.rules.ball_radius_ft;
    let intent = nba_sim_core::action::ActionIntent {
        actor: shooter,
        kind: nba_sim_core::action::ActionKind::Shoot(
            nba_sim_core::action::ShotKind::CatchShoot,
        ),
        target: None,
        from_ft: world.players[shooter].pos,
        to_ft: world.players[shooter].pos,
        start_time: world.now(),
        duration: 0.4,
    };
    nba_sim_core::resolve::shot::resolve(
        &mut world,
        &intent,
        nba_sim_core::action::ShotKind::CatchShoot,
    );
    // 球还在空中时比赛钟归零，此时防守人犯规。
    world.clock.set_game_clock(0.0);
    nba_sim_core::resolve::foul::apply_shooting_foul(&mut world, fouler, shooter, 2, false);
    assert!(
        matches!(world.phase(), Phase::FreeThrows { .. }),
        "投篮犯规必须先进入罚球程序，实际 {:?}",
        world.phase()
    );
    // 罚球程序走完之前不得结束本节。
    let mut saw_free_throw = false;
    for _ in 0..600 {
        if matches!(world.phase(), Phase::PeriodBreak { .. } | Phase::Finished) {
            break;
        }
        let before = world.events.len();
        world.step(TICK_SECONDS);
        for event in &world.events[before..] {
            if matches!(&event.fact, Fact::FreeThrow { .. }) {
                saw_free_throw = true;
            }
        }
    }
    assert!(saw_free_throw, "吹罚之后必须真的执行罚球");
    assert!(
        matches!(world.phase(), Phase::PeriodBreak { .. } | Phase::Finished),
        "罚球结束后本节必须收尾，实际 {:?}",
        world.phase()
    );
}

#[test]
fn the_period_ends_when_the_clock_hits_zero_during_a_dead_ball() {
    let mut world = world(367);
    let handler = world.on_court(TeamSide::Home).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == handler;
        player.vel = Vec2::ZERO;
    }
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.ball.set_held(handler);
    // 死球状态且比赛钟归零：本节的收尾不能被跳过。
    world.set_phase(Phase::DeadBall {
        team: TeamSide::Home,
        ready_at: world.now() + 3.0,
    });
    world.clock.set_game_clock(0.0);
    let before = world.events.len();
    for _ in 0..40 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::PeriodBreak { .. } | Phase::Finished) {
            break;
        }
    }
    assert!(
        world.events[before..]
            .iter()
            .any(|event| matches!(&event.fact, Fact::PeriodEnd { .. })),
        "死球且时钟归零必须发布节末事实"
    );
    assert!(
        matches!(world.phase(), Phase::PeriodBreak { .. } | Phase::Finished),
        "死球且时钟归零必须收尾本节，实际 {:?}",
        world.phase()
    );
}

#[test]
fn the_final_score_does_not_change_after_the_game_ends() {
    // 短节次：几步之内就能打到终场。
    let mut rules = GameRules::default();
    rules.league.period_duration_seconds = 2.0;
    rules.league.overtime_duration_seconds = 2.0;
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(rules), params, 373);
    let mut guard = 0;
    while !world.finished && guard < 200_000 {
        world.step(TICK_SECONDS);
        guard += 1;
    }
    assert!(world.finished, "比赛必须在限步内结束");
    let score = world.score;
    let before = world.events.len();
    for _ in 0..40 {
        world.step(TICK_SECONDS);
    }
    assert_eq!(world.score, score, "终场之后比分不得再变");
    assert!(
        !world.events[before..]
            .iter()
            .any(|event| matches!(&event.fact, Fact::FreeThrow { .. } | Fact::Shot { .. })),
        "终场之后不得再产生得分事件"
    );
}

#[test]
fn touching_a_descending_shot_inside_the_cylinder_is_goaltending() {
    let mut world = world(379);
    let shooter = world.on_court(TeamSide::Home).as_slice()[0];
    let defender = world.on_court(TeamSide::Away).as_slice()[1];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == shooter || index == defender;
        player.vel = Vec2::ZERO;
    }
    let hoop = world.hoop_for(TeamSide::Home);
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(shooter);
    let spot = hoop + Vec2::new(-14.0, 0.0);
    world.players[shooter].pos = spot;
    world.players[shooter].target = spot;
    world.ball.pos = spot;
    world.ball.z = world.rules.ball_radius_ft;
    let intent = nba_sim_core::action::ActionIntent {
        actor: shooter,
        kind: nba_sim_core::action::ActionKind::Shoot(
            nba_sim_core::action::ShotKind::CatchShoot,
        ),
        target: None,
        from_ft: spot,
        to_ft: spot,
        start_time: world.now(),
        duration: 0.4,
    };
    nba_sim_core::resolve::shot::resolve(
        &mut world,
        &intent,
        nba_sim_core::action::ShotKind::CatchShoot,
    );
    // 球在下落、已经进入篮筐上下的圆柱段，防守人在筐下伸手碰到它。
    let home = World::team_index(TeamSide::Home);
    let before = world.score[home];
    world.ball.pos = hoop;
    world.ball.z = world.rules.rim_height_ft - 0.2;
    world.ball.vz = -4.0;
    world.players[defender].pos = hoop + Vec2::new(0.4, 0.0);
    world.players[defender].target = world.players[defender].pos;
    let start = world.events.len();
    world.step(TICK_SECONDS);
    assert!(
        world.events[start..]
            .iter()
            .any(|event| matches!(event.fact, Fact::Goaltending { .. })),
        "干扰球必须被记录"
    );
    assert!(world.score[home] > before, "干扰球直接记中");
}

#[test]
fn an_ascending_ball_may_be_blocked_without_goaltending() {
    let mut world = world(383);
    let shooter = world.on_court(TeamSide::Home).as_slice()[0];
    let defender = world.on_court(TeamSide::Away).as_slice()[1];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == shooter || index == defender;
        player.vel = Vec2::ZERO;
    }
    let hoop = world.hoop_for(TeamSide::Home);
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball.set_held(shooter);
    let spot = hoop + Vec2::new(-14.0, 0.0);
    world.players[shooter].pos = spot;
    world.players[shooter].target = spot;
    world.ball.pos = spot;
    world.ball.z = world.rules.ball_radius_ft;
    let intent = nba_sim_core::action::ActionIntent {
        actor: shooter,
        kind: nba_sim_core::action::ActionKind::Shoot(
            nba_sim_core::action::ShotKind::CatchShoot,
        ),
        target: None,
        from_ft: spot,
        to_ft: spot,
        start_time: world.now(),
        duration: 0.4,
    };
    nba_sim_core::resolve::shot::resolve(
        &mut world,
        &intent,
        nba_sim_core::action::ShotKind::CatchShoot,
    );
    // 球还在上升、处在筐柱范围内：此时碰球是合法封盖，不是干扰球。
    let home = World::team_index(TeamSide::Home);
    let before = world.score[home];
    world.ball.pos = hoop;
    world.ball.z = world.rules.rim_height_ft + 0.4;
    world.ball.vz = 4.0;
    world.players[defender].pos = hoop + Vec2::new(0.4, 0.0);
    world.players[defender].target = world.players[defender].pos;
    let start = world.events.len();
    world.step(TICK_SECONDS);
    assert!(
        !world.events[start..]
            .iter()
            .any(|event| matches!(event.fact, Fact::Goaltending { .. })),
        "上升中的球不得判干扰球"
    );
    assert_eq!(world.score[home], before, "上升中的球不得直接记中");
}

#[test]
fn the_last_made_free_throw_sends_the_other_team_to_its_own_baseline() {
    let mut world = world(421);
    let shooter = world.on_court(TeamSide::Home).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == shooter;
        player.vel = Vec2::ZERO;
    }
    let hoop = world.hoop_for(TeamSide::Home);
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.ball.set_held(shooter);
    world.players[shooter].pos = hoop + Vec2::new(-18.0, 0.0);
    world.set_phase(Phase::FreeThrows {
        shooter,
        total: 1,
        remaining: 1,
        made: 0,
        next_at: world.now(),
        and_one: false,
        retain_possession: false,
        foul_id: 3,
        technical_retained_team: None,
        after_technical: None,
    });
    world.step(TICK_SECONDS);
    if let Some(pending) = world.pending_shot.as_mut() {
        pending.result = Some(nba_sim_core::resolve::shot::ShotResult::Made);
    }
    world.step(TICK_SECONDS);
    let team = match world.phase() {
        Phase::DeadBall { team, .. } | Phase::Inbound { team, .. } => team,
        other => panic!("最后一罚命中之后必须发球，实际 {other:?}"),
    };
    assert_eq!(team, TeamSide::Away, "最后一罚命中由对方从本方底线发球");
    assert!(
        world.inbound_spot.is_none(),
        "底线发球不设本场地点，实际 {:?}",
        world.inbound_spot
    );
}

#[test]
fn a_ball_leaving_behind_the_baseline_inbounds_from_that_baseline() {
    let mut world = world(433);
    let handler = world.on_court(TeamSide::Home).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == handler;
        player.vel = Vec2::ZERO;
    }
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.players[handler].pos = Vec2::new(10.0, 25.0);
    world.players[handler].target = world.players[handler].pos;
    world.last_touch = Some(handler);
    // 球从底线出界：发球点必须是这条底线上的点。
    world.ball = nba_sim_core::ball::BallBody::loose(Vec2::new(1.0, 25.0), Vec2::new(-30.0, 0.0));
    let mut contact = false;
    for _ in 0..40 {
        let before = world.events.len();
        world.step(TICK_SECONDS);
        if world.events[before..].iter().any(|event| {
            matches!(&event.fact, Fact::BallContact { kind: "out_of_bounds", .. })
        }) {
            contact = true;
            break;
        }
    }
    assert!(contact, "底线出界必须发布出界接触");
    let spot = world.inbound_spot.expect("出界必须记录发球地点");
    let radius = world.rules.player_radius_ft;
    assert!(
        spot.x <= radius + 1.0 || spot.x >= world.rules.court.width_ft - radius - 1.0,
        "底线出界的发球点必须在底线附近，实际 {spot:?}"
    );
}
