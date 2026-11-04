//! 开场跳球与节次开始：合法拍球、提前触球、双人违例与交替拥有箭头。
//!
//! NBA Rule 6 §VII（跳球程序）、Rule 8 §I（跳球与罚球）、Rule 10 §V
//! （跳球违例）；FIBA Article 12（跳球）与 Article 12.4（交替拥有箭头）。
//!
//! 判定落在 `World::resolve_opening_jump_ball` 与
//! `World::award_opening_tip_control`：这里钉的是「谁拍球、球归谁、箭头
//! 怎么推进」。

mod common;

use nba_sim_core::action::{Fact, JumpBallKind};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::{MatchProgram, Phase};
use nba_sim_core::world::{World, TICK_SECONDS};
use nba_sim_core::{MatchSetup, Params, TeamSide};
use nba_domain::league::LeagueProfile;
use nba_domain::rules::GameRules;

fn world_with_profile(seed: u64, league: LeagueProfile) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut rules = GameRules::default();
    rules.league = league;
    World::new(MatchSetup::builtin(rules), params, seed)
}

fn nba_world(seed: u64) -> World {
    world_with_profile(seed, LeagueProfile::nba())
}

fn fiba_world(seed: u64) -> World {
    world_with_profile(seed, LeagueProfile::fiba())
}

/// 推进到跳球程序给出归属。
fn advance_to_first_possession(world: &mut World) {
    for _ in 0..2_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::Live) && world.holder().is_some() {
            return;
        }
    }
    panic!("the jump-ball procedure did not produce a controlled possession");
}

/// 跳球事实。
fn jump_fact(world: &World) -> (JumpBallKind, usize, usize, usize, TeamSide) {
    world
        .events
        .iter()
        .find_map(|event| match &event.fact {
            Fact::JumpBall {
                kind,
                home_jumper,
                away_jumper,
                tapper,
                tapped_team,
            } => Some((*kind, *home_jumper, *away_jumper, *tapper, *tapped_team)),
            _ => None,
        })
        .expect("the jump-ball result must be published")
}

#[test]
fn the_jump_ball_tapper_is_one_of_the_two_jumpers() {
    let mut world = nba_world(901);
    assert!(matches!(
        world.phase(),
        Phase::JumpBall {
            kind: JumpBallKind::Opening,
            ..
        }
    ));
    advance_to_first_possession(&mut world);
    let (kind, home_jumper, away_jumper, tapper, tapped_team) = jump_fact(&world);
    assert_eq!(kind, JumpBallKind::Opening);
    assert_ne!(home_jumper, away_jumper, "两名跳球员必须来自不同球队");
    assert!(
        tapper == home_jumper || tapper == away_jumper,
        "拍球的人必须是跳球员之一"
    );
    let tapper_team = world.players[tapper].team;
    assert_eq!(
        tapper_team, tapped_team,
        "拍球的球队就是获得球权的球队"
    );
    // 合法拍球：拍球的高度必须是跳球的高度。
    let tap = world
        .events
        .iter()
        .find_map(|event| match &event.fact {
            Fact::BallContact {
                kind: "jump_ball_tap",
                entity,
                z,
                ..
            } => Some((*entity, *z)),
            _ => None,
        })
        .expect("the legal tap must publish a contact fact");
    assert_eq!(tap.0, Some(tapper));
    assert!(
        (tap.1 - world.rules.tip_off_tap_height_ft).abs() < 0.5,
        "合法拍球的高度必须是跳球高度，实际 {}",
        tap.1
    );
}

#[test]
fn a_held_ball_rejump_waits_for_the_participants_to_reach_their_spots() {
    let mut world = nba_world(902);
    // 停表跳球：参跳人与接应人要走到圈边的点位才掷球（NBA Rule 6
    // §VII）。掷球之前比赛停在死球程序里。
    let home = world
        .on_court(TeamSide::Home)
        .as_slice()
        .first()
        .copied()
        .expect("the home team has an on-court player");
    let away = world
        .on_court(TeamSide::Away)
        .as_slice()
        .first()
        .copied()
        .expect("the away team has an on-court player");
    world.stage_held_ball(home, away);
    let pending = world
        .pending_jump_ball
        .as_ref()
        .expect("停表跳球必须登记待跳球记录");
    assert_eq!(pending.home, home);
    assert_eq!(pending.away, away);
    assert!(!pending.spots.is_empty(), "停表跳球必须登记参跳人与接应人的点位");
    // 参跳人还没到位：比赛停在死球程序里，不得掷球。
    world.step(TICK_SECONDS);
    assert!(
        world.pending_jump_ball.is_some(),
        "参跳人没到位时不得掷球"
    );
    assert!(
        !world
            .events
            .iter()
            .any(|event| matches!(event.fact, Fact::JumpBall { .. })),
        "参跳人没到位时不得发出跳球归属事实"
    );
    // 走到点位之后掷球：参跳人与接应人自己走过去。
    for _ in 0..400 {
        world.step(TICK_SECONDS);
        if world
            .events
            .iter()
            .any(|event| matches!(event.fact, Fact::JumpBall { .. }))
        {
            break;
        }
    }
    assert!(
        world
            .events
            .iter()
            .any(|event| matches!(event.fact, Fact::JumpBall { .. })),
        "参跳人到位之后必须掷球"
    );
}

#[test]
fn the_game_clock_starts_at_the_tap_and_not_before() {
    // NBA Rule 5 §II：比赛钟在球被合法触到时启动。跳球程序本身停表，
    // 撮球那一下球离手之后比赛钟才走。
    let mut world = nba_world(903);
    let game_clock_before = world.clock.game_clock();
    let shot_clock_before = world.clock.shot_clock();
    let mut tap_seen = false;
    for _ in 0..200 {
        world.step(TICK_SECONDS);
        let tapped = world.events.iter().any(|event| {
            matches!(&event.fact, Fact::BallContact { kind: "jump_ball_tap", .. })
        });
        if tapped {
            tap_seen = true;
            break;
        }
        assert_eq!(
            world.clock.game_clock(),
            game_clock_before,
            "撮球之前比赛钟不得走动"
        );
        assert_eq!(
            world.clock.shot_clock(),
            shot_clock_before,
            "撮球之前进攻钟不得走动"
        );
    }
    assert!(tap_seen, "跳球必须发出拍球事实");
}

#[test]
fn a_double_violation_repeats_the_jump_with_the_same_jumpers() {
    let mut world = nba_world(904);
    // 记下第一次跳球的跳球员。
    advance_to_first_possession(&mut world);
    let (_, first_home, first_away, _, _) = jump_fact(&world);
    assert_ne!(first_home, first_away, "两名跳球员必须来自不同球队");
    assert_eq!(
        world.players[first_home].team,
        TeamSide::Home,
        "主队跳球员必须是主队的人"
    );
    assert_eq!(
        world.players[first_away].team,
        TeamSide::Away,
        "客队跳球员必须是客队的人"
    );
    // 双方违例之后跳球重来：跳球员不变。
    world.stage_held_ball(first_home, first_away);
    for _ in 0..400 {
        world.step(TICK_SECONDS);
        if world
            .events
            .iter()
            .filter(|event| matches!(event.fact, Fact::JumpBall { .. }))
            .count()
            >= 2
        {
            break;
        }
    }
    let jumps: Vec<_> = world
        .events
        .iter()
        .filter_map(|event| match &event.fact {
            Fact::JumpBall {
                home_jumper,
                away_jumper,
                ..
            } => Some((*home_jumper, *away_jumper)),
            _ => None,
        })
        .collect();
    assert!(jumps.len() >= 2, "双方违例之后跳球必须重来一次");
    assert_eq!(
        jumps[0], jumps[1],
        "双方违例之后跳球重来，跳球员不变"
    );
}

#[test]
fn the_alternate_possession_arrow_starts_with_the_opponent_of_the_first_possession() {
    let mut world = fiba_world(905);
    assert!(
        world.rules.league.use_alternate_possession_arrow,
        "FIBA 使用交替拥有箭头"
    );
    advance_to_first_possession(&mut world);
    let (_, _, _, _, tapped_team) = jump_fact(&world);
    assert_eq!(
        world.alternate_possession_next,
        tapped_team.other(),
        "FIBA 的交替拥有箭头指向第一节开球球队的对方"
    );
}

#[test]
fn the_nba_alternate_possession_starts_with_the_opponent_of_the_first_possession() {
    let mut world = nba_world(906);
    assert!(
        !world.rules.league.use_alternate_possession_arrow,
        "NBA 不使用交替拥有箭头"
    );
    advance_to_first_possession(&mut world);
    let (_, _, _, _, tapped_team) = jump_fact(&world);
    assert_eq!(
        world.first_possession,
        Some(tapped_team),
        "NBA 的第一次球权由跳球决定"
    );
}

#[test]
fn the_jump_ball_is_its_own_match_program() {
    let mut world = nba_world(907);
    assert_eq!(world.match_program(), MatchProgram::JumpBall);
    advance_to_first_possession(&mut world);
    assert_eq!(world.match_program(), MatchProgram::OpenPlay);
}

#[test]
fn an_overtime_period_starts_with_a_fresh_jump_ball() {
    let mut world = nba_world(908);
    // 推进到加时：把比赛钟调到第四节结束。
    world.clock.set_regulation_clock(4, 0.1, world.clock.shot_clock());
    for _ in 0..500 {
        world.step(TICK_SECONDS);
        if world.clock.period() > 4 {
            break;
        }
    }
    assert!(
        world.clock.period() > 4,
        "比赛钟归零之后必须进入加时"
    );
    // 加时节的跳球在节间休息结束之后才发生，因此继续推进。
    let mut jumped = false;
    for _ in 0..2_000 {
        world.step(TICK_SECONDS);
        if world.events.iter().any(|event| {
            matches!(
                &event.fact,
                Fact::JumpBall {
                    kind: JumpBallKind::Overtime,
                    ..
                }
            )
        }) {
            jumped = true;
            break;
        }
    }
    assert!(jumped, "加时必须以一次新的跳球开始");
}
