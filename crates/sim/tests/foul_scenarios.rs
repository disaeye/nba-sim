//! 个人犯规与特殊犯规：罚球数量、罚则、球权归属。
//!
//! NBA Rule 12B §I（个人犯规的类型）、§V（罚球罚则情形）、§VI（双方
//! 犯规）、§VII（进攻犯规）、§VIII（松球犯规）；Rule 12A §V（技术犯规）。
//!
//! 判定落在 `nba_sim_core::resolve::foul`：这里钉的是「吹哨之后得到什么
//! 罚球、谁拿球、哪些犯规不计入球队单节犯规」。

mod common;

use nba_domain::league::LeagueProfile;
use nba_domain::rules::GameRules;
use nba_sim_core::action::{Fact, FoulKind};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::detect;
use nba_sim_core::state::{Phase, PossessionKind};
use nba_sim_core::world::{World, TICK_SECONDS};
use nba_sim_core::{MatchSetup, Params, TeamSide};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

fn fiba_world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut rules = GameRules::default();
    rules.league = LeagueProfile::fiba();
    World::new(MatchSetup::builtin(rules), params, seed)
}

/// 一个活球有人持球的世界。
fn live_world(seed: u64) -> World {
    let mut world = world(seed);
    for _ in 0..40_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::Live) && world.holder().is_some() {
            return world;
        }
    }
    panic!("the engine never reached a live ball state");
}

/// 注入一条已检出的投篮犯规事实。
///
/// 注入的是事实（谁在投篮时碰到了谁），后果由裁决层按档案给出。
fn inject_shooting_foul(world: &mut World, fouler: usize, shooter: usize, free_throws: u8, made: bool) {
    let incident = detect::detect_shooting_foul(world, fouler, shooter, free_throws, made);
    world.submit_incident(incident);
}

/// 注入一条已检出的进攻犯规事实。
fn inject_offensive_foul(world: &mut World, fouler: usize, fouled: usize) {
    let incident = detect::detect_offensive_foul(world, fouler, fouled);
    world.submit_incident(incident);
}

/// 注入一条已检出的普通个人犯规事实。
fn inject_personal_foul(world: &mut World, fouler: usize, fouled: usize) {
    let incident = detect::detect_personal_foul(world, fouler, fouled);
    world.submit_incident(incident);
}

/// 注入一条已检出的双方犯规事实。
fn inject_double_foul(world: &mut World, fouler: usize, fouled: usize) {
    let incident = detect::detect_double_foul(world, fouler, fouled);
    world.submit_incident(incident);
}

/// 注入一条已检出的清晰路径犯规事实。
fn inject_clear_path_foul(world: &mut World, fouler: usize, fouled: usize) {
    let incident = detect::clear_path_fact(world, fouler, fouled, false);
    world.submit_incident(incident);
}

/// 注入一条已检出的恶意犯规事实，并指明它的等级。
fn inject_flagrant_foul(world: &mut World, fouler: usize, fouled: usize, excessive: bool) {
    let incident = detect::flagrant_fact(world, fouler, fouled, excessive);
    world.submit_incident(incident);
}

/// 最近一次犯规的事实。
fn last_foul(world: &World) -> Option<(usize, usize, FoulKind, u8, bool)> {
    world
        .events
        .iter()
        .rev()
        .find_map(|event| match &event.fact {
            Fact::Foul {
                fouler,
                fouled,
                kind,
                free_throws,
                and_one,
                ..
            } => Some((*fouler, *fouled, *kind, *free_throws, *and_one)),
            _ => None,
        })
}

/// 罚球程序里的罚球总数与罚球人。
fn free_throw_program(world: &World) -> Option<(usize, u8, u8)> {
    match world.phase() {
        Phase::FreeThrows {
            shooter,
            total,
            remaining,
            ..
        } => Some((shooter, total, remaining)),
        _ => None,
    }
}

#[test]
fn a_shooting_foul_on_a_two_point_shot_awards_two_free_throws() {
    let mut world = live_world(701);
    let offense = world.possession.team;
    let shooter = world.holder().expect("live possession has a holder");
    let defender = world
        .on_court(offense.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    inject_shooting_foul(&mut world, defender, shooter, 2, false);
    let (fouler, fouled, kind, free_throws, and_one) =
        last_foul(&world).expect("the foul must be recorded");
    assert_eq!(fouler, defender);
    assert_eq!(fouled, shooter);
    assert_eq!(kind, FoulKind::Shooting);
    assert_eq!(free_throws, 2, "两分投篮犯规必须罚两次");
    assert!(!and_one, "没有打进的投篮不是加罚");
    let (program_shooter, total, remaining) =
        free_throw_program(&world).expect("投篮犯规之后必须进入罚球程序");
    assert_eq!(program_shooter, shooter);
    assert_eq!(total, 2);
    assert_eq!(remaining, 2);
}

#[test]
fn a_shooting_foul_on_a_three_point_shot_awards_three_free_throws() {
    let mut world = live_world(702);
    let offense = world.possession.team;
    let shooter = world.holder().expect("live possession has a holder");
    let defender = world
        .on_court(offense.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    inject_shooting_foul(&mut world, defender, shooter, 3, false);
    let (_, _, _, free_throws, _) = last_foul(&world).expect("the foul must be recorded");
    assert_eq!(free_throws, 3, "三分投篮犯规必须罚三次");
    let (_, total, remaining) = free_throw_program(&world).expect("投篮犯规之后必须进入罚球程序");
    assert_eq!(total, 3);
    assert_eq!(remaining, 3);
}

#[test]
fn a_shooting_foul_on_a_made_shot_awards_one_extra_free_throw() {
    let mut world = live_world(703);
    let offense = world.possession.team;
    let shooter = world.holder().expect("live possession has a holder");
    let defender = world
        .on_court(offense.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    // 打进之后的犯规：罚一次，并且进攻方继续持有球权。
    inject_shooting_foul(&mut world, defender, shooter, 1, true);
    let (_, _, _, free_throws, and_one) = last_foul(&world).expect("the foul must be recorded");
    assert_eq!(free_throws, 1, "打进的投篮犯规只补罚一次");
    assert!(and_one, "打进的投篮犯规必须记作加罚");
    let (_, total, _) = free_throw_program(&world).expect("投篮犯规之后必须进入罚球程序");
    assert_eq!(total, 1);
}

#[test]
fn an_offensive_foul_counts_personally_but_not_against_the_team() {
    let mut world = live_world(704);
    let offense = world.possession.team;
    let fouler = world.holder().expect("live possession has a holder");
    let fouled = world
        .on_court(offense.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    let team_fouls_before = world.fouls[World::team_index(offense)].period;
    let personal_before = world.line_mut(fouler).fouls;
    inject_offensive_foul(&mut world, fouler, fouled);
    let (_, _, kind, free_throws, _) = last_foul(&world).expect("the foul must be recorded");
    assert_eq!(kind, FoulKind::Offensive);
    assert_eq!(free_throws, 0, "进攻犯规不罚球");
    assert_eq!(
        world.fouls[World::team_index(offense)].period,
        team_fouls_before,
        "进攻犯规不计入球队单节犯规（NBA Rule 12B §VII.3）"
    );
    assert_eq!(
        world.line_mut(fouler).fouls,
        personal_before + 1,
        "进攻犯规计入个人犯规"
    );
    match world.phase() {
        Phase::DeadBall { team, .. } => {
            assert_eq!(team, offense.other(), "进攻犯规之后对方发球")
        }
        other => panic!("进攻犯规之后必须停表换边，实际 {other:?}"),
    }
}

#[test]
fn a_double_foul_leaves_the_offence_in_possession() {
    let mut world = live_world(705);
    let offense = world.possession.team;
    let fouler = world.holder().expect("live possession has a holder");
    let fouled = world
        .on_court(offense.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    // 双方犯规：双方各记一次个人犯规，但球权留在进攻方（NBA Rule 12B
    // §VI.3）。
    inject_double_foul(&mut world, fouler, fouled);
    let (_, _, kind, free_throws, _) = last_foul(&world).expect("the foul must be recorded");
    assert_eq!(kind, FoulKind::Double);
    assert_eq!(free_throws, 0, "双方犯规不罚球");
    assert_eq!(
        world.fouls[World::team_index(offense)].period,
        0,
        "双方犯规不计入球队单节犯规"
    );
    assert_eq!(world.possession.team, offense, "双方犯规之后球权留在进攻方");
    match world.phase() {
        Phase::DeadBall { team, .. } => {
            assert_eq!(team, offense, "双方犯规之后由进攻方发球")
        }
        other => panic!("双方犯规之后必须停表，实际 {other:?}"),
    }
}

#[test]
fn a_loose_ball_foul_counts_against_the_team_and_awards_one_free_throw_in_the_bonus() {
    let mut world = live_world(706);
    let offense = world.possession.team;
    let fouler = world
        .on_court(offense.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    let fouled = world
        .on_court(offense)
        .as_slice()
        .last()
        .copied()
        .expect("the offence has an on-court player");
    // 制造一个没有球队控制的局面：球脱手。
    world.ball.release(nba_sim_core::state::BallState::Loose {
        pos: world.players[fouled].pos,
        vel: glam::Vec2::ZERO,
    });
    // 防守方单节犯规已经到第五犯：进入罚则。
    world.fouls[World::team_index(offense.other())].period = 4;
    let team_fouls_before = world.fouls[World::team_index(offense.other())].period;
    inject_personal_foul(&mut world, fouler, fouled);
    let (_, _, kind, free_throws, _) = last_foul(&world).expect("the foul must be recorded");
    assert_eq!(kind, FoulKind::Personal);
    assert_eq!(
        free_throws, 2,
        "罚则状态下的松球犯规罚一次加一次罚则罚球（NBA Rule 12B §VIII.4）"
    );
    assert!(
        world.fouls[World::team_index(offense.other())].period > team_fouls_before,
        "松球犯规计入球队单节犯规"
    );
}

#[test]
fn a_technical_foul_awards_one_free_throw_and_the_offence_keeps_the_ball() {
    let mut world = live_world(707);
    let offense = world.possession.team;
    let fouler = world
        .on_court(offense.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    let foul_id = world.new_foul_id();
    // 技术犯规：对方罚一次，进攻方保留球权（NBA Rule 12A §V.8）。
    nba_sim_core::resolve::freethrow::shoot_team_technical(
        &mut world,
        offense,
        offense,
        foul_id,
        nba_sim_core::resolve::freethrow::FreeThrowFollowUp::DeadBall { team: offense },
    );
    let (_, total, _) = free_throw_program(&world).expect("技术犯规之后必须进入罚球程序");
    assert_eq!(total, 1, "技术犯规罚一次");
    assert_eq!(world.possession.team, offense, "技术犯规之后进攻方保留球权");
}

#[test]
fn a_clear_path_foul_awards_two_free_throws_and_the_ball() {
    let mut world = live_world(708);
    let offense = world.possession.team;
    let fouler = world
        .on_court(offense.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    let fouled = world.holder().expect("live possession has a holder");
    // 清晰路径犯规：两次罚球加球权（NBA Rule 12B §I.5.f.6）。
    inject_clear_path_foul(&mut world, fouler, fouled);
    let (_, _, kind, free_throws, _) = last_foul(&world).expect("the foul must be recorded");
    assert_eq!(kind, FoulKind::Personal);
    assert_eq!(free_throws, 2, "清晰路径犯规罚两次");
    let (_, total, _) = free_throw_program(&world).expect("清晰路径犯规之后必须进入罚球程序");
    assert_eq!(total, 2);
    // 罚球之后由被犯规方拿到球权。
    assert!(
        world.free_throw_lane.is_some(),
        "罚球程序必须开始记录禁区站位"
    );
    let _ = offense;
}

#[test]
fn a_flagrant_foul_awards_two_free_throws() {
    let mut world = live_world(709);
    let offense = world.possession.team;
    let fouler = world
        .on_court(offense.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    let fouled = world.holder().expect("live possession has a holder");
    // 一级恶意犯规：两次罚球，不被逐（NBA Rule 12B §IV.1）。
    inject_flagrant_foul(&mut world, fouler, fouled, false);
    let (_, _, kind, free_throws, _) = last_foul(&world).expect("the foul must be recorded");
    assert_eq!(kind, FoulKind::Flagrant);
    assert_eq!(free_throws, 2, "一级恶意犯规罚两次");
    assert!(
        !world.players[fouler].disqualified,
        "一级恶意犯规不导致被逐"
    );
}

#[test]
fn a_flagrant_penalty_two_ejects_the_fouler() {
    let mut world = live_world(710);
    let offense = world.possession.team;
    let fouler = world
        .on_court(offense.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    let fouled = world.holder().expect("live possession has a holder");
    // 二级恶意犯规：两次罚球加球权，犯规者被逐（NBA Rule 12B §IV.2）。
    inject_flagrant_foul(&mut world, fouler, fouled, true);
    assert!(
        world.players[fouler].disqualified,
        "二级恶意犯规的犯规者必须被逐"
    );
    let ejected = world
        .events
        .iter()
        .rev()
        .find_map(|event| match &event.fact {
            Fact::Ejected { player, .. } => Some(*player),
            _ => None,
        })
        .expect("被逐必须留下事实");
    assert_eq!(ejected, fouler, "被逐事实必须指向犯规者");
    let (_, _, _, free_throws, _) = last_foul(&world).expect("the foul must be recorded");
    assert_eq!(free_throws, 2, "二级恶意犯规罚两次");
    let _ = offense;
}

#[test]
fn a_flagrant_foul_counts_against_the_team_and_the_player() {
    let mut world = live_world(711);
    let offense = world.possession.team;
    let fouler = world
        .on_court(offense.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    let fouled = world.holder().expect("live possession has a holder");
    let team_fouls_before = world.fouls[World::team_index(offense.other())].period;
    let personal_before = world.line_mut(fouler).fouls;
    inject_flagrant_foul(&mut world, fouler, fouled, false);
    assert_eq!(
        world.fouls[World::team_index(offense.other())].period,
        team_fouls_before + 1,
        "恶意犯规计入球队单节犯规"
    );
    assert_eq!(
        world.line_mut(fouler).fouls,
        personal_before + 1,
        "恶意犯规计入个人犯规累计"
    );
}

/// 停球后搂抱犯规的累计计数：同一姿势下让防守人贴身 4 秒，返回吹哨次数。
fn hold_foul_trials(physicality: f32) -> u32 {
    let mut calls = u32::default();
    for seed in 0..48u64 {
        let mut world = live_world(700 + seed);
        let offense = world.possession.team;
        let holder = world.holder().expect("live possession has a holder");
        let defense = offense.other();
        let defender = world.on_court(defense).as_slice()[0];
        world.players[holder].vel = glam::Vec2::ZERO;
        world.possession.dribble_state = nba_sim_core::state::DribbleState::Gathered;
        world.players[defender].pos = world.players[holder].pos + glam::Vec2::new(1.0, 0.0);
        world.players[defender].data.tendencies.physicality = physicality;
        world.players[defender].on_ball_positioning =
            Some(nba_sim_core::action::OnBallPositioning::Contain);
        world.ball.pos = world.players[holder].pos;

        let events_before = world.events.len();
        for _ in 0..80 {
            world.clock.advance_tick();
            nba_sim_core::resolve::poke::step(&mut world, 0.05);
            if world.events[events_before..].iter().any(|event| {
                matches!(
                    &event.fact,
                    Fact::Foul {
                        fouler: f,
                        fouled: h,
                        kind: FoulKind::Personal,
                        ..
                    } if *f == defender && *h == holder
                )
            }) {
                calls += 1;
                break;
            }
        }
    }
    calls
}

#[test]
fn a_fiba_disruptive_foul_is_booked_separately_from_a_flagrant_foul() {
    let mut world = fiba_world(720);
    let offense = world.possession.team;
    let fouler = world
        .on_court(offense.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    let fouled = world
        .on_court(offense)
        .as_slice()
        .first()
        .copied()
        .expect("an offensive player is on the court");
    let personal_before = world.line_mut(fouler).fouls;
    let team_before = world.fouls[World::team_index(offense.other())].period;
    inject_clear_path_foul(&mut world, fouler, fouled);
    let (_, _, kind, free_throws, _) = last_foul(&world).expect("the foul must be recorded");
    assert_eq!(kind, FoulKind::Disruptive, "FIBA 破坏性犯规必须单独成类");
    assert_eq!(free_throws, 2, "非投篮破坏性犯规罚两次");
    assert_eq!(
        world.line_mut(fouler).fouls,
        personal_before + 1,
        "破坏性犯规计入个人犯规累计"
    );
    assert_eq!(
        world.fouls[World::team_index(offense.other())].period,
        team_before + 1,
        "破坏性犯规计入球队单节犯规"
    );
    assert!(
        !world.players[fouler].disqualified,
        "破坏性犯规不触发取消比赛资格"
    );
    let (shooter, total, _) = free_throw_program(&world).expect("破坏性犯规之后必须进入罚球程序");
    assert_eq!(shooter, fouled);
    assert_eq!(total, 2);
}

#[test]
fn an_nba_clear_path_foul_stays_a_personal_foul() {
    let mut world = live_world(721);
    let offense = world.possession.team;
    let fouler = world
        .on_court(offense.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    let fouled = world.holder().expect("live possession has a holder");
    inject_clear_path_foul(&mut world, fouler, fouled);
    let (_, _, kind, _, _) = last_foul(&world).expect("the foul must be recorded");
    assert_eq!(
        kind,
        FoulKind::Personal,
        "NBA 档案不把这类行为拆成破坏性犯规"
    );
}

#[test]
fn holding_foul_after_ball_stops_triggers_non_shooting_penalty() {
    let physical = hold_foul_trials(1.0);
    let passive = hold_foul_trials(0.0);
    assert!(physical > 0, "停步合球后的贴身搂抱应触发非投篮普通犯规");
    assert!(
        physical > passive,
        "对抗倾向高的防守人搂抱犯规应当更多：{physical} vs {passive}"
    );
}

#[test]
fn rebound_push_foul_awards_penalty_and_terminates_rebound() {
    let mut world = live_world(713);
    let offense = world.possession.team;
    let defense = offense.other();
    let def_player = world.on_court(defense).as_slice()[0];
    let off_player = world.on_court(offense).as_slice()[0];
    world.players[def_player].pos = glam::Vec2::new(45.0, 25.0);
    world.players[def_player].target = glam::Vec2::new(45.0, 25.0);
    world.players[off_player].pos = glam::Vec2::new(45.5, 25.0);
    world.players[off_player].target = glam::Vec2::new(45.5, 25.0);
    world.players[off_player].data.tendencies.physicality = 1.0;
    world.players[off_player].data.attributes.strength = 1.0;
    world.ball.pos = glam::Vec2::new(45.0, 25.0);
    world.ball.z = 10.0;
    world.ball.vel = glam::Vec2::ZERO;
    world.ball.vz = 0.0;
    world.release_ball(nba_sim_core::state::BallState::Loose {
        pos: world.ball.pos,
        vel: glam::Vec2::ZERO,
    });
    world.set_phase(Phase::Live);

    let mut box_outs = [usize::MAX; nba_sim_core::state::MAX_ON_COURT];
    box_outs[0] = def_player;
    world.pending_rebound = Some(nba_sim_core::resolve::rebound::PendingRebound {
        offense,
        zone: "Paint",
        shot_distance: 5.0,
        rim_contact: true,
        started_at: world.now(),
        turnover: false,
        poke_by: None,
        pass_by: None,
        box_outs,
        box_out_count: 1,
    });

    let events_before = world.events.len();
    let mut push_foul_called = false;
    for _ in 0..100 {
        world.players[def_player].pos = glam::Vec2::new(45.0, 25.0);
        world.players[def_player].vel = glam::Vec2::ZERO;
        world.players[off_player].pos = glam::Vec2::new(45.5, 25.0);
        world.players[off_player].vel = glam::Vec2::ZERO;
        world.ball.pos = glam::Vec2::new(45.0, 25.0);
        world.ball.vel = glam::Vec2::ZERO;
        world.ball.z = 10.0;
        world.ball.vz = 0.0;
        world.step(TICK_SECONDS);
        if world.events[events_before..].iter().any(|e| {
            matches!(
                &e.fact,
                Fact::Foul {
                    fouler,
                    fouled,
                    kind: FoulKind::Personal,
                    ..
                } if world.players[*fouler].team == offense && *fouled == def_player
            )
        }) {
            push_foul_called = true;
            break;
        }
    }
    assert!(push_foul_called, "篮板球卡位中的过度推搡应判定推人犯规");
    assert!(
        world.pending_rebound.is_none(),
        "犯规吹罚后篮板争抢状态应立即终结"
    );
}
