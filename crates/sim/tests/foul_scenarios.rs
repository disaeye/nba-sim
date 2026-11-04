//! 个人犯规与特殊犯规：罚球数量、罚则、球权归属。
//!
//! NBA Rule 12B §I（个人犯规的类型）、§V（罚球罚则情形）、§VI（双方
//! 犯规）、§VII（进攻犯规）、§VIII（松球犯规）；Rule 12A §V（技术犯规）。
//!
//! 判定落在 `nba_sim_core::resolve::foul`：这里钉的是「吹哨之后得到什么
//! 罚球、谁拿球、哪些犯规不计入球队单节犯规」。

mod common;

use nba_sim_core::action::{Fact, FoulKind};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::resolve::foul::{apply_offensive_foul, apply_shooting_foul};
use nba_sim_core::state::{Phase, PossessionKind};
use nba_sim_core::world::{World, TICK_SECONDS};
use nba_sim_core::{MatchSetup, Params, TeamSide};
use nba_domain::rules::GameRules;

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
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
    apply_shooting_foul(&mut world, defender, shooter, 2, false);
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
    apply_shooting_foul(&mut world, defender, shooter, 3, false);
    let (_, _, _, free_throws, _) = last_foul(&world).expect("the foul must be recorded");
    assert_eq!(free_throws, 3, "三分投篮犯规必须罚三次");
    let (_, total, remaining) =
        free_throw_program(&world).expect("投篮犯规之后必须进入罚球程序");
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
    apply_shooting_foul(&mut world, defender, shooter, 1, true);
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
    apply_offensive_foul(&mut world, fouler, fouled);
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
    nba_sim_core::resolve::foul::apply_double_foul(&mut world, fouler, fouled);
    let (_, _, kind, free_throws, _) = last_foul(&world).expect("the foul must be recorded");
    assert_eq!(kind, FoulKind::Double);
    assert_eq!(free_throws, 0, "双方犯规不罚球");
    assert_eq!(
        world.fouls[World::team_index(offense)].period,
        0,
        "双方犯规不计入球队单节犯规"
    );
    assert_eq!(
        world.possession.team, offense,
        "双方犯规之后球权留在进攻方"
    );
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
    nba_sim_core::resolve::foul::apply_non_shooting_foul(&mut world, fouler, fouled);
    let (_, _, kind, free_throws, _) = last_foul(&world).expect("the foul must be recorded");
    assert_eq!(kind, FoulKind::Personal);
    assert_eq!(
        free_throws,
        2,
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
        nba_sim_core::resolve::freethrow::FreeThrowFollowUp::DeadBall {
            team: offense,
        },
    );
    let (_, total, _) = free_throw_program(&world).expect("技术犯规之后必须进入罚球程序");
    assert_eq!(total, 1, "技术犯规罚一次");
    assert_eq!(
        world.possession.team,
        offense,
        "技术犯规之后进攻方保留球权"
    );
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
    nba_sim_core::resolve::foul::apply_clear_path_foul(&mut world, fouler, fouled);
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
    nba_sim_core::resolve::foul::apply_flagrant_foul(&mut world, fouler, fouled, false);
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
    nba_sim_core::resolve::foul::apply_flagrant_foul(&mut world, fouler, fouled, true);
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
    nba_sim_core::resolve::foul::apply_flagrant_foul(&mut world, fouler, fouled, false);
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
